use crate::ChatsState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_more, |_ui, d| {
        d.sender.send(Intent::MoreChats);
    });

    global_callback!(ui, actions, G, on_select_chat, |ui, d, index| {
        d.select(&ui, index as usize);
    });
    global_callback!(ui, actions, G, on_switch_chat, |ui, d, delta| {
        let g = ui.global::<G>();
        let i = g.get_selected_chat() + delta;
        if i >= 0 && (i as usize) < d.state.chat_order.len() {
            d.select(&ui, i as usize);
        }
    });
    global_callback!(ui, actions, G, on_search_changed, |ui, d| {
        d.render(&ui);
    });
    global_callback!(ui, actions, G, on_filter_category, |ui, d| {
        let g = ui.global::<G>();
        d.category = g.get_category();
        d.render(&ui);
    });
}
