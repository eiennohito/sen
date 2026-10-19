use super::*;

impl Presenter {
    pub(super) fn present_chats(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        query: &str,
        category: i32,
    ) {
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
    }
    pub(super) fn present_header(&mut self, ui: &AppWindow, state: &AppState) {
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
