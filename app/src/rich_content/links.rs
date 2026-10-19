use crate::*;

pub(super) fn url_break(left: char, right: char) -> bool {
    !("/:.?&=-_#".contains(left) && "/:.?&=-_#".contains(right))
}

pub(crate) fn decorate_links(row: &mut MessageRow) {
    use std::collections::BTreeSet;
    row.has_inline_emoji = row.rich_runs.iter().any(|run| run.emoji);
    if row.links.row_count() == 0 || row.rich_fallback {
        return;
    }
    let rendered = crate::links::rendered_text(row);
    let mut spans = Vec::new();
    let mut offset = 0;
    for (index, link) in row.links.iter().enumerate() {
        if let Some(start) = rendered[offset..].find(link.label.as_str()) {
            let start = offset + start;
            offset = start + link.label.len();
            spans.push((start, offset, link.url, index as i32 + 1));
        }
    }
    let body = rendered.as_str();
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
        if breaks.len() > 2049 {
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
            if breaks.len() > 2049 {
                break;
            }
        }
        if breaks.len() > 2049 {
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
        if cuts.len() > 2049 {
            break;
        }
    }
    let mut result = Vec::new();
    if cuts.len() <= 2049 {
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
    if result.is_empty() || result.len() > 2048 {
        result.clear();
        row.rich_fallback = true;
    }
    row.rich_runs = ModelRc::new(VecModel::from(result));
}
