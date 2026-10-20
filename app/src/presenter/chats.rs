use super::system_events;
use super::*;

impl Presenter {
    pub(crate) fn present_chats(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        category: ChatCategory,
    ) {
        let g = ui.global::<ChatsState>();
        let query = Query::new(&g.get_chat_query());
        g.set_selected_chat(
            state
                .selected
                .as_ref()
                .and_then(|id| state.chat_order.iter().position(|c| c == id))
                .map(|n| n as i32)
                .unwrap_or(-1),
        );

        let mut chats = Vec::new();
        for (index, id) in state.chat_order.iter().enumerate() {
            let item = &state.chats[id];
            let c = &item.chat;
            let title = names::chat(state, id, names::CONVERSATION);
            if !query.matches([title], &mut self.search_fields)
                || (category == ChatCategory::Friends && c.is_group)
                || (category == ChatCategory::Groups && !c.is_group)
            {
                continue;
            }
            let inputs = ChatInputs {
                item: item.clone(),
                avatar: chat_avatar(state, id).cloned(),
                title: title.to_owned(),
                system_text: item
                    .preview
                    .as_ref()
                    .and_then(|m| system_events::text(&m.body, state, &m.sender_id)),
                offline: state.connection != Connection::Online,
                index,
            };
            let assets = inputs
                .avatar
                .iter()
                .map(|a| a.key())
                .chain(
                    inputs
                        .item
                        .preview
                        .iter()
                        .flat_map(|m| content_assets(&m.body)),
                )
                .collect();
            chats.push((id.clone(), inputs, assets));
        }
        self.chats.replace(chats);
    }

    pub fn patch_draft(&mut self, chat: &ChatId, draft: &Option<Draft>) {
        self.chats.update(chat, |inputs| {
            std::mem::replace(&mut inputs.item.draft, draft.clone()) != *draft
        });
    }
}

pub(crate) fn chat_row(inputs: ChatInputs, images: &Images) -> ChatRow {
    let mut row = ChatRow {
        id: inputs.item.chat.id.0.clone().into(),
        index: inputs.index as i32,
        title: single_line(&inputs.title).into(),
        initial: name_initial(&inputs.title),
        unread: inputs.item.unread_count as i32,
        failed: inputs.item.failed_count > 0,
        unread_unknown: inputs.item.chat.unread_stale || inputs.offline,
        muted: inputs.item.chat.muted,
        group: inputs.item.chat.is_group,
        member_count: inputs.item.chat.member_count.unwrap_or(0) as i32,
        time: list_time(inputs.item.activity_ms).into(),
        ..Default::default()
    };
    if let Some(m) = &inputs.item.preview {
        rich_content::preview::set(&mut row, &m.body, inputs.system_text.as_deref(), images);
    } else if inputs.item.activity_ms == 0 {
        row.time = "New".into();
        row.preview = "No messages yet".into();
    }
    if let Some(asset) = inputs.avatar.as_ref() {
        let key = asset.key();
        if let Some(image) = images.get(&key) {
            row.avatar = image.clone();
        }
    }
    let (prefix, preview) =
        if let Some(draft) = inputs.item.draft.as_ref().filter(|d| !d.text.is_empty()) {
            row.preview_mode = PreviewMode::Draft;
            row.preview_runs = ModelRc::default();
            ("Draft: ", single_line(&draft.text))
        } else if inputs.item.preview.as_ref().is_some_and(|m| m.outgoing) {
            row.preview_mode = PreviewMode::Outgoing;
            ("You: ", row.preview.to_string())
        } else {
            ("", row.preview.to_string())
        };
    row.preview_prefix = prefix.into();
    row.display_preview = format!("{prefix}{preview}").into();
    let mut label = vec![row.title.to_string()];
    if !row.time.is_empty() {
        label.push(row.time.to_string());
    }
    label.push(row.display_preview.to_string());
    if row.unread_unknown {
        label.push("Unread count unavailable".into());
    } else if row.unread > 0 {
        label.push(format!("{} unread", row.unread));
    }
    if row.failed {
        label.push("Send failed".into());
    }
    if row.muted {
        label.push("Muted".into());
    }
    row.accessible_label = label.join(", ").into();
    row.badge = unread_badge(row.unread);
    row
}

pub(super) fn chat_avatar<'a>(state: &'a AppState, id: &ChatId) -> Option<&'a Asset> {
    let chat = state.chats.get(id).map(|c| &c.chat);
    if chat.is_some_and(|c| c.is_group) {
        chat.and_then(|c| c.avatar.as_ref())
    } else {
        state
            .contact(&id.0)
            .and_then(|c| c.avatar.as_ref())
            .or_else(|| chat.and_then(|c| c.avatar.as_ref()))
    }
}
