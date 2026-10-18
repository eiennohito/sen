//! UI-independent projection. Deltas are applied immediately in FIFO order.
use crate::{engine::*, store::*};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Chats,
    Friends,
    Settings,
    Members,
    Profile(String),
    MemberProfile(String),
    Viewer,
    SignIn,
}
pub struct AppState {
    pub friends: Vec<crate::client::desktop::Friend>,
    pub members: Vec<crate::client::desktop::Member>,
    pub connection: Connection,
    pub panel: Panel,
    pub profile: serde_json::Value,
    pub chats: BTreeMap<ChatId, ChatListItem>,
    pub chat_order: Vec<ChatId>,
    pub chat_sub: u64,
    pub window_sub: u64,
    pub window_anchor: Anchor,
    pub selected: Option<ChatId>,
    pub messages: BTreeMap<Key, StoredMessage>,
    pub reply_targets: BTreeMap<ServerId, StoredMessage>,
    pub gaps: Vec<Gap>,
    pub unread_divider: Option<Key>,
    pub unread_boundary_unknown: bool,
    pub reaches_start: bool,
    pub reaches_end: bool,
    pub composer: String,
    pub submitting: Option<(ChatId, String)>,
    pub prefs: BTreeMap<String, serde_json::Value>,
    pub error: Option<String>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            friends: vec![],
            members: vec![],
            connection: Connection::Local,
            panel: Panel::Chats,
            profile: serde_json::Value::Null,
            chats: BTreeMap::new(),
            chat_order: vec![],
            chat_sub: 1,
            window_sub: 0,
            window_anchor: Anchor::Latest,
            selected: None,
            messages: BTreeMap::new(),
            reply_targets: BTreeMap::new(),
            gaps: vec![],
            unread_divider: None,
            unread_boundary_unknown: false,
            reaches_start: false,
            reaches_end: false,
            composer: String::new(),
            submitting: None,
            prefs: BTreeMap::new(),
            error: None,
        }
    }
}
impl AppState {
    pub fn can_send(&self) -> bool {
        self.connection == Connection::Online
            && self.selected.is_some()
            && self.submitting.is_none()
            && !self.composer.trim().is_empty()
    }
    pub fn submit(&mut self) -> Option<Intent> {
        if self.composer.trim().is_empty() || self.submitting.is_some() {
            return None;
        }
        let chat = self.selected.clone()?;
        let text = self.composer.clone();
        self.submitting = Some((chat.clone(), text.clone()));
        Some(Intent::Send { chat, text })
    }

    pub fn select(&mut self, chat: ChatId) -> Intent {
        if self.selected.as_ref() != Some(&chat) {
            self.composer.clear();
        }
        self.window_sub += 1;
        self.window_anchor = Anchor::Unread;
        self.selected = Some(chat.clone());
        self.messages.clear();
        self.reply_targets.clear();
        self.gaps.clear();
        self.unread_divider = None;
        self.unread_boundary_unknown = false;
        self.reaches_start = false;
        self.reaches_end = false;
        Intent::Window(WindowSub {
            sub: self.window_sub,
            chat,
            anchor: Anchor::Unread,
            before: 80,
            after: 80,
        })
    }
    pub fn subscribe_window(&mut self, anchor: Anchor, before: usize, after: usize) -> Intent {
        self.window_sub += 1;
        self.window_anchor = anchor;
        Intent::Window(WindowSub {
            sub: self.window_sub,
            chat: self.selected.clone().unwrap(),
            anchor,
            before,
            after,
        })
    }
    pub fn rows(&self) -> Vec<&StoredMessage> {
        let mut rows = self.messages.values().collect::<Vec<_>>();
        rows.sort_by(|a, b| a.sort.cmp(&b.sort));
        rows
    }
    fn locate_unread(&mut self) {
        let Some(chat) = self.selected.as_ref().and_then(|id| self.chats.get(id)) else {
            return;
        };
        if self.unread_divider.is_none() && chat.chat.unread_count > 0 {
            let incoming = self
                .rows()
                .into_iter()
                .filter(|m| !m.outgoing && m.server_id.is_some())
                .collect::<Vec<_>>();
            let key = if let Some(marker) = &chat.chat.last_read_server_id {
                let marker = format!("{:0>20}", marker.0);
                incoming
                    .iter()
                    .find(|m| m.sort.sort_id > marker)
                    .map(|m| m.key)
            } else if self.reaches_end && incoming.len() >= chat.chat.unread_count as usize {
                Some(incoming[incoming.len() - chat.chat.unread_count as usize].key)
            } else {
                None
            };
            self.unread_divider = key;
        }
        self.unread_boundary_unknown = chat.chat.unread_count > 0 && self.unread_divider.is_none();
    }
    pub fn apply(&mut self, delta: Delta) {
        match delta {
            Delta::Reset => *self = Self::default(),
            Delta::Submitted { chat, text } => {
                if self.selected.as_ref() == Some(&chat) && self.composer == text {
                    self.composer.clear();
                }
                self.submitting = None;
            }
            Delta::Friends(rows) => self.friends = rows,
            Delta::Members { chat, rows } if self.selected.as_ref() == Some(&chat) => {
                self.members = rows
            }
            Delta::Connection(c) => self.connection = c,
            Delta::Profile(p) => self.profile = p,
            Delta::Chats {
                sub,
                upsert,
                remove,
                order,
            } if sub == self.chat_sub => {
                for id in remove {
                    self.chats.remove(&id);
                }
                for row in upsert {
                    self.chats.insert(row.chat.id.clone(), row);
                }
                self.chat_order = order;
            }
            Delta::Window {
                sub,
                upsert,
                remove,
                gaps,
                reaches_start,
                reaches_end,
                replies,
                unread_anchor,
            } if sub == self.window_sub => {
                for key in remove {
                    self.messages.remove(&key);
                }
                for row in upsert {
                    self.messages.insert(row.key, row);
                }
                self.reply_targets = replies
                    .into_iter()
                    .map(|m| (m.server_id.clone().unwrap(), m))
                    .collect();
                if self.unread_divider.is_none() {
                    self.unread_divider = unread_anchor;
                }
                self.gaps = gaps;
                self.reaches_start = reaches_start;
                self.reaches_end = reaches_end;
            }
            Delta::Draft { chat, draft } if self.selected.as_ref() == Some(&chat) => {
                self.composer = draft.map(|d| d.text).unwrap_or_default()
            }
            Delta::Copy { chat, text } if self.selected.as_ref() == Some(&chat) => {
                self.composer = text
            }
            Delta::Pref { key, value } => {
                if key == "last_chat" && self.selected.is_none() {
                    self.selected = Some(ChatId(value.as_str().unwrap().into()));
                    self.window_sub = 1;
                    self.window_anchor = Anchor::Unread;
                }
                self.prefs.insert(key, value);
            }
            Delta::Error(e) => {
                self.error = Some(e);
                self.submitting = None;
            }
            Delta::StoreSaved => self.error = None,
            _ => {}
        }
        self.locate_unread();
    }
}

/// UI callbacks record actions, including callbacks invoked while an action is
/// being presented. The sole drain releases the queue borrow before dispatch.
pub struct ActionQueue<T>(std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<Action<T>>>>);
type Action<T> = Box<dyn FnOnce(&mut T)>;
impl<T> Default for ActionQueue<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}
impl<T> Clone for ActionQueue<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> ActionQueue<T> {
    pub fn push(&self, action: impl FnOnce(&mut T) + 'static) {
        self.0.borrow_mut().push_back(Box::new(action));
    }
    pub fn drain(&self, target: &mut T) {
        loop {
            let action = self.0.borrow_mut().pop_front();
            match action {
                Some(action) => action(target),
                None => break,
            }
        }
    }
}
