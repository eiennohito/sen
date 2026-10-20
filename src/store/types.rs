use crate::client::desktop::assets::Asset;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyReference {
    pub message_id: ServerId,
    pub service_code: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineEmoji {
    /// Half-open JavaScript UTF-16 offsets, never inside a surrogate pair.
    pub start_utf16: usize,
    pub end_utf16: usize,
    /// Equivalent UTF-8 boundaries for slicing the unchanged message body.
    pub start_byte: usize,
    pub end_byte: usize,
    pub fallback: String,
    pub product_id: String,
    pub sticon_id: String,
    pub version: Option<String>,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Content {
    Text {
        text: String,
        spans: Vec<InlineEmoji>,
    },
    Flex {
        alt_text: String,
        lines: Vec<String>,
        links: Vec<String>,
    },
    Photo {
        asset: Option<Asset>,
        size: Option<(u32, u32)>,
        album_count: Option<u64>,
        caption: Option<String>,
    },
    Sticker {
        pkg: String,
        id: String,
        ver: Option<String>,
        asset: Option<Asset>,
        text: Option<String>,
    },
    File {
        name: Option<String>,
        size: Option<u64>,
        asset: Option<Asset>,
    },
    Video {
        duration: Option<u64>,
        asset: Option<Asset>,
        preview: Option<Asset>,
        album_count: Option<u64>,
        caption: Option<String>,
    },
    Voice {
        duration: Option<u64>,
        asset: Option<Asset>,
    },
    Contact {
        mid: String,
        name: Option<String>,
    },
    Location {
        title: Option<String>,
        lat: f64,
        lon: f64,
    },
    System {
        kind: Option<String>,
        params: Option<String>,
        text: Option<String>,
    },
    Withdrawn,
    Undecryptable,
    Unknown {
        content_type: i32,
    },
}
impl Default for Content {
    fn default() -> Self {
        Self::Text {
            text: String::new(),
            spans: vec![],
        }
    }
}
impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            spans: vec![],
        }
    }
    pub fn is_withdrawn(&self) -> bool {
        matches!(self, Self::Withdrawn)
    }
    pub fn as_text(&self) -> &str {
        if let Self::Text { text, .. } = self {
            text
        } else {
            ""
        }
    }
    pub fn spans(&self) -> &[InlineEmoji] {
        if let Self::Text { spans, .. } = self {
            spans
        } else {
            &[]
        }
    }
    pub fn image(&self) -> Option<&Asset> {
        match self {
            Self::Photo { asset, .. } | Self::Sticker { asset, .. } => asset.as_ref(),
            Self::Video { preview, .. } => preview.as_ref(),
            _ => None,
        }
    }
    pub fn local_attachment(&self) -> Option<&crate::attachments::Attachment> {
        self.image()
            .or_else(|| self.attachment())
            .and_then(Asset::local_attachment)
    }
    pub fn attachment(&self) -> Option<&Asset> {
        match self {
            Self::File { asset, .. } | Self::Video { asset, .. } | Self::Voice { asset, .. } => {
                asset.as_ref()
            }
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub actor: String,
    pub kind: ReactionKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReactionKind {
    Predefined(u32),
    Paid(Option<Asset>),
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    pub mid: String,
    pub display_name: String,
    pub status_message: String,
    pub picture_path: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactProfile {
    #[serde(rename = "statusMessageContentMetadata")]
    pub status_metadata: serde_json::Value,
    #[serde(rename = "displayName")]
    pub original_name: Option<String>,
    #[serde(rename = "displayNameOverridden")]
    pub overridden_name: Option<String>,
    pub relation: Option<i32>,
    pub invited: bool,
    #[serde(rename = "type")]
    pub account_type: i32,
    #[serde(rename = "favoriteTime", deserialize_with = "optional_number")]
    pub favorite_time: Option<i64>,
}
fn optional_number<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<i64>, D::Error> {
    Ok(crate::client::wire_number(&serde_json::Value::deserialize(
        d,
    )?))
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Friend {
    pub profile: ContactProfile,
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub group: bool,
    pub avatar: Option<Asset>,
    pub member_count: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub contact: Friend,
    pub is_self: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub text_size: i32,
    pub enter_sends: bool,
    pub desktop: DesktopPrefs,
    pub last_chat: Option<ChatId>,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            text_size: 15,
            enter_sends: false,
            desktop: DesktopPrefs::default(),
            last_chat: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i32", into = "i32")]
pub enum NotificationPreview {
    NameAndText,
    #[default]
    NameOnly,
    NewMessage,
}
impl TryFrom<i32> for NotificationPreview {
    type Error = &'static str;
    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NameAndText),
            1 => Ok(Self::NameOnly),
            2 => Ok(Self::NewMessage),
            _ => Err("Invalid notification preview preference"),
        }
    }
}
impl From<NotificationPreview> for i32 {
    fn from(value: NotificationPreview) -> Self {
        match value {
            NotificationPreview::NameAndText => 0,
            NotificationPreview::NameOnly => 1,
            NotificationPreview::NewMessage => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopPrefs {
    pub background: bool,
    pub autostart: bool,
    pub notifications: bool,
    pub preview: NotificationPreview,
    pub sound: bool,
    pub mention_exception: bool,
    pub group_events: bool,
}
impl Default for DesktopPrefs {
    fn default() -> Self {
        Self {
            background: false,
            autostart: false,
            notifications: false,
            preview: NotificationPreview::NameOnly,
            sound: false,
            mention_exception: false,
            group_events: false,
        }
    }
}
/// Stable chat identity, independent of client transport types.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChatId(pub String);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidKind {
    User,
    Group,
    Room,
}

pub fn mid_kind(mid: &str) -> Option<MidKind> {
    match *mid.as_bytes().first()? {
        b'U' => Some(MidKind::User),
        b'C' => Some(MidKind::Group),
        b'R' => Some(MidKind::Room),
        _ => None,
    }
}
impl ChatId {
    pub fn is_room(&self) -> bool {
        mid_kind(&self.0) == Some(MidKind::Room)
    }
}

/// Permanent row identity, including across send confirmation and retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key(pub i64);
/// Canonical decimal server identity: 1–20 digits, no leading zeros except zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServerId(String);
impl Ord for ServerId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}
impl PartialOrd for ServerId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl ServerId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 20
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(StoreError::Invalid("noncanonical server id".into()));
        }
        Ok(Self(value))
    }
}
impl TryFrom<String> for ServerId {
    type Error = StoreError;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}
impl From<ServerId> for String {
    fn from(value: ServerId) -> Self {
        value.into_string()
    }
}
impl rusqlite::types::FromSql for ServerId {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        Self::new(value.as_str()?).map_err(|e| rusqlite::types::FromSqlError::Other(Box::new(e)))
    }
}
impl rusqlite::ToSql for ServerId {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}
/// Caller-generated, globally unique local send identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalId(pub String);
/// Confirmed delivery only advances from Sent to Read; failures are local only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Pending,
    Rejected {
        reason: crate::client::error::Refusal,
    },
    Sent,
    Read,
}
impl Delivery {
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }
}
/// Typed decoded server content. Nonpositive created_ms falls back to delivered_ms.
#[derive(Clone, Debug)]
pub struct MessageInput {
    pub server_id: ServerId,
    pub chat_id: ChatId,
    pub created_ms: i64,
    pub delivered_ms: Option<i64>,
    pub sender_id: String,
    pub outgoing: bool,
    pub mentioned: bool,
    pub reply: Option<Box<ReplyReference>>,
    pub skip_badge: bool,
    pub request_seq: Option<u32>,
    pub body: Content,
    pub reactions: Vec<Reaction>,
    pub rev: i64,
}
/// Single total ordering used by pages, windows, previews and segments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cursor {
    pub sort_ts: i64,
    pub sort_id: String,
}
/// Stored row; key is immutable, withdrawal sticky, and content decoded at the store boundary.
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
    pub body: Content,
    pub reactions: Vec<Reaction>,
    pub reactions_rev: i64,
    /// Reaction revision accompanying the last body update; reaction-only ops do not advance it.
    pub body_rev: Option<i64>,
    pub delivery: Delivery,
    pub created_at_local: i64,
    pub mentioned: bool,
    pub reply: Option<Box<ReplyReference>>,
    pub skip_badge: bool,
}
impl StoredMessage {
    pub fn can_react(&self) -> bool {
        self.server_id.is_some()
            && matches!(self.delivery, Delivery::Sent | Delivery::Read)
            && !matches!(self.body, Content::System { .. } | Content::Withdrawn)
    }
    pub fn can_unsend(&self) -> bool {
        self.outgoing && self.can_react()
    }
}
/// Server chat metadata; last-message hints never replace a stored preview.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatInput {
    pub id: ChatId,
    pub is_group: bool,
    pub title: String,
    pub avatar: Option<Asset>,
    pub member_count: Option<u32>,
    pub muted: bool,
    pub left: bool,
    pub last_message_id: Option<ServerId>,
    pub last_message_ms: Option<i64>,
    pub server_unread: u32,
    pub unread_stale: bool,
    pub last_read_server_id: Option<ServerId>,
}
/// Chats to republish and row identities removed by committed mutations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub messages_removed: BTreeSet<Key>,
    pub chats_changed: BTreeSet<ChatId>,
    pub drafts_changed: BTreeSet<ChatId>,
    pub chats_removed: BTreeSet<ChatId>,
}
/// Page cursors certify continuity between the fetched range and its anchor.
#[derive(Clone, Debug)]
pub enum PageKind {
    Around { id: ServerId },
    Recent,
    Older { before: Cursor },
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
#[derive(Clone, Debug, PartialEq)]
pub struct ChatListItem {
    pub unread_count: u32,
    pub chat: ChatInput,
    pub preview: Option<StoredMessage>,
    pub activity_ms: i64,
    pub failed_count: u32,
    pub draft: Option<Draft>,
}
/// Single account profile and durable sync checkpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub own_mid: String,
    pub profile: Profile,
    pub sync_revision: i64,
}
/// Durable composer content and selection position.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub reply: Option<ReplyReference>,
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
    #[error("invalid store operation: {0}")]
    Invalid(String),
}
pub(super) type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug)]
pub enum Operation {
    Message(Box<MessageInput>),
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
        curr: Option<ReactionKind>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StickerPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub stickers: Vec<Content>,
}

#[cfg(test)]
mod server_id_ordering {
    use super::*;
    #[test]
    fn canonical_ids_sort_numerically() {
        let mut ids: Vec<_> = [
            "10",
            "0",
            "9",
            "18446744073709551616",
            "9999999999999999999",
        ]
        .into_iter()
        .map(|id| ServerId::new(id).unwrap())
        .collect();
        ids.sort();
        assert_eq!(
            ids.iter().map(ServerId::as_str).collect::<Vec<_>>(),
            [
                "0",
                "9",
                "10",
                "9999999999999999999",
                "18446744073709551616"
            ]
        );
    }
}
