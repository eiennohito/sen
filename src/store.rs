//! Transactional, worker-owned SQLite source of truth. No client or UI dependencies.
//! Withdrawal tombstones survive missing messages; server IDs are canonical decimals.
mod messages;
mod queries;
#[cfg(test)]
mod tests;
mod types;
use rusqlite::{Connection, OptionalExtension, params};
use std::cmp::max;
use std::path::Path;
use types::Result;
pub use types::*;

const SCHEMA_VERSION: i64 = 13;

/// Sole-writer account store. Every mutation commits atomically before returning deltas.
pub struct Store {
    conn: Connection,
    writes: u64,
    changes: Changes,
}
impl Store {
    pub fn open_account(root: &Path, mid: &str) -> Result<Self> {
        let directory = crate::locations::account_directory(root, mid)?;
        if let Err(error) = Self::copy_first_start(root, mid, &directory) {
            crate::reporting::log("store-copy", "first-start", &error);
            return Self::open_in_memory();
        }
        Self::open(&directory)
    }

    // remove after the first release
    pub(crate) fn copy_first_start(root: &Path, mid: &str, directory: &Path) -> Result<()> {
        use sha2::{Digest, Sha256};
        if crate::locations::account_directory(&root.join("auth/forgotten"), mid)?.try_exists()? {
            return Ok(());
        }
        let destination = directory.join("store.sqlite3");
        let old = root
            .parent()
            .unwrap()
            .join("line-desktop/accounts")
            .join(format!("{:x}", Sha256::digest(mid)))
            .join("store.sqlite3");
        if !destination.try_exists()? && old.try_exists()? {
            crate::client::private_directory(directory)
                .map_err(|error| StoreError::Io(std::io::Error::other(error)))?;
            let connection =
                Connection::open_with_flags(old, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            connection.busy_timeout(std::time::Duration::from_secs(2))?;
            let temporary = tempfile::NamedTempFile::new_in(directory)?;
            connection.execute(
                "VACUUM INTO ?",
                [temporary.path().to_string_lossy().as_ref()],
            )?;
            Self::publish_copy(temporary, &destination, std::fs::File::sync_all)?;
        }
        Ok(())
    }

    // Verify before publishing; a crash must never leave a partial winning copy.
    fn publish_copy(
        temporary: tempfile::NamedTempFile,
        destination: &Path,
        sync: impl Fn(&std::fs::File) -> std::io::Result<()>,
    ) -> Result<()> {
        let copy = Connection::open_with_flags(
            temporary.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let check: String = copy.pragma_query_value(None, "quick_check", |r| r.get(0))?;
        if check != "ok" {
            return Err(StoreError::Invalid(
                "copied store failed quick_check".into(),
            ));
        }
        let version: i64 = copy.pragma_query_value(None, "user_version", |r| r.get(0))?;
        drop(copy);
        let directory = std::fs::File::open(destination.parent().unwrap())?;
        sync(temporary.as_file())?;
        temporary
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
        if let Err(error) = sync(&directory) {
            std::fs::remove_file(destination)?;
            directory.sync_all()?;
            return Err(error.into());
        }
        if version != SCHEMA_VERSION {
            eprintln!("Old history could not be carried over (schema mismatch); old file kept.");
        }
        Ok(())
    }

    pub fn open(dir: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let path = dir.join("store.sqlite3");
        let conn = Connection::open(&path)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let conn = if version != SCHEMA_VERSION {
            drop(conn);
            for path in [
                path.clone(),
                dir.join("store.sqlite3-wal"),
                dir.join("store.sqlite3-shm"),
            ] {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let mut conn = Connection::open(path)?;
            Self::create_schema(&mut conn)?;
            conn
        } else {
            conn
        };
        Self::initialize(conn)
    }
    /// Persist work composed before the first account identity was available.
    pub(crate) fn persist_to(&self, dir: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        self.conn.execute(
            "VACUUM INTO ?",
            [dir.join("store.sqlite3").to_string_lossy().as_ref()],
        )?;
        Self::open(dir)
    }
    /// Remove clipboard files only after collecting unsent references from all accounts.
    pub(crate) fn prune_clipboard(&self, root: &Path, accounts: &Path) -> Result<()> {
        let mut referenced = std::collections::BTreeSet::new();
        let mut collect = |bodies: Vec<Content>| {
            for body in bodies {
                if let Some(item) = body.local_attachment() {
                    referenced.insert(item.path.clone());
                }
            }
        };
        collect(
            self.all_pending()?
                .into_iter()
                .map(|row| row.body)
                .collect(),
        );
        if accounts.exists() {
            for entry in std::fs::read_dir(accounts)? {
                let database = entry?.path().join("store.sqlite3");
                if database.is_file() {
                    let conn = Connection::open_with_flags(
                        database,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )?;
                    let mut stmt =
                        conn.prepare("SELECT body_json FROM message WHERE server_id IS NULL")?;
                    collect(
                        stmt.query_map([], |r| messages::parse(r, 0))?
                            .collect::<std::result::Result<Vec<_>, _>>()?,
                    );
                }
            }
        }
        if root.exists() {
            // Prepared attachments persist canonical paths, including when app data
            // is reached through a symlink.
            for entry in std::fs::read_dir(root.canonicalize()?)? {
                let path = entry?.path();
                if !referenced.contains(&path) && path.is_file() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::create_schema(&mut conn)?;
        Self::initialize(conn)
    }
    fn create_schema(conn: &mut Connection) -> Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch(include_str!("store/schema.sql"))?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }
    fn initialize(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self {
            conn,
            writes: 0,
            changes: Changes::default(),
        })
    }
    pub fn sticker_packages(&self) -> Result<Vec<StickerPackage>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM sticker_packages WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(value
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn save_sticker_packages(&mut self, rows: &[StickerPackage]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sticker_packages VALUES(1,?)",
            [serde_json::to_string(rows)?],
        )?;
        self.writes += 1;
        Ok(())
    }
    /// Replace only one actor, preserving the server revision for subsequent echoes.
    pub fn own_reaction(
        &mut self,
        key: Key,
        actor: &str,
        kind: Option<ReactionKind>,
    ) -> Result<()> {
        self.mutate(|tx, changes| {
            let mut row = messages::by_key(tx, key)?;
            row.reactions.retain(|r| r.actor != actor);
            if let Some(kind) = kind {
                row.reactions.push(Reaction {
                    actor: actor.into(),
                    kind,
                });
            }
            messages::save(tx, &row)?;
            changes.chats_changed.insert(row.chat_id);
            Ok(())
        })
    }
    pub fn writes(&self) -> u64 {
        self.writes
    }
    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }
    fn mutate(&mut self, f: impl FnOnce(&Connection, &mut Changes) -> Result<()>) -> Result<()> {
        let tx = self.conn.savepoint()?;
        let mut changes = Changes::default();
        f(&tx, &mut changes)?;
        tx.commit()?;
        self.writes += 1;
        self.changes.chats_changed.extend(changes.chats_changed);
        self.changes.drafts_changed.extend(changes.drafts_changed);
        self.changes.chats_removed.extend(changes.chats_removed);
        self.changes
            .messages_removed
            .extend(changes.messages_removed);
        Ok(())
    }
}
fn ensure_chat(tx: &Connection, chat: &ChatId) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO chat(chat_id) VALUES(?)", [&chat.0])?;
    Ok(())
}
pub(crate) fn server_sort(id: &ServerId) -> String {
    format!("{:0>20}", id.as_str())
}

impl Store {
    /// Operation and checkpoint form one durable commit, including non-message ops.
    pub fn apply_operation(&mut self, live: &LiveOperation, continuous: bool) -> types::Result<()> {
        use Operation as Op;
        self.conn.execute_batch("SAVEPOINT live_operation")?;
        let result = (|| {
            let unknown_chat = if let Op::Message(m) = &live.op {
                !self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM chat WHERE chat_id=?)",
                    [&m.chat_id.0],
                    |r| r.get::<_, bool>(0),
                )?
            } else {
                false
            };
            let fresh = match &live.op {
                Op::Message(m) => !self.contains_server(&m.server_id)?,
                _ => false,
            };
            match &live.op {
                Op::Message(m) => self.apply_live(&m.chat_id, m, continuous)?,
                Op::Receipt { chat, up_to } => self.advance_read_receipt(chat, up_to.clone())?,
                Op::Read { chat, up_to } => {
                    self.mark_read(chat, up_to.clone())?;
                    self.confirm_read(chat, up_to)?;
                }
                Op::Withdraw(id) => self.withdraw(id)?,
                Op::Reaction { id, actor, curr } => {
                    if let Some(row) = messages::by_server(&self.conn, id)? {
                        let mut entries = row.reactions;
                        entries.retain(|entry| entry.actor != *actor);
                        if let Some(kind) = curr {
                            entries.push(Reaction {
                                actor: actor.clone(),
                                kind: kind.clone(),
                            });
                        }
                        self.set_reactions(id, entries, live.revision)?;
                    }
                }
                Op::Recent(_) | Op::Checkpoint => (),
            };
            if let Op::Message(m) = &live.op
                && fresh
                && !m.outgoing
                && !live.replay
                && !m.skip_badge
            {
                let marker: Option<ServerId> = self.conn.query_row(
                    "SELECT last_read_server_id FROM chat WHERE chat_id=?",
                    [&m.chat_id.0],
                    |r| r.get(0),
                )?;
                if marker.is_none_or(|id| m.server_id > id) {
                    self.increment_unread(&m.chat_id)?;
                }
                if unknown_chat {
                    self.conn.execute(
                        "UPDATE chat SET unread_stale=1 WHERE chat_id=?",
                        [&m.chat_id.0],
                    )?;
                }
            }
            self.set_sync_revision(live.revision)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE live_operation")?;
                Ok(())
            }
            Err(e) => {
                self.conn
                    .execute_batch("ROLLBACK TO live_operation; RELEASE live_operation")?;
                Err(e)
            }
        }
    }
}
