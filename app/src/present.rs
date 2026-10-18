use crate::*;

thread_local! {
    static FIXED_NOW: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn now_millis() -> i64 {
    FIXED_NOW.with(|now| {
        now.get()
            .unwrap_or_else(|| chrono::Local::now().timestamp_millis())
    })
}

pub(crate) fn fixed_now() -> Option<i64> {
    FIXED_NOW.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn set_test_clock(now: i64) {
    FIXED_NOW.with(|clock| clock.set(Some(now)));
}

pub(crate) fn fix_snapshot_clock() {
    use chrono::TimeZone;
    FIXED_NOW.with(|now| {
        now.set(Some(
            chrono::Local
                .with_ymd_and_hms(2026, 10, 18, 14, 0, 0)
                .unwrap()
                .timestamp_millis(),
        ))
    });
}

pub(crate) fn name_initial(name: &str) -> slint::SharedString {
    name.chars().next().unwrap_or('·').to_string().into()
}

pub(crate) fn chat_subtitle(row: &ChatRow) -> String {
    if row.group && row.member_count > 0 {
        format!("{} members", row.member_count)
    } else {
        String::new()
    }
}

pub(crate) fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Group only adjacent messages with known time and identity. Presentation is
/// derived after sorting/reconciliation, so refreshes and local sends agree.
pub(crate) fn group_messages(messages: &mut [MessageRow]) {
    let mut previous: Option<(String, bool, i64, String)> = None;
    for row in messages {
        row.sender_initial = row
            .sender_name
            .chars()
            .next()
            .unwrap_or('?')
            .to_string()
            .into();
        let (date, clock) = row.time.rsplit_once(' ').unwrap_or(("", row.time.as_str()));
        let date = date.to_owned();
        row.clock = clock.into();
        let timestamp = row.sort_key.parse::<i64>().unwrap_or(0);
        let sender = if row.outgoing {
            "self".to_owned()
        } else if !row.sender_id.is_empty() {
            row.sender_id.to_string()
        } else if row.sender_name.is_empty() {
            "direct-contact".to_owned()
        } else {
            row.id.to_string()
        };
        row.day_label = if previous.as_ref().is_none_or(|p| p.3 != date) {
            day_label(timestamp, now_millis()).into()
        } else {
            "".into()
        };
        row.group_start = previous.as_ref().is_none_or(|p| {
            p.0 != sender
                || p.1 != row.outgoing
                || p.3 != date
                || p.2 <= 0
                || timestamp <= 0
                || timestamp == i64::MAX
                || timestamp.saturating_sub(p.2) >= 5 * 60 * 1000
        });
        previous = Some((sender, row.outgoing, timestamp, date));
    }
}
pub(crate) fn order_key(timestamp: i64) -> String {
    format!("{:020}", timestamp.max(0))
}
pub(crate) fn display_time(timestamp: i64) -> String {
    if timestamp <= 0 || timestamp == i64::MAX {
        return String::new();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y/%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
pub(crate) fn list_time(timestamp: i64) -> String {
    relative_time(timestamp, now_millis())
}
/// Format a list timestamp using calendar boundaries in the desktop timezone.
pub(crate) fn relative_time(timestamp: i64, now: i64) -> String {
    use chrono::Datelike;
    let Some((time, today)) = calendar_pair(timestamp, now) else {
        return String::new();
    };
    let date = time.date_naive();
    let current = today.date_naive();
    let age = current.signed_duration_since(date).num_days();
    if age == 0 {
        time.format("%H:%M").to_string()
    } else if age == 1 {
        "Yesterday".into()
    } else if age > 1
        && date >= current - chrono::Duration::days(current.weekday().num_days_from_monday().into())
    {
        time.format("%a").to_string()
    } else if date.year() == current.year() {
        time.format("%-m/%-d").to_string()
    } else {
        time.format("%Y/%-m/%-d").to_string()
    }
}

pub(crate) fn day_label(timestamp: i64, now: i64) -> String {
    let Some((time, today)) = calendar_pair(timestamp, now) else {
        return String::new();
    };
    let age = today
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    match age {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => time.format("%A %-m/%-d").to_string(),
        _ => time.format("%Y/%-m/%-d").to_string(),
    }
}

fn calendar_pair(
    timestamp: i64,
    now: i64,
) -> Option<(
    chrono::DateTime<chrono::Local>,
    chrono::DateTime<chrono::Local>,
)> {
    if timestamp <= 0 || timestamp == i64::MAX {
        return None;
    }
    Some((
        chrono::DateTime::from_timestamp_millis(timestamp)?.with_timezone(&chrono::Local),
        chrono::DateTime::from_timestamp_millis(now)?.with_timezone(&chrono::Local),
    ))
}

pub(crate) fn outgoing_timestamp(item: &storage::Outgoing) -> i64 {
    if item.timestamp_ms > 0 {
        item.timestamp_ms
    } else {
        item.local_id
            .strip_prefix("local-")
            .and_then(|id| id.split('-').next())
            .and_then(|timestamp| timestamp.parse::<i64>().ok())
            .map(|micros| micros / 1000)
            .unwrap_or(i64::MAX)
    }
}

pub(crate) fn message_text(m: &account::MessageView) -> String {
    let kind = crate::links::kind(m.rich.content_type, m.rich.withdrawn, m.decryption_failed);
    let mut body = crate::links::placeholder(kind)
        .map(|label| {
            if kind == MessageKind::System {
                m.rich
                    .system_text
                    .clone()
                    .map(|text| format!("ⓘ {text}"))
                    .unwrap_or_else(|| label.into())
            } else if matches!(kind, MessageKind::File | MessageKind::Contact) && !m.body.is_empty()
            {
                format!("{label} · {}", m.body)
            } else {
                label.into()
            }
        })
        .unwrap_or_else(|| m.body.clone());
    if kind == MessageKind::Sticker && !m.body.is_empty() && m.body != "Sticker" {
        body = format!("{body} · {}", m.body);
    }
    if matches!(kind, MessageKind::Photo | MessageKind::Video) {
        if let Some(count) = m.rich.album_count {
            body = format!("▧ Album · {count} items");
        }
        if let Some(caption) = &m.rich.caption {
            body = format!("{body} · {caption}");
        }
    }
    body
}

pub(crate) fn message_row(m: account::MessageView) -> MessageRow {
    let kind = crate::links::kind(m.rich.content_type, m.rich.withdrawn, m.decryption_failed);
    let rich_runs = if kind == MessageKind::Text && !m.emoji.is_empty() {
        crate::rich_content::runs(&m.body, &m.emoji)
    } else {
        ModelRc::default()
    };
    let reply_id = m
        .rich
        .reply
        .as_ref()
        .map(|r| r.message_id.clone())
        .unwrap_or_default();
    let detail = m
        .rich
        .duration_ms
        .map(crate::rich_content::duration)
        .or_else(|| m.rich.file_size.map(crate::rich_content::file_size))
        .unwrap_or_default();
    let body = message_text(&m);
    let reactions = m
        .reactions
        .into_iter()
        .map(|r| ReactionRow {
            label: r.label.into(),
            image_key: r.image_key.into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    MessageRow {
        skip_badge: m.skip_badge,
        rich_fallback: kind == MessageKind::Text
            && !m.emoji.is_empty()
            && rich_runs.row_count() == 0,
        rich_runs,
        reply_id: reply_id.into(),
        reply_preview: "Reply to message".into(),
        detail: detail.into(),
        kind,
        delivered_time: order_key(m.delivered_time).into(),
        id: m.id.into(),
        sender_id: m.sender_id.into(),
        sender_name: m.sender_name.into(),
        sender_key: m.sender_key.into(),
        sticker: m.sticker,
        photo: m.photo,
        sort_key: order_key(m.timestamp).into(),
        body: body.into(),
        time: display_time(m.timestamp).into(),
        outgoing: m.outgoing,
        image_key: m.image_key.into(),
        reactions: ModelRc::new(VecModel::from(reactions)),
        ..Default::default()
    }
}

#[cfg(test)]
mod calendar_tests {
    use super::*;
    use chrono::TimeZone;
    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        chrono::Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn list_time_uses_calendar_midnight_and_week_boundaries() {
        let now = at(2026, 10, 17, 0, 1);
        assert_eq!(relative_time(at(2026, 10, 17, 0, 0), now), "00:00");
        assert_eq!(relative_time(at(2026, 10, 16, 23, 59), now), "Yesterday");
        assert_eq!(relative_time(at(2026, 10, 15, 12, 0), now), "Thu");
        assert_eq!(relative_time(at(2026, 10, 11, 12, 0), now), "10/11");
        let tuesday = at(2026, 10, 20, 12, 0);
        assert_eq!(relative_time(at(2026, 10, 18, 12, 0), tuesday), "10/18");
    }
    #[test]
    fn list_time_and_dividers_handle_year_and_rolling_week_edges() {
        let now = at(2026, 1, 2, 0, 1);
        assert_eq!(relative_time(at(2026, 1, 1, 23, 59), now), "Yesterday");
        assert_eq!(relative_time(at(2025, 12, 30, 12, 0), now), "Tue");
        assert_eq!(relative_time(at(2025, 12, 20, 12, 0), now), "2025/12/20");
        assert_eq!(day_label(at(2026, 1, 2, 0, 0), now), "Today");
        assert_eq!(day_label(at(2026, 1, 1, 23, 59), now), "Yesterday");
        assert_eq!(day_label(at(2025, 12, 27, 12, 0), now), "Saturday 12/27");
        assert_eq!(day_label(at(2025, 12, 26, 12, 0), now), "2025/12/26");
        assert!(relative_time(0, now).is_empty());
        assert!(day_label(i64::MAX, now).is_empty());
    }
}
