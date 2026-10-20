//! Slint is a presentation target; the controller owns the projection.
mod assets;
pub(crate) mod chats;
mod compose;
pub(crate) mod detail;
pub(crate) mod format;
pub(crate) mod messages;
mod people;
mod shell;
pub(crate) mod system_events;

use crate::*;
pub(crate) use assets::Images;
use assets::{asset_key, content_assets};
use line::client::desktop::{
    assets::{Asset, Failure},
    rich::emoji_spans,
};
use messages::compact_text;
use model::KeyedModel;
pub(crate) mod model;
#[derive(Default)]
pub struct Presenter {
    chats: Rc<KeyedModel<ChatId, ChatInputs>>,
    messages: Rc<KeyedModel<RowKey, messages::MessageData>>,
    history_chat: Option<ChatId>,
    pub images: Images,
    friends: Rc<KeyedModel<String, people::FriendEntry>>,
    members: Rc<KeyedModel<String, (Friend, bool, bool)>>,
    compose_friends: Rc<KeyedModel<String, (Friend, bool)>>,
    compose_selected: Rc<KeyedModel<String, (Friend, bool)>>,
    sticker_tabs: Rc<KeyedModel<usize, (String, Option<Content>)>>,
    sticker_items: Rc<KeyedModel<usize, (usize, Content)>>,
    status_assets: Vec<Asset>,
    compose_ids: std::collections::HashSet<String>,
}
#[derive(Clone, PartialEq)]
struct MessageInputs {
    message: StoredMessage,
    sender: Option<Friend>,
    reactors: Vec<(String, Option<Friend>)>,
    own_mid: String,
    sender_friend: bool,
    system_text: Option<String>,
    reply: Option<(StoredMessage, String)>,
    upload: f32,
    reply_text: Option<String>,
}
#[derive(Clone, PartialEq)]
struct ChatInputs {
    item: ChatListItem,
    title: String,
    avatar: Option<Asset>,
    system_text: Option<String>,
    offline: bool,
    index: usize,
    text_size: i32,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RowKey {
    Message(Key),
    Gap(Key),
}
impl Presenter {
    pub fn present(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        query: &str,
        category: ChatCategory,
    ) {
        self.present_compose(ui, state);
        self.present_people(ui, state);
        let detail_error = self.present_errors(ui, state);
        self.present_detail(ui, state, detail_error);
        self.present_chats(ui, state, query, category);
        self.present_messages(ui, state);
        self.present_shell(ui, state);
        self.present_header(ui, state);
        self.present_draft(ui, state);
        self.present_history(ui, state);
    }
}
