use super::*;

fn synthetic_ui() -> (AppWindow, View) {
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
    (ui, view)
}
fn unresolved(id: &str, status: &str, seq: Option<u32>) -> storage::Outgoing {
    storage::Outgoing {
        local_id: id.into(),
        timestamp_ms: 100,
        chat: "0".into(),
        text: "日本語 🐈\nrecovery".into(),
        status: status.into(),
        server_id: String::new(),
        request_seq: seq,
    }
}
#[test]
fn composer_selection_and_unchanged_chat() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let (ui, mut view) = synthetic_ui();
        ui.show().unwrap();
        ui.invoke_focus_composer();
        let draft = format!("日本語 🐈\n{}", "second line\n".repeat(24));
        ui.set_draft(draft.clone().into());
        ui.invoke_restore_editor(3, 14); // Japanese and astral character byte boundaries.
        ui.set_editor_y(-41.);
        let caret = ui.get_editor_caret();
        let anchor = ui.get_editor_anchor();
        assert_eq!((anchor, caret), (3, 14));
        view.select(&ui, 0);
        assert_eq!(
            (
                ui.get_editor_anchor(),
                ui.get_editor_caret(),
                ui.get_editor_y()
            ),
            (3, 14, -41.)
        );
        view.select(&ui, 1);
        ui.set_draft("other".into());
        view.select(&ui, 0);
        assert_eq!(ui.get_draft(), draft);
        assert_eq!(
            (
                ui.get_editor_anchor(),
                ui.get_editor_caret(),
                ui.get_editor_y()
            ),
            (anchor, caret, -41.)
        );
    })
    .join()
    .unwrap();
}
#[test]
fn outgoing_failures_recovery_and_late_echo() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let (ui, mut view) = synthetic_ui();
        ui.set_connected(true);
        view.saved
            .outbox
            .push(unresolved("reject", "Pending…", Some(1)));
        view.saved
            .outbox
            .push(unresolved("uncertain", "Pending…", Some(2)));
        view.update(
            &ui,
            account::Update::SendRejected {
                id: "0".into(),
                local_id: "reject".into(),
                reason: "Rejected".into(),
            },
        );
        view.update(
            &ui,
            account::Update::SendUncertain {
                id: "0".into(),
                local_id: "uncertain".into(),
            },
        );
        assert!(ui.get_connected());
        assert!(!ui.get_connection_failed());
        let rows = view.with_outbox("0", Vec::new());
        assert_eq!(rows[0].delivery, "Not sent");
        assert_eq!(rows[1].delivery, "Unconfirmed");
        ui.set_draft("newer draft".into());
        view.recover_outgoing(&ui, "reject");
        assert_eq!(ui.get_draft(), "newer draft");
        assert_eq!(ui.get_recovery_kind(), "edit");
        ui.window().set_size(slint::LogicalSize::new(360., 520.));
        ui.global::<State>().set_window_width(360.);
        assert!(ui.global::<State>().get_narrow());
        ui.set_show_conversation(true);
        ui.invoke_go_back();
        assert!(
            ui.get_show_conversation(),
            "Back navigated under the recovery modal"
        );
        assert_eq!(view.current, "0");
        view.resolve_recovery(&ui, "cancel");
        assert_eq!(ui.get_draft(), "newer draft");
        view.recover_outgoing(&ui, "reject");
        view.resolve_recovery(&ui, "append");
        assert_eq!(ui.get_draft(), "newer draft\n日本語 🐈\nrecovery");
        view.recover_outgoing(&ui, "reject");
        view.resolve_recovery(&ui, "replace");
        assert_eq!(ui.get_draft(), "日本語 🐈\nrecovery");
        let newer = ui.get_draft();
        view.retry_outgoing(&ui, "uncertain", false);
        assert_eq!(ui.get_recovery_kind(), "resend");
        assert_eq!(view.saved.outbox.len(), 2);
        view.resolve_recovery(&ui, "resend");
        assert_eq!(view.saved.outbox.len(), 3);
        assert!(view.saved.outbox[1].server_id.is_empty());
        assert_ne!(
            view.saved.outbox[1].request_seq,
            view.saved.outbox[2].request_seq
        );
        view.update(
            &ui,
            account::Update::OwnEcho {
                chat: "0".into(),
                request_seq: 2,
                server_id: "late-original".into(),
            },
        );
        assert_eq!(view.saved.outbox[1].server_id, "late-original");
        assert_ne!(
            view.saved.outbox[1].server_id,
            view.saved.outbox[2].server_id
        );
        assert_eq!(ui.get_draft(), newer);
    })
    .join()
    .unwrap();
}
#[test]
fn persisted_preferences_and_request_sequence() {
    let mut view = View::default();
    assert!(!view.saved.enter_sends);
    let fresh = view.allocate_request_seq().unwrap();
    assert!((1..=0x4000_0000).contains(&fresh));
    view.saved.next_request_seq = 1;
    assert_eq!(view.allocate_request_seq(), Some(1));
    view.saved.enter_sends = true;
    view.saved
        .outbox
        .push(unresolved("persisted", "Unconfirmed", Some(18)));
    let bytes = serde_json::to_vec(&view.saved).unwrap();
    let mut restored = View {
        saved: serde_json::from_slice(&bytes).unwrap(),
        ..Default::default()
    };
    assert!(restored.saved.enter_sends);
    assert_eq!(restored.allocate_request_seq(), Some(19));
    assert_eq!(restored.allocate_request_seq(), Some(20));
    restored.saved.next_request_seq = i32::MAX as u32;
    assert_eq!(restored.allocate_request_seq(), Some(i32::MAX as u32));
    assert_eq!(restored.allocate_request_seq(), None);
}

#[test]
fn exact_echo_alias_keeps_later_read_state() {
    let mut view = View::default();
    view.aliases
        .insert(("chat".into(), "server".into()), ("local".into(), 100));
    let rows = view.with_outbox(
        "chat",
        vec![MessageRow {
            id: "server".into(),
            outgoing: true,
            delivery: "Read".into(),
            ..Default::default()
        }],
    );
    assert_eq!(rows[0].id, "local");
    assert_eq!(rows[0].delivery, "Read");
}

#[test]
fn demo_composer_transport_and_history() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let (ui, mut view) = synthetic_ui();
        ui.set_connected(true);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = account::start_demo(
            view.demo_send_failure.clone(),
            HashMap::new(),
            move |event| {
                tx.send(event).unwrap();
            },
        );
        view.commands = Some(worker.commands.clone());
        let state = Rc::new(RefCell::new(view));
        let weak = ui.as_weak();
        let send_state = state.clone();
        ui.on_send(move || {
            send_state.borrow_mut().submit(&weak.unwrap());
        });
        ui.show().unwrap();
        ui.set_draft("送信テスト · local only".into());
        state.borrow().send_availability(&ui);
        assert!(ui.get_can_send());
        ui.global::<State>().invoke_send();
        assert!(ui.get_draft().is_empty());
        assert!(state.borrow().saved.drafts["0"].is_empty());
        let local_id = state.borrow().saved.outbox[0].local_id.clone();
        assert_eq!(state.borrow().saved.outbox[0].status, "Pending…");
        assert!(ui.get_following());
        let receive = || rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        state.borrow_mut().update(&ui, receive());
        assert_eq!(state.borrow().saved.outbox[0].status, "Sent");
        state.borrow_mut().update(&ui, receive());
        assert!(state.borrow().saved.outbox.is_empty());
        assert_eq!(
            state
                .borrow()
                .messages
                .iter()
                .filter(|row| row.body == "送信テスト · local only")
                .count(),
            1
        );
        let rows = state
            .borrow()
            .with_outbox("0", state.borrow().messages.clone());
        assert_eq!(rows.last().unwrap().id, local_id);
        assert_eq!(rows.last().unwrap().delivery, "Sent");
        assert_eq!(state.borrow().rows[0].preview, "送信テスト · local only");

        for (outcome, delivery) in [(1, "Unconfirmed"), (2, "Not sent")] {
            state
                .borrow()
                .demo_send_failure
                .store(outcome, std::sync::atomic::Ordering::Relaxed);
            ui.set_draft("帰りにコーヒーを買うね".into());
            ui.global::<State>().invoke_send();
            assert!(ui.get_draft().is_empty());
            let id = state.borrow().saved.outbox.last().unwrap().local_id.clone();
            ui.set_draft("A newer draft".into());
            state.borrow_mut().update(&ui, receive());
            assert_eq!(
                state
                    .borrow()
                    .with_outbox("0", Vec::new())
                    .last()
                    .unwrap()
                    .delivery,
                delivery
            );
            assert!(ui.get_connected());
            assert!(!ui.get_connection_failed());
            assert_eq!(ui.get_draft(), "A newer draft");
            if outcome == 1 {
                state.borrow_mut().retry_outgoing(&ui, &id, false);
                assert_eq!(ui.get_recovery_kind(), "resend");
                state.borrow_mut().resolve_recovery(&ui, "cancel");
                assert!(rx.try_recv().is_err());
                assert_eq!(state.borrow().saved.outbox.len(), 1);
                state.borrow_mut().retry_outgoing(&ui, &id, false);
                state.borrow_mut().resolve_recovery(&ui, "resend");
            } else {
                state.borrow_mut().retry_outgoing(&ui, &id, false);
                assert!(ui.get_recovery_kind().is_empty());
            }
            state.borrow_mut().update(&ui, receive());
            state.borrow_mut().update(&ui, receive());
            assert_eq!(ui.get_draft(), "A newer draft");
            assert!(
                state
                    .borrow()
                    .saved
                    .outbox
                    .iter()
                    .all(|item| item.status != "Pending…")
            );
        }
        assert_eq!(
            state
                .borrow()
                .messages
                .iter()
                .filter(|row| row.body == "帰りにコーヒーを買うね")
                .count(),
            2
        );
        snapshot::render(&window, (980, 720)).unwrap();
        state.borrow_mut().select(&ui, 1);
        state.borrow_mut().select(&ui, 0);
        assert_eq!(
            state
                .borrow()
                .messages
                .iter()
                .filter(|row| row.body == "送信テスト · local only")
                .count(),
            1
        );
        assert_eq!(ui.get_draft(), "A newer draft");
    })
    .join()
    .unwrap();
}

#[test]
fn demo_fixtures_open_latest_with_relative_dates() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        fix_snapshot_clock();
        let (ui, mut view) = synthetic_ui();
        ui.show().unwrap();
        snapshot::render(&window, (360, 520)).unwrap();
        assert!(ui.get_following());
        assert!(ui.get_history_y() < 0.);
        let mut rows = view.messages.clone();
        group_messages(&mut rows);
        let labels: Vec<_> = rows
            .iter()
            .filter(|row| !row.day_label.is_empty())
            .map(|row| row.day_label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["2025/10/18", "Thursday 10/15", "Yesterday", "Today"]
        );
        assert!(
            rows.iter()
                .all(|row| row.sort_key.parse::<i64>().unwrap() > 0)
        );
        view.select(&ui, 1);
        snapshot::render(&window, (360, 520)).unwrap();
        assert!(ui.get_following());
        assert!(ui.get_history_y() < 0.);
        assert_eq!(
            list_time(view.messages.last().unwrap().sort_key.parse().unwrap()),
            "11:06"
        );
        view.select(&ui, 2);
        assert_eq!(
            list_time(view.messages[0].sort_key.parse().unwrap()),
            "2025/10/18"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn demo_early_clock_fixtures_and_send_order() {
    std::thread::spawn(|| {
        use chrono::TimeZone;
        let window = snapshot::install().unwrap();
        for (hour, minute, second) in [(0, 0, 1), (0, 5, 0), (9, 0, 0), (14, 0, 0)] {
            let now = chrono::Local
                .with_ymd_and_hms(2026, 10, 18, hour, minute, second)
                .unwrap()
                .timestamp_millis();
            set_test_clock(now);
            let (ui, view) = synthetic_ui();
            ui.set_demo(true);
            ui.set_connected(true);
            let (tx, rx) = std::sync::mpsc::channel();
            let worker = account::start_demo(
                view.demo_send_failure.clone(),
                HashMap::new(),
                move |event| {
                    tx.send(event).unwrap();
                },
            );
            let state = Rc::new(RefCell::new(view));
            state.borrow_mut().commands = Some(worker.commands.clone());
            let check = |rows: &[MessageRow]| {
                assert!(!rows.is_empty());
                assert!(
                    rows.iter()
                        .all(|row| row.sort_key.parse::<i64>().unwrap() <= now)
                );
            };
            for chat in demo::examples() {
                check(&chat.messages);
                assert!(
                    chat.messages
                        .windows(2)
                        .all(|pair| pair[0].sort_key < pair[1].sort_key)
                );
            }
            let mut today_rows: Vec<_> = demo::examples()
                .remove(0)
                .messages
                .into_iter()
                .filter(|row| day_label(row.sort_key.parse().unwrap(), now) == "Today")
                .collect();
            assert!(!today_rows.is_empty());
            if hour == 14 {
                group_messages(&mut today_rows);
                let times: std::collections::HashSet<_> =
                    today_rows.iter().map(|row| row.clock.as_str()).collect();
                assert_eq!(times.len(), today_rows.len());
                assert!(today_rows.windows(2).any(|pair| {
                    !pair[0].outgoing
                        && !pair[1].outgoing
                        && pair[1].sort_key.parse::<i64>().unwrap()
                            - pair[0].sort_key.parse::<i64>().unwrap()
                            == 5 * 60_000
                        && pair[1].group_start
                }));
            }
            check(&demo::rich_fixture());
            check(&demo::paged_rows(0..1000));
            for kind in [
                0, 5, 6, 7, 8, 12, 14, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
            ] {
                demo::scenario(&state, &ui, kind);
                check(&state.borrow().messages);
            }
            for kind in [5, 7, 8, 12] {
                demo::scenario(&state, &ui, kind);
                ui.set_draft("ORDER PROBE".into());
                assert!(state.borrow_mut().submit(&ui));
                assert!(ui.get_draft().is_empty());
                let local_id = state.borrow().saved.outbox.last().unwrap().local_id.clone();
                let pending = state
                    .borrow()
                    .with_outbox("0", state.borrow().messages.clone());
                assert_eq!(pending.last().unwrap().id, local_id);
                for _ in 0..2 {
                    state.borrow_mut().update(
                        &ui,
                        rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),
                    );
                }
                assert_eq!(state.borrow().messages.last().unwrap().body, "ORDER PROBE");
                assert_eq!(state.borrow().rows[0].preview, "ORDER PROBE");
                assert_eq!(state.borrow().rows[0].activity, order_key(now));
                let confirmed = state
                    .borrow()
                    .with_outbox("0", state.borrow().messages.clone());
                assert_eq!(confirmed.last().unwrap().id, local_id);
                assert_eq!(confirmed.last().unwrap().delivery, "Sent");
                assert_eq!(
                    day_label(confirmed.last().unwrap().sort_key.parse().unwrap(), now),
                    "Today"
                );
            }
            ui.show().unwrap();
            snapshot::render(&window, (360, 520)).unwrap();
            ui.hide().unwrap();
        }
    })
    .join()
    .unwrap();
}

#[test]
fn demo_refused_prepared_send_does_not_arm_failure() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let (ui, view) = synthetic_ui();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = account::start_demo(
            view.demo_send_failure.clone(),
            HashMap::new(),
            move |event| {
                tx.send(event).unwrap();
            },
        );
        let state = Rc::new(RefCell::new(view));
        state.borrow_mut().commands = Some(worker.commands.clone());
        for kind in [3, 10] {
            ui.set_connected(false);
            ui.set_draft("Preserve this draft".into());
            demo::scenario(&state, &ui, kind);
            assert_eq!(
                state
                    .borrow()
                    .demo_send_failure
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            assert!(state.borrow().saved.outbox.is_empty());
            assert_eq!(ui.get_draft(), "Preserve this draft");
            assert!(rx.try_recv().is_err());
            ui.set_connected(true);
            assert!(state.borrow_mut().submit(&ui));
            let event = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
            assert!(matches!(event, account::Update::Sent { .. }));
            state.borrow_mut().update(&ui, event);
            state.borrow_mut().update(
                &ui,
                rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),
            );
            assert!(state.borrow().saved.outbox.is_empty());
        }
    })
    .join()
    .unwrap();
}

#[test]
fn demo_group_scenario_opens_at_latest() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let (ui, view) = synthetic_ui();
        let state = Rc::new(RefCell::new(view));
        ui.set_text_size(24);
        ui.set_show_conversation(true);
        ui.show().unwrap();
        snapshot::render(&window, (360, 520)).unwrap();
        ui.invoke_show_history(0., false);
        ui.window().request_redraw();
        snapshot::render(&window, (360, 520)).unwrap();
        assert!(!ui.get_following());
        demo::scenario(&state, &ui, 7);
        snapshot::render(&window, (360, 520)).unwrap();
        assert!(ui.get_following());
        let extent = ui.global::<State>().get_history_extent();
        assert!(extent > 0.);
        assert!((ui.get_history_y() + extent).abs() < 0.5);
    })
    .join()
    .unwrap();
}

#[test]
fn demo_midnight_gap_uses_valid_local_time() {
    use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike};
    #[derive(Clone)]
    struct MidnightGap;
    impl TimeZone for MidnightGap {
        type Offset = FixedOffset;
        fn from_offset(_: &Self::Offset) -> Self {
            Self
        }
        fn offset_from_local_date(&self, _: &NaiveDate) -> LocalResult<Self::Offset> {
            LocalResult::Single(FixedOffset::east_opt(0).unwrap())
        }
        fn offset_from_local_datetime(&self, time: &NaiveDateTime) -> LocalResult<Self::Offset> {
            if time.hour() == 0 {
                LocalResult::None
            } else {
                self.offset_from_local_date(&time.date())
            }
        }
        fn offset_from_utc_date(&self, _: &NaiveDate) -> Self::Offset {
            FixedOffset::east_opt(0).unwrap()
        }
        fn offset_from_utc_datetime(&self, time: &NaiveDateTime) -> Self::Offset {
            self.offset_from_utc_date(&time.date())
        }
    }
    let now = MidnightGap.with_ymd_and_hms(2026, 10, 18, 1, 5, 0).unwrap();
    let start = MidnightGap
        .with_ymd_and_hms(2026, 10, 18, 1, 0, 0)
        .unwrap()
        .timestamp_millis();
    for clock in ["09:18", "10:36", "10:41", "12:11"] {
        let timestamp = demo::fixture_time_at(clock, now);
        assert!(timestamp >= start && timestamp < now.timestamp_millis());
    }
    let past = demo::fixture_time_at("2026/10/17 00:30", now);
    assert_eq!(past, now.timestamp_millis() - 86_400_000);
}
