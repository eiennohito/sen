//! Live operation types from the Chrome extension's main.js operation table.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationType {
    AddContact,
    SendMessage,
    ReceiveMessage,
    SendChatChecked,
    SendChatRemoved,
    SendContent,
    UpdateContact,
    NotifiedReadMessage,
    DestroyMessage,
    NotifiedDestroyMessage,
    UpdateChat,
    NotifiedUpdateChat,
    DeleteSelfFromChat,
    NotifiedDeleteSelfFromChat,
    SendReaction,
    NotifiedSendReaction,
    Unknown,
}
impl From<i64> for OperationType {
    fn from(value: i64) -> Self {
        match value {
            4 => Self::AddContact,
            25 => Self::SendMessage,
            26 => Self::ReceiveMessage,
            40 => Self::SendChatChecked,
            41 => Self::SendChatRemoved,
            43 => Self::SendContent,
            49 => Self::UpdateContact,
            55 => Self::NotifiedReadMessage,
            64 => Self::DestroyMessage,
            65 => Self::NotifiedDestroyMessage,
            121 => Self::UpdateChat,
            122 => Self::NotifiedUpdateChat,
            127 => Self::DeleteSelfFromChat,
            128 => Self::NotifiedDeleteSelfFromChat,
            139 => Self::SendReaction,
            140 => Self::NotifiedSendReaction,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawOperation {
    pub kind: OperationType,
    pub data: Value,
}
impl From<Value> for RawOperation {
    fn from(data: Value) -> Self {
        Self {
            kind: data["type"]
                .as_i64()
                .map(OperationType::from)
                .unwrap_or(OperationType::Unknown),
            data,
        }
    }
}
