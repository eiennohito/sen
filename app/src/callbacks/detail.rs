use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_request_panel, |ui, d, panel| {
        let restore_viewer = panel != PanelState::Viewer && matches!(d.state.panel, Panel::Viewer);
        d.state.panel = match panel {
            PanelState::None => {
                if !d.state.detail.is_empty() && matches!(d.state.panel, Panel::Viewer) {
                    Panel::Detail
                } else if ui.global::<State>().get_section() == 1 {
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
                .global::<State>()
                .get_viewer_key()
                .starts_with("profile:")
            {
                ui.global::<State>().invoke_restore_detail_focus(false);
            } else {
                ui.invoke_focus_history();
            }
            ui.global::<State>().invoke_photo_closed();
        }
        match panel {
            PanelState::Settings => ui.invoke_focus_settings_close(),
            PanelState::SignIn => ui.invoke_focus_auth_start(),
            _ => {}
        }
    });
    global_callback!(ui, actions, on_pop_detail, |ui, d, all| {
        if all {
            d.state.close_detail();
        } else {
            d.state.pop_detail();
        }
        d.render(&ui);
        ui.global::<State>().invoke_restore_detail_focus(all);
    });
    global_callback!(ui, actions, on_detail_action, |ui, d, kind, value| {
        if kind == DetailAction::MemberChat {
            let id = ChatId(value.to_string());
            d.reveal = Some(id.clone());
            d.open_chat(&ui, id);
            d.state.detail.clear();
            d.state.panel = Panel::Chats;
            ui.global::<State>().set_section(0);
            ui.global::<State>().set_show_conversation(true);
            d.render(&ui);
            ui.invoke_focus_composer();
            return;
        }
        if d.state.action.is_some() {
            return;
        }
        let Some(page) = d.state.detail_page().cloned() else {
            return;
        };
        let action = match (kind, page) {
            (DetailAction::Rename, DetailPage::Profile(mid)) => ProfileAction::Rename {
                mid,
                name: value.to_string(),
            },
            (DetailAction::Add, DetailPage::Profile(mid)) => ProfileAction::AddFriend {
                mid,
                chat: d
                    .state
                    .selected
                    .as_ref()
                    .map(|c| c.0.clone())
                    .unwrap_or_default(),
            },
            (DetailAction::Mute, DetailPage::GroupInfo(chat)) => ProfileAction::Mute {
                disabled: !ui.global::<State>().get_group_muted(),
                chat,
            },
            (DetailAction::Leave, DetailPage::GroupInfo(chat)) => ProfileAction::Leave { chat },
            _ => return,
        };
        d.state.action_error = None;
        d.state.action = Some(action.clone());
        d.sender.send(Intent::ProfileAction(action));
        d.render(&ui);
    });
    global_callback!(ui, actions, on_open_members, |ui, d| {
        if let Some(chat) = d.state.selected.clone() {
            let push =
                matches!(d.state.detail_page(),Some(DetailPage::GroupInfo(id)) if id == &chat);
            if !push {
                d.state.detail.clear();
            }
            ui.global::<State>().invoke_remember_detail_focus(!push);
            ui.global::<State>().set_member_query("".into());
            d.state.push_detail(DetailPage::Members(chat.clone()));
            d.sender.send(Intent::Members(chat));
            d.render(&ui);
            ui.invoke_focus_profile_close();
        }
    });
    global_callback!(ui, actions, on_filter_members, |ui, d| {
        d.render(&ui);
    });
    global_callback!(ui, actions, on_open_profile, |ui, d, id| {
        d.state.detail.clear();
        ui.global::<State>().invoke_remember_detail_focus(true);
        let id = id.to_string();
        let group = d.state.friends.iter().any(|f| f.id == id && f.group)
            || d.state
                .chats
                .get(&ChatId(id.clone()))
                .is_some_and(|c| c.chat.is_group);
        if group {
            let chat = ChatId(id);
            if d.state.selected.as_ref() != Some(&chat) {
                d.open_chat(&ui, chat.clone());
            }
            ui.global::<State>().set_show_conversation(true);
            d.state.push_detail(DetailPage::GroupInfo(chat.clone()));
            d.sender.send(Intent::GroupInfo(chat.clone()));
            d.sender.send(Intent::Members(chat));
        } else {
            d.state.push_detail(DetailPage::Profile(id.clone()));
            if !d.state.profile_own(&id) {
                d.sender.send(Intent::Contact(id));
            }
        }
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    global_callback!(ui, actions, on_open_member, |ui, d, id| {
        ui.global::<State>()
            .invoke_remember_detail_focus(d.state.detail.is_empty());
        let id = id.to_string();
        d.state.push_detail(DetailPage::Profile(id.clone()));
        d.sender.send(Intent::Contact(id));
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    global_callback!(ui, actions, on_start_friend_chat, |ui, d| {
        if let Some(DetailPage::Profile(id)) = d.state.detail_page() {
            let id = ChatId(id.clone());
            d.reveal = Some(id.clone());
            d.open_chat(&ui, id);
            d.state.detail.clear();
            d.state.panel = Panel::Chats;
            ui.global::<State>().set_section(0);
            ui.global::<State>().set_show_conversation(true);
            d.render(&ui);
            ui.invoke_focus_composer();
        }
    });
}
