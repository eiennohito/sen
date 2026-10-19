//! UI-independent projection. Deltas are applied immediately in FIFO order.
use crate::{engine::*, store::*};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Chats,
    Friends,
    Settings,
    Detail,
    Viewer,
    SignIn,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailPage {
    Profile(String),
    GroupInfo(ChatId),
    Members(ChatId),
}
#[derive(Clone, Debug)]
pub struct DetailEntry {
    pub page: DetailPage,
    pub opener: String,
}
pub struct AppState {
    pub uploads: BTreeMap<Key, f32>,
    pub sticker_packages: Vec<StickerPackage>,
    pub reaction_failure: Option<Key>,
    pub created_chat: Option<std::result::Result<ChatId, crate::engine::CreateFailure>>,
    pub detail: Vec<DetailEntry>,
    pub contacts: BTreeMap<String, crate::store::Friend>,
    pub group_info: BTreeMap<ChatId, serde_json::Value>,
    pub notices: BTreeMap<String, serde_json::Value>,
    pub action: Option<crate::backend::ProfileAction>,
    pub action_error: Option<crate::engine::ProfileFailure>,
    pub friends: Vec<crate::store::Friend>,
    pub members: Vec<crate::store::Member>,
    pub connection: Connection,
    pub panel: Panel,
    pub profile: Profile,
    pub chats: BTreeMap<ChatId, ChatListItem>,
    pub chat_order: Vec<ChatId>,
    pub chat_sub: u64,
    pub window_sub: u64,
    pub window_loading: bool,
    pub refresh_failures:
        std::collections::BTreeMap<crate::engine::Refresh, crate::client::error::Failure>,
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
    pub prefs: Prefs,
    pub error: Option<StorageFailure>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            uploads: BTreeMap::new(),
            sticker_packages: vec![],
            reaction_failure: None,
            created_chat: None,
            detail: vec![],
            contacts: BTreeMap::new(),
            group_info: BTreeMap::new(),
            notices: BTreeMap::new(),
            action: None,
            action_error: None,
            friends: vec![],
            members: vec![],
            connection: Connection::Local,
            panel: Panel::Chats,
            profile: Profile::default(),
            chats: BTreeMap::new(),
            chat_order: vec![],
            chat_sub: 1,
            window_sub: 0,
            window_loading: false,
            refresh_failures: Default::default(),
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
            prefs: Prefs::default(),
            error: None,
        }
    }
}
impl AppState {
    pub fn push_detail(&mut self, page: DetailPage, opener: String) {
        self.detail.push(DetailEntry { page, opener });
        self.panel = Panel::Detail;
        self.action_error = None;
    }
    pub fn pop_detail(&mut self, all: bool) -> Option<String> {
        let entry = if all {
            let first = self.detail.first().cloned();
            self.detail.clear();
            first
        } else {
            self.detail.pop()
        };
        if self.detail.is_empty() {
            self.panel = Panel::Chats;
        } else {
            self.panel = Panel::Detail;
        }
        self.action_error = None;
        entry.map(|e| e.opener)
    }
    pub fn detail_page(&self) -> Option<&DetailPage> {
        self.detail.last().map(|e| &e.page)
    }
    pub fn can_send(&self) -> bool {
        self.connection == Connection::Online
            && self
                .selected
                .as_ref()
                .is_some_and(|id| self.chats.get(id).is_none_or(|c| !c.chat.left))
            && self.submitting.is_none()
            && !self.composer.trim().is_empty()
    }
    pub fn submit(&mut self) -> Option<Intent> {
        if self.composer.trim().is_empty()
            || self.submitting.is_some()
            || self
                .selected
                .as_ref()
                .and_then(|id| self.chats.get(id))
                .is_some_and(|c| c.chat.left)
        {
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
        if self.selected.as_ref() != Some(&chat) {
            self.detail.clear();
        }
        self.window_sub += 1;
        self.window_anchor = Anchor::Unread;
        self.selected = Some(chat.clone());
        self.window_loading = true;
        self.members.clear();
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
    pub fn search_chats(&mut self, query: String) -> Intent {
        self.chat_sub += 1;
        Intent::SearchChats {
            sub: self.chat_sub,
            query,
        }
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
    pub fn apply(&mut self, delta: Delta) {
        match delta {
            Delta::UploadFinished(key) => {
                self.uploads.remove(&key);
            }
            Delta::UploadProgress { key, fraction } => {
                if self
                    .messages
                    .get(&key)
                    .is_none_or(|row| row.delivery == crate::store::Delivery::Pending)
                {
                    self.uploads.insert(key, fraction);
                }
            }
            Delta::RefreshFailed { request, error } => {
                if matches!(
                    request,
                    crate::engine::Refresh::Recent(_)
                        | crate::engine::Refresh::Older(..)
                        | crate::engine::Refresh::Newer(..)
                ) {
                    self.window_loading = false;
                }
                self.refresh_failures.insert(request, error);
            }
            Delta::RefreshRecovered(request) => {
                self.refresh_failures.remove(&request);
            }
            Delta::StickerPackages(rows) => self.sticker_packages = rows,
            Delta::ReactionFailed(key) => self.reaction_failure = Some(key),
            Delta::CreatedChat(result) => self.created_chat = Some(result),
            Delta::Fatal => {
                self.connection = Connection::Fatal;
                self.submitting = None;
            }
            Delta::Reset => *self = Self::default(),
            Delta::Submitted { chat, text } => {
                if self.selected.as_ref() == Some(&chat) && self.composer == text {
                    self.composer.clear();
                }
                self.submitting = None;
            }
            Delta::Contact(row) => {
                for f in &mut self.friends {
                    if f.id == row.id {
                        *f = row.clone();
                    }
                }
                for member in &mut self.members {
                    if member.contact.id == row.id {
                        let invited = member.contact.profile.invited;
                        member.contact = row.clone();
                        member.contact.profile.invited = invited;
                    }
                }
                self.contacts.insert(row.id.clone(), row);
            }
            Delta::GroupInfo { chat, data } => {
                self.group_info.insert(chat, data);
            }
            Delta::Notice { mid, data } => {
                self.notices.insert(mid, data);
            }
            Delta::ProfileAction { action, error } if self.action.as_ref() == Some(&action) => {
                let relevant = match (self.detail_page(), &action) {
                    (None, _) => true,
                    (
                        Some(DetailPage::Profile(id)),
                        crate::backend::ProfileAction::Rename { mid, .. }
                        | crate::backend::ProfileAction::AddFriend { mid, .. },
                    ) => id == mid,
                    (
                        Some(DetailPage::GroupInfo(id)),
                        crate::backend::ProfileAction::Mute { chat, .. }
                        | crate::backend::ProfileAction::Leave { chat },
                    ) => id == chat,
                    _ => false,
                };
                self.action = None;
                self.action_error = if relevant { error } else { None };
            }
            Delta::Friends(rows) => {
                for r in &rows {
                    self.contacts.insert(r.id.clone(), r.clone());
                }
                self.friends = rows;
            }
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
                self.window_loading = false;
                for key in remove {
                    self.uploads.remove(&key);
                    self.messages.remove(&key);
                }
                for row in upsert {
                    if row.delivery != crate::store::Delivery::Pending {
                        self.uploads.remove(&row.key);
                    }
                    self.messages.insert(row.key, row);
                }
                self.reply_targets = replies
                    .into_iter()
                    .filter_map(|m| Some((m.server_id.clone()?, m)))
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
            Delta::Prefs(prefs) => {
                if self.selected.is_none()
                    && let Some(chat) = &prefs.last_chat
                {
                    self.selected = Some(chat.clone());
                    self.window_loading = true;
                    self.window_sub = 1;
                    self.window_anchor = Anchor::Unread;
                }
                self.prefs = prefs;
            }
            Delta::StorageFailed(e) => {
                self.error = Some(e);
                self.submitting = None;
            }
            Delta::StoreSaved => self.error = None,
            _ => {}
        }

        self.unread_boundary_unknown = self.unread_divider.is_none()
            && self
                .selected
                .as_ref()
                .and_then(|id| self.chats.get(id))
                .is_some_and(|c| c.chat.unread_count > 0);
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
