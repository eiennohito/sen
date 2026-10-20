use super::system_events;
use super::*;

impl Presenter {
    pub fn clear_compose_selection(&mut self) {
        self.compose_ids.clear();
    }
    pub fn toggle_compose_friend(&mut self, id: &str) {
        if !self.compose_ids.remove(id) {
            self.compose_ids.insert(id.into());
        }
    }
    pub fn present_compose(&mut self, ui: &AppWindow, state: &AppState) {
        self.present_quote(ui, state);
        let global = ui.global::<State>();
        if global.get_new_chat_open() {
            let query = global.get_new_chat_query().to_lowercase();
            let (mut friends, mut chosen, mut names) = (Vec::new(), Vec::new(), Vec::new());
            for f in state
                .friends
                .iter()
                .filter(|f| !f.group && f.id != state.profile.mid)
            {
                let name = single_line(text::friend_name(f));
                let selected = self.compose_ids.contains(&f.id);
                let row = (
                    f.id.clone(),
                    (f.clone(), selected),
                    f.avatar.iter().map(|a| a.key()).collect(),
                );
                if selected {
                    names.push(name.clone());
                    chosen.push(row.clone());
                }
                if name.to_lowercase().contains(&query) {
                    friends.push(row);
                }
            }
            global.set_new_chat_count(chosen.len() as i32);
            global.set_new_chat_placeholder(names.join(", ").into());
            self.compose_selected.replace(chosen);
            self.compose_friends.replace(friends);
        }
        global.set_stickers_failed(state.sticker_packages_failed);
        self.sticker_tabs.replace(
            state
                .sticker_packages
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    (
                        i,
                        (p.name.clone(), p.stickers.first().cloned()),
                        p.stickers.first().map(content_assets).unwrap_or_default(),
                    )
                })
                .collect(),
        );
        self.sticker_items.replace(
            state
                .sticker_packages
                .get(global.get_sticker_package() as usize)
                .map(|p| {
                    p.stickers
                        .iter()
                        .enumerate()
                        .map(|(i, s)| (i, (i, s.clone()), content_assets(s)))
                        .collect()
                })
                .unwrap_or_default(),
        );
    }
    pub(super) fn present_quote(&self, ui: &AppWindow, state: &AppState) {
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
    }
    pub(super) fn quote(&self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let image_key = m.body.image().map(|a| a.key()).unwrap_or_default();
        let image = self.images.get(&image_key);
        MessageRow {
            sender_name: if m.outgoing {
                "You".into()
            } else {
                system_events::name(state, &m.sender_id).into()
            },
            sender_id: m.sender_id.clone().into(),
            kind: kind_label(&m.body).0,
            body: single_line(&compact_text(&m.body, state, &m.sender_id)).into(),
            image_key: image_key.into(),
            picture: image.clone().unwrap_or_default(),
            has_picture: image.is_some(),
            ..Default::default()
        }
    }
    pub(crate) fn present_draft(&mut self, ui: &AppWindow, state: &AppState) {
        let global = ui.global::<State>();
        global.set_composer_mode(if state.selected.is_none() {
            ComposerMode::Unavailable
        } else if state
            .selected
            .as_ref()
            .is_some_and(|id| state.chats.get(id).is_some_and(|c| c.chat.left))
        {
            ComposerMode::Left
        } else if !matches!(state.connection, Connection::Online) {
            ComposerMode::Offline
        } else if state.submitting.is_none() && !state.composer.trim().is_empty() {
            ComposerMode::Ready
        } else {
            ComposerMode::Idle
        });
        if ui.global::<State>().get_draft().as_str() != state.composer {
            ui.global::<State>()
                .set_draft(state.composer.clone().into());
        }
    }
}

pub(super) fn compose_row((f, selected): (Friend, bool), images: &Images) -> ComposeFriend {
    ComposeFriend {
        id: f.id.clone().into(),
        name: single_line(text::friend_name(&f)).into(),
        initial: name_initial(text::friend_name(&f)),
        avatar: images.picture(f.avatar.as_ref()),
        selected,
    }
}
pub(super) fn tab_row((name, picture): (String, Option<Content>), images: &Images) -> StickerTab {
    StickerTab {
        name: single_line(&name).into(),
        picture: images.picture(picture.as_ref().and_then(Content::image)),
    }
}
pub(super) fn sticker_row((index, content): (usize, Content), images: &Images) -> StickerItem {
    StickerItem {
        index: index as i32,
        picture: images.picture(content.image()),
    }
}
