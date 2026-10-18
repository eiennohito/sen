//! Presentation data for existing rich-content rendering.
pub struct ReactionView {
    pub label: String,
    pub image_key: String,
}
pub struct EmojiView {
    pub span: line_wasm_poc::client::desktop::rich::InlineEmoji,
    pub image_key: String,
}
#[derive(Default)]
pub struct MessageView {
    pub id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_key: String,
    pub sticker: bool,
    pub photo: bool,
    pub body: String,
    pub timestamp: i64,
    pub delivered_time: i64,
    pub outgoing: bool,
    pub image_key: String,
    pub reactions: Vec<ReactionView>,
    pub rich: line_wasm_poc::client::desktop::rich::RichContent,
    pub decryption_failed: bool,
    pub emoji: Vec<EmojiView>,
    pub skip_badge: bool,
}
