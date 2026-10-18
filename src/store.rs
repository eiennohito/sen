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
        if version > 2 {
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
        Ok(Self { conn, writes: 0 })
    }
    pub fn writes(&self) -> u64 {
        self.writes
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
                        let mut entries = row.reactions.as_array().cloned().unwrap_or_default();
                        entries.retain(|entry| entry["fromUserMid"].as_str() != Some(actor));
                        if curr.as_object().is_some_and(|v| !v.is_empty()) {
                            entries
                                .push(serde_json::json!({"fromUserMid":actor,"reactionType":curr}));
                        }
                        self.set_reactions(id, entries.into(), live.revision)?
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
                && m.body["skip_badge"].as_bool() != Some(true)
            {
                let marker: Option<String> = self.conn.query_row(
                    "SELECT last_read_server_id FROM chat WHERE chat_id=?",
                    [&m.chat_id.0],
                    |r| r.get(0),
                )?;
                let marker = marker.map(|id| server_sort(&ServerId(id))).transpose()?;
                if marker.is_none_or(|id| server_sort(&m.server_id).unwrap() > id) {
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
