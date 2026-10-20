use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_start_sign_in, |ui, d| {
        ui.global::<State>().set_auth_error("".into());
        ui.global::<State>().set_auth_qr(slint::Image::default());
        ui.global::<State>().set_auth_pin("".into());
        ui.global::<State>()
            .set_auth_stage(text::PREPARING_SIGN_IN.into());
        ui.global::<State>().set_auth_cancelling(false);
        d.sender.send(Intent::SignIn);
    });
    global_callback!(ui, actions, on_cancel_sign_in, |ui, d| {
        ui.global::<State>().set_auth_cancelling(true);
        d.sender.cancel_sign_in();
        ui.global::<State>()
            .set_auth_stage(text::CANCELLING_SIGN_IN.into());
    });
    global_callback!(ui, actions, on_settings_changed, |ui, d| {
        let s = ui.global::<State>();
        d.sender.send(Intent::Prefs(Prefs {
            text_size: ui.global::<State>().get_text_size(),
            enter_sends: ui.global::<State>().get_enter_sends(),
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
            n.test(&ui, &d.state);
        } else {
            ui.global::<State>()
                .set_notification_status("Notification service not available".into());
        }
    });
}
