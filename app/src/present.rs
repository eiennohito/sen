use crate::*;

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
            date.clone().into()
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
    if timestamp <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format(
                    if time.with_timezone(&chrono::Local).date_naive()
                        == chrono::Local::now().date_naive()
                    {
                        "%H:%M"
                    } else {
                        "%m/%d"
                    },
                )
                .to_string()
        })
        .unwrap_or_default()
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

pub(crate) fn message_row(m: account::MessageView) -> MessageRow {
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
        delivered_time: order_key(m.delivered_time).into(),
        id: m.id.into(),
        sender_id: m.sender_id.into(),
        sender_name: m.sender_name.into(),
        sender_key: m.sender_key.into(),
        sticker: m.sticker,
        photo: m.photo,
        sort_key: order_key(m.timestamp).into(),
        body: m.body.into(),
        time: display_time(m.timestamp).into(),
        outgoing: m.outgoing,
        image_key: m.image_key.into(),
        reactions: ModelRc::new(VecModel::from(reactions)),
        ..Default::default()
    }
}
