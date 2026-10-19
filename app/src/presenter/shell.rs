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
        ui.set_chats(ModelRc::new(self.chats.clone()));
        ui.set_messages(ModelRc::new(self.messages.clone()));
        ui.set_friends(ModelRc::new(self.friends.clone()));
        ui.set_members(ModelRc::new(self.members.clone()));
    }
    pub(super) fn present_errors(&mut self, ui: &AppWindow, state: &AppState) -> String {
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
                Failure::Refused(reason) => {
                    format!("Couldn't load this item: {}", text::rejection(reason))
                }
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
        detail_error
    }
    pub(super) fn present_shell(&mut self, ui: &AppWindow, state: &AppState) {
        ui.global::<State>().set_panel(match &state.panel {
            Panel::Chats | Panel::Friends => PanelState::None,
            Panel::Settings => PanelState::Settings,
            Panel::Detail => PanelState::Detail,
            Panel::Viewer => PanelState::Viewer,
            Panel::SignIn => PanelState::SignIn,
        });
        ui.set_has_chat(state.selected.is_some());
        ui.global::<State>().set_window_sub(state.window_sub as i32);
        ui.global::<State>()
            .set_window_loading(state.window_loading);
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
    }
}
