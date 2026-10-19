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
impl Presenter {
    pub fn install(&self, ui: &AppWindow) {
        ui.set_chats(ModelRc::new(self.chats.clone()));
        ui.set_messages(ModelRc::new(self.messages.clone()));
        ui.set_friends(ModelRc::new(self.friends.clone()));
        ui.set_members(ModelRc::new(self.members.clone()));
    }
    pub fn present_compose(&mut self, ui: &AppWindow, state: &AppState) {
        let global = ui.global::<State>();
        let reply = state
            .composer_reply
            .as_ref()
            .and_then(|r| state.reply_target(&r.message_id));
        let mut quote = reply
            .map(|m| self.quote(m, state))
            .unwrap_or_else(|| MessageRow {
                sender_name: "Original message".into(),
                body: "Original message".into(),
                ..Default::default()
            });
        if let Some(reference) = &state.composer_reply {
            quote.reply_id = reference.message_id.clone().into();
        }
        global.set_composer_reply(quote);
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
                name: single_line(text::friend_name(f)).into(),
                initial: name_initial(text::friend_name(f)),
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
                    name: single_line(&p.name).into(),
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
    fn quote(&self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let image_key = m.body.image().map(|a| a.key()).unwrap_or_default();
        let image = self.images.get(&image_key);
        MessageRow {
            sender_name: if m.outgoing {
                "You".into()
            } else {
                crate::system_events::name(state, &m.sender_id).into()
            },
            sender_id: m.sender_id.clone().into(),
            body: single_line(&compact_text(m, state)).into(),
            image_key: image_key.into(),
            picture: image.cloned().unwrap_or_default(),
            has_picture: image.is_some(),
            photo: matches!(m.body, Content::Photo { .. }),
            sticker: matches!(m.body, Content::Sticker { .. }),
            ..Default::default()
        }
    }
    fn row(&mut self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let sender = state.contact(&m.sender_id).cloned();
        let system_text = crate::system_events::text(&m.body, state, &m.sender_id);
        let reply = m
            .reply
            .as_ref()
            .and_then(|r| state.reply_target(&r.message_id))
            .map(|target| {
                (
                    target.clone(),
                    crate::system_events::name(state, &target.sender_id),
                )
            });
        let inputs = MessageInputs {
            message: m.clone(),
            sender: sender.clone(),
            system_text: system_text.clone(),
            reply,
            upload: state.uploads.get(&m.key).copied().unwrap_or(0.),
        };
        if let Some((old, row)) = self.rendered.get(&m.key)
            && *old == inputs
        {
            return row.clone();
        }
        let mut row = message_row(m, system_text.as_deref());
        row.sender_name = sender
            .as_ref()
            .map(|s| text::member_name(s.name.as_deref()).to_owned())
            .unwrap_or_else(|| {
                if m.sender_id.is_empty() {
                    "Unknown sender".into()
                } else {
                    let start = m
                        .sender_id
                        .char_indices()
                        .rev()
                        .nth(5)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    format!("Member · {}", &m.sender_id[start..])
                }
            })
            .into();
        row.sender_key = sender
            .as_ref()
            .and_then(|s| s.avatar.as_ref())
            .map(|a| a.key())
            .unwrap_or_default()
            .into();
        row.delivery = match &m.delivery {
            Delivery::Pending => "Sending…",
            Delivery::Sent => "Sent",
            Delivery::Read => "Read",
            Delivery::Rejected { .. } => "Not sent",
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
                    reason: line::backend::Refusal::FileMissing
                }
            );
        }
        row.can_react = m.can_react();
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
        row.can_unsend = m.can_unsend();
        row.recoverable = matches!(m.delivery, Delivery::Rejected { .. });
        row.delivery_detail = match &m.delivery {
            Delivery::Rejected { reason } => format!("Not sent: {}", text::rejection(reason)),
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
            row.image_retryable = failure.retry_read();
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
        if let Some((target, _)) = &inputs.reply {
            let quoted = self.quote(target, state);
            row.reply_sender = quoted.sender_name;
            row.reply_sender_id = quoted.sender_id;
            row.reply_preview = quoted.body;
            row.reply_image_key = quoted.image_key;
            row.reply_picture = quoted.picture;
            row.reply_has_picture = quoted.has_picture && (quoted.photo || quoted.sticker);
        }
        links::decorate_message(&mut row, "Contact");
        self.rendered.insert(m.key, (inputs, row.clone()));
        row
    }
    pub fn present(&mut self, ui: &AppWindow, state: &AppState, query: &str, category: i32) {
        self.present_compose(ui, state);

        let friends = state
            .friends
            .iter()
            .filter(|f| {
                text::friend_name(f)
                    .to_lowercase()
                    .contains(&ui.get_friend_query().to_lowercase())
            })
            .map(|f| {
                if let Some(cached) = self.rendered_friends.get(&f.id)
                    && cached.friend == *f
                    && cached.size == ui.get_text_size()
                {
                    return cached.row.clone();
                }
                let spans =
                    line::client::desktop::rich::emoji_spans(&f.status, &f.profile.status_metadata);
                let assets = spans
                    .iter()
                    .filter_map(line::client::desktop::assets::Asset::inline_emoji)
                    .collect::<Vec<_>>();
                let metadata = spans.into_iter().map(EmojiView::new).collect::<Vec<_>>();
                let row = FriendRow {
                    id: f.id.clone().into(),
                    name: single_line(text::friend_name(f)).into(),
                    initial: name_initial(text::friend_name(f)),
                    status: single_line(&f.status).into(),
                    status_runs: preview::preview_runs(
                        &f.status,
                        &metadata,
                        &self.images,
                        ui.get_text_size(),
                    ),
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
                };
                self.rendered_friends.insert(
                    f.id.clone(),
                    CachedFriend {
                        friend: f.clone(),
                        size: ui.get_text_size(),
                        row: row.clone(),
                        assets,
                    },
                );
                row
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
                text::member_name(m.contact.name.as_deref())
                    .to_lowercase()
                    .contains(&query)
                    || m.contact.status.to_lowercase().contains(&query)
                    || (m.is_self && "you".contains(&query))
            })
            .map(|m| MemberRow {
                id: m.contact.id.clone().into(),
                name: single_line(text::member_name(m.contact.name.as_deref())).into(),
                initial: name_initial(text::member_name(m.contact.name.as_deref())),
                status: single_line(&m.contact.status).into(),
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
        let mut conversation_error = String::new();
        let mut friends_error = String::new();
        let mut chat_list_error = String::new();
        let mut detail_error = String::new();
        let detail_id = state.detail.last().map(|entry| match &entry.page {
            line::controller::DetailPage::Profile(id) => id.as_str(),
            line::controller::DetailPage::GroupInfo(id)
            | line::controller::DetailPage::Members(id) => id.0.as_str(),
        });
        for (request, failure) in &state.refresh_failures {
            use line::{client::error::Failure, engine::Refresh};
            let message = match failure {
                Failure::Refused(reason) => format!("Couldn't load this item: {reason}"),
                Failure::Bug(_) => "Couldn't load this item.".into(),
                _ => continue,
            };
            match request {
                Refresh::Friends => friends_error = message,
                Refresh::Profile | Refresh::ChatList | Refresh::MoreChats => {
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
        g.set_conversation_error(conversation_error.into());
        g.set_friends_error(friends_error.into());
        g.set_chat_list_error(chat_list_error.into());
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
                .map(str::to_owned)
                .unwrap_or(detail_error)
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
            let room = ChatId(id.into()).is_room();
            let own = state.profile_own(id);
            let f = state.contact(id);
            let c = state.chats.get(&ChatId(id.into())).map(|c| &c.chat);
            let name = if own {
                state.profile.display_name.as_str()
            } else if kind != 0 {
                text::group_name(
                    c.map(|c| c.title.as_str())
                        .filter(|name| !name.is_empty())
                        .or_else(|| f.and_then(|f| f.name.as_deref())),
                )
            } else if let Some(friend) =
                f.filter(|_| state.friends.iter().any(|friend| friend.id == id))
            {
                text::friend_name(friend)
            } else {
                text::member_name(f.and_then(|f| f.name.as_deref()))
            };
            let room_title = state
                .members
                .iter()
                .filter(|m| !m.is_self && !m.contact.profile.invited)
                .map(|m| text::member_name(m.contact.name.as_deref()))
                .collect::<Vec<_>>()
                .join(", ");
            let name = if kind != 0 && room && !room_title.is_empty() {
                room_title.as_str()
            } else {
                name
            };
            let status = if own {
                state.profile.status_message.as_str()
            } else {
                f.map(|f| f.status.as_str()).unwrap_or("")
            };
            let friend = kind == 0 && state.profile_friend(id);
            ui.set_profile_id(id.into());
            ui.set_profile_name(name.into());
            ui.set_profile_status(status.into());
            g.set_profile_initial(name_initial(name));
            g.set_profile_own(own);
            g.set_profile_friend(friend);
            g.set_profile_addable(kind == 0 && state.profile_addable(id));
            ui.set_profile_available(kind == 0 && state.profile_available(id));
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
            g.set_group_room(room);
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
                unread_count: item.unread_count,
                title: title.to_owned(),
                preview: item.preview.clone(),
                system_text: item
                    .preview
                    .as_ref()
                    .and_then(|m| crate::system_events::text(&m.body, state, &m.sender_id)),
                activity: item.activity_ms,
                failed: item.failed_count,
                draft: item.draft.clone(),
                offline: state.connection != Connection::Online,
                index,
                text_size: ui.get_text_size(),
            };
            if let Some((old, row)) = self.rendered_chats.get(id)
                && *old == inputs
            {
                chats.push((id.clone(), row.clone()));
                continue;
            }
            let mut row = ChatRow {
                id: id.0.clone().into(),
                index: index as i32,
                title: single_line(title).into(),
                initial: name_initial(title),
                unread: item.unread_count as i32,
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
                preview::set(
                    &mut row,
                    &m.body,
                    m.outgoing,
                    inputs.system_text.as_deref(),
                    &self.images,
                    ui.get_text_size(),
                );
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
            .filter(|m| line::system_events::displayable(&m.body))
            .collect::<Vec<_>>();
        // Choose on first population, then retain the mode for this conversation.
        // Switching delegate types during a live arrival would lose selection
        // and the current reading anchor at the size boundary.
        if self.history_chat != state.selected
            && ui.global::<State>().get_link_feedback() == text::LINK_FAILURE
        {
            ui.global::<State>().set_link_feedback("".into());
        }
        if self.messages.row_count() == 0 || self.history_chat != state.selected {
            ui.global::<State>().set_history_virtual(rows.len() > 60);
            self.history_chat = state.selected.clone();
        }
        let mut messages = rows.iter().map(|m| self.row(m, state)).collect::<Vec<_>>();

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
        let mut source = Vec::new();
        for (m, row) in rows.iter().zip(messages) {
            if state.gaps.iter().any(|g| g.after == m.key) {
                source.push(None);
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
            source.push(Some(*m));
            projected.push((RowKey::Message(m.key), row));
        }
        let (keys, mut messages): (Vec<_>, Vec<_>) = projected.into_iter().unzip();
        group_messages(&mut messages, &source);
        patch(
            &self.messages,
            &mut self.message_keys,
            keys.into_iter().zip(messages).collect(),
        );
        self.rendered.retain(|k, _| {
            state.messages.contains_key(k)
                || state.reply_targets.values().any(|m| m.key == *k)
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
        ui.global::<State>()
            .set_account_mid(state.profile.mid.clone().into());
        ui.set_account_name(state.profile.display_name.as_str().into());
        ui.global::<State>().set_connection(match state.connection {
            Connection::Local => ConnectionState::Local,
            Connection::Connecting { .. } => ConnectionState::Connecting,
            Connection::Online => ConnectionState::Online,
            Connection::Offline => ConnectionState::Offline,
            Connection::SignedOut { .. } => ConnectionState::SignedOut,
            Connection::Fatal => ConnectionState::Fatal,
            Connection::SigningIn => ConnectionState::SigningIn,
        });
        ui.set_connection_status(
            match &state.connection {
                Connection::Local => "Cached conversations",
                Connection::Online => "Connected",
                Connection::Connecting { .. } => "Connecting…",
                Connection::Offline => "Offline — reconnecting automatically",
                Connection::SignedOut { .. } => "Signed out",
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
            ui.set_chat_title(single_line(chat_title(&c.chat, state)).into());
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
            let title = friend.map(text::friend_name).unwrap_or(&id.0);
            ui.set_chat_title(title.into());
            ui.global::<State>().set_chat_id(id.0.clone().into());
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
            .set_unread_boundary_unknown(state.unread_boundary_unknown());
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
            state.chats.values().filter(|c| c.unread_count > 0).count() as i32,
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
        for (_, row) in self.rendered.values_mut() {
            patch_message_asset(row, key, image, failure);
        }
        for i in 0..self.messages.row_count() {
            let mut row = self.messages.row_data(i).unwrap();
            if !uses_asset(&row, key) {
                continue;
            }
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
        for cached in self.rendered_friends.values_mut() {
            let row = &mut cached.row;
            patch_runs(
                &row.status_runs,
                key,
                image,
                failure.is_some(),
                Some(ui.get_text_size()),
            );
            if row.avatar_key == key
                && let Some(image) = image
            {
                row.avatar = image.clone();
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
                Some(line::controller::DetailPage::Profile(id)) => state.contact(id),
                _ => None,
            };
            if profile
                .and_then(|f| f.avatar.as_ref())
                .is_some_and(|a| a.key() == key)
            {
                ui.set_profile_avatar(image.clone());
            }
            let other_avatar = match state.detail_page() {
                Some(line::controller::DetailPage::Profile(id)) if state.profile_own(id) => state
                    .profile
                    .picture_path
                    .as_deref()
                    .and_then(line::client::desktop::assets::Asset::avatar),
                Some(line::controller::DetailPage::GroupInfo(id)) => {
                    state.chats.get(id).and_then(|c| c.chat.avatar.clone())
                }
                _ => None,
            };
            if other_avatar.is_some_and(|a| a.key() == key) {
                ui.set_profile_avatar(image.clone());
            }
        }
        let mut quote = ui.global::<State>().get_composer_reply();
        if uses_asset(&quote, key) && patch_message_asset(&mut quote, key, image, failure) {
            ui.global::<State>().set_composer_reply(quote);
        }
    }
    pub fn request_status_assets(&self, assets: &mut assets::Assets, sender: &Sender) {
        for cached in self.rendered_friends.values() {
            for asset in &cached.assets {
                if assets.request(asset) {
                    sender.send(Intent::Asset(asset.clone()));
                }
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
    row.reply_image_key == key
        || row.image_key == key
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
    if row.reply_image_key == key
        && let Some(image) = image
    {
        row.reply_picture = image.clone();
        row.reply_has_picture = failure.is_none();
    }
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
        row.image_retryable = failure.is_some_and(|f| f.retry_read());
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

pub(crate) fn reaction_views(reactions: &[Reaction]) -> Vec<ReactionRow> {
    let mut groups: Vec<(String, String)> = vec![];
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
        if !groups.iter().any(|g| g.0 == label && g.1 == key) {
            groups.push((label, key));
        }
    }
    groups
        .into_iter()
        .take(3)
        .map(|(label, image_key)| ReactionRow {
            label: label.into(),
            image_key: image_key.into(),
            ..Default::default()
        })
        .collect()
}

pub(crate) fn chat_title<'a>(chat: &'a ChatInput, state: &'a AppState) -> &'a str {
    if !chat.title.trim().is_empty() {
        return &chat.title;
    }
    state
        .contact(&chat.id.0)
        .and_then(|c| c.name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Conversation")
}

fn compact_text(message: &StoredMessage, state: &AppState) -> String {
    crate::system_events::text(&message.body, state, &message.sender_id)
        .unwrap_or_else(|| kind_label(&message.body).1)
}
