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
pub struct AppState {
    pub uploads: BTreeMap<Key, f32>,
    pub sticker_packages: Vec<StickerPackage>,
    pub sticker_packages_failed: bool,
    pub detail: Vec<DetailPage>,
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
    pub self_contact: Friend,
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
    pub reaches_start: bool,
    pub reaches_end: bool,
    pub composer: String,
    pub composer_reply: Option<crate::store::ReplyReference>,
    pub submitting: Option<(ChatId, String)>,
    pub prefs: Prefs,
    pub error: Option<StorageFailure>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            uploads: BTreeMap::new(),
            sticker_packages: vec![],
            sticker_packages_failed: false,
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
            self_contact: Friend::default(),
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
            reaches_start: false,
            reaches_end: false,
            composer: String::new(),
            composer_reply: None,
            submitting: None,
            prefs: Prefs::default(),
            error: None,
        }
    }
}
impl AppState {
    pub fn contact(&self, mid: &str) -> Option<&Friend> {
        if !mid.is_empty() && mid == self.profile.mid {
            return Some(&self.self_contact);
        }
        self.contacts
            .get(mid)
            .or_else(|| self.friends.iter().find(|f| f.id == mid))
            .or_else(|| {
                self.members
                    .iter()
                    .find(|m| m.contact.id == mid)
                    .map(|m| &m.contact)
            })
    }
    pub fn reply_target(&self, message_id: &str) -> Option<&StoredMessage> {
        self.messages
            .values()
            .find(|m| {
                m.server_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == message_id)
            })
            .or_else(|| {
                ServerId::new(message_id.to_owned())
                    .ok()
                    .and_then(|id| self.reply_targets.get(&id))
            })
    }
    pub fn profile_own(&self, mid: &str) -> bool {
        mid == self.profile.mid
            || self
                .members
                .iter()
                .any(|m| m.is_self && m.contact.id == mid)
    }
    pub fn profile_friend(&self, mid: &str) -> bool {
        !self.profile_own(mid)
            && self.contact(mid).is_some_and(|f| {
                f.profile.account_type == 0
                    && (f.profile.relation == Some(0)
                        || (f.profile.relation.is_none()
                            && self.friends.iter().any(|r| r.id == mid && !r.group)))
            })
    }
    pub fn profile_addable(&self, mid: &str) -> bool {
        !self.profile_own(mid)
            && !self.profile_friend(mid)
            && self
                .contact(mid)
                .is_some_and(|f| f.name.is_some() && f.profile.account_type == 0)
    }
    pub fn profile_available(&self, mid: &str) -> bool {
        let friend = self.profile_friend(mid);
        !self.profile_own(mid)
            && self.contact(mid).is_some_and(|f| {
                friend || (f.name.is_some() && self.chats.contains_key(&ChatId(mid.into())))
            })
    }
    pub fn push_detail(&mut self, page: DetailPage) {
        self.detail.push(page);
        self.panel = Panel::Detail;
        self.action_error = None;
    }
    pub fn pop_detail(&mut self) {
        self.detail.pop();
        self.finish_detail_navigation();
    }
    pub fn close_detail(&mut self) {
        self.detail.clear();
        self.finish_detail_navigation();
    }
    fn finish_detail_navigation(&mut self) {
        self.panel = if self.detail.is_empty() {
            Panel::Chats
        } else {
            Panel::Detail
        };
        self.action_error = None;
    }
    pub fn detail_page(&self) -> Option<&DetailPage> {
        self.detail.last()
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
        Some(Intent::Send {
            chat,
            text,
            reply: self.composer_reply.clone(),
        })
    }

    pub fn select(&mut self, chat: ChatId) -> Intent {
        if self.selected.as_ref() != Some(&chat) {
            self.composer.clear();
            self.composer_reply = None;
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
                    &request,
                    crate::engine::Refresh::Recent(chat) | crate::engine::Refresh::Older(chat, _)
                        if self.selected.as_ref() == Some(chat)
                ) {
                    self.window_loading = false;
                }
                self.refresh_failures.insert(request, error);
            }
            Delta::RefreshRecovered(request) => {
                self.refresh_failures.remove(&request);
            }
            Delta::StickerPackages(rows) => {
                self.sticker_packages = rows;
                self.sticker_packages_failed = false;
            }
            Delta::StickerPackagesFailed => self.sticker_packages_failed = true,
            Delta::Fatal => {
                self.connection = Connection::Fatal;
                self.submitting = None;
            }
            Delta::Reset => *self = Self::default(),
            Delta::Submitted { chat, text } => {
                if self.selected.as_ref() == Some(&chat) && self.composer == text {
                    self.composer.clear();
                }
                if self.selected.as_ref() == Some(&chat) {
                    self.composer_reply = None;
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
            Delta::Profile(p) => {
                self.self_contact = Friend {
                    id: p.mid.clone(),
                    name: Some(p.display_name.clone()),
                    status: p.status_message.clone(),
                    avatar: p
                        .picture_path
                        .as_deref()
                        .and_then(crate::client::desktop::assets::Asset::avatar),
                    ..Default::default()
                };
                self.profile = p;
            }
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
                self.composer_reply = draft.as_ref().and_then(|d| d.reply.clone());
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
            Delta::ReactionFailed(_)
            | Delta::CreatedChat(_)
            | Delta::Notify(_)
            | Delta::AssetEvicted { .. }
            | Delta::Original { .. }
            | Delta::Asset { .. }
            | Delta::File { .. }
            | Delta::Pair(_)
            | Delta::PairFailed { .. }
            | Delta::MoreChats(_)
            | Delta::MediaFailed { .. }
            | Delta::UnsendFailed(_)
            | Delta::InternalError(_)
            | Delta::ProfileAction { .. }
            | Delta::Members { .. }
            | Delta::Chats { .. }
            | Delta::Window { .. }
            | Delta::Draft { .. }
            | Delta::Copy { .. } => {}
        }
    }
}

#[cfg(test)]
mod content_model_tests {
    use super::*;

    #[test]
    fn stale_chat_failure_does_not_finish_selected_history_loading() {
        let mut state = AppState::default();
        let a = ChatId("a".into());
        let b = ChatId("b".into());
        state.select(a.clone());
        state.select(b.clone());
        state.apply(Delta::RefreshFailed {
            request: Refresh::Recent(a),
            error: crate::client::error::Failure::NotDelivered,
        });
        assert!(
            state.window_loading,
            "A's late failure exposed B's empty state"
        );
        state.apply(Delta::RefreshFailed {
            request: Refresh::Recent(b),
            error: crate::client::error::Failure::NotDelivered,
        });
        assert!(!state.window_loading);
    }

    #[test]
    fn own_member_resolves_profile_and_survives_storage() {
        let mut state = AppState::default();
        state.apply(Delta::Profile(Profile {
            mid: "self".into(),
            display_name: "My name".into(),
            status_message: "My status".into(),
            picture_path: Some("/self-photo".into()),
        }));
        let member = Member {
            contact: Friend {
                id: "self".into(),
                ..Default::default()
            },
            is_self: true,
        };
        state.members.push(member.clone());
        let contact = state.contact("self").unwrap();
        assert_eq!(contact.name.as_deref(), Some("My name"));
        assert_eq!(contact.status, "My status");
        assert_eq!(
            contact.avatar,
            crate::client::desktop::assets::Asset::avatar("/self-photo")
        );
        state.apply(Delta::Profile(Profile {
            mid: "self".into(),
            display_name: "Renamed".into(),
            ..Default::default()
        }));
        let contact = state.contact("self").unwrap();
        assert_eq!(contact.name.as_deref(), Some("Renamed"));
        assert!(contact.avatar.is_none());
        let mut store = Store::open_in_memory().unwrap();
        let chat = ChatId("group".into());
        store.save_members(&chat, &[member]).unwrap();
        let members = store.members(&chat).unwrap();
        assert_eq!(members.len(), 1);
        assert!(members[0].is_self);
    }
    #[test]
    fn selecting_keeps_navigation_and_waits_for_matching_window() {
        let mut state = AppState::default();
        state.push_detail(DetailPage::Profile("member".into()));
        state.panel = Panel::Settings;
        state.select(ChatId("chat".into()));
        assert_eq!(state.panel, Panel::Settings);
        assert_eq!(
            state.detail_page(),
            Some(&DetailPage::Profile("member".into()))
        );
        assert!(state.window_loading);
        state.apply(Delta::Window {
            sub: state.window_sub,
            upsert: vec![],
            remove: vec![],
            gaps: vec![],
            reaches_start: true,
            reaches_end: true,
            replies: vec![],
            unread_anchor: None,
        });
        assert!(!state.window_loading);
    }
    #[test]
    fn profile_rules_keep_unnamed_friends_but_disable_unresolved_members() {
        let mut state = AppState::default();
        let chat_id = ChatId("member".into());
        state.chats.insert(
            chat_id.clone(),
            ChatListItem {
                chat: ChatInput {
                    id: chat_id.clone(),
                    ..Default::default()
                },
                unread_count: 0,
                preview: None,
                activity_ms: 0,
                failed_count: 0,
                draft: None,
            },
        );
        assert!(!state.profile_available("member"));
        let friend = Friend {
            id: "member".into(),
            name: None,
            ..Default::default()
        };
        state.friends.push(friend.clone());
        assert!(state.profile_friend("member"));
        assert!(!state.profile_addable("member"));
        assert!(state.profile_available("member"));
        let mut contact = friend;
        contact.profile.relation = Some(2);
        state.contacts.insert(contact.id.clone(), contact);
        assert!(!state.profile_friend("member"));
        assert!(!state.profile_addable("member"));
        assert!(!state.profile_available("member"));
        state.chats.remove(&chat_id);
        state.contacts.get_mut("member").unwrap().name = Some("Member".into());
        assert!(state.profile_addable("member"));
        assert!(!state.profile_available("member"));
        let contact = state.contacts.get_mut("member").unwrap();
        contact.name = Some("Official".into());
        contact.profile.account_type = 1;
        assert!(!state.profile_addable("member"));
    }
}
