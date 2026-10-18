//! Link parsing and desktop dispatch. Parsing uses UTF-8 boundaries and never
//! treats adjacent Japanese sentence punctuation as part of a URL.
use crate::{LinkRow, MessageKind, MessageRow};
use slint::{ModelRc, VecModel};

pub(crate) fn detect(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for token in text
        .split(|c: char| c.is_whitespace() || "<>\"'「」『』。、！？（）【】〈〉《》".contains(c))
    {
        for (start, c) in token.char_indices() {
            let tail = &token[start..];
            let explicit = ["https://", "http://", "www."].iter().any(|prefix| {
                tail.get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            });
            let boundary = start == 0
                || token[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| !c.is_ascii_alphanumeric() && !"-_.@".contains(c));
            if !explicit && (!c.is_ascii_alphanumeric() || !boundary) {
                continue;
            }
            let mut label = tail;
            loop {
                let trimmed = label.trim_end_matches(['.', ',', ';', ':', '!', '?']);
                label = trimmed;
                if label.ends_with(')') && label.matches(')').count() > label.matches('(').count() {
                    label = &label[..label.len() - 1];
                } else {
                    break;
                }
            }
            let lower = label.to_ascii_lowercase();
            let scheme = if lower.starts_with("https://") {
                8
            } else if lower.starts_with("http://") {
                7
            } else {
                0
            };
            let address = &label[scheme..];
            let host = address.split(['/', '?', '#', ':']).next().unwrap_or("");
            let domain = host.rsplit_once('.').is_some_and(|(name, tld)| {
                !name.is_empty()
                    && name.split('.').all(|part| {
                        !part.is_empty()
                            && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                    && tld.len() >= 2
                    && tld.bytes().all(|b| b.is_ascii_alphabetic())
                    && (address.contains('/')
                        || lower.starts_with("www.")
                        || matches!(
                            tld.to_ascii_lowercase().as_str(),
                            "com"
                                | "org"
                                | "net"
                                | "edu"
                                | "gov"
                                | "io"
                                | "dev"
                                | "app"
                                | "jp"
                                | "uk"
                                | "de"
                                | "fr"
                                | "co"
                        ))
            });
            if (scheme > 0 && !host.is_empty()) || domain {
                let url = if scheme > 0 {
                    format!("{}{}", &lower[..scheme], address)
                } else {
                    format!("https://{label}")
                };
                found.push((label.to_owned(), url));
                break;
            }
        }
    }
    found
}

pub(crate) fn rendered_text(row: &MessageRow) -> String {
    use slint::Model;
    if row.rich_runs.row_count() == 0 || row.rich_fallback {
        row.body.to_string()
    } else {
        let rendered: String = row
            .rich_runs
            .iter()
            .map(|run| run.text.to_string())
            .collect();
        // A reused row with a changed body still has the previous decorated runs.
        if rendered != row.body.as_str() && row.rich_runs.iter().any(|run| run.link_id != 0) {
            row.body.to_string()
        } else {
            rendered
        }
    }
}

pub(crate) fn decorate_message(row: &mut MessageRow, fallback_sender: &str) {
    row.links = ModelRc::new(VecModel::from(
        detect(&rendered_text(row))
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
        assert!(detect("https:// www. text file.txt report.rs x.a foo.123").is_empty());
        assert_eq!(
            detect(
                "example.com/path www.example.xyz example.org HTTPS://example.com/a HTTP://example.com/b"
            ),
            vec![
                ("example.com/path".into(), "https://example.com/path".into()),
                ("www.example.xyz".into(), "https://www.example.xyz".into()),
                ("example.org".into(), "https://example.org".into()),
                (
                    "HTTPS://example.com/a".into(),
                    "https://example.com/a".into()
                ),
                ("HTTP://example.com/b".into(), "http://example.com/b".into()),
            ]
        );
        assert_eq!(
            detect("example.com/a.,;:!?)."),
            vec![("example.com/a".into(), "https://example.com/a".into())]
        );
        use crate::RichRun;
        use slint::Model;
        let mut row = MessageRow {
            body: "pre-run body".into(),
            rich_runs: ModelRc::new(VecModel::from(vec![
                RichRun {
                    text: "https://exa".into(),
                    ..Default::default()
                },
                RichRun {
                    text: "m".into(),
                    emoji: true,
                    ..Default::default()
                },
                RichRun {
                    text: "ple.com/path".into(),
                    ..Default::default()
                },
            ])),
            ..Default::default()
        };
        decorate_message(&mut row, "contact");
        assert_eq!(row.links.row_count(), 1);
        assert_eq!(
            row.links.row_data(0).unwrap().url,
            "https://example.com/path"
        );
        assert!(
            row.rich_runs
                .iter()
                .all(|run| run.url == "https://example.com/path")
        );
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
