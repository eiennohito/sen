use super::*;

impl Presenter {
    pub(super) fn present_shell(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<ShellState>();
        g.set_unread_conversations(
            state.chats.values().filter(|c| c.unread_count > 0).count() as i32
        );
        g.set_unread_badge(unread_badge(g.get_unread_conversations()));
        g.set_panel(match &state.panel {
            Panel::Chats | Panel::Friends => PanelState::None,
            Panel::Settings => PanelState::Settings,
            Panel::Detail => PanelState::Detail,
            Panel::Viewer => PanelState::Viewer,
            Panel::SignIn => PanelState::SignIn,
        });
        g.set_storage_error(
            state
                .error
                .map(crate::text::storage_failure)
                .unwrap_or("")
                .into(),
        );
        g.set_account_avatar(self.images.picture(state.self_contact.avatar.as_ref()));
        g.set_account_initial(name_initial(state.profile.display_name.as_str()));
        g.set_account_mid(state.profile.mid.clone().into());
        g.set_account_name(state.profile.display_name.as_str().into());
        g.set_connection(match state.connection {
            Connection::Local => ConnectionState::Local,
            Connection::Connecting => ConnectionState::Connecting,
            Connection::Online => ConnectionState::Online,
            Connection::Offline => ConnectionState::Offline,
            Connection::SignedOut { .. } => ConnectionState::SignedOut,
            Connection::Fatal => ConnectionState::Fatal,
            Connection::SigningIn => ConnectionState::SigningIn,
        });
    }
}
