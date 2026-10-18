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

/// Sole-writer account store. Every mutation commits atomically before returning deltas.
pub struct Store {
    conn: Connection,
    writes: u64,
    changes: Changes,
    settings_changed: bool,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        Self::initialize(Connection::open(dir.join("store.sqlite3"))?)
    }
    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }
    fn initialize(mut conn: Connection) -> Result<Self> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 5 {
            return Err(StoreError::NewerSchema(version));
        }
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        if version == 0 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("store/001.sql"))?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = conn.transaction()?;
            tx.execute_batch("ALTER TABLE draft ADD COLUMN anchor INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("store/002.sql"))?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        if version < 4 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("store/003.sql"))?;
            tx.pragma_update(None, "user_version", 4)?;
            tx.commit()?;
        }
        if version < 5 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("store/004.sql"))?;
            tx.pragma_update(None, "user_version", 5)?;
            tx.commit()?;
        }
        conn.execute_batch("CREATE TABLE IF NOT EXISTS sticker_packages (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);")?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS message_pending_chat ON message(chat_id,delivery) WHERE server_id IS NULL;
            CREATE INDEX IF NOT EXISTS message_server_order ON message(chat_id,sort_id) WHERE server_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS message_incoming_order ON message(chat_id,sort_id) WHERE outgoing=0 AND server_id IS NOT NULL;")?;
        Ok(Self {
            conn,
            writes: 0,
            changes: Changes::default(),
            settings_changed: false,
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
    /// Carry only work created before the first account identity was known.
    pub fn carry_local_to(&self, target: &mut Store) -> Result<Vec<Key>> {
        let source = self.prefs()?;
        let mut prefs = target.prefs()?;
        let mut seq = prefs.request_seq.max(source.request_seq);
        prefs.request_seq = seq;
        if self.settings_changed {
            prefs.text_size = source.text_size;
            prefs.enter_sends = source.enter_sends;
            prefs.desktop = source.desktop;
        }
        if source.last_chat.is_some() {
            prefs.last_chat = source.last_chat;
        }
        let tx = target.conn.savepoint()?;
        tx.execute(
            "UPDATE preferences SET value=?",
            [serde_json::to_string(&prefs)?],
        )?;
        let mut drafts = self.conn.prepare("SELECT chat_id FROM draft")?;
        for chat in drafts.query_map([], |r| r.get::<_, String>(0))? {
            let chat = ChatId(chat?);
            let draft = self.load_draft(&chat)?.unwrap();
            ensure_chat(&tx, &chat)?;
            tx.execute("INSERT OR REPLACE INTO draft(chat_id,text,cursor,updated,anchor) VALUES(?,?,?,?,?)",
                params![chat.0,draft.text,draft.cursor as i64,draft.updated,draft.anchor as i64])?;
        }
        let mut keys = Vec::new();
        for mut row in self.all_pending()? {
            seq += 1;
            row.request_seq = Some(seq);
            row.key = messages::insert(
                &tx,
                &row.chat_id,
                row.local_id.as_ref(),
                Some(seq),
                row.created_at_local,
            )?
            .key;
            messages::save(&tx, &row)?;
            if row.delivery == Delivery::Pending {
                keys.push(row.key);
            }
        }
        tx.execute(
            "UPDATE preferences SET value=json_set(value,'$.request_seq',?)",
            [seq],
        )?;
        tx.commit()?;
        Ok(keys)
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
fn server_sort(id: &ServerId) -> Result<String> {
    let s = &id.0;
    if s.is_empty()
        || s.len() > 20
        || !s.bytes().all(|b| b.is_ascii_digit())
        || (s.len() > 1 && s.starts_with('0'))
    {
        return Err(StoreError::Invalid("noncanonical server id".into()));
    }
    Ok(format!("{:0>20}", s))
}
fn numeric_max(a: Option<ServerId>, b: Option<ServerId>) -> Result<Option<ServerId>> {
    match (a, b) {
        (Some(a), Some(b)) => Ok(Some(if server_sort(&a)? >= server_sort(&b)? {
            a
        } else {
            b
        })),
        (a, b) => {
            let v = a.or(b);
            if let Some(id) = &v {
                server_sort(id)?;
            }
            Ok(v)
        }
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
                Op::Message(m) => self.apply_live(&m.chat_id, m, continuous, None)?,
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
                let marker = marker.map(|id| server_sort(&ServerId(id))).transpose()?;
                let incoming = server_sort(&m.server_id)?;
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
