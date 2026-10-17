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
        row("3", "a", 600_000, "2026/10/15"), // exactly five minutes: break
        row("4", "b", 600_001, "2026/10/15"), // same name, different identity
        row("5", "b", 600_002, "2026/10/16"), // midnight: break
    ];
    group_messages(&mut rows);
    assert_eq!(
        rows.iter().map(|m| m.group_start).collect::<Vec<_>>(),
        [true, false, true, true, true]
    );
    let now = chrono::Local::now().timestamp_millis();
    assert_eq!(rows[0].day_label, day_label(1, now));
    assert_eq!(rows[4].day_label, day_label(600_002, now));
    assert!(rows[1..4].iter().all(|row| row.day_label.is_empty()));
    assert!(rows.iter().all(|m| m.clock == "12:00"));
    rows[3].outgoing = true;
    rows[4].outgoing = true;
    rows[4].time = rows[3].time.clone();
    group_messages(&mut rows);
    assert!(rows[3].group_start);
    assert!(!rows[4].group_start);
    // Unknown group senders must never merge into a fictitious person.
    rows[3].outgoing = false;
    rows[4].outgoing = false;
    rows[3].sender_id = "".into();
    rows[4].sender_id = "".into();
    group_messages(&mut rows);
    assert!(rows[3].group_start && rows[4].group_start);
}
