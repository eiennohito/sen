use super::*;
use sen::backend::{Failure, FakeBackend, FakeRpc};

#[test]
fn own_profile_editor_saves_cancels_validates_and_keeps_failed_edits() {
    std::thread::spawn(|| {
        use slint::platform::Key;
        i_slint_backend_testing::init_no_event_loop();
        let fake = FakeBackend::canned();
        let (ui, runtime, worker) = test_window(fake.clone(), |_, _, _| {});
        let wait = |done: &dyn Fn(&Delta) -> bool| loop {
            let delta = worker
                .deltas
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let finished = done(&delta);
            runtime.update(|d| d.apply_batch(&ui, [delta]));
            if finished {
                break;
            }
        };
        wait(&|d| matches!(d, Delta::Connection(Connection::Online)));
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        ui.show().unwrap();
        ui.global::<DetailState>()
            .invoke_open_profile("self".into());
        slint::platform::update_timers_and_animations();
        let click = |label| {
            element(&ui, label).mock_single_click(slint::platform::PointerEventButton::Left);
        };
        let action_count = || {
            fake.calls()
                .iter()
                .filter(|c| matches!(c, FakeRpc::Action(_)))
                .count()
        };
        click("Edit name");
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::EditName
        );
        let _ = element(&ui, "Name");
        assert_eq!(focus_label(&ui), "Name");
        element(&ui, "Name").set_accessible_value("Cancelled");
        press(&ui, Key::Escape);
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::None
        );
        assert!(!ui.global::<DetailState>().get_detail_busy());
        assert_eq!(action_count(), 0);
        click("Edit name");
        for invalid in ["", "   "] {
            element(&ui, "Name").set_accessible_value(invalid);
            assert_eq!(element(&ui, "Save").accessible_enabled(), Some(false));
            press(&ui, Key::Return);
            assert_eq!(action_count(), 0);
        }
        element(&ui, "Name").set_accessible_value("1234567890123456789");
        press(&ui, Key::End);
        press(&ui, "🌸");
        assert_eq!(
            element(&ui, "Name").accessible_value().as_deref(),
            Some("1234567890123456789")
        );
        assert!(visible(&ui, "(19/20)"));
        element(&ui, "Name").set_accessible_value("  New name 🌸  ");
        press(&ui, Key::Return);
        // The UI stays busy until it consumes the worker's completion.
        assert!(ui.global::<DetailState>().get_detail_busy());
        assert_eq!(element(&ui, "Save").accessible_enabled(), Some(false));
        press(&ui, Key::Return);
        ui.global::<DetailState>()
            .invoke_detail_action(DetailAction::EditName, "Double submit".into());
        wait(&|d| matches!(d, Delta::ProfileAction { .. }));
        assert_eq!(action_count(), 1);
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::None
        );
        assert_eq!(ui.global::<ShellState>().get_account_name(), "New name 🌸");
        assert_eq!(ui.global::<DetailState>().get_profile_name(), "New name 🌸");
        runtime.update(|d| {
            assert_eq!(
                d.state.contact("self").unwrap().name.as_deref(),
                Some("New name 🌸")
            )
        });
        click("Edit status message");
        element(&ui, "Status message").set_accessible_value("Previous status");
        press(&ui, Key::Return);
        wait(&|d| matches!(d, Delta::ProfileAction { .. }));
        for (label, field, kind, action, value, error) in [
            (
                "Edit name",
                "Name",
                DetailAction::EditName,
                ProfileAction::EditName {
                    name: "Rejected".into(),
                },
                "Rejected",
                "Couldn't save name.",
            ),
            (
                "Edit status message",
                "Status message",
                DetailAction::EditStatus,
                ProfileAction::EditStatus {
                    status: "Rejected status".into(),
                },
                "Rejected status",
                "Couldn't save status message.",
            ),
        ] {
            click(label);
            element(&ui, field).set_accessible_value(value);
            fake.fail_rpc(FakeRpc::Action(action), Failure::NotDelivered);
            press(&ui, Key::Return);
            wait(&|d| matches!(d, Delta::ProfileAction { .. }));
            assert_eq!(ui.global::<DetailState>().get_detail_dialog(), kind);
            assert_eq!(
                element(&ui, field).accessible_value().as_deref(),
                Some(value)
            );
            assert!(visible(&ui, error));
            assert_eq!(ui.global::<ShellState>().get_account_name(), "New name 🌸");
            assert_eq!(
                ui.global::<DetailState>().get_profile_status(),
                "Previous status"
            );
            press(&ui, Key::Escape);
        }
        click("Edit status message");
        element(&ui, "Status message").set_accessible_value("x".repeat(501));
        assert_eq!(
            element(&ui, "Status message")
                .accessible_value()
                .unwrap()
                .len(),
            500
        );
        assert_eq!(element(&ui, "Save").accessible_enabled(), Some(true));
        assert!(visible(&ui, "(500/500)"));
        element(&ui, "Status message").set_accessible_value("At the station");
        press(&ui, Key::Return);
        wait(&|d| matches!(d, Delta::ProfileAction { .. }));
        assert_eq!(
            ui.global::<DetailState>().get_profile_status(),
            "At the station"
        );
        click("Edit status message");
        element(&ui, "Status message").set_accessible_value("");
        press(&ui, Key::Return);
        wait(&|d| matches!(d, Delta::ProfileAction { .. }));
        assert_eq!(ui.global::<DetailState>().get_profile_status(), "");
    })
    .join()
    .unwrap();
}
