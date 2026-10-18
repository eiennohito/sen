use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    callback!(ui, actions, on_start_sign_in, |ui, d| {
        ui.set_auth_error("".into());
        d.sender.send(Intent::SignIn);
    });
    callback!(ui, actions, on_cancel_sign_in, |ui, d| {
        d.sender.cancel_sign_in();
        ui.set_auth_stage("Cancelling sign-in…".into());
    });
    callback!(ui, actions, on_settings_changed, |ui, d| {
        if !ui.global::<State>().get_notifications_enabled()
            && let Some(n) = &d.notifications
        {
            n.service.withdraw_notifications();
        }
        let s = ui.global::<State>();
        d.sender.send(Intent::Prefs(Prefs {
            text_size: ui.get_text_size(),
            enter_sends: ui.get_enter_sends(),
            desktop: DesktopPrefs {
                background: s.get_background_mode(),
                autostart: s.get_autostart(),
                notifications: s.get_notifications_enabled(),
                preview: s.get_notification_preview(),
                sound: s.get_notification_sound(),
                mention_exception: s.get_notification_mention_exception(),
                group_events: s.get_notification_group_events(),
            },
            ..d.state.prefs.clone()
        }));
    });
    global_callback!(ui, actions, on_test_notification, |ui, d| {
        if let Some(n) = &d.notifications {
            n.test(&ui);
        } else {
            ui.global::<State>()
                .set_notification_status("Notification service not available".into());
        }
    });
}
