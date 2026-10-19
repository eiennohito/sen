use crate::*;

pub struct EmojiView {
    pub span: line::store::InlineEmoji,
    pub image_key: String,
}
impl EmojiView {
    pub(crate) fn new(span: line::store::InlineEmoji) -> Self {
        let image_key = line::client::desktop::assets::Asset::inline_emoji(&span)
            .map(|a| a.key())
            .unwrap_or_default();
        Self { span, image_key }
    }
}

#[cfg(feature = "demo")]
thread_local! {
    static FIXED_NOW: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn now_millis() -> i64 {
    #[cfg(feature = "demo")]
    if let Some(now) = FIXED_NOW.with(|now| now.get()) {
        return now;
    }
    chrono::Local::now().timestamp_millis()
}

#[cfg(feature = "demo")]
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
    unicode_segmentation::UnicodeSegmentation::graphemes(name, true)
        .next()
        .unwrap_or("·")
        .into()
}

pub(crate) fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Group only adjacent messages with known time and identity. Presentation is
/// derived after sorting/reconciliation, so refreshes and local sends agree.
pub(crate) fn group_messages(messages: &mut [MessageRow], source: &[Option<&StoredMessage>]) {
    let mut previous_date = None;
    for (row, message) in messages.iter_mut().zip(source) {
        row.sender_initial = name_initial(&row.sender_name);
        let time = message.and_then(|m| local_time(m.sort.sort_ts));
        let date = time.map(|time| time.date_naive());
        row.clock = time
            .map(|time| time.format("%H:%M").to_string())
            .unwrap_or_default()
            .into();
        row.day_label = if date != previous_date {
            message
                .map(|m| day_label(m.sort.sort_ts, now_millis()))
                .unwrap_or_default()
                .into()
        } else {
            "".into()
        };
        previous_date = date;
    }
    let joins = |left: usize, right: usize| {
        let (Some(left), Some(right)) = (source[left], source[right]) else {
            return false;
        };
        let a = left.sort.sort_ts;
        let b = right.sort.sort_ts;
        !matches!(left.body, Content::System { .. } | Content::Withdrawn)
            && !matches!(right.body, Content::System { .. } | Content::Withdrawn)
            && matches!(left.delivery, Delivery::Sent | Delivery::Read)
            && matches!(right.delivery, Delivery::Sent | Delivery::Read)
            && local_time(a)
                .zip(local_time(b))
                .is_some_and(|(a, b)| a.date_naive() == b.date_naive())
            && left.outgoing == right.outgoing
            && (left.outgoing || (!left.sender_id.is_empty() && left.sender_id == right.sender_id))
            && a > 0
            && b >= a
            && b - a <= 300_000
    };
    for i in 0..messages.len() {
        let before = i > 0 && joins(i - 1, i);
        let after = i + 1 < messages.len() && joins(i, i + 1);
        messages[i].run_position = match (before, after) {
            (false, false) => line_desktop_ui::RunPosition::Single,
            (false, true) => line_desktop_ui::RunPosition::First,
            (true, true) => line_desktop_ui::RunPosition::Middle,
            (true, false) => line_desktop_ui::RunPosition::Last,
        };
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

fn local_time(timestamp: i64) -> Option<chrono::DateTime<chrono::Local>> {
    if timestamp <= 0 || timestamp == i64::MAX {
        return None;
    }
    Some(chrono::DateTime::from_timestamp_millis(timestamp)?.with_timezone(&chrono::Local))
}

fn calendar_pair(
    timestamp: i64,
    now: i64,
) -> Option<(
    chrono::DateTime<chrono::Local>,
    chrono::DateTime<chrono::Local>,
)> {
    Some((local_time(timestamp)?, local_time(now)?))
}

/// Compact labels shared by previews, notifications and the composer reply bar.
pub(crate) fn kind_label(content: &Content) -> (MessageKind, String) {
    use MessageKind::*;
    match content {
        Content::Text { text, .. } => (Text, text.clone()),
        Content::Photo { .. } => (Photo, "Photo".into()),
        Content::Video { .. } => (Video, "Video".into()),
        Content::Voice { .. } => (Voice, "Voice message".into()),
        Content::Sticker { .. } => (Sticker, "Sticker".into()),
        Content::File { name, .. } => (
            File,
            name.as_ref()
                .map(|n| format!("File · {n}"))
                .unwrap_or_else(|| "File".into()),
        ),
        Content::Contact { name, .. } => (
            Contact,
            name.as_ref()
                .map(|n| format!("Contact · {n}"))
                .unwrap_or_else(|| "Contact".into()),
        ),
        Content::Location { .. } => (Location, "Location".into()),
        Content::System { text, .. } => (System, text.clone().unwrap_or_default()),
        Content::Withdrawn => (Withdrawn, "Message unsent".into()),
        Content::Undecryptable => (Undecryptable, "Couldn't decrypt this message".into()),
        Content::Unknown { .. } => (Unknown, "This message type isn't supported yet".into()),
    }
}

/// Full row text for display, accessibility and copy.
pub(crate) fn message_text(content: &Content) -> (MessageKind, String) {
    let (kind, mut body) = kind_label(content);
    match content {
        Content::Photo {
            album_count,
            caption,
            ..
        }
        | Content::Video {
            album_count,
            caption,
            ..
        } => {
            if let Some(count) = album_count {
                body = format!("Album · {count} items");
            }
            if let Some(caption) = caption {
                body = format!("{body} · {caption}");
            }
        }
        Content::Sticker {
            text: Some(text), ..
        } => body = format!("{body} · {text}"),
        Content::File {
            name: Some(name), ..
        }
        | Content::Contact {
            name: Some(name), ..
        } => {
            body = name.clone();
        }
        _ => {}
    }
    (kind, body)
}

pub(crate) fn emoji_views(content: &Content) -> Vec<EmojiView> {
    content
        .spans()
        .iter()
        .cloned()
        .map(EmojiView::new)
        .collect()
}

pub(crate) fn reaction_views(reactions: &[Reaction]) -> Vec<ReactionRow> {
    let mut groups: Vec<(String, String)> = vec![];
    for reaction in reactions {
        let (label, key) = match &reaction.kind {
            ReactionKind::Predefined(id) => (
                match id {
                    2 => "👍",
                    3 => "❤️",
                    4 => "😆",
                    5 => "😮",
                    6 => "😢",
                    7 => "😰",
                    _ => "Reaction",
                }
                .to_owned(),
                String::new(),
            ),
            ReactionKind::Emoji => ("Emoji".into(), String::new()),
            ReactionKind::Paid(asset) => (
                "Emoji".into(),
                asset.as_ref().map(|a| a.key()).unwrap_or_default(),
            ),
        };
        if !groups.iter().any(|g| g.0 == label && g.1 == key) {
            groups.push((label, key));
        }
    }
    groups
        .into_iter()
        .take(3)
        .map(|(label, image_key)| ReactionRow {
            label: label.into(),
            image_key: image_key.into(),
            ..Default::default()
        })
        .collect()
}

pub(crate) fn message_row(m: &StoredMessage, system_text: Option<&str>) -> MessageRow {
    let (kind, body) = message_text(&m.body);
    let emoji = emoji_views(&m.body);
    let rich_runs = if kind == MessageKind::Text && !emoji.is_empty() {
        crate::rich_content::runs(m.body.as_text(), &emoji)
    } else {
        ModelRc::default()
    };
    let detail = match &m.body {
        Content::Video { duration, .. } | Content::Voice { duration, .. } => {
            duration.map(crate::rich_content::duration)
        }
        Content::File { size, .. } => size.map(crate::rich_content::file_size),
        _ => None,
    }
    .unwrap_or_default();
    let reactions = if m.body.is_withdrawn() {
        vec![]
    } else {
        reaction_views(&m.reactions)
    };
    MessageRow {
        skip_badge: m.skip_badge,
        rich_fallback: kind == MessageKind::Text && !emoji.is_empty() && rich_runs.row_count() == 0,
        rich_runs,
        reply_id: m
            .reply
            .as_ref()
            .map(|r| r.message_id.as_str())
            .unwrap_or_default()
            .into(),
        reply_preview: "Original message".into(),
        detail: detail.into(),
        kind,
        delivered_time: order_key(m.sort.sort_ts).into(),
        id: m.key.0.to_string().into(),
        sender_id: m.sender_id.clone().into(),
        sticker: kind == MessageKind::Sticker,
        photo: kind == MessageKind::Photo,
        body: system_text.unwrap_or(&body).into(),
        time: display_time(m.sort.sort_ts).into(),
        outgoing: m.outgoing,
        image_key: m.body.image().map(|a| a.key()).unwrap_or_default().into(),
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
