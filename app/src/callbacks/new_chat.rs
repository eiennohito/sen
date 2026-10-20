use crate::NewChatState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_open_new_chat, |ui, d| {
        let g = ui.global::<G>();
        g.set_new_chat_busy(false);
        g.set_new_chat_uncertain(false);
        g.set_new_chat_query("".into());
        g.set_new_chat_name("".into());
        g.set_new_chat_error("".into());
        d.presenter.clear_compose_selection();
        g.set_new_chat_open(true);
        d.presenter.present_new_chat(&ui, &d.state);
        d.sender.send(Intent::Friends);
    });
    global_callback!(ui, actions, G, on_new_chat_filter, |ui, d| {
        d.presenter.present_new_chat(&ui, &d.state);
    });
    global_callback!(ui, actions, G, on_new_chat_toggle, |ui, d, id| {
        d.presenter.toggle_compose_friend(id.as_str());
        d.presenter.present_new_chat(&ui, &d.state);
    });
    global_callback!(ui, actions, G, on_new_chat_submit, |ui, d| {
        let g = ui.global::<G>();
        if g.get_new_chat_uncertain() {
            g.set_new_chat_open(false);
            d.state.panel = Panel::Chats;
            ui.global::<ShellState>().set_section(Section::Chats);
            ui.global::<ShellState>().set_show_conversation(false);
            d.render(&ui);
            ui.invoke_focus_search();
            return;
        }
        let targets = g
            .get_new_chat_selected()
            .iter()
            .map(|f| f.id.to_string())
            .collect::<Vec<_>>();
        if targets.len() == 1 {
            let id = ChatId(targets[0].clone());
            g.set_new_chat_open(false);
            d.show_chat(&ui, id);
        } else if targets.len() > 1 {
            g.set_new_chat_busy(true);
            g.set_new_chat_error("".into());
            d.sender.send(Intent::CreateChat {
                name: g.get_new_chat_name().to_string(),
                targets,
            });
        }
    });
}
