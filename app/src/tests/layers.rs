use super::*;

#[test]
fn resizing_layers_keeps_focus_on_visible_content() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        ui.show().unwrap();
        #[cfg(feature = "demo")]
        {
            press(&ui, slint::platform::Key::F12);
            assert!(!ui.global::<DemoState>().get_demo_scenarios_open());
        }
        for layer in ["profile", "members", "member-profile", "settings"] {
            ui.global::<ShellState>().set_panel(PanelState::None);
            ui.window().set_size(slint::LogicalSize::new(980., 720.));
            slint::platform::update_timers_and_animations();
            if layer == "settings" {
                ui.global::<ShellState>().set_panel(PanelState::Settings);
                ui.invoke_focus_settings_close();
            } else {
                ui.global::<ShellState>().set_panel(PanelState::Detail);
                ui.global::<DetailState>().set_detail_depth(1);
                ui.global::<DetailState>()
                    .set_detail_page(if layer == "members" {
                        DetailPageMode::Members
                    } else {
                        DetailPageMode::Profile
                    });
                ui.invoke_focus_profile_close();
            }
            ui.window().set_size(slint::LogicalSize::new(980., 720.));
            slint::platform::update_timers_and_animations();
            if layer != "settings" {
                ui.invoke_focus_search();
            }
            for width in [739, 740, 1059, 1060, 1059, 739] {
                ui.window()
                    .set_size(slint::LogicalSize::new(width as f32, 720.));
                slint::platform::update_timers_and_animations();
                let item = focus_item(&ui);
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
fn escape_busy_sign_in_requires_a_decision() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        let cancelled = Rc::new(std::cell::Cell::new(0));
        let count = cancelled.clone();
        ui.global::<AuthState>()
            .on_cancel_sign_in(move || count.set(count.get() + 1));
        ui.global::<ShellState>().set_panel(PanelState::SignIn);
        ui.global::<ShellState>()
            .set_connection(ConnectionState::SigningIn);
        ui.invoke_focus_auth_start();
        ui.show().unwrap();
        press(&ui, "\u{1b}");
        assert!(visible(&ui, "Keep waiting"));
        assert_eq!(cancelled.get(), 0);
        assert!(ui.global::<AuthState>().get_auth_busy());
        press(&ui, "\n"); // Default action keeps waiting.
        assert!(!visible(&ui, "Keep waiting"));
        assert_eq!(cancelled.get(), 0);
        press(&ui, "\u{1b}");
        press(&ui, "\t");
        press(&ui, "\n");
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
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
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
        for (text, mode) in [
            (draft, ComposerMode::Ready),
            ("", ComposerMode::Idle),
            (draft, ComposerMode::Ready),
        ] {
            ui.global::<ComposerState>().set_draft(text.into());
            state.composer = text.into();
            presenter.present_draft(&ui, &state);
            assert_eq!(ui.global::<ComposerState>().get_composer_mode(), mode);
        }
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
                "unusable reading width at {width}"
            );
            assert_eq!(ui.global::<ComposerState>().get_draft(), draft);
            assert!(ui.global::<DetailState>().get_detail_open());
        }
        ui.global::<DetailState>().invoke_pop_detail(true);
        assert_eq!(ui.global::<ComposerState>().get_draft(), draft);
        assert!(!ui.global::<DetailState>().get_detail_open());
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(2);
        ui.global::<DetailState>()
            .set_detail_page(DetailPageMode::Profile);
        ui.invoke_focus_profile_close();
        assert!(visible(&ui, "Profile"));
        let focused = focus_item(&ui);
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
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        ui.show().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        let visible = |label| {
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, label)
                .next()
                .is_some()
        };
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
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
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
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        ui.show().unwrap();
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

#[test]
fn logout_dialog_and_signed_out_screen() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let (ui, runtime, worker) = test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        let apply = |delta| runtime.update(|d| d.apply_batch(&ui, [delta]));
        let timeout = std::time::Duration::from_secs(10);
        let wait = |online| loop {
            let delta = worker.deltas.recv_timeout(timeout).unwrap();
            let done =
                matches!(&delta, Delta::Connection(c) if matches!(c, Connection::Online) == online);
            apply(delta);
            if done {
                break;
            }
        };
        wait(true);
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        ui.show().unwrap();
        ui.invoke_focus_settings_opener();
        let settings_focus = focus_item(&ui);
        ui.invoke_focus_auth_start();
        apply(Delta::Pair(sen::backend::PairEvent::Stage(
            sen::client::pairing::Stage::Authenticated,
        )));
        apply(Delta::Reset);
        apply(Delta::Connection(Connection::Online));
        assert!(focus_item(&ui) != settings_focus);
        ui.global::<ShellState>().set_account_mid("self".into());
        ui.global::<ShellState>().set_panel(PanelState::Settings);
        ui.invoke_focus_settings_close();
        press(&ui, "\t");
        press(&ui, slint::platform::Key::End);
        press(&ui, "\t");
        press(&ui, "\n");
        assert!(!ui.global::<SettingsState>().get_logout_delete_data());
        assert_eq!(focus_label(&ui), "Cancel");
        let checkbox = "Also delete this account's chats and drafts from this computer";
        for _ in 0..8 {
            press(&ui, "\t");
            assert!(["Cancel", "Log out", checkbox].contains(&focus_label(&ui).as_str()));
        }
        press(&ui, "\u{1b}");
        assert!(!ui.global::<SettingsState>().get_logout_open());
        element(&ui, "Log out…").invoke_accessible_default_action();
        ui.global::<SettingsState>().set_logout_busy(true);
        let confirmed = Rc::new(std::cell::Cell::new(false));
        let flag = confirmed.clone();
        ui.global::<SettingsState>()
            .on_confirm_logout(move || flag.set(true));
        for label in ["Cancel", "Log out"] {
            let button = element(&ui, label);
            assert_eq!(button.accessible_enabled(), Some(false));
            button.invoke_accessible_default_action();
            button.mock_single_click(slint::platform::PointerEventButton::Left);
        }
        for key in ["\u{1b}", "\t", "\n"] {
            press(&ui, key);
        }
        assert!(!confirmed.get());
        assert!(ui.global::<SettingsState>().get_logout_open());
        callbacks::install(&ui, runtime.clone());
        ui.global::<SettingsState>().set_logout_busy(false);
        element(&ui, "Log out").invoke_accessible_default_action();
        assert!(ui.global::<SettingsState>().get_logout_busy());
        wait(false);
        assert!(!ui.global::<SettingsState>().get_logout_open());
        assert!(!ui.global::<SettingsState>().get_logout_busy());
        for label in ["Not signed in", "Sign in"] {
            assert!(visible(&ui, label));
        }
        assert_eq!(ui.global::<ShellState>().get_unread_conversations(), 0);
        assert_eq!(ui.global::<ShellState>().get_account_mid(), "");
        assert_eq!(ui.global::<ComposerState>().get_draft(), "");
        apply(Delta::LogoutCleanupFailed);
        assert!(visible(&ui, "Some local data could not be deleted"));
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
