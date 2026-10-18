use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Stable chat identity, independent of client transport types.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChatId(pub String);
/// Permanent row identity, including across send confirmation and retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Key(pub i64);
/// Canonical decimal server identity: 1–20 digits, no leading zeros except zero.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServerId(pub String);
/// Caller-generated, globally unique local send identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalId(pub String);
/// Confirmed delivery only advances from Sent to Read; failures are local only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Pending,
    Rejected { reason: String },
    Uncertain,
    Sent,
    Read,
}
/// Opaque decoded server content. Nonpositive created_ms falls back to delivered_ms.
#[derive(Clone, Debug)]
pub struct MessageInput {
    pub server_id: ServerId,
    pub chat_id: ChatId,
    pub created_ms: i64,
    pub delivered_ms: Option<i64>,
    pub sender_id: String,
    pub outgoing: bool,
    pub content_type: i32,
    pub request_seq: Option<u32>,
    pub body: Value,
    pub reactions: Value,
    pub reactions_rev: i64,
}
/// Single total ordering used by pages, windows, previews and segments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cursor {
    pub sort_ts: i64,
    pub sort_id: String,
}
/// Stored row; key is immutable, withdrawal sticky, and JSON content opaque.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMessage {
    pub key: Key,
    pub chat_id: ChatId,
    pub server_id: Option<ServerId>,
    pub local_id: Option<LocalId>,
    pub request_seq: Option<u32>,
    pub sort: Cursor,
    pub sender_id: String,
    pub outgoing: bool,
    pub content_type: i32,
    pub body: Value,
    pub reactions: Value,
    pub reactions_rev: i64,
    /// Last page/live revision used for body fields; reaction-only ops do not advance it.
    pub content_rev: Option<i64>,
    pub withdrawn: bool,
    pub delivery: Delivery,
    pub created_at_local: i64,
}
/// Server chat metadata; last-message hints never replace a stored preview.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatInput {
    pub id: ChatId,
    pub kind: i32,
    pub title: String,
    pub avatar: Option<String>,
    pub member_count: Option<u32>,
    pub muted: bool,
    pub left: bool,
    pub last_message_id: Option<ServerId>,
    pub last_message_ms: Option<i64>,
    pub unread_count: u32,
    pub unread_stale: bool,
    pub last_read_server_id: Option<ServerId>,
    pub list_rank: i64,
}
/// Persisted metadata with monotonic read marker and server activity hints.
pub type ChatRecord = ChatInput;
/// Committed delta identities; removed duplicate rows are never also upserted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub messages_upserted: BTreeSet<Key>,
    pub messages_removed: BTreeSet<Key>,
    pub chats_changed: BTreeSet<ChatId>,
    pub chats_removed: BTreeSet<ChatId>,
}
/// Page cursors certify continuity between the fetched range and its anchor.
#[derive(Clone, Debug)]
pub enum PageKind {
    Recent,
    Older { before: Cursor },
    Newer { after: Cursor },
}
/// Window selection; Key anchors must belong to the requested chat.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Unread,
    Latest,
    Key(Key),
}
/// Missing continuity before `after`; None means history before the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub before: Option<Key>,
    pub after: Key,
}
/// Ascending rows and explicit continuity boundaries; ends refer to known segments.
#[derive(Clone, Debug)]
pub struct Window {
    pub rows: Vec<StoredMessage>,
    pub gaps: Vec<Gap>,
    pub reaches_start: bool,
    pub reaches_end: bool,
}
/// Visible chat with preview taken exclusively from its newest stored row.
#[derive(Clone, Debug)]
pub struct ChatListItem {
    pub chat: ChatRecord,
    pub preview: Option<StoredMessage>,
    pub activity_ms: i64,
    pub failed_count: u32,
    pub draft: Option<Draft>,
}
/// Single account profile and durable sync checkpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub own_mid: String,
    pub profile_json: Value,
    pub sync_revision: i64,
    pub global_revision: i64,
    pub individual_revision: i64,
    pub opened_at: i64,
}
/// Durable composer content and selection position.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
    pub updated: i64,
}
/// Store failures never partially commit mutations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("database schema {0} is newer than supported version 1")]
    NewerSchema(i64),
    #[error("invalid store operation: {0}")]
    Invalid(String),
}
pub(super) type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug)]
pub enum Operation {
    Message(MessageInput),
    Receipt {
        chat: ChatId,
        up_to: ServerId,
    },
    Read {
        chat: ChatId,
        up_to: ServerId,
    },
    Withdraw(ServerId),
    Reaction {
        id: ServerId,
        actor: String,
        curr: serde_json::Value,
    },
    Recent(ChatId),
    Checkpoint,
}
#[derive(Clone, Debug)]
pub struct LiveOperation {
    pub revision: i64,
    pub replay: bool,
    pub op: Operation,
}
