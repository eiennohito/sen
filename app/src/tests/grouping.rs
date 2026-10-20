use super::*;

#[test]
fn grouping_uses_identity_time_and_delivery() {
    use chrono::TimeZone;
    use line_desktop_ui::RunPosition::{First, Last, Single};
    let midnight = chrono::Local
        .with_ymd_and_hms(2026, 10, 16, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let message = |id: u64, sender: &str, time| {
        let mut m = super::message("Hello");
        m.key = Key(id as i64);
        m.server_id = Some(ServerId::new(id.to_string()).unwrap());
        m.sort.sort_ts = time;
        m.sender_id = sender.into();
        m
    };
    let mut source = vec![
        message(1, "a", midnight - 600_001),
        message(2, "a", midnight - 300_002),
        message(3, "a", midnight - 1),
        message(4, "b", midnight - 1),
        message(5, "b", midnight),
    ];
    let mut rows = vec![
        MessageRow {
            sender_name: "Same name".into(),
            time: "Arbitrary display text".into(),
            delivery: DeliveryState::Rejected,
            ..Default::default()
        };
        5
    ];
    let group = |rows: &mut [MessageRow], source: &[StoredMessage]| {
        group_messages(rows, &source.iter().map(Some).collect::<Vec<_>>());
    };
    group(&mut rows, &source);
    assert_eq!(
        rows.iter().map(|m| m.run_position).collect::<Vec<_>>(),
        [First, Last, Single, Single, Single]
    );
    assert!(!rows[0].day_label.is_empty());
    assert!(rows[1..4].iter().all(|row| row.day_label.is_empty()));
    assert!(!rows[4].day_label.is_empty());
    assert_eq!(rows[4].clock, "00:00");
    source[3].outgoing = true;
    source[4].outgoing = true;
    source[4].sender_id.clear();
    source[4].sort.sort_ts = source[3].sort.sort_ts;
    group(&mut rows, &source);
    assert_eq!((rows[3].run_position, rows[4].run_position), (First, Last));
    for delivery in [
        Delivery::Pending,
        Delivery::Rejected {
            reason: line::backend::Refusal::FileMissing,
        },
    ] {
        source[4].delivery = delivery;
        group(&mut rows, &source);
        assert_eq!(
            (rows[3].run_position, rows[4].run_position),
            (Single, Single)
        );
    }
    for message in &mut source[3..] {
        message.outgoing = false;
        message.sender_id.clear();
        message.delivery = Delivery::Sent;
    }
    group(&mut rows, &source);
    assert_eq!(
        (rows[3].run_position, rows[4].run_position),
        (Single, Single)
    );
    group_messages(&mut rows[..3], &[Some(&source[0]), None, Some(&source[1])]);
    assert!(rows[..3].iter().all(|row| row.run_position == Single));
}

#[test]
fn own_member_uses_account_profile_and_keeps_you_note() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = test_window();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.apply(Delta::Profile(Profile {
            mid: "self".into(),
            display_name: "Account name".into(),
            status_message: "Account status".into(),
            picture_path: Some("/own".into()),
        }));
        state.members.push(Member {
            contact: Friend {
                id: "self".into(),
                ..Default::default()
            },
            is_self: true,
        });
        presenter.present(&ui, &state, "", ChatCategory::All);
        let member = ui.global::<State>().get_members().row_data(0).unwrap();
        assert_eq!(member.name, "Account name");
        assert_eq!(member.status, "Account status");
        assert!(member.is_self, "member row must keep its You note");
        assert_eq!(
            member.avatar_key,
            state
                .contact("self")
                .unwrap()
                .avatar
                .as_ref()
                .unwrap()
                .key()
        );
    })
    .join()
    .unwrap();
}
