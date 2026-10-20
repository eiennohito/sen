use crate::ShellState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_browse, |ui, d, section| {
        let g = ui.global::<G>();
        d.state.panel = if section == Section::Friends {
            Panel::Friends
        } else {
            Panel::Chats
        };
        d.state.detail.clear();
        g.set_section(section);
        if section == Section::Friends {
            d.sender.send(Intent::Friends);
        }
        d.render(&ui);
    });
    global_callback!(ui, actions, G, on_request_panel, |ui, d, panel| {
        let g = ui.global::<G>();
        let restore_viewer = panel != PanelState::Viewer && matches!(d.state.panel, Panel::Viewer);
        d.state.panel = match panel {
            PanelState::None => {
                if !d.state.detail.is_empty() && matches!(d.state.panel, Panel::Viewer) {
                    Panel::Detail
                } else if g.get_section() == Section::Friends {
                    Panel::Friends
                } else {
                    Panel::Chats
                }
            }
            PanelState::Settings => Panel::Settings,
            PanelState::Detail => Panel::Detail,
            PanelState::Viewer => Panel::Viewer,
            PanelState::SignIn => Panel::SignIn,
        };
        d.render(&ui);
        if restore_viewer {
            if ui
                .global::<ViewerState>()
                .get_viewer_key()
                .starts_with("profile:")
            {
                focus::restore_detail(&ui, false);
            } else {
                ui.invoke_focus_history();
            }
            ui.global::<ViewerState>()
                .set_viewer_image(slint::Image::default());
            d.viewer_asset = None;
            d.viewer_source = None;
        }
        match panel {
            PanelState::Settings => ui.invoke_focus_settings_close(),
            PanelState::SignIn => ui.invoke_focus_auth_start(),
            _ => {}
        }
    });
}
