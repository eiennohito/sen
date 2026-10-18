use super::*;
#[test]
fn demo_scenarios_keep_keyboard_input_inside_the_dialog() {
    std::thread::spawn(|| {
        use slint::platform::WindowEvent;
        let _platform = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let weak = ui.as_weak();
        ui.on_demo_event(move |kind| {
            assert!(
                !weak
                    .upgrade()
                    .unwrap()
                    .global::<State>()
                    .get_demo_scenarios_open()
            );
            captured.borrow_mut().push(kind);
        });
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let captured = notifications.clone();
        ui.global::<State>()
            .on_demo_notification(move |kind| captured.borrow_mut().push(kind));
        let key = |text: &str| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
            slint::platform::update_timers_and_animations();
        };
        ui.show().unwrap();
        for (width, height) in [(980., 720.), (760., 600.), (360., 520.)] {
            ui.window().set_size(slint::LogicalSize::new(width, height));
            ui.invoke_focus_search();
            ui.set_demo(false);
            key("\u{f70f}"); // F12 is unavailable in account windows.
            assert!(!ui.global::<State>().get_demo_scenarios_open());
            ui.set_demo(true);
            ui.global::<State>().set_demo_notifications(true);
            key("\u{f70f}");
            assert!(ui.global::<State>().get_demo_scenarios_open());
            assert!(ui.global::<State>().get_modal_open());
            // Reverse Tab wraps to the final action; forward Tab returns to Close.
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: "\u{10}".into(),
            });
            key("\t");
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: "\u{10}".into(),
            });
            key("\t");
            key("\n");
            assert!(!ui.global::<State>().get_modal_open());
            events.borrow_mut().clear();
            notifications.borrow_mut().clear();
            for index in 0..24 {
                key("\u{f70f}");
                for _ in 0..=index {
                    key("\t");
                }
                key("\n");
                assert!(!ui.global::<State>().get_demo_scenarios_open());
                assert!(!ui.global::<State>().get_modal_open());
            }
            assert_eq!(
                *events.borrow(),
                [
                    0, 1, 2, 10, 3, 11, 5, 6, 7, 8, 9, 14, 12, 15, 4, 16, 17, 19, 18
                ]
            );
            assert_eq!(*notifications.borrow(), [0, 1, 2, 3, 4]);
            ui.invoke_focus_search();
            key("\u{f70f}");
            assert!(ui.global::<State>().get_demo_scenarios_open());
            key("\u{1b}");
            assert!(!ui.global::<State>().get_modal_open());
            // Escape restores the actual search target, with no intervening click.
            key("x");
            assert_eq!(ui.get_query(), "x");
            ui.set_query("".into());
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn escape_busy_sign_in_requires_a_decision() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let cancelled = Rc::new(std::cell::Cell::new(0));
        let count = cancelled.clone();
        ui.on_cancel_sign_in(move || count.set(count.get() + 1));
        ui.set_auth_busy(true);
        ui.set_auth_expired(true);
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
        let _platform = snapshot::install().unwrap();
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
            ui.window()
                .set_size(slint::LogicalSize::new(width as f32, 720.));
            slint::platform::update_timers_and_animations();
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

#[test]
fn profile_sheet_scopes_browser_shortcuts_and_escape() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let state = Rc::new(RefCell::new(View {
            demo: true,
            friends_loaded: true,
            ..Default::default()
        }));
        for (index, chat) in demo::examples().into_iter().enumerate() {
            let mut view = state.borrow_mut();
            view.ids.push(index.to_string());
            view.rows.push(ChatRow {
                index: index as i32,
                title: chat.title.into(),
                activity: order_key(3 - index as i64).into(),
                ..Default::default()
            });
            view.friends.push(FriendRow {
                id: index.to_string().into(),
                name: chat.title.into(),
                ..Default::default()
            });
        }
        state.borrow().filter(&ui);
        state.borrow_mut().select(&ui, 0);
        let weak = ui.as_weak();
        let view = state.clone();
        ui.on_browse(move |section| view.borrow_mut().browse(&weak.upgrade().unwrap(), section));
        let weak = ui.as_weak();
        let view = state.clone();
        ui.on_switch_chat(move |delta| {
            view.borrow_mut()
                .switch_visible_chat(&weak.upgrade().unwrap(), delta)
        });
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(980, 720));
        slint::platform::update_timers_and_animations();
        let key = |text: &str, pressed: bool| {
            ui.window().dispatch_event(if pressed {
                slint::platform::WindowEvent::KeyPressed { text: text.into() }
            } else {
                slint::platform::WindowEvent::KeyReleased { text: text.into() }
            });
        };
        let modified = |modifier: &str, text: &str| {
            key(modifier, true);
            key(text, true);
            key(text, false);
            key(modifier, false);
        };
        state.borrow_mut().browse(&ui, 1);
        state.borrow_mut().open_profile(&ui, "0");
        ui.invoke_focus_friend_search();
        modified("\u{11}", "2");
        assert_eq!(ui.get_section(), 0);
        assert!(!ui.get_profile_open());
        state.borrow_mut().browse(&ui, 1);
        ui.set_friend_query("Keep query".into());
        state.borrow_mut().open_profile(&ui, "0");
        ui.invoke_focus_friend_search();
        key("\u{1b}", true);
        key("\u{1b}", false);
        assert!(!ui.get_profile_open());
        assert_eq!(ui.get_friend_query(), "Keep query");
        state.borrow_mut().open_profile(&ui, "0");
        ui.invoke_focus_friend_search();
        modified("\u{12}", "\u{f701}");
        assert_eq!(ui.get_selected_chat(), 1);
        assert!(!ui.get_profile_open());
        state.borrow_mut().open_profile(&ui, "1");
        window.set_size(slint::PhysicalSize::new(360, 520));
        slint::platform::update_timers_and_animations();
        ui.invoke_focus_profile_close();
        modified("\u{11}", "2");
        assert_eq!(
            ui.get_section(),
            1,
            "full-window sheet must block browser shortcuts"
        );
        assert!(ui.get_profile_open());
        modified("\u{11}", "\n");
        assert!(
            ui.get_profile_open(),
            "Ctrl+Enter must not activate the sheet's close button"
        );
        key("\u{1b}", true);
        key("\u{1b}", false);
        assert!(!ui.get_profile_open());
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
