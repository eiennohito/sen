use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_focus_after_browser, |ui, _d| {
        ui.invoke_focus_after_browser();
    });
    global_callback!(ui, actions, on_focus_after_conversation, |ui, _d| {
        ui.invoke_focus_after_conversation();
    });
    global_callback!(ui, actions, on_focus_after_layer, |ui, _d| {
        ui.invoke_focus_after_layer();
    });
    global_callback!(ui, actions, on_focus_before_layer, |ui, _d| {
        ui.invoke_focus_before_layer();
    });
    global_callback!(ui, actions, on_focus_chat_list, |ui, _d| {
        ui.invoke_focus_chat_list();
    });
    global_callback!(ui, actions, on_focus_composer, |ui, _d| {
        ui.invoke_focus_composer();
    });
    global_callback!(ui, actions, on_focus_connection_first, |ui, _d| {
        ui.invoke_focus_connection_first();
    });
    global_callback!(ui, actions, on_focus_cycle_end, |ui, _d| {
        ui.invoke_focus_cycle_end();
    });
    global_callback!(ui, actions, on_focus_friend_list, |ui, _d| {
        ui.invoke_focus_friend_list();
    });
    global_callback!(ui, actions, on_focus_friend_search, |ui, _d| {
        ui.invoke_focus_friend_search();
    });
    global_callback!(ui, actions, on_focus_history, |ui, _d| {
        ui.invoke_focus_history();
    });
    global_callback!(ui, actions, on_focus_profile_close, |ui, _d| {
        ui.invoke_focus_profile_close();
    });
    global_callback!(ui, actions, on_focus_reading_before_editor, |ui, _d| {
        ui.invoke_focus_reading_before_editor();
    });
    global_callback!(ui, actions, on_focus_search, |ui, _d| {
        ui.invoke_focus_search();
    });
    global_callback!(ui, actions, on_focus_settings_opener, |ui, _d| {
        ui.invoke_focus_settings_opener();
    });
}
