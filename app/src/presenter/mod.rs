//! Slint is a presentation target; the controller owns the projection.
use crate::*;
use messages::compact_text;
#[derive(Default)]
pub struct Presenter {
    chats: Rc<VecModel<ChatRow>>,
    messages: Rc<VecModel<MessageRow>>,
    chat_keys: Vec<ChatId>,
    history_chat: Option<ChatId>,
    message_keys: Vec<RowKey>,
    pub images: HashMap<String, slint::Image>,
    pub failures: HashMap<String, line::client::desktop::assets::Failure>,
    friends: Rc<VecModel<FriendRow>>,
    friend_keys: Vec<String>,
    members: Rc<VecModel<MemberRow>>,
    member_keys: Vec<String>,
    rendered_friends: std::collections::BTreeMap<String, CachedFriend>,
    rendered_chats: std::collections::BTreeMap<ChatId, (ChatInputs, ChatRow)>,
    rendered: std::collections::BTreeMap<Key, (MessageInputs, MessageRow)>,
}
struct CachedFriend {
    friend: Friend,
    size: i32,
    row: FriendRow,
    assets: Vec<line::client::desktop::assets::Asset>,
}
#[derive(PartialEq)]
struct MessageInputs {
    message: StoredMessage,
    sender: Option<Friend>,
    sender_friend: bool,
    system_text: Option<String>,
    reply: Option<(StoredMessage, String)>,
    upload: f32,
}
// Asset pixels are patched in place; these are the non-asset row inputs.
#[derive(PartialEq)]
struct ChatInputs {
    chat: ChatInput,
    unread_count: u32,
    title: String,
    preview: Option<StoredMessage>,
    system_text: Option<String>,
    activity: i64,
    failed: u32,
    draft: Option<Draft>,
    offline: bool,
    index: usize,
    text_size: i32,
}
#[derive(Clone, PartialEq)]
enum RowKey {
    Message(Key),
    Gap(Key),
}
fn patch<K: PartialEq + Clone, R: Clone + PartialEq + 'static>(
    model: &VecModel<R>,
    keys: &mut Vec<K>,
    next: Vec<(K, R)>,
) {
    for (index, (key, row)) in next.iter().enumerate() {
        if keys.get(index) != Some(key) {
            if let Some(old) = keys.iter().position(|k| k == key) {
                keys.remove(old);
                model.remove(old);
            }
            keys.insert(index, key.clone());
            model.insert(index, row.clone());
        } else if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
        }
    }
    while keys.len() > next.len() {
        keys.pop();
        model.remove(model.row_count() - 1);
    }
}

mod assets;
pub(crate) mod chats;
mod compose;
mod detail;
pub(crate) mod messages;
mod people;
mod shell;

impl Presenter {
    pub fn present(&mut self, ui: &AppWindow, state: &AppState, query: &str, category: i32) {
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
