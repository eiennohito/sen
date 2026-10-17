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
