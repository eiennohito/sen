use super::*;
fn controller_scenario(stop: usize) {
    let _window = snapshot::install().unwrap();
    let ui = create_app_window().unwrap();
    let mut view = View {
        demo: true,
        ..Default::default()
    };
    for (index, chat) in demo::examples().into_iter().enumerate() {
        view.ids.push(index.to_string());
        view.rows.push(ChatRow {
            index: index as i32,
            title: chat.title.into(),
            ..Default::default()
        });
    }
    view.filter(&ui);
    view.select(&ui, 0);
    ui.set_draft("日本語 🐈\nsecond line\n".into());
    ui.set_following(false);
    ui.set_history_y(-73.);
    view.select(&ui, 1);
    assert_eq!(view.history["0"].y, -73.);
    ui.set_draft("another draft".into());
    view.select(&ui, 0);
    assert_eq!(ui.get_draft(), "日本語 🐈\nsecond line\n");
    let original_model = ui.get_messages();
    view.select(&ui, 0);
    assert_eq!(ui.get_messages(), original_model);
    view.query = "weekend".into();
    view.filter(&ui);
    assert_eq!(ui.get_chats().row_count(), 1);
    assert_eq!(view.current, "0");
    assert_eq!(ui.get_draft(), "日本語 🐈\nsecond line\n");
    let generation = view.generation;
    view.update(
        &ui,
        account::Update::Messages {
            id: "1".into(),
            generation: generation - 1,
            messages: vec![message("late", "previous chat")],
        },
    );
    assert_eq!(view.current, "0");
    assert_eq!(view.history["1"].messages[0].id, "late");
    let subtitle = ui.get_chat_subtitle();
    view.update(
        &ui,
        account::Update::Error {
            message: "old failure".into(),
            generation: Some(generation - 1),
        },
    );
    assert_eq!(ui.get_chat_subtitle(), subtitle);
    assert!(ui.get_history_error().is_empty());
    view.update(
        &ui,
        account::Update::Messages {
            id: "0".into(),
            generation,
            messages: vec![message("a", "選択できるメッセージ 🐈\nline two")],
        },
    );
    let model = ui.get_messages();
    ui.set_new_count(0);
    view.update(
        &ui,
        account::Update::Messages {
            id: "0".into(),
            generation,
            messages: vec![
                message("a", "選択できるメッセージ 🐈\nline two"),
                message("b", "new arrival"),
            ],
        },
    );
    assert_eq!(ui.get_messages(), model, "append rebuilt the message model");
    assert_eq!(ui.get_new_count(), 1);
    assert!(!ui.get_following());
    view.update(
        &ui,
        account::Update::ChatsFailed(account::ListRequest::Refresh),
    );
    assert_eq!(view.ids.len(), 3);
    assert_eq!(view.current, "0");
    assert_eq!(ui.get_messages(), model);
    view.saved.outbox.push(storage::Outgoing {
        local_id: "pending".into(),
        request_seq: None,
        timestamp_ms: 1,
        chat: "1".into(),
        text: "submitted text".into(),
        status: "Pending…".into(),
        server_id: String::new(),
    });
    ui.set_draft("newer typing".into());
    view.update(
        &ui,
        account::Update::SendFailed {
            id: "1".into(),
            local_id: "pending".into(),
        },
    );
    assert!(view.saved.outbox[0].status.contains("may have arrived"));
    assert_eq!(ui.get_draft(), "newer typing");
    assert_eq!(ui.get_messages().row_count(), 2);
    view.select(&ui, 1);
    assert_eq!(ui.get_messages().row_count(), 2);
    let pending_model = ui.get_messages();
    let pending_time = pending_model.row_data(1).unwrap().time;
    view.update(
        &ui,
        account::Update::Sent {
            id: "1".into(),
            local_id: "pending".into(),
            server_id: "server".into(),
        },
    );
    assert!(view.saved.outbox[0].status.starts_with("Server confirmed"));
    assert_eq!(ui.get_messages().row_data(1).unwrap().delivery, "Sent");
    assert_eq!(ui.get_messages().row_data(1).unwrap().time, pending_time);
    view.update(
        &ui,
        account::Update::Messages {
            id: "1".into(),
            generation: view.generation,
            messages: vec![
                message("late", "previous chat"),
                message("server", "submitted text"),
            ],
        },
    );
    assert_eq!(
        ui.get_messages().row_count(),
        2,
        "confirmation duplicated a local item"
    );
    assert_eq!(
        ui.get_messages(),
        pending_model,
        "server reconciliation replaced the pending row model"
    );
    assert_eq!(ui.get_messages().row_data(1).unwrap().id, "pending");
    assert_eq!(ui.get_messages().row_data(1).unwrap().time, pending_time);
    assert_eq!(ui.get_messages().row_data(1).unwrap().delivery, "Sent");
    assert!(view.saved.outbox.is_empty());
    assert_eq!(ui.get_draft(), "another draft");
    if stop == 0 {
        ui.hide().unwrap();
        return;
    }
    // An asset update changes only the affected row and reaction, keeping
    // both models alive while text selection/context menus are active.
    let reactions = Rc::new(VecModel::from(vec![ReactionRow {
        image_key: "reaction".into(),
        ..Default::default()
    }]));
    view.messages = vec![MessageRow {
        id: "photo".into(),
        image_key: "photo".into(),
        sender_key: "sender-avatar".into(),
        reactions: ModelRc::from(reactions.clone()),
        ..Default::default()
    }];
    view.render(&ui);
    let model = ui.get_messages();
    for key in ["photo", "reaction", "sender-avatar"] {
        view.update(
            &ui,
            account::Update::Image {
                key: key.into(),
                pixels: line_wasm_poc::client::desktop::assets::Pixels {
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                }
                .into(),
            },
        );
    }
    assert_eq!(ui.get_messages(), model);
    assert!(model.row_data(0).unwrap().has_picture);
    assert!(model.row_data(0).unwrap().has_sender_avatar);
    assert_eq!(view.messages[0].reactions, ModelRc::from(reactions.clone()));
    assert!(reactions.row_data(0).unwrap().has_picture);
    if stop == 1 {
        ui.hide().unwrap();
        return;
    }
    // Exercise the actual submission controller with an in-memory command
    // queue and a synthetic saved session. No account worker is started.
    let root = std::env::temp_dir().join(format!("line-ui-submit-{}-{stop}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let session = root.join("session.json");
    std::fs::write(&session, br#"{"login_key":"c3ludGhldGljIG9ubHk="}"#).unwrap();
    let paths = account::Paths {
        state: session,
        wasm: root.join("unused"),
        cache: root.join("cache"),
    };
    let (storage, _) = storage::Storage::open(&paths, || {}).unwrap();
    view.storage = Some(storage);
    view.demo = false;
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    view.commands = Some(tx.into());
    ui.set_connected(true);
    for draft in ["", " \n\t"] {
        ui.set_draft(draft.into());
        view.submit(&ui);
        assert!(!ui.get_can_send());
        assert!(
            ui.get_send_reason().is_empty(),
            "empty drafts need no instruction"
        );
        assert!(rx.try_recv().is_err());
    }
    ui.set_draft("".into());
    if stop == 2
        && let Ok(path) = std::env::var("LINE_COMPOSER_SNAPSHOT")
    {
        snapshot::save(&_window, &path, (980, 720)).unwrap();
    }
    ui.set_draft("synthetic submission 🐈".into());
    view.send_availability(&ui);
    assert!(ui.get_can_send());
    assert!(ui.get_send_reason().is_empty());
    ui.set_connected(false);
    view.send_availability(&ui);
    assert!(!ui.get_can_send());
    assert!(ui.get_send_reason().starts_with("Offline"));
    ui.set_connected(true);
    view.submit(&ui);
    assert!(ui.get_draft().is_empty());
    let account::Command::Send {
        id, text, local_id, ..
    } = rx.try_recv().unwrap()
    else {
        panic!("wrong command");
    };
    assert_eq!(text, "synthetic submission 🐈");
    assert_eq!(view.saved.outbox.last().unwrap().status, "Pending…");
    ui.set_draft("a newer committed draft".into());
    view.update(&ui, account::Update::SendFailed { id, local_id });
    assert_eq!(ui.get_draft(), "a newer committed draft");
    assert!(
        rx.try_iter()
            .all(|command| matches!(command, account::Command::RetainSendRequests { .. })),
        "ambiguous send was automatically retried"
    );
    view.storage.take();
    std::fs::remove_dir_all(root).unwrap();
    if stop == 2 {
        ui.hide().unwrap();
        return;
    }
    ui.set_draft("preserve this draft during sign-in".into());
    view.update(
        &ui,
        account::Update::ConnectionFailed(account::ConnectionFailure::Session),
    );
    assert!(ui.get_connection_failed());
    assert!(!ui.get_connected());
    assert!(!ui.get_auth_open(), "failure started pairing automatically");
    view.begin_sign_in(&ui);
    assert!(rx.try_recv().is_err());
    assert!(!ui.get_auth_expired());
    assert!(ui.get_session_unavailable());
    ui.invoke_open_sign_in();
    assert!(ui.get_auth_open());
    assert!(
        rx.try_recv().is_err(),
        "opening the explanation started pairing"
    );
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    view.pairing_cancel = Some(cancel.clone());
    view.begin_sign_in(&ui);
    assert!(ui.get_auth_busy());
    assert!(matches!(
        rx.try_recv().unwrap(),
        account::Command::Reauthenticate
    ));
    assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
    view.cancel_sign_in(&ui);
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert!(ui.get_auth_cancelling());
    view.update(&ui, account::Update::SignInFailed { cancelled: true });
    assert!(!ui.get_auth_open());
    assert!(!ui.get_auth_busy());
    assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
    view.reconnect(&ui);
    assert!(ui.get_retrying_connection());
    assert!(matches!(
        rx.try_recv().unwrap(),
        account::Command::Reconnect
    ));
    view.update(&ui, account::Update::Connected("Synthetic account".into()));
    assert!(!ui.get_connection_failed());
    assert!(!ui.get_retrying_connection());
    assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
    view.update(
        &ui,
        account::Update::Sync(line_wasm_poc::client::desktop::sync::SyncState::AuthExpired),
    );
    ui.invoke_open_sign_in();
    ui.set_auth_busy(true);
    view.update(
        &ui,
        account::Update::SignIn(line_wasm_poc::client::pairing::Event::Qr {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        }),
    );
    view.update(
        &ui,
        account::Update::SignIn(line_wasm_poc::client::pairing::Event::Pin("123456".into())),
    );
    assert_eq!(ui.get_auth_qr().size().width, 2);
    assert_eq!(ui.get_auth_pin(), "123456");
    view.update(&ui, account::Update::SignInFailed { cancelled: false });
    assert!(!ui.get_auth_busy());
    assert_eq!(ui.get_auth_qr().size().width, 0);
    assert!(ui.get_auth_pin().is_empty());
    assert!(!ui.get_auth_error().is_empty());
    view.update(&ui, account::Update::SessionChanged);
    assert!(view.saved.drafts.is_empty());
    assert!(view.saved.outbox.is_empty());
    assert!(view.current.is_empty());
    assert_eq!(ui.get_messages().row_count(), 0);
    assert!(ui.get_draft().is_empty());
    assert_eq!(ui.get_chat_title(), "Your conversations");
    if stop == 3 {
        ui.hide().unwrap();
        return;
    }
    // API order is not presentation order. Keep selection/draft by identity
    // while both refresh and paged arrivals sort by latest activity.
    view.demo = true;
    let chat = |id: &str, timestamp| account::ChatView {
        id: id.into(),
        title: id.into(),
        preview: "first line\nsecond line\tlast".into(),
        timestamp,
        unread: None,
        muted: None,
        last_read_message: None,
        last_message_id: None,
        outgoing_preview: false,
        group: false,
        member_count: None,
        avatar_key: String::new(),
    };
    view.update(
        &ui,
        account::Update::Chats {
            chats: vec![chat("older", 100), chat("newest", 300), chat("middle", 200)],
            more: true,
        },
    );
    assert_eq!(ui.get_chats().row_data(0).unwrap().title, "newest");
    assert_eq!(view.current, "newest");
    assert_eq!(
        ui.get_chats().row_data(0).unwrap().preview,
        "first line second line last"
    );
    ui.set_draft("keep selected chat draft".into());
    ui.set_list_pressed(true);
    view.update(
        &ui,
        account::Update::Chats {
            chats: vec![chat("older", 400)],
            more: false,
        },
    );
    assert_eq!(ui.get_chats().row_data(0).unwrap().title, "newest");
    ui.set_list_pressed(false);
    for event in std::mem::take(&mut view.deferred) {
        view.update_render(&ui, event);
    }
    assert_eq!(ui.get_chats().row_data(0).unwrap().title, "older");
    assert_eq!(view.current, "newest");
    assert_eq!(ui.get_draft(), "keep selected chat draft");
    assert_eq!(view.ids.len(), 3);
    let mut later = message("11", "later");
    later.timestamp = 300;
    let mut earlier = message("9", "earlier");
    earlier.timestamp = 100;
    let mut same_time = message("10", "same time");
    same_time.timestamp = 300;
    view.update(
        &ui,
        account::Update::Messages {
            id: view.current.clone(),
            generation: view.generation,
            messages: vec![later, earlier, same_time],
        },
    );
    assert_eq!(
        view.messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["9", "10", "11"]
    );
    view.saved.outbox.push(storage::Outgoing {
        local_id: "outbox-between".into(),
        request_seq: None,
        timestamp_ms: 200,
        chat: view.current.clone(),
        text: "pending between messages".into(),
        status: "Pending…".into(),
        server_id: String::new(),
    });
    let ordered = view.with_outbox(&view.current, view.messages.clone());
    assert_eq!(
        ordered.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["9", "outbox-between", "10", "11"]
    );
    if stop == 4 {
        ui.hide().unwrap();
        return;
    }
    // An old original-photo completion cannot replace a newer viewer or
    // reopen a dismissed surface. Preview/failure remains local.
    view.messages[0].photo = true;
    view.messages[0].image_key = "test-photo".into();
    view.open_photo(&ui, "test-photo");
    assert!(ui.get_viewer_open());
    let before = ui.get_viewer_image();
    view.update(
        &ui,
        account::Update::OriginalFailed {
            request: view.viewer_request + 1,
        },
    );
    assert_eq!(ui.get_viewer_image(), before);
    assert!(ui.get_viewer_error().is_empty());
    view.update(
        &ui,
        account::Update::OriginalFailed {
            request: view.viewer_request,
        },
    );
    assert!(!ui.get_viewer_error().is_empty());
    ui.set_viewer_open(false);
    view.update(
        &ui,
        account::Update::Original {
            request: view.viewer_request,
            pixels: line_wasm_poc::client::desktop::assets::Pixels {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            }
            .into(),
        },
    );
    assert!(!ui.get_viewer_open());
    assert_eq!(ui.get_viewer_image(), before);
    if stop == 5 {
        ui.hide().unwrap();
        return;
    }
    // Exercise the actual composer shortcut with a local callback only.
    // No account worker or transport is involved.
    ui.set_auth_open(false);
    ui.set_settings_open(false);
    ui.set_has_chat(true);
    ui.set_show_conversation(true);
    ui.set_can_send(true);
    ui.set_draft("日本語 🐈".into());
    let submissions = Rc::new(std::cell::Cell::new(0));
    let count = submissions.clone();
    ui.on_send(move || count.set(count.get() + 1));
    ui.show().unwrap();
    ui.invoke_focus_composer();
    let key = |text: &str, pressed: bool| {
        ui.window().dispatch_event(if pressed {
            slint::platform::WindowEvent::KeyPressed { text: text.into() }
        } else {
            slint::platform::WindowEvent::KeyReleased { text: text.into() }
        });
    };
    key("\u{11}", true);
    key("\n", true);
    key("\n", false);
    key("\u{11}", false);
    assert_eq!(submissions.get(), 1);
    assert_eq!(ui.get_draft(), "日本語 🐈");
    ui.set_can_send(false);
    key("\u{11}", true);
    key("\n", true);
    key("\n", false);
    key("\u{11}", false);
    assert_eq!(submissions.get(), 1, "disabled shortcut submitted");
    assert_eq!(ui.get_draft(), "日本語 🐈");
    ui.set_can_send(true);
    ui.set_ime_active(true);
    key("\u{11}", true);
    key("\n", true);
    key("\n", false);
    key("\u{11}", false);
    assert_eq!(submissions.get(), 1, "IME composition submitted");
    ui.set_ime_active(false);
    let before_enter = ui.get_draft().to_string();
    key("\n", true);
    key("\n", false);
    assert_eq!(submissions.get(), 1);
    assert_eq!(ui.get_draft().len(), before_enter.len() + 1);
    // Opt-in Enter send, Shift+Enter newline, and composition precedence.
    ui.set_enter_sends(true);
    let count_before = submissions.get();
    key("\n", true);
    key("\n", false);
    assert_eq!(submissions.get(), count_before + 1);
    let length = ui.get_draft().len();
    key("\u{10}", true);
    key("\n", true);
    key("\n", false);
    key("\u{10}", false);
    assert_eq!(submissions.get(), count_before + 1);
    assert_eq!(ui.get_draft().len(), length + 1);
    ui.set_ime_active(true);
    key("\n", true);
    key("\n", false);
    assert_eq!(submissions.get(), count_before + 1);
    ui.set_ime_active(false);
    let draft_before_escape = ui.get_draft();
    key("\u{1b}", true);
    key("\u{1b}", false);
    assert_eq!(ui.get_draft(), draft_before_escape);
    ui.set_enter_sends(false);
    let quits = Rc::new(std::cell::Cell::new(0));
    let quit_count = quits.clone();
    ui.on_quit(move || quit_count.set(quit_count.get() + 1));
    ui.set_ime_active(true);
    key("\u{11}", true);
    key("q", true);
    key("q", false);
    key("\u{11}", false);
    assert_eq!(quits.get(), 0, "Ctrl+Q fired during composition");
    ui.set_ime_active(false);
    key("\u{11}", true);
    key("q", true);
    key("q", false);
    key("\u{11}", false);
    assert_eq!(quits.get(), 1);
    if stop == 6 {
        ui.hide().unwrap();
        return;
    }
    // Friend browsing and profile activation never submit or replace a draft.
    view.update(
        &ui,
        account::Update::Friends(vec![
            account::FriendView {
                id: view.current.clone(),
                name: "葵 🌸".into(),
                status: "こんにちは\n日本語".into(),
                group: false,
                member_count: None,
                avatar_key: "".into(),
            },
            account::FriendView {
                id: "new-friend".into(),
                name: "New friend".into(),
                status: "".into(),
                group: false,
                member_count: None,
                avatar_key: "".into(),
            },
            account::FriendView {
                id: "new-group".into(),
                name: "Group".into(),
                status: "".into(),
                group: true,
                member_count: None,
                avatar_key: "".into(),
            },
        ]),
    );
    let current = view.current.clone();
    let draft = ui.get_draft();
    let count = view.ids.len();
    view.browse(&ui, 1);
    assert_eq!(view.current, current);
    assert_eq!(ui.get_draft(), draft);
    assert_eq!(ui.get_friends().row_count(), 5);
    ui.set_friend_query("葵".into());
    view.filter_friends(&ui);
    assert_eq!(ui.get_friends().row_count(), 2);
    assert_eq!(ui.get_friends().row_data(0).unwrap().id, "section-friends");
    assert_eq!(ui.get_friends().row_data(1).unwrap().initial, "葵");
    ui.set_friend_query("no match".into());
    view.filter_friends(&ui);
    assert_eq!(ui.get_friends().row_count(), 0);
    view.open_profile(&ui, &current);
    assert!(ui.get_profile_open());
    assert_eq!(ui.get_profile_status(), "こんにちは\n日本語");
    ui.invoke_close_profile();
    assert_eq!(view.current, current);
    assert_eq!(ui.get_draft(), draft);
    ui.set_friend_query("".into());
    ui.set_groups_expanded(false);
    view.filter_friends(&ui);
    assert_eq!(ui.get_friends().row_count(), 4);
    view.update(&ui, account::Update::FriendsFailed);
    assert_eq!(ui.get_friends().row_count(), 4);
    assert!(!ui.get_friends_error().is_empty());
    view.open_profile(&ui, &current);
    view.start_friend_chat(&ui);
    assert_eq!(view.ids.len(), count, "existing friend chat duplicated");
    view.open_profile(&ui, "new-friend");
    view.start_friend_chat(&ui);
    assert_eq!(view.current, "new-friend");
    assert_eq!(view.ids.len(), count + 1);
    assert!(ui.get_draft().is_empty());
    assert_eq!(
        view.saved.drafts.get(&current).map(String::as_str),
        Some(draft.as_str())
    );
    view.open_profile(&ui, "new-group");
    view.update(&ui, account::Update::Friends(vec![]));
    assert!(!ui.get_profile_available());
    view.start_friend_chat(&ui);
    assert_eq!(
        view.current, "new-friend",
        "removed group remained actionable"
    );
    view.update(&ui, account::Update::SessionChanged);
    assert!(view.friends.is_empty());
    assert!(!ui.get_profile_open());
    assert_eq!(ui.get_friend_query(), "");
    ui.hide().unwrap();
}

#[test]
fn navigation_refresh_and_outbox() {
    std::thread::spawn(|| controller_scenario(0))
        .join()
        .unwrap();
}
#[test]
fn incremental_asset_updates() {
    std::thread::spawn(|| controller_scenario(1))
        .join()
        .unwrap();
}
#[test]
fn durable_submission_failure() {
    std::thread::spawn(|| controller_scenario(2))
        .join()
        .unwrap();
}
#[test]
fn authentication_recovery_preserves_drafts() {
    std::thread::spawn(|| controller_scenario(3))
        .join()
        .unwrap();
}
#[test]
fn chronological_order_and_outbox_merge() {
    std::thread::spawn(|| controller_scenario(4))
        .join()
        .unwrap();
}
#[test]
fn stale_photo_completion() {
    std::thread::spawn(|| controller_scenario(5))
        .join()
        .unwrap();
}
#[test]
fn composer_shortcuts_and_ime_guard() {
    std::thread::spawn(|| controller_scenario(6))
        .join()
        .unwrap();
}
#[test]
fn friend_profile_navigation() {
    std::thread::spawn(|| controller_scenario(7))
        .join()
        .unwrap();
}

#[test]
fn deferred_incoming_counts_unread_immediately_and_only_once() {
    let _window = snapshot::install().unwrap();
    let ui = create_app_window().unwrap();
    let mut view = View {
        current: "chat".into(),
        ..Default::default()
    };
    view.unread.server("chat", Some(0), None);
    ui.global::<State>().set_history_interacting(true);
    view.update(
        &ui,
        account::Update::Incoming {
            chat: "chat".into(),
            messages: vec![message("101", "synthetic")],
            replay: false,
        },
    );
    assert_eq!(view.unread.chats["chat"].count, Some(1));
    assert!(view.messages.is_empty());
    assert_eq!(view.deferred.len(), 1);
    ui.global::<State>().set_history_interacting(false);
    for event in std::mem::take(&mut view.deferred) {
        view.update_render(&ui, event);
    }
    assert_eq!(view.unread.chats["chat"].count, Some(1));
    assert_eq!(view.messages.len(), 1);
}

#[test]
fn profile_panel_allows_send_shortcut_and_sheet_blocks_it() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.set_can_send(true);
        ui.set_draft("Eligible message".into());
        let submissions = Rc::new(std::cell::Cell::new(0));
        let count = submissions.clone();
        ui.on_send(move || count.set(count.get() + 1));
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(1400, 900));
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1400, 900);
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), 1400);
        });
        ui.set_profile_open(true);
        assert!(!ui.global::<State>().get_modal_open());
        ui.invoke_focus_composer();
        let send = || {
            for (text, pressed) in [
                ("\u{11}", true),
                ("\n", true),
                ("\n", false),
                ("\u{11}", false),
            ] {
                ui.window().dispatch_event(if pressed {
                    slint::platform::WindowEvent::KeyPressed { text: text.into() }
                } else {
                    slint::platform::WindowEvent::KeyReleased { text: text.into() }
                });
            }
        };
        send();
        assert_eq!(submissions.get(), 1);
        assert_eq!(ui.get_draft(), "Eligible message");
        ui.set_can_send(false);
        send();
        assert_eq!(submissions.get(), 1);
        assert_eq!(ui.get_draft(), "Eligible message");
        ui.set_can_send(true);
        window.set_size(slint::PhysicalSize::new(980, 720));
        slint::platform::update_timers_and_animations();
        assert!(ui.global::<State>().get_modal_open());
        send();
        assert_eq!(submissions.get(), 1, "profile sheet must block submission");
        assert_eq!(ui.get_draft(), "Eligible message");
        if let Ok(path) = std::env::var("LINE_PROFILE_STRIP_SNAPSHOT") {
            ui.set_connection_failed(true);
            ui.set_profile_name("葵".into());
            ui.set_profile_available(true);
            snapshot::save(&window, &path, (980, 720)).unwrap();
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
