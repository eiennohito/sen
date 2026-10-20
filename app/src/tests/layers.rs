use super::*;

#[test]
fn resizing_layers_keeps_focus_on_visible_content() {
    std::thread::spawn(|| {
        use slint::private_unstable_api::re_exports::WindowInner;
        let window = snapshot::install().unwrap();
        let ui = test_window();
        ui.show().unwrap();
        for layer in ["profile", "members", "member-profile", "settings"] {
            ui.global::<ShellState>().set_panel(PanelState::None);
            snapshot::render(&window, (980, 720)).unwrap();
            if layer == "settings" {
                ui.global::<ShellState>().set_panel(PanelState::Settings);
                ui.invoke_focus_settings_close();
            } else {
                ui.global::<ShellState>().set_panel(match layer {
                    "profile" => PanelState::Detail,
                    "members" => PanelState::Detail,
                    _ => PanelState::Detail,
                });
                ui.global::<DetailState>().set_detail_depth(1);
                ui.global::<DetailState>()
                    .set_detail_page(if layer == "members" {
                        DetailPageMode::Members
                    } else {
                        DetailPageMode::Profile
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

#[cfg(feature = "demo")]
#[test]
fn demo_scenarios_keep_keyboard_input_inside_the_dialog() {
    std::thread::spawn(|| {
        use slint::platform::WindowEvent;
        let _platform = snapshot::install().unwrap();
        let ui = test_window();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let weak = ui.as_weak();
        ui.global::<DemoState>().on_demo_event(move |kind| {
            assert!(
                !weak
                    .upgrade()
                    .unwrap()
                    .global::<DemoState>()
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
            ui.global::<DemoState>().set_demo(false);
            key("\u{f70f}"); // F12 is unavailable in account windows.
            assert!(!ui.global::<DemoState>().get_demo_scenarios_open());
            ui.global::<DemoState>().set_demo(true);
            key("\u{f70f}");
            assert!(ui.global::<DemoState>().get_demo_scenarios_open());
            assert!(ui.global::<ShellState>().get_modal_open());
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
            assert!(!ui.global::<ShellState>().get_modal_open());
            events.borrow_mut().clear();
            for index in 0..27 {
                key("\u{f70f}");
                for _ in 0..=index {
                    key("\t");
                }
                key("\n");
                if index == 2 {
                    // Media failures has a separate bounded scenario list.
                    assert!(ui.global::<DemoState>().get_demo_scenarios_open());
                    key("\t"); // Back
                    key("\t"); // First failure fixture
                    key("\n");
                }
                assert!(
                    !ui.global::<DemoState>().get_demo_scenarios_open(),
                    "scenario index {index}"
                );
                assert!(!ui.global::<ShellState>().get_modal_open());
            }
            assert_eq!(
                *events.borrow(),
                [
                    DemoScenario::History,
                    DemoScenario::Arrivals,
                    DemoScenario::MediaTimeout,
                    DemoScenario::Rejected,
                    DemoScenario::RejectNext,
                    DemoScenario::Unanswered,
                    DemoScenario::Reading,
                    DemoScenario::Rich,
                    DemoScenario::Photo,
                    DemoScenario::Sticker,
                    DemoScenario::ChatList,
                    DemoScenario::Group,
                    DemoScenario::Unread,
                    DemoScenario::Actions,
                    DemoScenario::Attachments,
                    DemoScenario::Protocol,
                    DemoScenario::Reaction,
                    DemoScenario::SignInCrypto,
                    DemoScenario::SignInState,
                    DemoScenario::SignInKeyring,
                    DemoScenario::SignInNetwork,
                    DemoScenario::SignInServer,
                    DemoScenario::SignInPhone,
                    DemoScenario::SignInCancelled,
                    DemoScenario::Expired,
                    DemoScenario::Reconnect,
                    DemoScenario::Offline
                ]
            );
            ui.invoke_focus_search();
            key("\u{f70f}");
            assert!(ui.global::<DemoState>().get_demo_scenarios_open());
            key("\u{1b}");
            assert!(!ui.global::<ShellState>().get_modal_open());
            // Escape restores the actual search target, with no intervening click.
            key("x");
            assert_eq!(search(&ui).accessible_value().unwrap(), "x");
            search(&ui).set_accessible_value("");
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
        ui.global::<AuthState>()
            .on_cancel_sign_in(move || count.set(count.get() + 1));
        ui.global::<ShellState>().set_panel(PanelState::SignIn);
        ui.global::<ShellState>()
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
        assert!(visible(&ui, "Keep waiting"));
        assert_eq!(cancelled.get(), 0);
        assert!(ui.global::<AuthState>().get_auth_busy());
        key("\n"); // Default action keeps waiting.
        assert!(!visible(&ui, "Keep waiting"));
        assert_eq!(cancelled.get(), 0);
        key("\u{1b}");
        key("\t");
        key("\n");
        assert!(!visible(&ui, "Keep waiting"));
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
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<ConversationState>()
            .set_chat_title("Preview conversation".into());
        ui.global::<DetailState>()
            .set_profile_name("Preview contact".into());
        ui.global::<DetailState>().set_profile_available(true);
        ui.global::<ConversationState>()
            .set_messages(ModelRc::from(Rc::new(VecModel::from(vec![MessageRow {
                body: "日本語の履歴 🐈\nThe reading surface stays usable.".into(),
                id: "synthetic-profile-message".into(),
                ..Default::default()
            }]))));
        let draft = "日本語の下書き 🐈";
        let mut presenter = presenter::Presenter::default();
        let mut state = AppState {
            selected: Some(ChatId("chat".into())),
            connection: Connection::Online,
            ..Default::default()
        };
        ui.global::<ShellState>()
            .set_connection(ConnectionState::Online);
        ui.global::<ComposerState>().set_draft(draft.into());
        state.composer = draft.into();
        presenter.present_draft(&ui, &state);
        assert_eq!(
            ui.global::<ComposerState>().get_composer_mode(),
            ComposerMode::Ready
        );
        ui.global::<ComposerState>().set_draft("".into());
        state.composer.clear();
        presenter.present_draft(&ui, &state);
        assert_eq!(
            ui.global::<ComposerState>().get_composer_mode(),
            ComposerMode::Idle
        );
        ui.global::<ComposerState>().set_draft(draft.into()); // Programmatic restoration emits no typing event.
        state.composer = draft.into();
        presenter.present_draft(&ui, &state);
        assert_eq!(
            ui.global::<ComposerState>().get_composer_mode(),
            ComposerMode::Ready
        );
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();
        let history = i_slint_backend_testing::ElementHandle::find_by_element_id(
            &ui,
            "Conversation::history",
        )
        .next()
        .unwrap();
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        for width in [740, 800] {
            ui.window()
                .set_size(slint::LogicalSize::new(width as f32, 720.));
            slint::platform::update_timers_and_animations();
            assert!(
                history.size().width >= 300.,
                "profile left an unusable reading width at {width}"
            );
            assert_eq!(ui.global::<ComposerState>().get_draft(), draft);
            assert!(ui.global::<DetailState>().get_detail_open());
        }
        ui.global::<DetailState>().invoke_pop_detail(true);
        assert!(!ui.global::<DetailState>().get_detail_open());
        assert_eq!(ui.global::<ComposerState>().get_draft(), draft);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(2);
        ui.global::<DetailState>()
            .set_detail_page(DetailPageMode::Profile);
        ui.invoke_focus_profile_close();
        assert!(visible(&ui, "Profile"));
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

#[test]
fn loading_history_never_renders_the_loaded_empty_identity() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let ui = test_window();
        ui.show().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        let visible = |label| {
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, label)
                .next()
                .is_some()
        };
        let advance =
            |ms| i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
        state.select(ChatId("empty".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        assert!(!visible("Loading history…"));
        assert!(!visible("No messages yet"));
        advance(300);
        assert!(visible("Loading history…"));
        state.window_loading = false;
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        assert!(!visible("Loading history…"));
        assert!(visible("No messages yet"));
    })
    .join()
    .unwrap();
}

#[test]
fn selecting_another_loading_chat_restarts_the_indicator_delay() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let ui = test_window();
        ui.show().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        let visible = || {
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(
                &ui,
                "Loading history…",
            )
            .next()
            .is_some()
        };
        let advance =
            |ms| i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
        state.select(ChatId("a".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        advance(200);
        state.select(ChatId("b".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        advance(100);
        assert!(!visible(), "B inherited A's elapsed delay");
        advance(200);
        assert!(visible());
        state.select(ChatId("c".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        assert!(!visible(), "C inherited the visible indicator");
        advance(299);
        assert!(!visible());
        advance(1);
        assert!(visible());
        state.select(ChatId("c".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        slint::platform::update_timers_and_animations();
        assert!(!visible(), "reselecting C must restart its loading delay");
        advance(300);
        assert!(visible());
    })
    .join()
    .unwrap();
}

#[test]
fn replacing_feedback_gets_a_full_dismissal_interval() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let ui = test_window();
        ui.show().unwrap();
        let advance = |ms| {
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
        };
        for reaction in [false, true] {
            let set = |value: &str| {
                if reaction {
                    ui.global::<ConversationState>()
                        .set_reaction_error_id(value.into());
                } else {
                    ui.global::<ConversationState>()
                        .set_unsend_feedback_id(value.into());
                }
                slint::platform::update_timers_and_animations();
            };
            let get = || {
                if reaction {
                    ui.global::<ConversationState>().get_reaction_error_id()
                } else {
                    ui.global::<ConversationState>().get_unsend_feedback_id()
                }
            };
            let duration = if reaction { 4000 } else { 3000 };
            set("first");
            advance(duration - 1000);
            set("replacement");
            advance(1000);
            assert_eq!(
                get(),
                "replacement",
                "replacement inherited the old timeout"
            );
            advance(duration - 1001);
            assert_eq!(get(), "replacement");
            advance(1);
            assert_eq!(get(), "", "feedback did not dismiss");
        }
    })
    .join()
    .unwrap();
}
