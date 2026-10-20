//! Slint is a presentation target; the controller owns the projection.
mod assets;
pub(crate) mod chats;
mod compose;
pub(crate) mod detail;
pub(crate) mod format;
pub(crate) mod messages;
pub(crate) mod names;
mod new_chat;
mod people;
mod search;
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
use search::Query;
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
pub(crate) struct ChatInputs {
    pub(crate) item: ChatListItem,
    pub(crate) title: String,
    pub(crate) avatar: Option<Asset>,
    pub(crate) system_text: Option<String>,
    pub(crate) offline: bool,
    pub(crate) index: usize,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum RowKey {
    Message(Key),
    Gap(Key),
}
impl Presenter {
    pub(super) fn present_errors(&mut self, ui: &AppWindow, state: &AppState) -> String {
        let conversation = ui.global::<ConversationState>();
        let friends = ui.global::<FriendsState>();
        let chats = ui.global::<ChatsState>();
        let mut conversation_error = String::new();
        let mut friends_error = String::new();
        let mut chat_list_error = String::new();
        let mut detail_error = String::new();
        let detail_id = state.detail.last().map(|page| match page {
            line::controller::DetailPage::Profile(id) => id.as_str(),
            line::controller::DetailPage::GroupInfo(id)
            | line::controller::DetailPage::Members(id) => id.0.as_str(),
        });
        for (request, failure) in &state.refresh_failures {
            use line::{client::error::Failure, engine::Refresh};
            let message = if matches!(request, Refresh::Session) {
                match failure {
                    Failure::Refused(_) | Failure::Bug(_) => {
                        "Couldn't check your saved session.".into()
                    }
                    _ => continue,
                }
            } else {
                match failure {
                    Failure::Refused(reason) => {
                        format!("Couldn't load this item: {}", text::rejection(reason))
                    }
                    Failure::Bug(_) => "Couldn't load this item.".into(),
                    _ => continue,
                }
            };
            match request {
                Refresh::Friends => friends_error = message,
                Refresh::Session | Refresh::Profile | Refresh::ChatList | Refresh::MoreChats => {
                    chat_list_error = message
                }
                Refresh::Recent(chat)
                | Refresh::Older(chat, _)
                | Refresh::Receipts(chat)
                | Refresh::Details(chat)
                    if state.selected.as_ref() == Some(chat) =>
                {
                    conversation_error = message
                }
                Refresh::Contact(mid) | Refresh::Notice(mid) if detail_id == Some(mid.as_str()) => {
                    detail_error = message
                }
                Refresh::GroupInfo(chat) | Refresh::Members(chat)
                    if detail_id == Some(chat.0.as_str()) =>
                {
                    detail_error = message
                }
                Refresh::Stream if state.selected.is_some() => {
                    conversation_error = format!("Couldn't receive updates. {message}")
                }
                _ => {}
            }
        }
        conversation.set_conversation_error(conversation_error.into());
        friends.set_friends_error(friends_error.into());
        chats.set_chat_list_error(chat_list_error.into());
        detail_error
    }
    pub fn install(&self, ui: &AppWindow) {
        let content = ui.global::<ContentState>();
        let chats = ui.global::<ChatsState>();
        let conversation = ui.global::<ConversationState>();
        let friends = ui.global::<FriendsState>();
        let detail = ui.global::<DetailState>();
        let new_chat = ui.global::<NewChatState>();
        let composer = ui.global::<ComposerState>();
        content.set_reaction_kinds(ModelRc::new(VecModel::from(
            PREDEFINED_REACTIONS
                .iter()
                .map(|(id, label, glyph)| ReactionRow {
                    id: *id as i32,
                    label: (*label).into(),
                    glyph: (*glyph).into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        chats.set_chats(self.images.map(self.chats.clone(), chats::chat_row));
        conversation.set_messages(self.images.map(self.messages.clone(), messages::project));
        friends.set_friends(self.images.map(self.friends.clone(), people::friend_row));
        detail.set_members(self.images.map(self.members.clone(), people::member_row));
        new_chat.set_new_chat_friends(
            self.images
                .map(self.compose_friends.clone(), compose::compose_row),
        );
        new_chat.set_new_chat_selected(
            self.images
                .map(self.compose_selected.clone(), compose::compose_row),
        );
        composer.set_sticker_tabs(self.images.map(self.sticker_tabs.clone(), compose::tab_row));
        composer.set_sticker_items(
            self.images
                .map(self.sticker_items.clone(), compose::sticker_row),
        );
    }
    pub fn present(&mut self, ui: &AppWindow, state: &AppState, category: ChatCategory) {
        self.present_new_chat(ui, state);
        self.present_compose(ui, state);
        self.present_people(ui, state);
        self.present_members(ui, state);
        let detail_error = self.present_errors(ui, state);
        self.present_detail(ui, state, detail_error);
        self.present_chats(ui, state, category);
        self.present_messages(ui, state, &state.rows());
        self.present_shell(ui, state);
        self.present_header(ui, state);
        self.present_draft(ui, state);
        self.present_history(ui, state);
    }
}
