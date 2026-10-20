use super::system_events;
use super::*;

impl Presenter {
    pub(super) fn present_chats(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        query: &str,
        category: ChatCategory,
    ) {
        let mut chats = Vec::new();
        for (index, id) in state.chat_order.iter().enumerate() {
            let item = &state.chats[id];
            let c = &item.chat;
            let title = chat_title(c, state);
            if !title.to_lowercase().contains(query)
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
                text_size: ui.global::<State>().get_text_size(),
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
    pub(super) fn present_header(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<State>();
        g.set_selected_chat(
            state
                .selected
                .as_ref()
                .and_then(|id| state.chat_order.iter().position(|c| c == id))
                .map(|n| n as i32)
                .unwrap_or(-1),
        );
        if let Some(id) = &state.selected {
            let chat = state.chats.get(id).map(|c| &c.chat);
            let contact = state.contact(&id.0);
            let title = chat
                .map(|c| chat_title(c, state))
                .unwrap_or_else(|| contact.map(text::friend_name).unwrap_or("Conversation"));
            let group = chat.is_some_and(|c| c.is_group);
            let avatar = chat_avatar(state, id);
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
            g.set_chat_subtitle(
                if group {
                    format!("{count} members")
                } else {
                    String::new()
                }
                .into(),
            );
        }
    }

    pub fn patch_draft(&mut self, chat: &ChatId, draft: &Option<Draft>) {
        self.chats
            .update(chat, |inputs| inputs.item.draft = draft.clone());
    }
}

pub(super) fn chat_row(inputs: ChatInputs, images: &Images) -> ChatRow {
    let mut row = ChatRow {
        id: inputs.item.chat.id.0.clone().into(),
        index: inputs.index as i32,
        title: single_line(&inputs.title).into(),
        initial: name_initial(&inputs.title),
        unread: inputs.item.unread_count as i32,
        failed: inputs.item.failed_count > 0,
        draft: inputs
            .item
            .draft
            .as_ref()
            .is_some_and(|d| !d.text.is_empty()),
        draft_preview: inputs
            .item
            .draft
            .as_ref()
            .map(|d| single_line(&d.text))
            .unwrap_or_default()
            .into(),
        unread_unknown: inputs.item.chat.unread_stale || inputs.offline,
        muted: inputs.item.chat.muted,
        group: inputs.item.chat.is_group,
        member_count: inputs.item.chat.member_count.unwrap_or(0) as i32,
        time: list_time(inputs.item.activity_ms).into(),
        ..Default::default()
    };
    if let Some(m) = &inputs.item.preview {
        rich_content::preview::set(
            &mut row,
            &m.body,
            m.outgoing,
            inputs.system_text.as_deref(),
            images,
            inputs.text_size,
        );
    } else if inputs.item.activity_ms == 0 {
        row.time = "New".into();
        row.preview = "No messages yet".into();
    }
    if let Some(asset) = inputs.avatar.as_ref() {
        row.avatar_key = asset.key().into();
        if let Some(image) = images.get(row.avatar_key.as_str()) {
            row.avatar = image.clone();
        }
    }
    row
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
