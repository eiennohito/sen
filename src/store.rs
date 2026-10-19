//! Transactional, worker-owned SQLite source of truth. No client or UI dependencies.
//! Withdrawal tombstones survive missing messages; server IDs are canonical decimals.
mod messages;
mod queries;
#[cfg(test)]
mod tests;
mod types;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use types::Result;
pub use types::*;

const SCHEMA_VERSION: i64 = 9;

/// Sole-writer account store. Every mutation commits atomically before returning deltas.
pub struct Store {
    conn: Connection,
    writes: u64,
    changes: Changes,
}
impl Store {
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
                if let Some(item) = body
                    .image()
                    .or_else(|| body.attachment())
                    .and_then(|asset| asset.local_attachment())
                {
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
            changes.messages_upserted.insert(key);
            changes.chats_changed.insert(row.chat_id);
            Ok(())
        })?;
        Ok(())
    }
    pub fn writes(&self) -> u64 {
        self.writes
    }
    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }
    fn mutate(
        &mut self,
        f: impl FnOnce(&Connection, &mut Changes) -> Result<()>,
    ) -> Result<Changes> {
        let tx = self.conn.savepoint()?;
        let mut changes = Changes::default();
        f(&tx, &mut changes)?;
        tx.commit()?;
        self.writes += 1;
        self.changes
            .chats_changed
            .extend(changes.chats_changed.iter().cloned());
        self.changes
            .chats_removed
            .extend(changes.chats_removed.iter().cloned());
        self.changes
            .messages_upserted
            .extend(changes.messages_upserted.iter().copied());
        self.changes
            .messages_removed
            .extend(changes.messages_removed.iter().copied());
        Ok(changes)
    }
}
fn ensure_chat(tx: &Connection, chat: &ChatId) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO chat(chat_id) VALUES(?)", [&chat.0])?;
    Ok(())
}
fn server_sort(id: &ServerId) -> String {
    format!("{:0>20}", id.as_str())
}
fn numeric_max(a: Option<ServerId>, b: Option<ServerId>) -> Option<ServerId> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if server_sort(&a) >= server_sort(&b) {
            a
        } else {
            b
        }),
        (a, b) => a.or(b),
    }
}

impl Store {
    /// Operation and checkpoint form one durable commit, including non-message ops.
    pub fn apply_operation(
        &mut self,
        live: &LiveOperation,
        continuous: bool,
    ) -> types::Result<Changes> {
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
            let mut changes = match &live.op {
                Op::Message(m) => self.apply_live(&m.chat_id, m, continuous)?,
                Op::Receipt { chat, up_to } => self.advance_read_receipt(chat, up_to.clone())?,
                Op::Read { chat, up_to } => {
                    let changes = self.mark_read(chat, up_to.clone())?;
                    self.confirm_read(chat, up_to)?;
                    changes
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
                        self.set_reactions(id, entries, live.revision)?
                    } else {
                        Changes::default()
                    }
                }
                Op::Recent(_) | Op::Checkpoint => Changes::default(),
            };
            if let Op::Message(m) = &live.op
                && fresh
                && !m.outgoing
                && !live.replay
                && !m.skip_badge
            {
                let marker: Option<String> = self.conn.query_row(
                    "SELECT last_read_server_id FROM chat WHERE chat_id=?",
                    [&m.chat_id.0],
                    |r| r.get(0),
                )?;
                let marker = marker
                    .map(ServerId::new)
                    .transpose()?
                    .map(|id| server_sort(&id));
                let incoming = server_sort(&m.server_id);
                if marker.is_none_or(|id| incoming > id) {
                    let c = self.increment_unread(&m.chat_id)?;
                    changes.chats_changed.extend(c.chats_changed);
                }
                if unknown_chat {
                    self.conn.execute(
                        "UPDATE chat SET unread_stale=1 WHERE chat_id=?",
                        [&m.chat_id.0],
                    )?;
                }
            }
            self.set_sync_revision(live.revision)?;
            Ok(changes)
        })();
        match result {
            Ok(c) => {
                self.conn.execute_batch("RELEASE live_operation")?;
                Ok(c)
            }
            Err(e) => {
                self.conn
                    .execute_batch("ROLLBACK TO live_operation; RELEASE live_operation")?;
                Err(e)
            }
        }
    }
}
