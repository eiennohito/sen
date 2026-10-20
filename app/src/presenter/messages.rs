use super::format::{joins, local_time};
use super::system_events;
use super::*;

impl Presenter {
    pub(super) fn present_header(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<ConversationState>();
        g.set_has_chat(state.selected.is_some());
        g.set_window_sub(state.window_sub as i32);
        g.set_window_loading(state.window_loading);

        if let Some(id) = &state.selected {
            let chat = state.chats.get(id).map(|c| &c.chat);
            let title = if chat.is_some() {
                names::chat(state, id, names::CONVERSATION)
            } else {
                state
                    .contact(&id.0)
                    .map(names::friend_name)
                    .unwrap_or(names::CONVERSATION)
            };
            let group = chat.is_some_and(|c| c.is_group);
            let avatar = chats::chat_avatar(state, id);
            g.set_chat_title(single_line(title).into());
            g.set_chat_id(id.0.clone().into());
            g.set_chat_initial(name_initial(title));
            g.set_chat_avatar(
                avatar
                    .and_then(|a| self.images.get(&a.key()))
                    .unwrap_or_default(),
            );
            g.set_chat_group(group);
            let count = chat.and_then(|c| c.member_count).unwrap_or(0);
            g.set_member_count(count as i32);
        }
    }
    pub(super) fn present_messages(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        rows: &[&StoredMessage],
    ) {
        let content = ui.global::<ContentState>();
        let conversation = ui.global::<ConversationState>();
        let rows = rows
            .iter()
            .copied()
            .filter(|m| !m.skip_badge || sen::system_events::displayable(&m.body))
            .collect::<Vec<_>>();
        if self.history_chat != state.selected && content.get_link_feedback() == text::LINK_FAILURE
        {
            content.set_link_feedback("".into());
        }
        if self.messages.row_count() == 0 || self.history_chat != state.selected {
            conversation.set_history_virtual(rows.len() > 60);
            self.history_chat = state.selected.clone();
        }
        let group = state
            .selected
            .as_ref()
            .and_then(|id| state.chats.get(id))
            .is_some_and(|c| c.chat.is_group);
        let gaps: std::collections::BTreeSet<_> = state.gaps.iter().map(|g| g.after).collect();
        let mut source = Vec::new();
        for m in rows {
            if gaps.contains(&m.key) {
                source.push(None);
            }
            source.push(Some(m));
        }
        let today = local_time(now_millis()).unwrap().date_naive();
        let next = source
            .iter()
            .enumerate()
            .map(|(i, message)| {
                let key = match message {
                    Some(m) => RowKey::Message(m.key),
                    None => RowKey::Gap(source[i + 1].unwrap().key),
                };
                let (data, assets) = if let Some(m) = message {
                    let inputs = message_inputs(m, state);
                    let assets = message_assets(&inputs);
                    (
                        MessageData::Message {
                            inputs,
                            today,
                            group,
                            previous: i
                                .checked_sub(1)
                                .and_then(|i| source[i])
                                .map(|previous| (previous.sort.sort_ts, joins(previous, *message))),
                            joins_next: joins(m, source.get(i + 1).copied().flatten()),
                        },
                        assets,
                    )
                } else {
                    (MessageData::Gap(source[i + 1].unwrap().key), Vec::new())
                };
                (key, data, assets)
            })
            .collect();
        self.messages.replace(next);
    }
    pub(super) fn present_history(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<ConversationState>();
        g.set_unread_divider(
            state
                .unread_divider
                .map(|k| k.0.to_string())
                .unwrap_or_default()
                .into(),
        );
        g.set_older_status(
            if state.reaches_start {
                "Beginning of conversation"
            } else {
                ""
            }
            .into(),
        );
        g.set_history_can_follow(state.reaches_end);
    }
}

fn message_inputs(m: &StoredMessage, state: &AppState) -> MessageInputs {
    let sender = state.contact(&m.sender_id).cloned();
    let system_text = system_events::text(&m.body, state, &m.sender_id);
    let reply = m
        .reply
        .as_ref()
        .and_then(|r| state.reply_target(&r.message_id))
        .map(|target| {
            (
                target.clone(),
                system_events::name(state, &target.sender_id),
            )
        });
    MessageInputs {
        message: m.clone(),
        reactors: m
            .reactions
            .iter()
            .map(|r| {
                (
                    system_events::name(state, &r.actor),
                    state.contact(&r.actor).cloned(),
                )
            })
            .collect(),
        own_mid: state.profile.mid.clone(),
        sender: sender.clone(),
        // Only an unnamed sender needs the friend lookup for its fallback.
        sender_friend: sender
            .as_ref()
            .is_some_and(|s| names::nonblank(s.name.as_deref()).is_none())
            && state.profile_friend(&m.sender_id),
        system_text: system_text.clone(),
        reply_text: reply
            .as_ref()
            .and_then(|(target, _)| system_events::text(&target.body, state, &target.sender_id)),
        reply,
        upload: state.uploads.get(&m.key).copied().unwrap_or(0.),
    }
}

fn build_row(inputs: &MessageInputs, images: &Images) -> MessageRow {
    let m = &inputs.message;
    let mut row = message_row(m, inputs.system_text.as_deref(), images);
    if !m.body.is_withdrawn() {
        let mut reactions = reaction_views(
            &m.reactions,
            |id| {
                let (name, contact) = inputs
                    .reactors
                    .iter()
                    .zip(&m.reactions)
                    .find(|(_, r)| r.actor == id)
                    .map(|(actor, _)| actor)
                    .unwrap();
                let key = contact
                    .as_ref()
                    .and_then(|c| c.avatar.as_ref())
                    .map(|a| a.key())
                    .unwrap_or_default();
                ReactorRow {
                    id: id.into(),
                    initial: name_initial(if id == inputs.own_mid {
                        names::member_name(contact.as_ref().and_then(|c| c.name.as_deref()))
                    } else {
                        name
                    }),
                    name: name.as_str().into(),
                    avatar: images.get(&key).unwrap_or_default(),
                }
            },
            &inputs.own_mid,
        );
        for reaction in &mut reactions {
            images.reaction(reaction);
        }
        row.reactions = ModelRc::new(VecModel::from(reactions));
    }
    row.sender_name = names::sender(inputs.sender.as_ref(), inputs.sender_friend, &m.sender_id)
        .as_ref()
        .into();
    let sender_key = asset_key(inputs.sender.as_ref().and_then(|s| s.avatar.as_ref()));
    row.delivery = match &m.delivery {
        Delivery::Pending => DeliveryState::Pending,
        Delivery::Sent => DeliveryState::Sent,
        Delivery::Read => DeliveryState::Read,
        Delivery::Rejected { .. } => DeliveryState::Rejected,
    };
    if let Some(_item) = m
        .body
        .image()
        .or_else(|| m.body.attachment())
        .and_then(|asset| asset.local_attachment())
    {
        row.local_media = true;
        row.upload_progress = inputs.upload;
        row.uploading = m.delivery == Delivery::Pending && row.upload_progress < 1.;
        row.file_missing = matches!(
            m.delivery,
            Delivery::Rejected {
                reason: sen::backend::Refusal::FileMissing
            }
        );
    }
    row.can_react = m.can_react();
    row.reaction_count = m.reactions.len() as i32;
    row.own_reaction = m
        .reactions
        .iter()
        .find_map(|r| match r.kind {
            ReactionKind::Predefined(n) if r.actor == inputs.own_mid => Some(n as i32),
            _ => None,
        })
        .unwrap_or(0);
    row.can_unsend = m.can_unsend();
    row.recoverable = matches!(m.delivery, Delivery::Rejected { .. });
    row.delivery_detail = match &m.delivery {
        Delivery::Rejected { reason } => format!("Not sent: {}", text::rejection(reason)),
        _ => String::new(),
    }
    .into();
    if let Some(image) = images.get(row.image_key.as_str()) {
        row.picture = image;
        row.has_picture = true;
    }
    if let Some(image) = images.get(sender_key.as_str()) {
        row.sender_avatar = image;
        row.has_sender_avatar = true;
    }
    if let Some(failure) = images.failure(row.image_key.as_str()) {
        row.image_failed = true;
        row.image_error = text::image_text(failure).into();
        row.image_retryable = failure.retry_read();
    }
    if let Some((target, name)) = &inputs.reply {
        row.reply_sender = if target.outgoing { names::YOU } else { name }.into();
        row.reply_sender_id = target.sender_id.clone().into();
        row.reply_preview = single_line(
            &inputs
                .reply_text
                .clone()
                .unwrap_or_else(|| message_text(&target.body).1),
        )
        .into();
        let reply_image_key = asset_key(target.body.image());
        let picture = images.small(reply_image_key.as_str());
        row.reply_has_picture = picture.is_some()
            && matches!(target.body, Content::Photo { .. } | Content::Sticker { .. });
        row.reply_picture = picture.unwrap_or_default();
    }
    rich_content::links::decorate_message(&mut row);
    row
}

pub(crate) fn compact_text(content: &Content, state: &AppState, sender: &str) -> String {
    system_events::text(content, state, sender).unwrap_or_else(|| kind_label(content).1)
}

#[derive(Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Rows own their inputs; gaps need no allocation.
pub(super) enum MessageData {
    Gap(Key),
    Message {
        inputs: MessageInputs,
        previous: Option<(i64, bool)>,
        joins_next: bool,
        group: bool,
        today: chrono::NaiveDate,
    },
}

fn message_assets(inputs: &MessageInputs) -> Vec<String> {
    let mut assets = content_assets(&inputs.message.body);
    for contact in inputs
        .sender
        .iter()
        .chain(inputs.reactors.iter().filter_map(|(_, f)| f.as_ref()))
    {
        if let Some(avatar) = &contact.avatar {
            assets.push(avatar.key());
        }
    }
    if let Some((reply, _)) = &inputs.reply {
        assets.extend(content_assets(&reply.body));
    }
    for reaction in &inputs.message.reactions {
        if let ReactionKind::Paid(Some(asset)) = &reaction.kind {
            assets.push(asset.key());
        }
    }
    assets
}

pub(super) fn project(data: MessageData, images: &Images) -> MessageRow {
    let (inputs, previous, joins_next, group) = match data {
        MessageData::Gap(key) => {
            return MessageRow {
                id: format!("gap-{}", key.0).into(),
                row_kind: HistoryRowKind::Gap,
                body: "Loading messages…".into(),
                kind: MessageKind::System,
                ..Default::default()
            };
        }
        MessageData::Message {
            inputs,
            previous,
            joins_next,
            group,
            today: _,
        } => (inputs, previous, joins_next, group),
    };
    let mut row = build_row(&inputs, images);
    if !group {
        row.sender_name = "".into();
        row.has_sender_avatar = false;
        row.sender_avatar = Default::default();
    }
    let timestamp = inputs.message.sort.sort_ts;
    row.sender_initial = name_initial(&row.sender_name);
    let time = local_time(timestamp);
    row.clock = time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
        .into();
    if time.map(|t| t.date_naive())
        != previous
            .and_then(|(t, _)| local_time(t))
            .map(|t| t.date_naive())
    {
        row.day_label = day_label(timestamp, now_millis()).into();
    }
    row.run_position = match (previous.is_some_and(|(_, joins)| joins), joins_next) {
        (false, false) => RunPosition::Single,
        (false, true) => RunPosition::First,
        (true, true) => RunPosition::Middle,
        (true, false) => RunPosition::Last,
    };
    row
}
