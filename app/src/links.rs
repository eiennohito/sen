//! Link parsing and desktop dispatch. Parsing uses UTF-8 boundaries and never
//! treats adjacent Japanese sentence punctuation as part of a URL.
use crate::{LinkRow, MessageKind, MessageRow};
use slint::{ModelRc, VecModel};

pub(crate) fn detect(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let remaining = &text[offset..];
        let Some((start, _)) = remaining.char_indices().find(|(i, _)| {
            let tail = &remaining[*i..];
            tail.starts_with("https://") || tail.starts_with("http://") || tail.starts_with("www.")
        }) else {
            break;
        };
        let tail = &remaining[start..];
        let end = tail
            .char_indices()
            .find(|(_, c)| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '<' | '>'
                            | '"'
                            | '\''
                            | '「'
                            | '」'
                            | '『'
                            | '』'
                            | '。'
                            | '、'
                            | '！'
                            | '？'
                            | '（'
                            | '）'
                            | '【'
                            | '】'
                            | '〈'
                            | '〉'
                            | '《'
                            | '》'
                    )
            })
            .map_or(tail.len(), |(i, _)| i);
        let raw = &tail[..end];
        let label = raw.trim_end_matches(['.', ',', '!', '?', ';', ':']);
        // Keep balanced parentheses used by e.g. Wikipedia, discard sentence wraps.
        let mut label = label;
        while label.ends_with(')') && label.matches(')').count() > label.matches('(').count() {
            label = &label[..label.len() - 1];
        }
        if label.len()
            > if label.starts_with("www.") {
                4
            } else if label.starts_with("https://") {
                8
            } else {
                7
            }
        {
            let url = if label.starts_with("www.") {
                format!("https://{label}")
            } else {
                label.to_owned()
            };
            found.push((label.to_owned(), url));
        }
        offset += start + end.max(1);
        while !text.is_char_boundary(offset) {
            offset += 1;
        }
    }
    found
}

pub(crate) fn decorate_message(row: &mut MessageRow, fallback_sender: &str) {
    row.links = ModelRc::new(VecModel::from(
        detect(&row.body)
            .into_iter()
            .map(|(label, url)| LinkRow {
                label: label.into(),
                url: url.into(),
            })
            .collect::<Vec<_>>(),
    ));
    crate::rich_content::decorate_links(row);
    let sender = if row.sender_name.is_empty() {
        fallback_sender
    } else {
        row.sender_name.as_str()
    };
    row.accessible_label = if row.outgoing {
        format!(
            "Outgoing, {}, {}: {}",
            row.time,
            if row.delivery.is_empty() {
                "Sent"
            } else {
                &row.delivery
            },
            row.body
        )
    } else {
        format!("Incoming from {sender}, {}: {}", row.time, row.body)
    }
    .into();
}

pub(crate) fn kind(content_type: i64, withdrawn: bool, decryption_failed: bool) -> MessageKind {
    if withdrawn {
        return MessageKind::Withdrawn;
    }
    if decryption_failed {
        return MessageKind::Undecryptable;
    }
    match content_type {
        0 => MessageKind::Text,
        1 => MessageKind::Photo,
        2 => MessageKind::Video,
        3 => MessageKind::Voice,
        7 => MessageKind::Sticker,
        13 => MessageKind::Contact,
        14 => MessageKind::File,
        15 => MessageKind::Location,
        18 => MessageKind::System,
        _ => MessageKind::Unknown,
    }
}

pub(crate) fn placeholder(kind: MessageKind) -> Option<&'static str> {
    match kind {
        MessageKind::Text => None,
        MessageKind::Photo => Some("▧ Photo"),
        MessageKind::Sticker => Some("☺ Sticker"),
        MessageKind::Video => Some("▶ Video"),
        MessageKind::Voice => Some("♫ Voice message"),
        MessageKind::File => Some("▤ File"),
        MessageKind::Location => Some("⌖ Location"),
        MessageKind::Contact => Some("♙ Contact"),
        MessageKind::System => Some("ⓘ System message"),
        MessageKind::Withdrawn => Some("Message unsent"),
        MessageKind::Undecryptable => Some("⚿ Unable to decrypt message"),
        MessageKind::Unknown => Some("◇ Unknown message"),
    }
}

/// Only detected web URLs reach the desktop, and child startup never blocks UI.
pub(crate) fn open(url: String) {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("open-link".into())
        .spawn(move || {
            if let Ok(mut child) = std::process::Command::new("xdg-open").arg(url).spawn() {
                let _ = child.wait();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn japanese_punctuation_and_www() {
        assert_eq!(
            detect("参照「https://example.org/a」。次はwww.example.jp、どう？"),
            vec![
                (
                    "https://example.org/a".into(),
                    "https://example.org/a".into()
                ),
                ("www.example.jp".into(), "https://www.example.jp".into())
            ]
        );
    }
    #[test]
    fn balanced_url_parentheses_and_trailing_punctuation() {
        assert_eq!(
            detect("(https://example.org/wiki/Foo_(bar)).")[0].0,
            "https://example.org/wiki/Foo_(bar)"
        );
        assert!(detect("https:// www. text").is_empty());
    }
    #[test]
    fn message_labels_include_identity_time_and_delivery() {
        let mut row = MessageRow {
            body: "Hello".into(),
            time: "12:30".into(),
            ..Default::default()
        };
        decorate_message(&mut row, "葵");
        assert_eq!(row.accessible_label, "Incoming from 葵, 12:30: Hello");
        row.outgoing = true;
        row.delivery = "Unconfirmed".into();
        decorate_message(&mut row, "葵");
        assert_eq!(row.accessible_label, "Outgoing, 12:30, Unconfirmed: Hello");
    }

    #[test]
    fn placeholder_kinds_are_distinct() {
        let kinds = [1, 2, 3, 7, 13, 14, 15, 18, 99];
        let labels = kinds.map(|value| placeholder(kind(value, false, false)).unwrap());
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            kinds.len()
        );
        assert_eq!(kind(0, true, false), MessageKind::Withdrawn);
        assert_eq!(kind(0, false, true), MessageKind::Undecryptable);
    }
}
