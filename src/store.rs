//! Transactional, worker-owned SQLite source of truth. No client or UI dependencies.
//! Withdrawal tombstones survive missing messages; server IDs are canonical decimals.
mod messages;
mod queries;
#[cfg(test)]
mod tests;
mod types;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::path::Path;
use types::Result;
pub use types::*;

/// Sole-writer account store. Every mutation commits atomically before returning deltas.
pub struct Store {
    conn: Connection,
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
        if version > 1 {
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
        Ok(Self { conn })
    }
    fn mutate(
        &mut self,
        f: impl FnOnce(&Transaction<'_>, &mut Changes) -> Result<()>,
    ) -> Result<Changes> {
        let tx = self.conn.transaction()?;
        let mut changes = Changes::default();
        f(&tx, &mut changes)?;
        tx.commit()?;
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
