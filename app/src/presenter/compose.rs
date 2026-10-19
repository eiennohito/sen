use super::*;

impl Presenter {
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
        global.set_stickers_failed(state.sticker_packages_failed);
        let tabs = state
            .sticker_packages
            .iter()
            .map(|p| StickerTab {
                name: single_line(&p.name).into(),
                picture: p.stickers.first().map(picture).unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        // Retain tab FocusScopes while selection and sticker assets change.
        if global.get_sticker_tabs().iter().collect::<Vec<_>>() != tabs {
            global.set_sticker_tabs(ModelRc::new(VecModel::from(tabs)));
        }
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
    pub(super) fn quote(&self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let image_key = m.body.image().map(|a| a.key()).unwrap_or_default();
        let image = self.images.get(&image_key);
        MessageRow {
            sender_name: if m.outgoing {
                "You".into()
            } else {
                crate::system_events::name(state, &m.sender_id).into()
            },
            sender_id: m.sender_id.clone().into(),
            body: single_line(&compact_text(&m.body, state, &m.sender_id)).into(),
            image_key: image_key.into(),
            picture: image.cloned().unwrap_or_default(),
            has_picture: image.is_some(),
            photo: matches!(m.body, Content::Photo { .. }),
            sticker: matches!(m.body, Content::Sticker { .. }),
            ..Default::default()
        }
    }
    pub(super) fn present_draft(&mut self, ui: &AppWindow, state: &AppState) {
        if ui.get_draft().as_str() != state.composer {
            ui.set_draft(state.composer.clone().into());
        }
    }
}
