//! Link parsing and desktop dispatch. Parsing uses UTF-8 boundaries and never
//! treats adjacent Japanese sentence punctuation as part of a URL.
use crate::{LinkRow, MessageRow, RichRun};
use slint::{Model, ModelRc, VecModel};

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
        if rendered != row.body.as_str() && row.rich_runs.iter().any(|run| run.link_id != 0) {
            row.body.to_string()
        } else {
            rendered
        }
    }
}

pub(crate) fn decorate_message(row: &mut MessageRow) {
    let rendered = rendered_text(row);
    if row.kind != crate::MessageKind::Flex {
        row.links = ModelRc::new(VecModel::from(
            detect(&rendered)
                .into_iter()
                .map(|(label, url)| LinkRow {
                    label: label.into(),
                    url: url.into(),
                })
                .collect::<Vec<_>>(),
        ));
    }
    decorate_links(row, &rendered);
    let sender = if row.sender_name.is_empty() {
        crate::presenter::names::CONTACT
    } else {
        row.sender_name.as_str()
    };
    let body = if row.alt_text.is_empty() || row.alt_text == row.body {
        row.body.to_string()
    } else {
        format!("{}\n{}", row.alt_text, row.body)
    };
    row.accessible_label = if row.outgoing {
        format!(
            "Outgoing, {}, {}: {}",
            row.time,
            match row.delivery {
                crate::DeliveryState::Pending => "Sending…",
                crate::DeliveryState::Sent => "Sent",
                crate::DeliveryState::Read => "Read",
                crate::DeliveryState::Rejected => "Not sent",
            },
            body
        )
    } else {
        format!("Incoming from {sender}, {}: {}", row.time, body)
    }
    .into();
}

/// Only detected web URLs reach the desktop; callers dispatch off the UI thread.
pub(crate) fn open(url: &str) -> std::io::Result<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Unsupported link",
        ));
    }
    let status = std::process::Command::new("xdg-open").arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("Link opener failed"))
    }
}

pub(super) fn url_break(left: char, right: char) -> bool {
    !("/:.?&=-_#".contains(left) && "/:.?&=-_#".contains(right))
}

fn decorate_links(row: &mut MessageRow, body: &str) {
    use std::collections::BTreeSet;
    row.has_inline_emoji = row.rich_runs.iter().any(|run| run.emoji);
    if row.links.row_count() == 0 || row.rich_fallback {
        return;
    }
    let mut spans = Vec::new();
    let mut offset = 0;
    for (index, link) in row.links.iter().enumerate() {
        if let Some(start) = body[offset..].find(link.label.as_str()) {
            let start = offset + start;
            offset = start + link.label.len();
            spans.push((start, offset, link.url, index as i32 + 1));
        }
    }
    let mut breaks = BTreeSet::from([0, body.len()]);
    for (end, _) in unicode_linebreak::linebreaks(body) {
        let inside = spans.iter().any(|(a, b, _, _)| *a < end && end < *b);
        if !inside
            || url_break(
                body[..end].chars().next_back().unwrap(),
                body[end..].chars().next().unwrap(),
            )
        {
            breaks.insert(end);
        }
        if breaks.len() > (crate::rich_content::MAX_RUNS + 1) {
            break;
        }
    }
    for (a, b, _, _) in &spans {
        for (i, left) in body[*a..*b].char_indices() {
            let end = a + i + left.len_utf8();
            if end < *b
                && "/.?&=-_#".contains(left)
                && url_break(left, body[end..].chars().next().unwrap())
            {
                breaks.insert(end);
            }
            if breaks.len() > (crate::rich_content::MAX_RUNS + 1) {
                break;
            }
        }
        if breaks.len() > (crate::rich_content::MAX_RUNS + 1) {
            break;
        }
    }
    let mut cuts = breaks.clone();
    for (a, b, _, _) in &spans {
        cuts.extend([*a, *b]);
    }
    let mut images = Vec::new();
    let mut offset = 0;
    for run in row.rich_runs.iter() {
        let end = offset + run.text.len();
        if run.emoji {
            cuts.extend([offset, end]);
            images.push((offset, end, run));
        }
        offset = end;
    }
    for (i, c) in body.char_indices() {
        if c == '\n' {
            cuts.extend([i, i + 1]);
        }
        if cuts.len() > (crate::rich_content::MAX_RUNS + 1) {
            break;
        }
    }
    let mut result = Vec::new();
    if cuts.len() <= (crate::rich_content::MAX_RUNS + 1) {
        let mut start = 0;
        for end in cuts.into_iter().skip(1) {
            if images.iter().any(|(a, b, _)| *a < end && end < *b) {
                continue;
            }
            let mut run = images
                .iter()
                .find(|(a, b, _)| *a == start && *b == end)
                .map(|(_, _, r)| r.clone())
                .unwrap_or_default();
            run.text = body[start..end].into();
            run.newline = run.text == "\n";
            run.break_before = breaks.contains(&start);
            if let Some((_, _, url, id)) =
                spans.iter().find(|(a, b, _, _)| *a <= start && start < *b)
            {
                run.url = url.clone();
                run.link_id = *id;
            }
            result.push(run);
            start = end;
        }
        if result.last().is_some_and(|run| run.newline) {
            result.push(RichRun {
                break_before: true,
                ..Default::default()
            });
        }
    }
    if result.is_empty() || result.len() > crate::rich_content::MAX_RUNS {
        result.clear();
        row.rich_fallback = true;
    }
    row.rich_runs = ModelRc::new(VecModel::from(result));
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
        decorate_message(&mut row);
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
            sender_name: "葵".into(),
            body: "Hello".into(),
            time: "12:30".into(),
            ..Default::default()
        };
        decorate_message(&mut row);
        assert_eq!(row.accessible_label, "Incoming from 葵, 12:30: Hello");
        row.outgoing = true;
        row.delivery = crate::DeliveryState::Rejected;
        decorate_message(&mut row);
        assert_eq!(row.accessible_label, "Outgoing, 12:30, Not sent: Hello");
    }
}
