use super::*;

#[test]
fn resizing_layers_keeps_focus_on_visible_content() {
    std::thread::spawn(|| {
        use slint::private_unstable_api::re_exports::WindowInner;
        let window = snapshot::install().unwrap();
        let ui = test_window();
        ui.show().unwrap();
        for layer in ["profile", "members", "member-profile", "settings"] {
            ui.global::<State>().set_panel(PanelState::None);
            snapshot::render(&window, (980, 720)).unwrap();
            if layer == "settings" {
                ui.global::<State>().set_panel(PanelState::Settings);
                ui.invoke_focus_settings_close();
            } else {
                ui.global::<State>().set_panel(match layer {
                    "profile" => PanelState::Profile,
                    "members" => PanelState::Members,
                    _ => PanelState::MemberProfile,
                });
                ui.invoke_focus_profile_close();
            }
            snapshot::render(&window, (980, 720)).unwrap();
            if layer != "settings" {
                ui.invoke_focus_search();
            }
            for width in [739, 740, 1059, 1060, 1059, 739] {
                snapshot::render(&window, (width, 720)).unwrap();
                let item = WindowInner::from_pub(ui.window())
                    .focus_item
                    .borrow()
                    .upgrade()
                    .expect("resize lost keyboard focus");
                assert!(item.is_visible(), "{layer} at {width}: focus is hidden");
                if layer != "settings" && width == 1059 {
                    ui.invoke_focus_search();
                }
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn demo_scenarios_keep_keyboard_input_inside_the_dialog() {
    std::thread::spawn(|| {
        use slint::platform::WindowEvent;
        let _platform = snapshot::install().unwrap();
        let ui = test_window();
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
            for index in 0..18 {
                key("\u{f70f}");
                for _ in 0..=index {
                    key("\t");
                }
                key("\n");
                if index == 2 {
                    // Media failures has a separate bounded scenario list.
                    assert!(ui.global::<State>().get_demo_scenarios_open());
                    key("\t"); // Back
                    key("\t"); // First failure fixture
                    key("\n");
                }
                assert!(
                    !ui.global::<State>().get_demo_scenarios_open(),
                    "scenario index {index}"
                );
                assert!(!ui.global::<State>().get_modal_open());
            }
            assert_eq!(
                *events.borrow(),
                [
                    0, 1, 20, 10, 31, 3, 8, 12, 5, 6, 15, 7, 14, 32, 33, 19, 16, 17
                ]
            );
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
        let ui = test_window();
        let cancelled = Rc::new(std::cell::Cell::new(0));
        let count = cancelled.clone();
        ui.on_cancel_sign_in(move || count.set(count.get() + 1));
        ui.global::<State>().set_panel(PanelState::SignIn);
        ui.global::<State>()
            .set_connection(ConnectionState::SigningIn);
        ui.invoke_focus_auth_start();
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
        let ui = test_window();
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
        ui.global::<State>().set_connection(ConnectionState::Online);
        ui.set_draft(draft.into());
        assert!(ui.get_can_send());
        ui.set_draft("".into());
        assert!(!ui.get_can_send());
        ui.set_draft(draft.into()); // Programmatic restoration emits no typing event.
        assert!(ui.get_can_send());
        ui.show().unwrap();
        ui.global::<State>().set_panel(PanelState::Profile);
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
        ui.global::<State>().set_panel(PanelState::MemberProfile);
        ui.invoke_focus_profile_close();
        assert!(ui.get_member_profile());
        use slint::private_unstable_api::re_exports::WindowInner;
        let focused = WindowInner::from_pub(ui.window())
            .focus_item
            .borrow()
            .upgrade()
            .expect("member profile has no focus");
        assert!(focused.is_visible(), "member profile focus is hidden");
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
