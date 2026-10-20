use super::*;

impl Presenter {
    pub fn install(&self, ui: &AppWindow) {
        let state = ui.global::<State>();
        state.set_reaction_kinds(ModelRc::new(VecModel::from(
            PREDEFINED_REACTIONS
                .iter()
                .map(|(id, label, glyph)| ReactionRow {
                    id: *id as i32,
                    label: (*label).into(),
                    glyph: (*glyph).into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        state.set_preparing_sign_in_text(text::PREPARING_SIGN_IN.into());
        state.set_signed_in_text(text::SIGNED_IN.into());
        state.set_chats(self.images.map(self.chats.clone(), chats::chat_row));
        state.set_messages(self.images.map(self.messages.clone(), messages::project));
        state.set_friends(self.images.map(self.friends.clone(), people::friend_row));
        state.set_members(self.images.map(self.members.clone(), people::member_row));
        state.set_new_chat_friends(
            self.images
                .map(self.compose_friends.clone(), compose::compose_row),
        );
        state.set_new_chat_selected(
            self.images
                .map(self.compose_selected.clone(), compose::compose_row),
        );
        state.set_sticker_tabs(self.images.map(self.sticker_tabs.clone(), compose::tab_row));
        state.set_sticker_items(
            self.images
                .map(self.sticker_items.clone(), compose::sticker_row),
        );
    }
    pub(super) fn present_errors(&mut self, ui: &AppWindow, state: &AppState) -> String {
        let g = ui.global::<State>();
        let mut conversation_error = String::new();
        let mut friends_error = String::new();
        let mut chat_list_error = String::new();
        let mut detail_error = String::new();
        let detail_id = state.detail.last().map(|page| match page {
            line::controller::DetailPage::Profile(id) => id.as_str(),
            line::controller::DetailPage::GroupInfo(id)
            | line::controller::DetailPage::Members(id) => id.0.as_str(),
        });
        for (request, failure) in &state.refresh_failures {
            use line::{client::error::Failure, engine::Refresh};
            let message = if matches!(request, Refresh::Session) {
                match failure {
                    Failure::Refused(_) | Failure::Bug(_) => {
                        "Couldn't check your saved session.".into()
                    }
                    _ => continue,
                }
            } else {
                match failure {
                    Failure::Refused(reason) => {
                        format!("Couldn't load this item: {}", text::rejection(reason))
                    }
                    Failure::Bug(_) => "Couldn't load this item.".into(),
                    _ => continue,
                }
            };
            match request {
                Refresh::Friends => friends_error = message,
                Refresh::Session | Refresh::Profile | Refresh::ChatList | Refresh::MoreChats => {
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
        detail_error
    }
    pub(super) fn present_shell(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<State>();
        g.set_panel(match &state.panel {
            Panel::Chats | Panel::Friends => PanelState::None,
            Panel::Settings => PanelState::Settings,
            Panel::Detail => PanelState::Detail,
            Panel::Viewer => PanelState::Viewer,
            Panel::SignIn => PanelState::SignIn,
        });
        g.set_has_chat(state.selected.is_some());
        g.set_window_sub(state.window_sub as i32);
        g.set_window_loading(state.window_loading);
        g.set_storage_error(
            state
                .error
                .map(crate::text::storage_failure)
                .unwrap_or("")
                .into(),
        );
        g.set_account_initial(name_initial(state.profile.display_name.as_str()));
        g.set_has_account(!state.profile.mid.is_empty());
        g.set_account_mid(state.profile.mid.clone().into());
        g.set_account_name(state.profile.display_name.as_str().into());
        g.set_connection(match state.connection {
            Connection::Local => ConnectionState::Local,
            Connection::Connecting { .. } => ConnectionState::Connecting,
            Connection::Online => ConnectionState::Online,
            Connection::Offline => ConnectionState::Offline,
            Connection::SignedOut { .. } => ConnectionState::SignedOut,
            Connection::Fatal => ConnectionState::Fatal,
            Connection::SigningIn => ConnectionState::SigningIn,
        });
        g.set_send_pending(state.submitting.is_some());
    }
}
