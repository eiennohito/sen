use crate::SettingsState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_confirm_logout, |ui, d| {
        d.capture_draft(&ui);
        let g = ui.global::<G>();
        g.set_logout_busy(true);
        d.sender.send(Intent::Logout {
            delete_data: g.get_logout_delete_data(),
        });
    });
    global_callback!(ui, actions, G, on_settings_changed, |ui, d| {
        let g = ui.global::<G>();
        d.sender.send(Intent::Prefs(Prefs {
            text_size: g.get_text_size(),
            enter_sends: g.get_enter_sends(),
            desktop: DesktopPrefs {
                background: g.get_background_mode(),
                autostart: g.get_autostart(),
                notifications: g.get_notifications_enabled(),
                preview: match g.get_notification_preview() {
                    sen_ui::NotificationPreview::NameAndText => {
                        sen::store::NotificationPreview::NameAndText
                    }
                    sen_ui::NotificationPreview::NameOnly => {
                        sen::store::NotificationPreview::NameOnly
                    }
                    sen_ui::NotificationPreview::NewMessage => {
                        sen::store::NotificationPreview::NewMessage
                    }
                },
                sound: g.get_notification_sound(),
                mention_exception: g.get_notification_mention_exception(),
                group_events: g.get_notification_group_events(),
            },
            ..d.state.prefs.clone()
        }));
    });
    global_callback!(ui, actions, G, on_test_notification, |ui, d| {
        let g = ui.global::<G>();
        if let Some(n) = &d.notifications {
            n.test(&ui, &d.state);
        } else {
            g.set_notification_status("Notification service not available".into());
        }
    });
}
