use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    focus_bridge!(ui, actions, on_focus_cycle_end, invoke_focus_cycle_end);
    focus_bridge!(
        ui,
        actions,
        on_focus_after_browser,
        invoke_focus_after_browser
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_after_conversation,
        invoke_focus_after_conversation
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_before_layer,
        invoke_focus_before_layer
    );
    focus_bridge!(ui, actions, on_focus_after_layer, invoke_focus_after_layer);
    focus_bridge!(ui, actions, on_focus_friend_list, invoke_focus_friend_list);
    focus_bridge!(ui, actions, on_focus_chat_list, invoke_focus_chat_list);
    focus_bridge!(ui, actions, on_focus_search, invoke_focus_search);
    focus_bridge!(
        ui,
        actions,
        on_focus_friend_search,
        invoke_focus_friend_search
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_settings_opener,
        invoke_focus_settings_opener
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_settings_close,
        invoke_focus_settings_close
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_profile_close,
        invoke_focus_profile_close
    );
    focus_bridge!(ui, actions, on_focus_auth_start, invoke_focus_auth_start);
    focus_bridge!(
        ui,
        actions,
        on_focus_connection_first,
        invoke_focus_connection_first
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_sign_in_opener,
        invoke_focus_sign_in_opener
    );
    focus_bridge!(
        ui,
        actions,
        on_focus_viewer_close,
        invoke_focus_viewer_close
    );
    focus_bridge!(ui, actions, on_focus_history, invoke_focus_history);
    focus_bridge!(ui, actions, on_focus_composer, invoke_focus_composer);
    focus_bridge!(
        ui,
        actions,
        on_focus_reading_before_editor,
        invoke_focus_reading_before_editor
    );
}
