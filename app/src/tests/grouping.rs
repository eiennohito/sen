use super::*;
#[test]
fn grouping_respects_identity_five_minutes_and_calendar_days() {
    let row = |id: &str, sender: &str, timestamp: i64, date: &str| MessageRow {
        id: id.into(),
        sender_id: sender.into(),
        sender_name: "Same display name".into(),
        sort_key: order_key(timestamp).into(),
        time: format!("{date} 12:00").into(),
        ..Default::default()
    };
    let mut rows = vec![
        row("1", "a", 1, "2026/10/15"),
        row("2", "a", 300_000, "2026/10/15"), // 4:59.999: grouped
        row("3", "a", 600_001, "2026/10/15"), // over five minutes: break
        row("4", "b", 600_002, "2026/10/15"), // same name, different identity
        row("5", "b", 600_003, "2026/10/16"), // midnight: break
    ];
    group_messages(&mut rows);
    assert_eq!(
        rows.iter().map(|m| m.run_position).collect::<Vec<_>>(),
        [
            line_desktop_ui::RunPosition::First,
            line_desktop_ui::RunPosition::Last,
            line_desktop_ui::RunPosition::Single,
            line_desktop_ui::RunPosition::Single,
            line_desktop_ui::RunPosition::Single
        ]
    );
    let now = chrono::Local::now().timestamp_millis();
    assert_eq!(rows[0].day_label, day_label(1, now));
    assert_eq!(rows[4].day_label, day_label(600_003, now));
    assert!(rows[1..4].iter().all(|row| row.day_label.is_empty()));
    assert!(rows.iter().all(|m| m.clock == "12:00"));
    rows[3].outgoing = true;
    rows[4].outgoing = true;
    rows[4].sender_id = "".into(); // Pending sends have no server sender identity yet.
    rows[4].time = rows[3].time.clone();
    group_messages(&mut rows);
    assert_eq!(rows[3].run_position, line_desktop_ui::RunPosition::First);
    assert_eq!(rows[4].run_position, line_desktop_ui::RunPosition::Last);
    rows[4].delivery = "Sending…".into();
    group_messages(&mut rows);
    assert_eq!(rows[3].run_position, line_desktop_ui::RunPosition::Single);
    assert_eq!(rows[4].run_position, line_desktop_ui::RunPosition::Single);
    rows[4].delivery = "Not sent".into();
    group_messages(&mut rows);
    assert_eq!(rows[4].run_position, line_desktop_ui::RunPosition::Single);
    // Unknown group senders must never merge into a fictitious person.
    rows[3].outgoing = false;
    rows[4].outgoing = false;
    rows[3].sender_id = "".into();
    rows[4].sender_id = "".into();
    group_messages(&mut rows);
    assert_eq!(rows[3].run_position, line_desktop_ui::RunPosition::Single);
    assert_eq!(rows[4].run_position, line_desktop_ui::RunPosition::Single);
}
