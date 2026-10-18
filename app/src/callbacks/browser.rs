use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_open_new_chat, |ui, d| {
        let global = ui.global::<State>();
        global.set_new_chat_busy(false);
        global.set_new_chat_uncertain(false);
        global.set_new_chat_query("".into());
        global.set_new_chat_name("".into());
        global.set_new_chat_error("".into());
        global.set_new_chat_selected(ModelRc::new(VecModel::<ComposeFriend>::default()));
        d.presenter.present_compose(&ui, &d.state);
        global.set_new_chat_open(true);
        d.sender.send(Intent::Friends);
    });
    global_callback!(ui, actions, on_new_chat_filter, |ui, d| {
        d.presenter.present_compose(&ui, &d.state);
    });
    global_callback!(ui, actions, on_new_chat_toggle, |ui, d, id| {
        let global = ui.global::<State>();
        let mut selected = global.get_new_chat_selected().iter().collect::<Vec<_>>();
        if selected.iter().any(|f| f.id == id) {
            selected.retain(|f| f.id != id);
        } else if let Some(mut friend) = global.get_new_chat_friends().iter().find(|f| f.id == id) {
            friend.selected = true;
            selected.push(friend);
        }
        global.set_new_chat_selected(ModelRc::new(VecModel::from(selected)));
        d.presenter.present_compose(&ui, &d.state);
    });
    global_callback!(ui, actions, on_new_chat_submit, |ui, d| {
        let global = ui.global::<State>();
        if global.get_new_chat_uncertain() {
            global.set_new_chat_open(false);
            d.state.panel = Panel::Chats;
            ui.set_section(0);
            ui.set_show_conversation(false);
            d.render(&ui);
            ui.invoke_focus_search();
            return;
        }
        let targets = global
            .get_new_chat_selected()
            .iter()
            .map(|f| f.id.to_string())
            .collect::<Vec<_>>();
        if targets.len() == 1 {
            d.capture_draft(&ui);
            let id = ChatId(targets[0].clone());
            d.reveal = Some(id.clone());
            let intent = d.state.select(id);
            d.sender.send(intent);
            d.state.panel = Panel::Chats;
            ui.set_section(0);
            ui.set_show_conversation(true);
            global.set_new_chat_open(false);
            d.render(&ui);
            ui.invoke_focus_composer();
        } else if targets.len() > 1 {
            global.set_new_chat_busy(true);
            global.set_new_chat_error("".into());
            d.sender.send(Intent::CreateChat {
                name: global.get_new_chat_name().to_string(),
                targets,
            });
        }
    });
    callback!(ui, actions, on_select_chat, |ui, d, index| {
        d.select(&ui, index as usize);
    });
    callback!(ui, actions, on_switch_chat, |ui, d, delta| {
        let i = ui.get_selected_chat() + delta;
        if i >= 0 && (i as usize) < d.state.chat_order.len() {
            d.select(&ui, i as usize);
        }
    });
    callback!(ui, actions, on_search_changed, |ui, d, query| {
        d.query = query.to_lowercase();
        d.render(&ui);
    });
    callback!(ui, actions, on_filter_category, |ui, d| {
        d.category = ui.get_category();
        d.render(&ui);
    });
    callback!(ui, actions, on_browse, |ui, d, section| {
        d.state.panel = if section == 1 {
            Panel::Friends
        } else {
            Panel::Chats
        };
        d.state.detail.clear();
        ui.set_section(section);
        if section == 1 {
            d.sender.send(Intent::Friends);
        }
        d.render(&ui);
    });
    callback!(ui, actions, on_refresh_friends, |_ui, d| {
        d.sender.send(Intent::Friends);
    });
    callback!(ui, actions, on_filter_friends, |ui, d| {
        d.render(&ui);
    });
}
