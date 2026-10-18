//! Slint is a presentation target; the controller owns the projection.
use crate::*;
#[derive(Default)]
pub struct Presenter {
    chats: Rc<VecModel<ChatRow>>,
    messages: Rc<VecModel<MessageRow>>,
    chat_keys: Vec<ChatId>,
    message_keys: Vec<RowKey>,
    pub images: HashMap<String, slint::Image>,
    pub failures: HashMap<String, line_wasm_poc::client::desktop::assets::Failure>,
    friends: Rc<VecModel<FriendRow>>,
    friend_keys: Vec<String>,
    members: Rc<VecModel<MemberRow>>,
    member_keys: Vec<String>,
    rendered: std::collections::BTreeMap<Key, (StoredMessage, MessageRow)>,
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
    fn row(&mut self, m: &StoredMessage) -> MessageRow {
        if let Some((stored, row)) = self.rendered.get(&m.key)
            && stored == m
        {
            return row.clone();
        }
        let raw =
            serde_json::from_value::<line_wasm_poc::client::desktop::Message>(m.body.clone()).ok();
        let mut row = if let Some(raw) = raw {
            message_row(account::MessageView {
                id: m.key.0.to_string(),
                sender_id: raw.sender_id,
                sender_name: raw.sender_name,
                sender_key: raw.sender_avatar.map(|a| a.key()).unwrap_or_default(),
                sticker: raw.rich.content_type == 7,
                photo: raw.rich.content_type == 1,
                body: raw.body,
                timestamp: m.sort.sort_ts,
                delivered_time: raw.delivered_time,
                outgoing: m.outgoing,
                image_key: raw.image.map(|a| a.key()).unwrap_or_default(),
                reactions: if m
                    .reactions
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v.get("label").is_some()))
                {
                    serde_json::from_value(m.reactions.clone()).unwrap_or_default()
                } else {
                    line_wasm_poc::client::desktop::reactions(&m.reactions)
                }
                .into_iter()
                .map(|r| account::ReactionView {
                    label: format!("{} {}", r.label, r.count),
                    image_key: r.asset.map(|a| a.key()).unwrap_or_default(),
                })
                .collect(),
                emoji: raw
                    .rich
                    .emoji
                    .iter()
                    .map(|span| account::EmojiView {
                        span: span.clone(),
                        image_key: line_wasm_poc::client::desktop::assets::Asset::inline_emoji(
                            span,
                        )
                        .map(|a| a.key())
                        .unwrap_or_default(),
                    })
                    .collect(),
                decryption_failed: raw.decryption_failed,
                rich: raw.rich,
                skip_badge: raw.skip_badge,
            })
        } else {
            let mut rich = line_wasm_poc::client::desktop::rich::RichContent {
                content_type: m.content_type as i64,
                ..Default::default()
            };
            rich.withdrawn = m.withdrawn;
            message_row(account::MessageView {
                id: m.key.0.to_string(),
                body: m.body["body"].as_str().unwrap_or_default().into(),
                outgoing: m.outgoing,
                timestamp: m.sort.sort_ts,
                sender_id: m.sender_id.clone(),
                rich,
                ..Default::default()
            })
        };
        if m.withdrawn {
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
        row.can_unsend = m.outgoing && m.server_id.is_some() && !m.withdrawn;
        row.recoverable = matches!(m.delivery, Delivery::Rejected { .. } | Delivery::Uncertain);
        row.delivery_detail = match &m.delivery {
            Delivery::Rejected { reason } => reason.clone(),
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
            row.image_error = failure.image_text().into();
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
        links::decorate_message(&mut row, "Contact");
        self.rendered.insert(m.key, (m.clone(), row.clone()));
        row
    }
    pub fn present(&mut self, ui: &AppWindow, state: &AppState, query: &str, category: i32) {
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
        let members = state
            .members
            .iter()
            .filter(|m| {
                m.contact
                    .name
                    .to_lowercase()
                    .contains(&ui.get_member_query().to_lowercase())
            })
            .map(|m| MemberRow {
                id: m.contact.id.clone().into(),
                name: m.contact.name.clone().into(),
                initial: name_initial(&m.contact.name),
                status: m.contact.status.clone().into(),
                is_self: m.is_self,
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
        if let Panel::Profile(id) | Panel::MemberProfile(id) = &state.panel
            && let Some(f) = state
                .friends
                .iter()
                .chain(state.members.iter().map(|m| &m.contact))
                .find(|f| &f.id == id)
        {
            ui.set_profile_name(f.name.clone().into());
            ui.global::<State>()
                .set_profile_initial(name_initial(&f.name));
            ui.set_profile_id(f.id.clone().into());
            ui.set_profile_status(f.status.clone().into());
            ui.set_profile_group(f.group);
            ui.set_profile_available(
                !state
                    .members
                    .iter()
                    .any(|m| m.is_self && &m.contact.id == id)
                    && (!matches!(state.panel, Panel::MemberProfile(_))
                        || state.chats.contains_key(&ChatId(id.clone()))
                        || state.friends.iter().any(|f| &f.id == id)),
            );
            ui.set_profile_avatar(
                f.avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            );
        }

        let mut chats = Vec::new();
        for (index, id) in state.chat_order.iter().enumerate() {
            let item = &state.chats[id];
            let c = &item.chat;
            if !c.title.to_lowercase().contains(query)
                || (category == 1 && c.kind != 0)
                || (category == 2 && c.kind == 0)
            {
                continue;
            }
            let mut row = ChatRow {
                index: index as i32,
                title: c.title.clone().into(),
                initial: name_initial(&c.title),
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
                group: c.kind != 0,
                member_count: c.member_count.unwrap_or(0) as i32,
                time: list_time(item.activity_ms).into(),
                ..Default::default()
            };
            if let Some(m) = &item.preview {
                let message = self.row(m);
                preview::set(&mut row, &message, ui.get_text_size());
            } else if item.activity_ms == 0 {
                row.time = "New".into();
                row.preview = "No messages yet".into();
            }
            if let Some(asset) = c.avatar.as_ref().and_then(|v| {
                serde_json::from_str::<line_wasm_poc::client::desktop::assets::Asset>(v).ok()
            }) {
                row.avatar_key = asset.key().into();
                if let Some(image) = self.images.get(row.avatar_key.as_str()) {
                    row.avatar = image.clone();
                }
            }
            chats.push((id.clone(), row));
        }
        patch(&self.chats, &mut self.chat_keys, chats);
        let rows = state.rows();
        let mut messages = rows
            .iter()
            .map(|m| {
                let mut row = self.row(m);
                if let Some(target) = state.reply_targets.get(&ServerId(row.reply_id.to_string())) {
                    let quoted = self.row(target);
                    let name = if quoted.outgoing {
                        "You"
                    } else {
                        quoted.sender_name.as_str()
                    };
                    row.reply_preview = format!("{name}: {}", quoted.body).into();
                }
                row
            })
            .collect::<Vec<_>>();
        group_messages(&mut messages);
        let group = state
            .selected
            .as_ref()
            .and_then(|id| state.chats.get(id))
            .is_some_and(|c| c.chat.kind != 0);
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
            Panel::Members => PanelState::Members,
            Panel::Profile(_) => PanelState::Profile,
            Panel::MemberProfile(_) => PanelState::MemberProfile,
            Panel::Viewer => PanelState::Viewer,
            Panel::SignIn => PanelState::SignIn,
        });
        ui.set_has_chat(state.selected.is_some());
        ui.set_storage_error(
            if state.error.is_some() {
                "Local data could not be saved."
            } else {
                ""
            }
            .into(),
        );
        ui.set_account_name(
            state.profile["displayName"]
                .as_str()
                .unwrap_or("LINE")
                .into(),
        );
        ui.global::<State>().set_connection(match state.connection {
            Connection::Local => ConnectionState::Local,
            Connection::Connecting { .. } => ConnectionState::Connecting,
            Connection::Online => ConnectionState::Online,
            Connection::Offline { .. } => ConnectionState::Offline,
            Connection::AuthExpired => ConnectionState::AuthExpired,
            Connection::SessionInvalid => ConnectionState::SessionInvalid,
            Connection::SigningIn => ConnectionState::SigningIn,
        });
        ui.set_connection_status(
            match &state.connection {
                Connection::Local => "Cached conversations",
                Connection::Online => "Connected",
                Connection::Connecting { .. } => "Connecting…",
                Connection::Offline { .. } => "Offline — reconnecting automatically",
                Connection::AuthExpired => "Sign in required",
                Connection::SessionInvalid => "Session unavailable",
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
            ui.set_chat_title(c.chat.title.clone().into());
            ui.set_chat_group(c.chat.kind != 0);
            ui.set_member_count(c.chat.member_count.unwrap_or(0) as i32);
            ui.set_chat_subtitle(
                if c.chat.kind != 0 {
                    format!("{} members", c.chat.member_count.unwrap_or(0))
                } else {
                    String::new()
                }
                .into(),
            );
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
    pub fn invalidate_assets(&mut self) {
        self.rendered.clear();
    }
}
