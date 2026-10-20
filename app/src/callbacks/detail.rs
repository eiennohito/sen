use crate::DetailState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_open_detail_dialog, |ui, d, kind| {
        d.state.action_error = None;
        ui.global::<G>().set_detail_error("".into());
        ui.global::<G>().set_detail_dialog(kind);
    });
    global_callback!(ui, actions, G, on_profile_photo, |ui, d| {
        d.open_profile_photo(&ui);
    });

    global_callback!(ui, actions, G, on_pop_detail, |ui, d, all| {
        if all {
            d.state.close_detail();
        } else {
            d.state.pop_detail();
        }
        d.render(&ui);
        focus::restore_detail(&ui, all);
    });
    global_callback!(ui, actions, G, on_detail_action, |ui, d, kind, value| {
        let g = ui.global::<G>();
        if kind == DetailAction::MemberChat {
            let id = ChatId(value.to_string());
            d.open_conversation(&ui, id);
            return;
        }
        if d.state.action.is_some() {
            return;
        }
        let Some(page) = d.state.detail_page().cloned() else {
            return;
        };
        if matches!(kind, DetailAction::EditName | DetailAction::EditStatus)
            && !g.invoke_profile_edit_valid(kind, value.clone())
        {
            return;
        }
        let action = match (kind, page) {
            (DetailAction::EditName, DetailPage::Profile(mid)) if d.state.profile_own(&mid) => {
                ProfileAction::EditName {
                    name: value.trim().to_owned(),
                }
            }
            (DetailAction::EditStatus, DetailPage::Profile(mid)) if d.state.profile_own(&mid) => {
                ProfileAction::EditStatus {
                    status: value.to_string(),
                }
            }
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
                disabled: !g.get_group_muted(),
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
    global_callback!(ui, actions, G, on_open_members, |ui, d| {
        let g = ui.global::<G>();
        if let Some(chat) = d.state.selected.clone() {
            let push =
                matches!(d.state.detail_page(),Some(DetailPage::GroupInfo(id)) if id == &chat);
            if !push {
                d.state.detail.clear();
            }
            focus::remember_detail(&ui, !push);
            g.set_member_query("".into());
            d.state.push_detail(DetailPage::Members(chat.clone()));
            d.sender.send(Intent::Members(chat));
            d.render(&ui);
            ui.invoke_focus_profile_close();
        }
    });
    global_callback!(ui, actions, G, on_filter_members, |ui, d| {
        d.presenter.present_members(&ui, &d.state);
    });
    global_callback!(ui, actions, G, on_open_profile, |ui, d, id| {
        d.state.detail.clear();
        focus::remember_detail(&ui, true);
        let id = id.to_string();
        let group = d.state.friends.iter().any(|f| f.id == id && f.group)
            || d.state
                .chats
                .get(&ChatId(id.clone()))
                .is_some_and(|c| c.chat.is_group);
        if group {
            let chat = ChatId(id);
            if d.state.selected.as_ref() != Some(&chat) {
                d.switch_chat(&ui, chat.clone());
            }
            ui.global::<ShellState>().set_show_conversation(true);
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
    global_callback!(ui, actions, G, on_open_member, |ui, d, id| {
        focus::remember_detail(&ui, d.state.detail.is_empty());
        let id = id.to_string();
        d.state.push_detail(DetailPage::Profile(id.clone()));
        d.sender.send(Intent::Contact(id));
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    global_callback!(ui, actions, G, on_start_friend_chat, |ui, d| {
        if let Some(DetailPage::Profile(id)) = d.state.detail_page() {
            let id = ChatId(id.clone());
            d.open_conversation(&ui, id);
        }
    });
}
