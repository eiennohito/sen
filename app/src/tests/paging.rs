use super::*;
#[test]
fn retained_pixel_models_are_inside_the_history_byte_budget() {
    let picture =
        slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(512, 512));
    let mut rows: Vec<MessageRow> = (0..1000)
        .map(|n| MessageRow {
            id: n.to_string().into(),
            picture: picture.clone(),
            has_picture: true,
            ..Default::default()
        })
        .collect();
    history_window::bound(&mut rows, None, |row| row.id.as_str(), crate::paging::bytes);
    assert!(rows.iter().map(crate::paging::bytes).sum::<usize>() <= history_window::RETAINED_BYTES);
    assert!(rows.len() < 5);
}
#[test]
fn deferred_arrival_flood_is_bounded_and_discloses_lost_count_precision() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View::default();
        for n in 0..1000 {
            view.defer_update(&ui, account::Update::Incoming { chat: format!("chat-{n}"), messages: vec![message(&n.to_string(), "incoming")], replay: false });
        }
        assert!(view.deferred.len() <= 128);
        assert!(ui.global::<State>().get_history_needs_refresh());
        for n in 0..1000 {
            view.defer_update(&ui, account::Update::Incoming { chat: "same-chat".into(), messages: vec![message(&n.to_string(), "incoming")], replay: false });
        }
        assert!(view.deferred.iter().all(|event| match event { account::Update::Incoming { messages, .. } => messages.len() <= 50, _ => true }));
        view.defer_update(&ui, account::Update::Sent { id: "chat".into(), local_id: "pending".into(), server_id: "confirmed".into() });
        view.defer_update(&ui, account::Update::SendUncertain { id: "chat".into(), local_id: "pending".into() });
        assert!(view.deferred.iter().any(|event| matches!(event, account::Update::Sent { local_id, .. } if local_id == "pending")));
        assert!(!view.deferred.iter().any(|event| matches!(event, account::Update::SendUncertain { local_id, .. } if local_id == "pending")));
    }).join().unwrap();
}
#[test]
fn confirmed_local_alias_remains_an_anchor_through_bounds_and_reply_jump() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View {
            current: "chat".into(),
            demo: true,
            ..Default::default()
        };
        view.messages = (0..1000)
            .map(|n| MessageRow {
                id: n.to_string().into(),
                body: "text".into(),
                sort_key: order_key(n).into(),
                ..Default::default()
            })
            .collect();
        view.aliases
            .insert(("chat".into(), "10".into()), ("local-pending".into(), 10));
        let displayed: Vec<String> = view
            .messages
            .iter()
            .map(|row| {
                if row.id == "10" {
                    "local-pending".into()
                } else {
                    row.id.to_string()
                }
            })
            .collect();
        view.paging
            .borrow_mut()
            .entry("chat".into())
            .or_default()
            .reveal(&displayed, "local-pending", 25.);
        view.bound_history(&ui, "chat");
        assert!(view.messages.iter().any(|row| row.id == "10"));
        assert!(view.aliases.contains_key(&("chat".into(), "10".into())));
        ui.set_following(false);
        view.render(&ui);
        ui.global::<State>().set_history_viewport_height(400.);
        view.history_geometry(&ui, "local-pending", 250., 40.);
        assert_eq!(ui.get_history_y(), -225.);
        assert!(view.jump_to_message(&ui, "10"));
        assert_eq!(
            ui.global::<State>().get_highlight_message(),
            "local-pending"
        );
        assert!(view.aliases.len() <= view.messages.len());
    })
    .join()
    .unwrap();
}
#[test]
fn oversized_anchored_text_is_retained_as_one_explicit_budget_exception() {
    let mut rows = vec!["large".repeat(history_window::RETAINED_BYTES)];
    let anchor = rows[0].clone();
    history_window::bound(&mut rows, Some(&anchor), String::as_str, String::len);
    assert_eq!(rows.len(), 1);
}
#[test]
fn pending_outbox_row_stays_an_anchor_when_incoming_rows_are_bounded() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View {
            current: "chat".into(),
            demo: true,
            ..Default::default()
        };
        view.messages = demo::paged_rows(0..1000);
        view.saved.outbox.push(storage::Outgoing {
            local_id: "local-pending".into(),
            request_seq: Some(1),
            timestamp_ms: 1_760_000_001_000,
            chat: "chat".into(),
            text: "Pending".into(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        view.paging
            .borrow_mut()
            .entry("chat".into())
            .or_default()
            .anchor = Some(history_window::Anchor {
            id: "local-pending".into(),
            screen_y: 20.,
        });
        view.bound_history(&ui, "chat");
        assert_eq!(
            view.paging.borrow()["chat"].anchor.as_ref().unwrap().id,
            "local-pending"
        );
        assert!(
            view.with_outbox("chat", view.messages.clone())
                .iter()
                .any(|row| row.id == "local-pending")
        );
    })
    .join()
    .unwrap();
}
#[test]
fn distinct_conversation_union_and_terminal_flood_are_bounded_without_loss() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View {
            current: "chat-0".into(),
            demo: true,
            ..Default::default()
        };
        view.messages = (0..600)
            .map(|n| MessageRow {
                id: format!("message-{n}").into(),
                ..Default::default()
            })
            .collect();
        for n in 1..10 {
            view.history.insert(
                format!("chat-{n}"),
                view::Reading {
                    messages: view.messages.clone(),
                    ..Default::default()
                },
            );
        }
        view.bound_history(&ui, "chat-0");
        let retained_union = view
            .history
            .keys()
            .filter(|chat| **chat != view.current)
            .count()
            + 1;
        assert_eq!(retained_union, 8);
        let chats: Vec<_> = view
            .history
            .keys()
            .cloned()
            .chain(std::iter::once(view.current.clone()))
            .collect();
        for chat in chats {
            for n in 0..600 {
                view.defer_update(
                    &ui,
                    account::Update::Withdrawn {
                        chat: chat.clone(),
                        message_id: format!("message-{n}"),
                    },
                );
            }
        }
        for n in 0..256 {
            view.saved.outbox.push(storage::Outgoing {
                local_id: format!("local-{n}"),
                request_seq: Some(n),
                timestamp_ms: 1,
                chat: "chat-0".into(),
                text: "text".into(),
                status: "Pending…".into(),
                server_id: String::new(),
            });
            view.defer_update(
                &ui,
                account::Update::Sent {
                    id: "chat-0".into(),
                    local_id: format!("local-{n}"),
                    server_id: format!("server-{n}"),
                },
            );
            view.defer_update(
                &ui,
                account::Update::OwnEcho {
                    chat: "chat-0".into(),
                    request_seq: n,
                    server_id: format!("server-{n}"),
                },
            );
        }
        assert_eq!(
            view.deferred
                .iter()
                .filter(|event| matches!(event, account::Update::Withdrawn { .. }))
                .count(),
            4800
        );
        assert_eq!(
            view.deferred
                .iter()
                .filter(|event| matches!(
                    event,
                    account::Update::Sent { .. } | account::Update::OwnEcho { .. }
                ))
                .count(),
            512
        );
        assert!(view.deferred.len() <= 5500);
    })
    .join()
    .unwrap();
}

#[test]
fn latest_keeps_loaded_history_when_no_newer_range_was_evicted() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut view = View {
            current: "chat".into(),
            demo: true,
            ..Default::default()
        };
        view.messages = demo::paged_rows(0..400);
        view.keep_recent("chat", true);
        view.history_latest(&ui);
        assert_eq!(view.messages.len(), 400);
        assert_eq!(ui.get_messages().row_count(), 200);
        assert!(ui.get_following());
    })
    .join()
    .unwrap();
}
