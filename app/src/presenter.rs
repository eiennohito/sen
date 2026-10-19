//! Slint is a presentation target; the controller owns the projection.
use crate::*;
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
    rendered_chats: std::collections::BTreeMap<ChatId, (ChatInputs, ChatRow)>,
    rendered: std::collections::BTreeMap<Key, (StoredMessage, Option<Friend>, MessageRow)>,
}
// Asset pixels are patched in place; these are the non-asset row inputs.
#[derive(PartialEq)]
struct ChatInputs {
    chat: ChatInput,
    title: String,
    preview: Option<StoredMessage>,
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
impl Presenter {
    pub fn install(&self, ui: &AppWindow) {
        ui.set_chats(ModelRc::new(self.chats.clone()));
        ui.set_messages(ModelRc::new(self.messages.clone()));
        ui.set_friends(ModelRc::new(self.friends.clone()));
        ui.set_members(ModelRc::new(self.members.clone()));
    }
    pub fn present_compose(&self, ui: &AppWindow, state: &AppState) {
        let global = ui.global::<State>();
        let selected = global
            .get_new_chat_selected()
            .iter()
            .map(|f| f.id.to_string())
            .collect::<Vec<_>>();
        let friends = state
            .friends
            .iter()
            .filter(|f| !f.group && f.id != state.profile.mid)
            .map(|f| ComposeFriend {
                id: f.id.clone().into(),
                name: f.name.clone().into(),
                initial: name_initial(&f.name),
                avatar: f
                    .avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
                selected: selected.contains(&f.id),
            })
            .collect::<Vec<_>>();
        let chosen = friends
            .iter()
            .filter(|f| f.selected)
            .cloned()
            .collect::<Vec<_>>();
        global.set_new_chat_count(chosen.len() as i32);
        global.set_new_chat_placeholder(
            chosen
                .iter()
                .map(|f| f.name.to_string())
                .collect::<Vec<_>>()
                .join(", ")
                .into(),
        );
        global.set_new_chat_selected(ModelRc::new(VecModel::from(chosen)));
        global.set_new_chat_friends(ModelRc::new(VecModel::from(
            friends
                .into_iter()
                .filter(|f| {
                    f.name
                        .to_lowercase()
                        .contains(&global.get_new_chat_query().to_lowercase())
                })
                .collect::<Vec<_>>(),
        )));
        let picture = |content: &Content| {
            content
                .image()
                .and_then(|a| self.images.get(&a.key()))
                .cloned()
                .unwrap_or_default()
        };
        global.set_sticker_tabs(ModelRc::new(VecModel::from(
            state
                .sticker_packages
                .iter()
                .map(|p| StickerTab {
                    name: p.name.clone().into(),
                    picture: p.stickers.first().map(picture).unwrap_or_default(),
                })
                .collect::<Vec<_>>(),
        )));
        let items = state
            .sticker_packages
            .get(global.get_sticker_package() as usize)
            .map(|p| {
                p.stickers
                    .iter()
                    .enumerate()
                    .map(|(index, s)| StickerItem {
                        index: index as i32,
                        picture: picture(s),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        global.set_sticker_items(ModelRc::new(VecModel::from(items)));
    }
    fn row(&mut self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let sender = state
            .contacts
            .get(&m.sender_id)
            .or_else(|| state.friends.iter().find(|f| f.id == m.sender_id))
            .or_else(|| {
                state
                    .members
                    .iter()
                    .find(|f| f.contact.id == m.sender_id)
                    .map(|m| &m.contact)
            })
            .cloned();
        if !matches!(m.body, Content::System { .. })
            && let Some((stored, identity, row)) = self.rendered.get(&m.key)
            && stored == m
            && identity == &sender
            && (m.delivery != Delivery::Pending
                || row.upload_progress == state.uploads.get(&m.key).copied().unwrap_or(0.))
        {
            return row.clone();
        }
        let mut view = content_view(&m.body);
        if matches!(m.body, Content::System { .. }) {
            view.rich.system_text = crate::system_events::text(&m.body, state, &m.sender_id);
        }
        view.rich.reply = m.reply.as_deref().cloned();
        view.id = m.key.0.to_string();
        view.sender_id = m.sender_id.clone();
        view.sender_name = sender.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| {
            if m.sender_id.is_empty() {
                "Unknown sender".into()
            } else {
                format!(
                    "Member · {}",
                    m.sender_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                )
            }
        });
        view.sender_key = sender
            .as_ref()
            .and_then(|s| s.avatar.as_ref())
            .map(|a| a.key())
            .unwrap_or_default();
        view.timestamp = m.sort.sort_ts;
        view.delivered_time = m.sort.sort_ts;
        view.outgoing = m.outgoing;
        view.skip_badge = m.skip_badge;
        if !m.body.is_withdrawn() {
            view.reactions = reaction_views(&m.reactions);
        }
        let mut row = message_row(view);
        if m.body.is_withdrawn() {
            row.kind = MessageKind::Withdrawn;
            row.body = "Message unsent".into();
            row.image_key = "".into();
        }
        row.delivery = match &m.delivery {
            Delivery::Pending => "Sending…",
            Delivery::Sent => "Sent",
            Delivery::Read => "Read",
            Delivery::Rejected { .. } => "Rejected",
            Delivery::Uncertain => "Unconfirmed",
        }
        .into();
        if let Some(_item) = m
            .body
            .image()
            .or_else(|| m.body.attachment())
            .and_then(|asset| asset.local_attachment())
        {
            row.local_media = true;
            row.upload_progress = state.uploads.get(&m.key).copied().unwrap_or(0.);
            row.uploading = m.delivery == Delivery::Pending && row.upload_progress < 1.;
            row.file_missing = matches!(
                m.delivery,
                Delivery::Rejected {
                    reason: line::backend::RejectReason::FileMissing
                }
            );
        }
        row.can_react = m.server_id.is_some()
            && matches!(m.delivery, Delivery::Sent | Delivery::Read)
            && !matches!(m.body, Content::System { .. } | Content::Withdrawn);
        row.reaction_count = m.reactions.len() as i32;
        row.own_reaction = m
            .reactions
            .iter()
            .find(|r| r.actor == state.profile.mid)
            .and_then(|r| {
                if let ReactionKind::Predefined(n) = r.kind {
                    Some(n as i32)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        row.can_unsend = m.outgoing
            && m.server_id.is_some()
            && matches!(m.delivery, Delivery::Sent | Delivery::Read)
            && !matches!(m.body, Content::System { .. } | Content::Withdrawn);
        row.recoverable = matches!(m.delivery, Delivery::Rejected { .. } | Delivery::Uncertain);
        row.delivery_detail = match &m.delivery {
            Delivery::Rejected { reason } => text::rejection(reason).into(),
            Delivery::Uncertain => {
                "The message may have arrived. Check before sending again.".into()
            }
            _ => String::new(),
        }
        .into();
        if let Some(image) = self.images.get(row.image_key.as_str()) {
            row.picture = image.clone();
            row.has_picture = true;
        }
        if let Some(image) = self.images.get(row.sender_key.as_str()) {
            row.sender_avatar = image.clone();
            row.has_sender_avatar = true;
        }
        if let Some(failure) = self.failures.get(row.image_key.as_str()) {
            row.image_failed = true;
            row.image_failure = format!("{failure:?}").into();
            row.image_error = text::image_text(*failure).into();
            row.image_retryable = failure.retriable();
        }
        if let Some(runs) = row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>() {
            for i in 0..runs.row_count() {
                let mut run = runs.row_data(i).unwrap();
                if let Some(picture) = self.images.get(run.image_key.as_str()) {
                    run.picture = picture.clone();
                    run.has_picture = true;
                    runs.set_row_data(i, run);
                }
            }
        }
        if let Some(reactions) = row
            .reactions
            .as_any()
            .downcast_ref::<VecModel<ReactionRow>>()
        {
            for i in 0..reactions.row_count() {
                let mut reaction = reactions.row_data(i).unwrap();
                if let Some(image) = self.images.get(reaction.image_key.as_str()) {
                    reaction.picture = image.clone();
                    reaction.has_picture = true;
                    reactions.set_row_data(i, reaction);
                }
            }
        }
        links::decorate_message(&mut row, "Contact");
        self.rendered
            .insert(m.key, (m.clone(), sender, row.clone()));
        row
    }
    pub fn present(&mut self, ui: &AppWindow, state: &AppState, query: &str, category: i32) {
        self.present_compose(ui, state);

        let friends = state
            .friends
            .iter()
            .filter(|f| {
                f.name
                    .to_lowercase()
                    .contains(&ui.get_friend_query().to_lowercase())
            })
            .map(|f| FriendRow {
                id: f.id.clone().into(),
                name: f.name.clone().into(),
                initial: name_initial(&f.name),
                status: f.status.clone().into(),
                group: f.group,
                avatar_key: f
                    .avatar
                    .as_ref()
                    .map(|a| a.key())
                    .unwrap_or_default()
                    .into(),
                avatar: f
                    .avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut grouped = Vec::new();
        for (group, label, expanded) in [
            (true, "Groups", ui.get_groups_expanded()),
            (false, "Friends", ui.get_friends_expanded()),
        ] {
            let rows = friends
                .iter()
                .filter(|row| row.group == group)
                .cloned()
                .collect::<Vec<_>>();
            if state.friends.is_empty() || (!ui.get_friend_query().is_empty() && rows.is_empty()) {
                continue;
            }
            grouped.push(FriendRow {
                id: format!("header-{label}").into(),
                name: label.into(),
                count: rows.len() as i32,
                header: true,
                group,
                ..Default::default()
            });
            if expanded {
                grouped.extend(rows);
            }
        }
        patch(
            &self.friends,
            &mut self.friend_keys,
            grouped.into_iter().map(|r| (r.id.to_string(), r)).collect(),
        );
        let mut ordered_members = state.members.iter().collect::<Vec<_>>();
        ordered_members.sort_by_key(|m| (m.contact.profile.invited, !m.is_self));
        let members = ordered_members
            .into_iter()
            .filter(|m| {
                let query = ui.get_member_query().to_lowercase();
                m.contact.name.to_lowercase().contains(&query)
                    || m.contact.status.to_lowercase().contains(&query)
                    || (m.is_self && "you".contains(&query))
            })
            .map(|m| MemberRow {
                id: m.contact.id.clone().into(),
                name: m.contact.name.clone().into(),
                initial: name_initial(&m.contact.name),
                status: m.contact.status.clone().into(),
                is_self: m.is_self,
                friend: state
                    .friends
                    .iter()
                    .any(|f| f.id == m.contact.id && !f.group)
                    || m.contact.profile.relation == Some(0),
                invited: m.contact.profile.invited,
                avatar_key: m
                    .contact
                    .avatar
                    .as_ref()
                    .map(|a| a.key())
                    .unwrap_or_default()
                    .into(),
                avatar: m
                    .contact
                    .avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        patch(
            &self.members,
            &mut self.member_keys,
            members.into_iter().map(|r| (r.id.to_string(), r)).collect(),
        );
        let g = ui.global::<State>();
        g.set_detail_depth(state.detail.len() as i32);
        g.set_invitee_count(
            state
                .members
                .iter()
                .filter(|m| m.contact.profile.invited)
                .count() as i32,
        );
        g.set_detail_busy(state.action.is_some());
        g.set_detail_running(
            state
                .action
                .as_ref()
                .map(|a| match a {
                    line::backend::ProfileAction::Rename { .. } => "rename",
                    line::backend::ProfileAction::AddFriend { .. } => "add",
                    line::backend::ProfileAction::Mute { .. } => "mute",
                    line::backend::ProfileAction::Leave { .. } => "leave",
                })
                .unwrap_or("")
                .into(),
        );
        g.set_detail_error(
            state
                .action_error
                .map(text::profile_failure)
                .unwrap_or_default()
                .into(),
        );
        g.set_chat_left(
            state
                .selected
                .as_ref()
                .is_some_and(|id| state.chats.get(id).is_some_and(|c| c.chat.left)),
        );
        if let Some(page) = state.detail_page() {
            use line::controller::DetailPage;
            let (id, kind) = match page {
                DetailPage::Profile(id) => (id.as_str(), 0),
                DetailPage::GroupInfo(id) => (id.0.as_str(), 1),
                DetailPage::Members(id) => (id.0.as_str(), 2),
            };
            g.set_detail_page(kind);
            let own = id == "self"
                || state.profile.mid == *id
                || state
                    .members
                    .iter()
                    .any(|m| m.is_self && m.contact.id == id);
            let f = state
                .contacts
                .get(id)
                .or_else(|| state.friends.iter().find(|f| f.id == id))
                .or_else(|| {
                    state
                        .members
                        .iter()
                        .find(|m| m.contact.id == id)
                        .map(|m| &m.contact)
                });
            let c = state.chats.get(&ChatId(id.into())).map(|c| &c.chat);
            let name = if own {
                state.profile.display_name.as_str()
            } else if kind != 0 {
                c.map(|c| c.title.as_str())
                    .or_else(|| f.map(|f| f.name.as_str()))
                    .unwrap_or("Group")
            } else {
                f.map(|f| f.name.as_str()).unwrap_or("Unknown member")
            };
            let room_title = state
                .members
                .iter()
                .filter(|m| !m.is_self && !m.contact.profile.invited)
                .map(|m| m.contact.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let name = if kind != 0
                && (id.starts_with('r') || c.is_some_and(|c| c.id.0.starts_with('r')))
                && !room_title.is_empty()
            {
                room_title.as_str()
            } else {
                name
            };
            let status = if own {
                state.profile.status_message.as_str()
            } else {
                f.map(|f| f.status.as_str()).unwrap_or("")
            };
            let friend = !own
                && kind == 0
                && f.is_some_and(|f| f.name != "Unknown member")
                && f.is_some_and(|f| {
                    f.profile.relation == Some(0)
                        || (f.profile.relation.is_none()
                            && state.friends.iter().any(|r| r.id == id && !r.group))
                });
            let official = f.is_some_and(|f| f.profile.account_type != 0);
            let friend = friend && !official;
            ui.set_profile_id(id.into());
            ui.set_profile_name(name.into());
            ui.set_profile_status(status.into());
            g.set_profile_initial(name_initial(name));
            g.set_profile_own(own);
            g.set_profile_friend(friend);
            g.set_profile_addable(
                !own && kind == 0
                    && !friend
                    && !official
                    && f.is_some_and(|f| f.name != "Unknown member"),
            );
            ui.set_profile_available(
                !own && kind == 0 && name != "Unknown member" && (friend || c.is_some()),
            );
            ui.set_profile_group(kind != 0);
            g.set_profile_original(
                f.and_then(|f| {
                    let original = f.profile.original_name.as_deref()?;
                    (original != name).then_some(original)
                })
                .unwrap_or("")
                .into(),
            );
            g.set_profile_favourite(f.is_some_and(|f| f.profile.favorite_time.unwrap_or(0) > 0));
            g.set_profile_notice(
                if !friend
                    && state
                        .notices
                        .get(id)
                        .is_some_and(|v| v["notice"].as_object().is_some_and(|o| !o.is_empty()))
                {
                    "Be careful when sharing personal information or sending money."
                } else {
                    ""
                }
                .into(),
            );
            g.set_profile_name_runs(rich_content::runs(name, &[]));
            g.set_profile_status_runs(rich_content::runs(status, &[]));
            let avatar = if own {
                state
                    .profile
                    .picture_path
                    .as_deref()
                    .and_then(line::client::desktop::assets::Asset::avatar)
            } else {
                f.and_then(|f| f.avatar.clone())
                    .or_else(|| c.and_then(|c| c.avatar.clone()))
            };
            ui.set_profile_avatar(
                avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            );
            g.set_group_count(
                c.and_then(|c| c.member_count)
                    .map(|n| n as i32)
                    .unwrap_or(-1),
            );
            g.set_group_muted(c.is_some_and(|c| c.muted));
            g.set_group_left(c.is_some_and(|c| c.left));
            g.set_group_room(id.starts_with('r') || c.is_some_and(|c| c.id.0.starts_with('r')));
            g.set_group_writable(
                state
                    .group_info
                    .get(&ChatId(id.into()))
                    .is_some_and(|v| v.is_object()),
            );
        }
        let mut chats = Vec::new();
        for (index, id) in state.chat_order.iter().enumerate() {
            let item = &state.chats[id];
            let c = &item.chat;
            let title = chat_title(c, state);
            if !title.to_lowercase().contains(query)
                || (category == 1 && c.is_group)
                || (category == 2 && !c.is_group)
            {
                continue;
            }
            let inputs = ChatInputs {
                chat: c.clone(),
                title: title.to_owned(),
                preview: item.preview.clone(),
                activity: item.activity_ms,
                failed: item.failed_count,
                draft: item.draft.clone(),
                offline: state.connection != Connection::Online,
                index,
                text_size: ui.get_text_size(),
            };
            if !matches!(
                item.preview.as_ref().map(|m| &m.body),
                Some(Content::System { .. })
            ) && let Some((old, row)) = self.rendered_chats.get(id)
                && *old == inputs
            {
                chats.push((id.clone(), row.clone()));
                continue;
            }
            let mut row = ChatRow {
                id: id.0.clone().into(),
                index: index as i32,
                title: title.into(),
                initial: name_initial(title),
                unread: c.unread_count as i32,
                failed: item.failed_count > 0,
                draft: item.draft.as_ref().is_some_and(|d| !d.text.is_empty()),
                draft_preview: item
                    .draft
                    .as_ref()
                    .map(|d| single_line(&d.text))
                    .unwrap_or_default()
                    .into(),
                unread_unknown: c.unread_stale || state.connection != Connection::Online,
                muted: c.muted,
                group: c.is_group,
                member_count: c.member_count.unwrap_or(0) as i32,
                time: list_time(item.activity_ms).into(),
                ..Default::default()
            };
            if let Some(m) = &item.preview {
                let message = self.row(m, state);
                preview::set(&mut row, &message, ui.get_text_size());
            } else if item.activity_ms == 0 {
                row.time = "New".into();
                row.preview = "No messages yet".into();
            }
            if let Some(asset) = c.avatar.as_ref() {
                row.avatar_key = asset.key().into();
                if let Some(image) = self.images.get(row.avatar_key.as_str()) {
                    row.avatar = image.clone();
                }
            }
            self.rendered_chats
                .insert(id.clone(), (inputs, row.clone()));
            chats.push((id.clone(), row));
        }
        patch(&self.chats, &mut self.chat_keys, chats);
        self.rendered_chats
            .retain(|id, _| state.chats.contains_key(id));
        let rows = state
            .rows()
            .into_iter()
            .filter(|m| {
                if matches!(m.body, Content::System { .. })
                    && crate::system_events::text(&m.body, state, &m.sender_id).is_none()
                {
                    #[cfg(debug_assertions)]
                    eprintln!("Unknown chat event: {:?}", m.body);
                    return false;
                }
                true
            })
            .collect::<Vec<_>>();
        // Choose on first population, then retain the mode for this conversation.
        // Switching delegate types during a live arrival would lose selection
        // and the current reading anchor at the size boundary.
        if self.messages.row_count() == 0 || self.history_chat != state.selected {
            ui.global::<State>().set_history_virtual(rows.len() > 60);
            self.history_chat = state.selected.clone();
        }
        let mut messages = rows
            .iter()
            .map(|m| {
                let mut row = self.row(m, state);
                if let Some(target) = state.reply_targets.get(&ServerId(row.reply_id.to_string())) {
                    let quoted = self.row(target, state);
                    let name = if quoted.outgoing {
                        "You"
                    } else {
                        quoted.sender_name.as_str()
                    };
                    row.reply_sender = name.into();
                    row.reply_preview = quoted.body;
                }
                row
            })
            .collect::<Vec<_>>();
        group_messages(&mut messages);
        let group = state
            .selected
            .as_ref()
            .and_then(|id| state.chats.get(id))
            .is_some_and(|c| c.chat.is_group);
        if !group {
            for row in &mut messages {
                row.sender_name = "".into();
                row.has_sender_avatar = false;
                row.sender_avatar = Default::default();
            }
        }
        let mut projected = Vec::new();
        for (m, row) in rows.iter().zip(messages) {
            if state.gaps.iter().any(|g| g.after == m.key) {
                projected.push((
                    RowKey::Gap(m.key),
                    MessageRow {
                        id: format!("gap-{}", m.key.0).into(),
                        body: "Loading messages…".into(),
                        kind: MessageKind::System,
                        ..Default::default()
                    },
                ));
            }
            projected.push((RowKey::Message(m.key), row));
        }
        patch(&self.messages, &mut self.message_keys, projected);
        self.rendered.retain(|k, _| {
            state.messages.contains_key(k)
                || state
                    .chats
                    .values()
                    .any(|c| c.preview.as_ref().is_some_and(|m| m.key == *k))
        });
        ui.global::<State>().set_panel(match &state.panel {
            Panel::Chats | Panel::Friends => PanelState::None,
            Panel::Settings => PanelState::Settings,
            Panel::Detail => PanelState::Detail,
            Panel::Viewer => PanelState::Viewer,
            Panel::SignIn => PanelState::SignIn,
        });
        ui.set_has_chat(state.selected.is_some());
        ui.set_storage_error(
            state
                .error
                .map(crate::text::storage_failure)
                .unwrap_or("")
                .into(),
        );
        ui.global::<State>()
            .set_account_initial(name_initial(state.profile.display_name.as_str()));
        ui.global::<State>()
            .set_has_account(!state.profile.mid.is_empty());
        ui.set_account_name(state.profile.display_name.as_str().into());
        ui.global::<State>().set_connection(match state.connection {
            Connection::Local => ConnectionState::Local,
            Connection::Connecting { .. } => ConnectionState::Connecting,
            Connection::Online => ConnectionState::Online,
            Connection::Offline { .. } => ConnectionState::Offline,
            Connection::SignedOut | Connection::AuthExpired => ConnectionState::AuthExpired,
            Connection::SessionInvalid | Connection::Fatal => ConnectionState::SessionInvalid,
            Connection::SigningIn => ConnectionState::SigningIn,
        });
        ui.set_connection_status(
            match &state.connection {
                Connection::Local => "Cached conversations",
                Connection::Online => "Connected",
                Connection::Connecting { .. } => "Connecting…",
                Connection::Offline { .. } => "Offline — reconnecting automatically",
                Connection::SignedOut => "Not signed in",
                Connection::AuthExpired => "Sign in required",
                Connection::SessionInvalid => "Session unavailable",
                Connection::Fatal => "Something went wrong — restart",
                Connection::SigningIn => "Signing in…",
            }
            .into(),
        );
        ui.global::<State>()
            .set_send_pending(state.submitting.is_some());
        ui.set_send_reason(
            if state.connection == Connection::Online {
                ""
            } else {
                "Messages can be sent when connected"
            }
            .into(),
        );
        ui.set_selected_chat(
            state
                .selected
                .as_ref()
                .and_then(|id| state.chat_order.iter().position(|c| c == id))
                .map(|n| n as i32)
                .unwrap_or(-1),
        );
        if let Some(c) = state.selected.as_ref().and_then(|id| state.chats.get(id)) {
            ui.set_chat_title(chat_title(&c.chat, state).into());
            ui.global::<State>().set_chat_id(c.chat.id.0.clone().into());
            ui.global::<State>()
                .set_chat_initial(name_initial(chat_title(&c.chat, state)));
            let avatar = c
                .chat
                .avatar
                .as_ref()
                .and_then(|asset| self.images.get(&asset.key()))
                .cloned()
                .unwrap_or_default();
            ui.global::<State>().set_chat_avatar(avatar);
            ui.set_chat_group(c.chat.is_group);
            ui.set_member_count(c.chat.member_count.unwrap_or(0) as i32);
            ui.set_chat_subtitle(
                if c.chat.is_group {
                    format!("{} members", c.chat.member_count.unwrap_or(0))
                } else {
                    String::new()
                }
                .into(),
            );
        }
        if let Some(id) = &state.selected
            && !state.chats.contains_key(id)
        {
            let friend = state.friends.iter().find(|f| f.id == id.0);
            ui.set_chat_title(friend.map(|f| f.name.as_str()).unwrap_or(&id.0).into());
            ui.global::<State>().set_chat_id(id.0.clone().into());
            let title = friend.map(|f| f.name.as_str()).unwrap_or(&id.0);
            ui.global::<State>().set_chat_initial(name_initial(title));
            ui.global::<State>().set_chat_avatar(
                friend
                    .and_then(|f| f.avatar.as_ref())
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            );
            ui.set_chat_group(false);
            ui.set_member_count(0);
            ui.set_chat_subtitle("".into());
        }
        if ui.get_draft().as_str() != state.composer {
            ui.set_draft(state.composer.clone().into());
        }
        ui.global::<State>().set_unread_divider(
            state
                .unread_divider
                .map(|k| k.0.to_string())
                .unwrap_or_default()
                .into(),
        );
        ui.global::<State>()
            .set_unread_boundary_unknown(state.unread_boundary_unknown);
        ui.global::<State>()
            .set_older_start_known(state.reaches_start);
        ui.global::<State>().set_older_status(
            if state.reaches_start {
                "Beginning of conversation"
            } else {
                ""
            }
            .into(),
        );
        ui.global::<State>()
            .set_history_can_follow(state.reaches_end);
        ui.global::<State>().set_unread_conversations(
            state
                .chats
                .values()
                .filter(|c| c.chat.unread_count > 0)
                .count() as i32,
        );
        ui.global::<State>().set_unread_stale(
            state.connection != Connection::Online
                || state.chats.values().any(|c| c.chat.unread_stale),
        );
    }
    /// Patch only consumers of this key. Nested models retain their identity.
    pub fn patch_asset(&mut self, ui: &AppWindow, state: &AppState, key: &str) {
        let image = self.images.get(key);
        let failure = self.failures.get(key);
        if let Some(image) = image {
            let global = ui.global::<State>();
            for model in [
                global.get_new_chat_friends(),
                global.get_new_chat_selected(),
            ] {
                if let Some(rows) = model.as_any().downcast_ref::<VecModel<ComposeFriend>>() {
                    for i in 0..rows.row_count() {
                        let mut row = rows.row_data(i).unwrap();
                        if state.friends.iter().any(|friend| {
                            friend.id == row.id.as_str()
                                && friend
                                    .avatar
                                    .as_ref()
                                    .is_some_and(|asset| asset.key() == key)
                        }) && row.avatar != *image
                        {
                            row.avatar = image.clone();
                            rows.set_row_data(i, row);
                        }
                    }
                }
            }
            let tabs = global.get_sticker_tabs();
            if let Some(rows) = tabs.as_any().downcast_ref::<VecModel<StickerTab>>() {
                for (i, package) in state.sticker_packages.iter().enumerate() {
                    if package
                        .stickers
                        .first()
                        .and_then(Content::image)
                        .is_some_and(|asset| asset.key() == key)
                        && let Some(mut row) = rows.row_data(i)
                        && row.picture != *image
                    {
                        row.picture = image.clone();
                        rows.set_row_data(i, row);
                    }
                }
            }
            let items = global.get_sticker_items();
            if let Some(rows) = items.as_any().downcast_ref::<VecModel<StickerItem>>()
                && let Some(package) = state
                    .sticker_packages
                    .get(global.get_sticker_package() as usize)
            {
                for (i, sticker) in package.stickers.iter().enumerate() {
                    if sticker.image().is_some_and(|asset| asset.key() == key)
                        && let Some(mut row) = rows.row_data(i)
                        && row.picture != *image
                    {
                        row.picture = image.clone();
                        rows.set_row_data(i, row);
                    }
                }
            }
        }
        for (_, _, row) in self.rendered.values_mut() {
            patch_message_asset(row, key, image, failure);
        }
        for (i, id) in self.message_keys.iter().enumerate() {
            let RowKey::Message(key_id) = id else {
                continue;
            };
            if !self
                .rendered
                .get(key_id)
                .is_some_and(|(_, _, row)| uses_asset(row, key))
            {
                continue;
            }
            let mut row = self.messages.row_data(i).unwrap();
            if patch_message_asset(&mut row, key, image, failure) {
                self.messages.set_row_data(i, row);
            }
        }
        for (_, row) in self.rendered_chats.values_mut() {
            patch_chat_asset(row, key, image, failure.is_some(), ui.get_text_size());
        }
        for (i, id) in self.chat_keys.iter().enumerate() {
            if let Some((_, cached)) = self.rendered_chats.get(id)
                && cached.avatar_key == key
                && self.chats.row_data(i).as_ref() != Some(cached)
            {
                self.chats.set_row_data(i, cached.clone());
            }
        }
        for i in 0..self.friends.row_count() {
            let mut row = self.friends.row_data(i).unwrap();
            if row.avatar_key == key
                && let Some(image) = image
                && row.avatar != *image
            {
                row.avatar = image.clone();
                self.friends.set_row_data(i, row);
            }
        }
        for i in 0..self.members.row_count() {
            let mut row = self.members.row_data(i).unwrap();
            if row.avatar_key == key
                && let Some(image) = image
                && row.avatar != *image
            {
                row.avatar = image.clone();
                self.members.set_row_data(i, row);
            }
        }
        if let Some(image) = image {
            if state
                .selected
                .as_ref()
                .and_then(|id| state.chats.get(id))
                .and_then(|item| item.chat.avatar.as_ref())
                .is_some_and(|asset| asset.key() == key)
            {
                ui.global::<State>().set_chat_avatar(image.clone());
            }
            let profile = match state.detail_page() {
                Some(line::controller::DetailPage::Profile(id)) => state
                    .contacts
                    .get(id)
                    .or_else(|| state.friends.iter().find(|f| &f.id == id))
                    .or_else(|| {
                        state
                            .members
                            .iter()
                            .find(|m| &m.contact.id == id)
                            .map(|m| &m.contact)
                    }),
                _ => None,
            };
            if profile
                .and_then(|f| f.avatar.as_ref())
                .is_some_and(|a| a.key() == key)
            {
                ui.set_profile_avatar(image.clone());
            }
            let other_avatar = match state.detail_page() {
                Some(line::controller::DetailPage::Profile(id))
                    if id == "self" || state.profile.mid == *id =>
                {
                    state
                        .profile
                        .picture_path
                        .as_deref()
                        .and_then(line::client::desktop::assets::Asset::avatar)
                }
                Some(line::controller::DetailPage::GroupInfo(id)) => {
                    state.chats.get(id).and_then(|c| c.chat.avatar.clone())
                }
                _ => None,
            };
            if other_avatar.is_some_and(|a| a.key() == key) {
                ui.set_profile_avatar(image.clone());
            }
        }
    }
    pub fn patch_draft(&mut self, chat: &ChatId, draft: &Option<Draft>) {
        let Some((inputs, row)) = self.rendered_chats.get_mut(chat) else {
            return;
        };
        inputs.draft = draft.clone();
        row.draft = draft.as_ref().is_some_and(|d| !d.text.is_empty());
        row.draft_preview = draft
            .as_ref()
            .map(|d| single_line(&d.text))
            .unwrap_or_default()
            .into();
        if let Some(i) = self.chat_keys.iter().position(|id| id == chat)
            && self.chats.row_data(i).as_ref() != Some(row)
        {
            self.chats.set_row_data(i, row.clone());
        }
    }
}

fn uses_asset(row: &MessageRow, key: &str) -> bool {
    row.image_key == key
        || row.sender_key == key
        || row.rich_runs.iter().any(|run| run.image_key == key)
        || row.reactions.iter().any(|run| run.image_key == key)
}
fn patch_message_asset(
    row: &mut MessageRow,
    key: &str,
    image: Option<&slint::Image>,
    failure: Option<&line::client::desktop::assets::Failure>,
) -> bool {
    if !uses_asset(row, key) {
        return false;
    }
    let old = row.clone();
    if row.image_key == key {
        if let Some(image) = image {
            row.picture = image.clone();
            row.has_picture = true;
        }
        row.image_failed = failure.is_some();
        row.image_failure = failure.map(|f| format!("{f:?}")).unwrap_or_default().into();
        row.image_error = failure
            .map(|f| text::image_text(*f))
            .unwrap_or_default()
            .into();
        row.image_retryable = failure.is_some_and(|f| f.retriable());
    }
    if row.sender_key == key
        && let Some(image) = image
    {
        row.sender_avatar = image.clone();
        row.has_sender_avatar = true;
    }
    patch_runs(&row.rich_runs, key, image, failure.is_some(), None);
    if let Some(model) = row
        .reactions
        .as_any()
        .downcast_ref::<VecModel<ReactionRow>>()
    {
        for i in 0..model.row_count() {
            let mut run = model.row_data(i).unwrap();
            if run.image_key == key
                && let Some(image) = image
                && run.picture != *image
            {
                run.picture = image.clone();
                run.has_picture = true;
                model.set_row_data(i, run);
            }
        }
    }
    *row != old
}
fn patch_chat_asset(
    row: &mut ChatRow,
    key: &str,
    image: Option<&slint::Image>,
    failed: bool,
    size: i32,
) -> bool {
    patch_runs(&row.preview_runs, key, image, failed, Some(size));
    if row.avatar_key == key
        && let Some(image) = image
        && row.avatar != *image
    {
        row.avatar = image.clone();
        return true;
    }
    false
}
fn patch_runs(
    runs: &ModelRc<RichRun>,
    key: &str,
    image: Option<&slint::Image>,
    failed: bool,
    size: Option<i32>,
) {
    if let Some(model) = runs.as_any().downcast_ref::<VecModel<RichRun>>() {
        // Do the thumbnail conversion once, even when an emoji repeats.
        let mut picture = None;
        for i in 0..model.row_count() {
            let mut run = model.row_data(i).unwrap();
            if run.image_key == key {
                if failed {
                    if run.has_picture {
                        run.has_picture = false;
                        model.set_row_data(i, run);
                    }
                    continue;
                }
                let Some(image) = image else {
                    continue;
                };
                let image = picture.get_or_insert_with(|| {
                    size.map(|size| preview::small(image, size))
                        .unwrap_or_else(|| image.clone())
                });
                if !run.has_picture || run.picture != *image {
                    run.picture = image.clone();
                    run.has_picture = true;
                    model.set_row_data(i, run);
                }
            }
        }
    }
}

/// All content labels belong to the presentation layer.
pub(crate) fn content_view(content: &Content) -> account::MessageView {
    use line::client::desktop::rich::RichContent;
    let mut view = account::MessageView::default();
    let mut rich = RichContent::default();
    view.body = match content {
        Content::Text { text, spans } => {
            rich.emoji = spans.clone();
            text.clone()
        }
        Content::Photo {
            album_count,
            caption,
            ..
        } => {
            view.photo = true;
            rich.content_type = 1;
            rich.album_count = *album_count;
            rich.caption = caption.clone();
            "Photo".into()
        }
        Content::Sticker { text, .. } => {
            view.sticker = true;
            rich.content_type = 7;
            text.clone().unwrap_or_else(|| "Sticker".into())
        }
        Content::File { name, size, .. } => {
            rich.content_type = 14;
            rich.file_name = name.clone();
            rich.file_size = *size;
            name.clone().unwrap_or_else(|| "File".into())
        }
        Content::Video {
            duration,
            album_count,
            caption,
            ..
        } => {
            rich.content_type = 2;
            rich.duration_ms = *duration;
            rich.album_count = *album_count;
            rich.caption = caption.clone();
            "Video".into()
        }
        Content::Voice { duration, .. } => {
            rich.content_type = 3;
            rich.duration_ms = *duration;
            "Voice message".into()
        }
        Content::Contact { name, .. } => {
            rich.content_type = 13;
            rich.contact_name = name.clone();
            name.clone().unwrap_or_else(|| "Contact".into())
        }
        Content::Location { .. } => {
            rich.content_type = 15;
            "Location".into()
        }
        Content::System { kind, params, text } => {
            rich.content_type = 18;
            rich.system_key = kind.clone();
            rich.system_args = params.clone();
            rich.system_text = text.clone();
            text.clone().unwrap_or_default()
        }
        Content::Withdrawn => {
            rich.withdrawn = true;
            "Message unsent".into()
        }
        Content::Undecryptable => {
            view.decryption_failed = true;
            "Unable to decrypt this message with the saved keys".into()
        }
        Content::Unknown { content_type } => {
            rich.content_type = i64::from(*content_type);
            "This message type isn't supported yet".into()
        }
    };
    view.image_key = content.image().map(|a| a.key()).unwrap_or_default();
    view.emoji = content
        .spans()
        .iter()
        .map(|span| account::EmojiView {
            span: span.clone(),
            image_key: line::client::desktop::assets::Asset::inline_emoji(span)
                .map(|a| a.key())
                .unwrap_or_default(),
        })
        .collect();
    view.rich = rich;
    view
}
fn reaction_views(reactions: &[Reaction]) -> Vec<account::ReactionView> {
    let mut groups: Vec<(String, String, usize)> = vec![];
    for reaction in reactions {
        let (label, key) = match &reaction.kind {
            ReactionKind::Predefined(id) => (
                match id {
                    2 => "👍",
                    3 => "❤️",
                    4 => "😆",
                    5 => "😮",
                    6 => "😢",
                    7 => "😰",
                    _ => "Reaction",
                }
                .to_owned(),
                String::new(),
            ),
            ReactionKind::Emoji => ("Emoji".into(), String::new()),
            ReactionKind::Paid(asset) => (
                "Emoji".into(),
                asset.as_ref().map(|a| a.key()).unwrap_or_default(),
            ),
        };
        if let Some(group) = groups.iter_mut().find(|g| g.0 == label && g.1 == key) {
            group.2 += 1;
        } else {
            groups.push((label, key, 1));
        }
    }
    groups
        .into_iter()
        .take(3)
        .map(|(label, image_key, _count)| account::ReactionView { label, image_key })
        .collect()
}

pub(crate) fn chat_title<'a>(chat: &'a ChatInput, state: &'a AppState) -> &'a str {
    if !chat.title.trim().is_empty() {
        return &chat.title;
    }
    state
        .contacts
        .get(&chat.id.0)
        .map(|c| c.name.as_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Conversation")
}
