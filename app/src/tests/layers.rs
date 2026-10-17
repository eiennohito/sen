use super::*;
#[test]
fn escape_busy_sign_in_requires_a_decision() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let cancelled = Rc::new(std::cell::Cell::new(0));
        let count = cancelled.clone();
        ui.on_cancel_sign_in(move || count.set(count.get() + 1));
        ui.set_auth_busy(true);
        ui.invoke_open_sign_in();
        ui.show().unwrap();
        let key = |text: &str| {
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.into() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: text.into() });
        };
        key("\u{1b}");
        assert!(ui.global::<State>().get_auth_cancel_confirm());
        assert_eq!(cancelled.get(), 0);
        assert!(ui.get_auth_busy());
        key("\n"); // Default action keeps waiting.
        assert!(!ui.global::<State>().get_auth_cancel_confirm());
        assert_eq!(cancelled.get(), 0);
        key("\u{1b}");
        key("\t");
        key("\n");
        assert!(!ui.global::<State>().get_auth_cancel_confirm());
        assert_eq!(cancelled.get(), 1);
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn profile_panel_preserves_reading_at_wide_boundaries() {
    std::thread::spawn(|| {
        let platform = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.set_demo(true);
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.set_chat_title("Preview conversation".into());
        ui.set_profile_name("Preview contact".into());
        ui.set_profile_available(true);
        ui.set_messages(ModelRc::from(Rc::new(VecModel::from(vec![MessageRow {
            body: "日本語の履歴 🐈\nThe reading surface stays usable.".into(),
            id: "synthetic-profile-message".into(),
            ..Default::default()
        }]))));
        let draft = "日本語の下書き 🐈";
        ui.set_draft(draft.into());
        ui.show().unwrap();
        ui.set_profile_open(true);
        for width in [740, 800] {
            snapshot::save(
                &platform,
                &format!("/tmp/line-profile-{width}.ppm"),
                (width, 720),
            )
            .unwrap();
            assert!(
                ui.global::<State>().get_history_width() >= 300.,
                "profile left an unusable reading width at {width}"
            );
            assert_eq!(ui.get_draft(), draft);
            assert!(ui.get_profile_open());
        }
        ui.invoke_close_profile();
        assert!(!ui.get_profile_open());
        assert_eq!(ui.get_draft(), draft);
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn deferred_incoming_attention_runs_once_before_rendering() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.global::<State>().set_demo_notifications(true);
        ui.global::<State>().set_window_active(true);
        ui.set_list_pressed(true);
        let mut view = View {
            demo: true,
            current: "0".into(),
            ..Default::default()
        };
        let mut incoming = message("100", "mention before delayed rendering");
        incoming.mentioned = true;
        view.update(
            &ui,
            account::Update::Incoming {
                chat: "1".into(),
                messages: vec![incoming],
                replay: false,
            },
        );
        assert!(
            !ui.global::<State>().get_transient_notice().is_empty(),
            "attention was delayed behind rendering"
        );
        assert!(view.messages.is_empty());
        assert_eq!(
            view.notification_through.get("1").map(String::as_str),
            Some("100")
        );
        ui.global::<State>().set_transient_notice("".into());
        let mut incoming = message("100", "reaction updates the same message");
        incoming.mentioned = true;
        view.update(
            &ui,
            account::Update::Incoming {
                chat: "1".into(),
                messages: vec![incoming],
                replay: false,
            },
        );
        assert!(
            ui.global::<State>().get_transient_notice().is_empty(),
            "duplicate deferred row alerted again"
        );
        ui.set_list_pressed(false);
        for event in std::mem::take(&mut view.deferred) {
            view.update_render(&ui, event);
        }
        assert!(
            ui.global::<State>().get_transient_notice().is_empty(),
            "draining replayed attention"
        );
    })
    .join()
    .unwrap();
}
