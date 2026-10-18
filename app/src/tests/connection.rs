use super::*;
use account::{ConnectionFailure, ListRequest};
use line_wasm_poc::client::desktop::sync::SyncState;

fn render(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    (width, height): (u32, u32),
) {
    window.set_size(slint::PhysicalSize::new(width, height));
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), width as usize);
        });
    }
}

fn fixture(ui: &AppWindow) -> View {
    ui.set_demo(true);
    let mut view = View {
        demo: true,
        ..Default::default()
    };
    for (index, chat) in demo::examples().into_iter().enumerate() {
        view.ids.push(index.to_string());
        view.rows.push(ChatRow {
            index: index as i32,
            title: chat.title.into(),
            preview: chat.preview.into(),
            initial: chat.initial.into(),
            ..Default::default()
        });
    }
    view.filter(ui);
    view.select(ui, 0);
    view
}

#[test]
fn connection_feedback_and_list_failure() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        ui.set_draft("Keep this draft 🐈".into());
        ui.show().unwrap();
        view.update(&ui, account::Update::Sync(SyncState::Connected));
        assert!(!ui.global::<State>().get_reconnected_notice());
        for attempt in 1..5 {
            view.update(
                &ui,
                account::Update::Sync(SyncState::Reconnecting { attempt }),
            );
            assert!(!ui.get_connection_failed());
            assert!(!ui.global::<State>().get_sign_in_available());
            assert!(ui.get_retrying_connection());
            assert_eq!(ui.get_connection_status(), "Connecting…");
        }
        view.update(&ui, account::Update::Sync(SyncState::Connected));
        assert!(!ui.get_connection_failed());
        assert!(!ui.get_retrying_connection());
        assert!(ui.global::<State>().get_reconnected_notice());
        slint::platform::update_timers_and_animations();
        std::thread::sleep(std::time::Duration::from_millis(3050));
        slint::platform::update_timers_and_animations();
        assert!(!ui.global::<State>().get_reconnected_notice());
        view.update(&ui, account::Update::Sync(SyncState::Offline));
        assert!(ui.get_connection_failed());
        assert!(!ui.global::<State>().get_sign_in_available());
        view.update(&ui, account::Update::Sync(SyncState::AuthExpired));
        assert!(ui.get_connection_failed());
        assert!(ui.global::<State>().get_sign_in_available());
        view.update(&ui, account::Update::Sync(SyncState::Connected));
        ui.set_loading_chats(true);
        ui.set_can_send(true);
        let send_reason = ui.get_send_reason();
        view.update(&ui, account::Update::ChatsFailed(ListRequest::Details));
        assert!(ui.get_connected());
        assert!(ui.get_can_send());
        assert_eq!(ui.get_send_reason(), send_reason);
        assert!(!ui.get_connection_failed());
        assert!(!ui.get_loading_chats());
        assert!(!ui.get_more_chats(), "details failure invented a next page");
        assert_eq!(ui.get_list_error(), "Couldn’t load conversations. Retry.");
        assert_eq!(ui.get_draft(), "Keep this draft 🐈");

        {
            let directory = std::env::var("LINE_CONNECTION_SNAPSHOTS").ok();
            let mut geometry = Vec::new();
            for (width, height) in [(980, 720), (360, 520), (1400, 900)] {
                for text in [15, 24] {
                    ui.set_text_size(text);
                    for name in [
                        "connected",
                        "connecting",
                        "reconnected",
                        "offline",
                        "retrying",
                        "auth-expired",
                        "expired-retrying",
                        "startup-failure",
                        "list-failure",
                    ] {
                        view.update(&ui, account::Update::Sync(SyncState::Connected));
                        ui.global::<State>().set_reconnected_notice(false);
                        ui.set_list_error("".into());
                        ui.set_more_chats(false);
                        match name {
                            "connecting" | "reconnected" => {
                                view.update(
                                    &ui,
                                    account::Update::Sync(SyncState::Reconnecting { attempt: 1 }),
                                );
                                if name == "reconnected" {
                                    view.update(&ui, account::Update::Sync(SyncState::Connected));
                                }
                            }
                            "offline" | "retrying" => {
                                view.update(&ui, account::Update::Sync(SyncState::Offline));
                                ui.set_retrying_connection(name == "retrying");
                            }
                            "auth-expired" | "expired-retrying" => {
                                view.update(&ui, account::Update::Sync(SyncState::AuthExpired));
                                ui.set_retrying_connection(name == "expired-retrying");
                            }
                            "startup-failure" => view.update(
                                &ui,
                                account::Update::ConnectionFailed(ConnectionFailure::Session),
                            ),
                            "list-failure" => {
                                view.update(&ui, account::Update::ChatsFailed(ListRequest::Details))
                            }
                            _ => {}
                        }
                        for surface in ["conversation", "browser", "profile"] {
                            ui.set_show_conversation(surface != "browser");
                            ui.set_profile_open(surface == "profile");
                            ui.set_profile_name("葵".into());
                            ui.set_profile_available(true);
                            ui.window().request_redraw();
                            if let Some(directory) = &directory {
                                snapshot::save(&window, &format!("{directory}/{name}-{surface}-{width}x{height}-text{text}.ppm"), (width, height)).unwrap();
                            } else {
                                render(&window, (width, height));
                            }
                            let state = ui.global::<State>();
                            let x = state.get_connection_area_x();
                            let y = state.get_connection_area_y();
                            let w = state.get_connection_area_width();
                            let h = state.get_connection_notice_height();
                            let visible = state.get_connection_feedback_visible();
                            if visible { assert!(ui.invoke_connection_actions_fit(), "{name}/{surface}/{width}/text{text}: actions escaped the strip"); }
                            let obstacles = ui.invoke_connection_obstacles();
                            let mut measured = Vec::new();
                            for index in 0..obstacles.row_count() {
                                let rect = obstacles.row_data(index).unwrap();
                                let intersects = visible && rect.width > 0. && rect.height > 0.
                                    && x < rect.x + rect.width - 0.5 && rect.x < x + w - 0.5
                                    && y < rect.y + rect.height - 0.5 && rect.y < y + h - 0.5;
                                assert!(!intersects, "{name}/{surface}/{width}/text{text}: strip intersects protected surface {index}");
                                measured.push(serde_json::json!({"surface": index, "x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}));
                            }
                            geometry.push(serde_json::json!({"state": name, "surface": surface, "size": [width,height], "text": text, "visible": visible, "strip": [x,y,w,h], "obstacles": measured}));
                        }
                        ui.set_profile_open(false);
                    }
                }
            }
            if let Some(directory) = directory {
                std::fs::write(format!("{directory}/geometry.json"), serde_json::to_vec_pretty(&geometry).unwrap()).unwrap();
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn startup_and_failed_retry_preserve_sign_in_and_drafts() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        view.demo = false;
        let (sender, commands) = std::sync::mpsc::sync_channel(8);
        view.commands = Some(sender.into());
        ui.set_draft("Preserve this draft".into());
        view.update(
            &ui,
            account::Update::ConnectionFailed(ConnectionFailure::Session),
        );
        assert!(ui.get_session_unavailable());
        assert!(ui.global::<State>().get_sign_in_available());
        ui.invoke_open_sign_in();
        assert!(ui.get_auth_open());
        ui.invoke_close_sign_in();
        view.update(&ui, account::Update::Sync(SyncState::AuthExpired));
        for reason in [ConnectionFailure::Network, ConnectionFailure::Session] {
            view.reconnect(&ui);
            assert!(ui.get_retrying_connection());
            assert!(matches!(
                commands.try_recv().unwrap(),
                account::Command::Reconnect
            ));
            view.update(&ui, account::Update::ConnectionFailed(reason));
            assert!(ui.get_auth_expired());
            assert!(ui.global::<State>().get_sign_in_available());
            assert!(!ui.get_retrying_connection());
            assert_eq!(ui.get_draft(), "Preserve this draft");
        }
        view.update(&ui, account::Update::Connected("Local account".into()));
        assert!(!ui.global::<State>().get_sign_in_available());
        view.update(
            &ui,
            account::Update::ConnectionFailed(ConnectionFailure::Network),
        );
        assert!(!ui.global::<State>().get_sign_in_available());
    })
    .join()
    .unwrap();
}

#[test]
fn footer_retry_preserves_request_and_network_state() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        let (sender, commands) = std::sync::mpsc::sync_channel(1);
        view.commands = Some(sender.clone().into());
        ui.set_connected(true);
        for request in [
            ListRequest::Refresh,
            ListRequest::More,
            ListRequest::Details,
        ] {
            sender.send(account::Command::Friends).unwrap();
            assert!(!view.request(&ui, request.command()));
            assert!(!ui.get_list_error().is_empty());
            commands.try_recv().unwrap();
            view.retry_list(&ui);
            assert!(ui.get_loading_chats());
            assert!(ui.get_list_error().is_empty());
            match (request, commands.try_recv().unwrap()) {
                (ListRequest::Refresh, account::Command::Chats { next: false })
                | (ListRequest::More, account::Command::Chats { next: true })
                | (ListRequest::Details, account::Command::ChatDetails) => {}
                _ => panic!("footer retried a different operation"),
            }
            view.update(&ui, account::Update::ChatsFailed(request));
            view.retry_list(&ui);
            match (request, commands.try_recv().unwrap()) {
                (ListRequest::Refresh, account::Command::Chats { next: false })
                | (ListRequest::More, account::Command::Chats { next: true })
                | (ListRequest::Details, account::Command::ChatDetails) => {}
                _ => panic!("footer changed a failed operation"),
            }
            assert!(ui.get_connected());
            assert!(!ui.get_connection_failed());
        }
        view.update(
            &ui,
            account::Update::Error {
                message: "Try again".into(),
                generation: None,
            },
        );
        view.retry_list(&ui);
        assert!(matches!(
            commands.try_recv().unwrap(),
            account::Command::ChatDetails
        ));
    })
    .join()
    .unwrap();
}

#[test]
fn reconnect_events_never_notify_or_sound() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        view.attention = Some(attention::Attention::recording());
        let state = ui.global::<State>();
        state.set_notifications_enabled(true);
        state.set_notification_sound(true);
        state.set_demo_notifications(true);
        for active in [false, true] {
            state.set_window_active(active);
            view.update(&ui, account::Update::Sync(SyncState::Connected));
            for attempt in 1..5 {
                view.update(
                    &ui,
                    account::Update::Sync(SyncState::Reconnecting { attempt }),
                );
            }
            view.update(&ui, account::Update::Sync(SyncState::Connected));
            let notices = view.attention.as_ref().unwrap().take_recorded();
            assert!(
                notices.is_empty(),
                "reconnect posted a desktop notification or sound"
            );
            assert!(state.get_reconnected_notice());
        }
        state.set_window_active(false);
        view.update(&ui, account::Update::Sync(SyncState::Offline));
        let notices = view.attention.as_ref().unwrap().take_recorded();
        assert_eq!(
            notices.len(),
            1,
            "recorder failed to observe an account alert"
        );
        assert!(!notices[0].sound);
    })
    .join()
    .unwrap();
}

#[test]
fn connection_feedback_does_not_cover_history_retry() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        ui.set_text_size(24);
        ui.set_show_conversation(true);
        ui.set_history_error("Couldn’t load messages.".into());
        view.update(&ui, account::Update::Sync(SyncState::Offline));
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(360, 520));
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(360, 520);
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), 360);
        });
        let state = ui.global::<State>();
        let obstacles = ui.invoke_connection_obstacles();
        let history_retry = obstacles.row_data(5).unwrap();
        assert!(history_retry.width > 0.);
        assert!(
            history_retry.y >= state.get_connection_area_y() + state.get_connection_notice_height()
        );
        if let Ok(directory) = std::env::var("LINE_CONNECTION_SNAPSHOTS") {
            ui.window().request_redraw();
            snapshot::save(
                &window,
                &format!("{directory}/offline-history-error-360x520-text24.ppm"),
                (360, 520),
            )
            .unwrap();
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn covered_scroll_content_can_be_fully_revealed() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        ui.show().unwrap();
        for (width, height) in [(980, 720), (360, 520), (1400, 900)] {
            for text in [15, 24] {
                ui.set_text_size(text);
                for history in [false, true] {
                    if !history && width != 360 {
                        continue;
                    }
                    ui.set_show_conversation(history);
                    ui.set_following(false);
                    ui.global::<State>().set_older_failed(history);
                    ui.global::<State>()
                        .set_older_status("Couldn’t load older messages.".into());
                    ui.set_messages(slint::ModelRc::new(slint::VecModel::from(
                        demo::paged_rows(0..30),
                    )));
                    if !history {
                        ui.set_chats(slint::ModelRc::new(slint::VecModel::from(vec![
                            view.rows[0].clone(),
                        ])));
                    }
                    view.update(&ui, account::Update::Sync(SyncState::Connected));
                    ui.global::<State>().set_reconnected_notice(false);
                    render(&window, (width, height));
                    if history {
                        ui.invoke_history_jump(0.);
                    } else {
                        ui.invoke_list_jump(0.);
                    }
                    render(&window, (width, height));
                    let before = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    let old_extent = ui.global::<State>().get_history_extent();
                    view.update(&ui, account::Update::Sync(SyncState::Offline));
                    render(&window, (width, height));
                    let covered = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    assert!(
                        (before.y - covered.y).abs() < 0.5,
                        "feedback moved anchored content"
                    );
                    let state = ui.global::<State>();
                    let inset = state.get_connection_notice_height();
                    if history {
                        assert!((state.get_history_extent() - old_extent - inset).abs() < 0.5);
                    }
                    if history {
                        ui.invoke_history_jump(0.);
                    } else {
                        ui.invoke_list_jump(0.);
                    }
                    render(&window, (width, height));
                    let revealed = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    assert!(revealed.width > 0. && revealed.height > 0.);
                    assert!(revealed.x >= state.get_connection_area_x() - 0.5);
                    assert!(
                        revealed.x + revealed.width
                            <= state.get_connection_area_x()
                                + state.get_connection_area_width()
                                + 0.5,
                        "first control escapes horizontally"
                    );
                    assert!(
                        revealed.y >= state.get_connection_area_y() + inset - 0.5,
                        "Home left the first control covered"
                    );
                    assert!(
                        revealed.y + revealed.height
                            <= state.get_connection_area_y()
                                + state.get_connection_area_height()
                                + 0.5,
                        "first control cannot fit in viewport"
                    );
                    if let Ok(directory) = std::env::var("LINE_CONNECTION_SNAPSHOTS") {
                        ui.window().request_redraw();
                        snapshot::save(
                            &window,
                            &format!(
                                "{directory}/offline-home-{}-{width}x{height}-text{text}.ppm",
                                if history { "history" } else { "short-list" }
                            ),
                            (width, height),
                        )
                        .unwrap();
                    }
                    view.update(&ui, account::Update::Sync(SyncState::Connected));
                    state.set_reconnected_notice(false);
                    render(&window, (width, height));
                    let uncovered = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    assert!(
                        (revealed.y - uncovered.y).abs() < 0.5,
                        "removing feedback moved anchored content"
                    );
                    let offset = if history {
                        ui.get_history_y()
                    } else {
                        ui.get_list_y()
                    };
                    assert!(offset <= 0., "residual escaped into the public offset");
                    if history {
                        assert!(offset >= -state.get_history_extent());
                    }
                    if history {
                        ui.invoke_history_scroll(5.);
                    } else {
                        ui.invoke_list_scroll(5.);
                    }
                    render(&window, (width, height));
                    let upward = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    assert!(
                        (upward.y - uncovered.y).abs() < 0.5,
                        "upward input moved rows upward"
                    );
                    if history {
                        ui.invoke_history_scroll(-5.);
                    } else {
                        ui.invoke_list_scroll(-5.);
                    }
                    render(&window, (width, height));
                    let downward = if history {
                        ui.invoke_connection_obstacles().row_data(6).unwrap()
                    } else {
                        ui.invoke_connection_first_row()
                    };
                    assert!(
                        (downward.y - upward.y + 5.).abs() < 0.5,
                        "small downward input jumped over the residual"
                    );
                    if history {
                        ui.invoke_history_scroll(-inset);
                    } else {
                        ui.invoke_list_scroll(-inset);
                    }
                    render(&window, (width, height));
                    let settled = if history {
                        ui.get_history_y()
                    } else {
                        ui.get_list_y()
                    };
                    assert!(settled <= 0.);
                    if history {
                        assert!(settled >= -state.get_history_extent());
                    }
                }
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn exposure_uses_only_the_visible_history_viewport() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        let (sender, commands) = std::sync::mpsc::sync_channel(8);
        view.commands = Some(sender.into());
        view.demo = false;
        ui.show().unwrap();
        for (width, height) in [(980, 720), (360, 520), (1400, 900)] {
            for text in [15, 24] {
                ui.set_text_size(text);
                ui.set_show_conversation(true);
                ui.set_following(false);
                for feedback in [
                    SyncState::Offline,
                    SyncState::AuthExpired,
                    SyncState::Connected,
                ] {
                    view.update(
                        &ui,
                        account::Update::Sync(SyncState::Reconnecting { attempt: 1 }),
                    );
                    view.update(&ui, account::Update::Sync(feedback));
                    render(&window, (width, height));
                    ui.invoke_history_jump(0.);
                    render(&window, (width, height));
                    let state = ui.global::<State>();
                    let visible = state.get_history_viewport_height();
                    let raw = state.get_connection_area_height();
                    assert!((visible - (raw - state.get_connection_notice_height())).abs() < 0.5);
                    let mut measured = history_window::HistoryWindow::default();
                    measured.measured("10", visible - 30., 20., false);
                    measured.measured("11", visible + 2., 10., false);
                    measured.measured("12", visible, 10., false);
                    view.paging
                        .borrow_mut()
                        .insert(view.current.clone(), measured);
                    view.messages = ["10", "11", "12"]
                        .into_iter()
                        .map(|id| MessageRow {
                            id: id.into(),
                            ..Default::default()
                        })
                        .collect();
                    view.unread = Default::default();
                    view.unread.server(&view.current, Some(3), Some("9"));
                    view.unread.page(&view.current, &view.messages);
                    state.set_window_active(true);
                    ui.set_loading_history(false);
                    ui.set_history_error("".into());
                    view.history_exposure(&ui);
                    assert_eq!(
                        view.unread.exposed,
                        ["10"],
                        "covered bottom counted as exposed"
                    );
                    let acknowledgements: Vec<_> = commands
                        .try_iter()
                        .filter_map(|command| match command {
                            account::Command::MarkRead { up_to_message, .. } => Some(up_to_message),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(acknowledgements, ["10"], "acknowledged an unseen row");
                    ui.set_connection_failed(false);
                    state.set_reconnected_notice(false);
                    render(&window, (width, height));
                    ui.invoke_history_jump(0.);
                    render(&window, (width, height));
                    assert!((state.get_history_viewport_height() - raw).abs() < 0.5);
                    view.history_exposure(&ui);
                    assert_eq!(
                        view.unread.exposed,
                        ["10", "12", "11"],
                        "hidden strip changed normal exposure"
                    );
                    let acknowledgements: Vec<_> = commands
                        .try_iter()
                        .filter_map(|command| match command {
                            account::Command::MarkRead { up_to_message, .. } => Some(up_to_message),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        acknowledgements,
                        ["12"],
                        "normal read acknowledgement changed"
                    );
                }
                ui.set_show_conversation(false);
                view.update(&ui, account::Update::Sync(SyncState::Offline));
                render(&window, (width, height));
                if width == 360 {
                    ui.invoke_list_jump(0.);
                    render(&window, (width, height));
                    assert!(
                        (ui.get_chat_list_height()
                            - (ui.global::<State>().get_connection_area_height()
                                - ui.global::<State>().get_connection_notice_height()))
                        .abs()
                            < 0.5
                    );
                }
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn dark_connection_feedback_geometry() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        ui.invoke_preview_dark(true);
        ui.show().unwrap();
        for (width, height) in [(980, 720), (360, 520), (1400, 900)] {
            for text in [15, 24] {
                ui.set_text_size(text);
                for (name, sync) in [("connecting", SyncState::Reconnecting { attempt: 1 }), ("offline", SyncState::Offline), ("auth-expired", SyncState::AuthExpired), ("reconnected", SyncState::Connected)] {
                    view.update(&ui, account::Update::Sync(SyncState::Reconnecting { attempt: 1 }));
                    view.update(&ui, account::Update::Sync(sync));
                    for surface in ["conversation", "browser", "profile"] {
                        ui.set_show_conversation(surface != "browser");
                        ui.set_profile_open(surface == "profile");
                        render(&window, (width, height));
                        if ui.global::<State>().get_connection_feedback_visible() { assert!(ui.invoke_connection_actions_fit()); }
                        if let Ok(directory) = std::env::var("LINE_CONNECTION_SNAPSHOTS") {
                            ui.window().request_redraw();
                            snapshot::save(&window, &format!("{directory}/dark-{name}-{surface}-{width}x{height}-text{text}.ppm"), (width, height)).unwrap();
                        }
                    }
                    ui.set_profile_open(false);
                }
            }
        }
        ui.hide().unwrap();
    }).join().unwrap();
}

#[test]
fn residual_inset_keeps_short_history_reading_until_scrolled_to_latest() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = fixture(&ui);
        ui.set_show_conversation(true);
        ui.set_following(false);
        ui.set_messages(slint::ModelRc::new(slint::VecModel::from(
            Vec::<MessageRow>::new(),
        )));
        ui.show().unwrap();
        view.update(&ui, account::Update::Sync(SyncState::Offline));
        render(&window, (980, 720));
        ui.invoke_history_jump(0.);
        render(&window, (980, 720));
        let state = ui.global::<State>();
        let inset = state.get_connection_notice_height();
        ui.set_connection_failed(false);
        state.set_reconnected_notice(false);
        render(&window, (980, 720));
        let visible = state.get_history_viewport_height();
        ui.invoke_history_scroll(5.);
        render(&window, (980, 720));
        assert!(!ui.get_following());
        assert!((state.get_history_viewport_height() - visible).abs() < 0.5);
        assert_eq!(ui.get_history_y(), 0.);
        ui.invoke_history_scroll(-inset);
        render(&window, (980, 720));
        assert!(ui.get_following());
        assert_eq!(ui.get_history_y(), 0.);
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
