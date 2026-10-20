use crate::*;
use unicode_segmentation::UnicodeSegmentation;

fn byte(text: &str, chars: i32) -> usize {
    text.char_indices()
        .nth(chars.max(0) as usize)
        .map_or(text.len(), |(i, _)| i)
}

pub(crate) fn offset(text: &str, chars: i32, mut measure: impl FnMut(&str) -> f32) -> f32 {
    measure(&text[..byte(text, chars)])
}

/// The URL of the link run under the pointer, or "" (cursor and tooltip only; no measuring).
pub(crate) fn link_at(
    rows: &ModelRc<RichRun>,
    x: f32,
    y: f32,
    centered: bool,
    width: f32,
) -> slint::SharedString {
    rows.iter()
        .find(|run| {
            let left = run.x
                + if centered {
                    ((width - run.line_width) / 2.).max(0.)
                } else {
                    0.
                };
            !run.url.is_empty()
                && (left..left + run.measured_width).contains(&x)
                && (run.y..run.y + run.measured_height).contains(&y)
        })
        .map(|run| run.url)
        .unwrap_or_default()
}

pub(crate) fn hit(
    rows: &ModelRc<RichRun>,
    x: f32,
    y: f32,
    centered: bool,
    width: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> RichPoint {
    let mut point = RichPoint::default();
    for (i, run) in rows.iter().enumerate() {
        point = RichPoint {
            run: i as i32,
            char: run.text.chars().count() as i32,
        };
        if y >= run.y + run.measured_height {
            continue;
        }
        let left = run.x
            + if centered {
                ((width - run.line_width) / 2.).max(0.)
            } else {
                0.
            };
        let local = x - left;
        if local > run.measured_width && rows.row_data(i + 1).is_some_and(|next| next.y == run.y) {
            continue;
        }
        if run.emoji {
            point.char = if local < run.measured_width / 2. {
                0
            } else {
                point.char
            };
        } else {
            let boundaries: Vec<_> = std::iter::once(0)
                .chain(run.text.grapheme_indices(true).map(|(b, g)| b + g.len()))
                .collect();
            let mut lo = 0;
            let mut hi = boundaries.len() - 1;
            while lo < hi {
                let mid = (lo + hi) / 2;
                let left = measure(&run.text[..boundaries[mid]]);
                let right = measure(&run.text[..boundaries[mid + 1]]);
                if local < (left + right) / 2. {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            point.char = run.text[..boundaries[lo]].chars().count() as i32;
        }
        return point;
    }
    point
}

pub(crate) fn word(
    source: &ModelRc<RichRun>,
    rows: &ModelRc<RichRun>,
    point: RichPoint,
) -> RichSelection {
    let Some(run) = rows.row_data(point.run.max(0) as usize) else {
        return RichSelection::default();
    };
    if run.emoji {
        return RichSelection {
            anchor: RichPoint {
                run: point.run,
                char: 0,
            },
            focus: RichPoint {
                run: point.run,
                char: run.text.chars().count() as i32,
            },
        };
    }
    let text: String = source.iter().map(|r| r.text.to_string()).collect();
    let position =
        (run.source_start as usize + byte(&run.text, point.char)).min(text.len().saturating_sub(1));
    let (start, end) = text
        .split_word_bound_indices()
        .find(|(i, word)| *i <= position && position < i + word.len())
        .map_or((0, text.len()), |(i, word)| (i, i + word.len()));
    let locate = |offset: usize| {
        let mut result = RichPoint::default();
        for (i, run) in rows.iter().enumerate() {
            let start = run.source_start as usize;
            result = RichPoint {
                run: i as i32,
                char: run.text.chars().count() as i32,
            };
            if offset <= start + run.text.len() {
                result.char = run.text[..offset.saturating_sub(start).min(run.text.len())]
                    .chars()
                    .count() as i32;
                break;
            }
        }
        result
    };
    RichSelection {
        anchor: locate(start),
        focus: locate(end),
    }
}

pub(crate) fn copy(
    source: &ModelRc<RichRun>,
    rows: &ModelRc<RichRun>,
    selection: RichSelection,
) -> slint::SharedString {
    let text: String = source.iter().map(|run| run.text.to_string()).collect();
    let position = |point: RichPoint| {
        if point.run == 0 && point.char == 0 {
            return 0;
        }
        if point.run == rows.row_count() as i32 - 1 && point.char == i32::MAX {
            return text.len();
        }
        rows.row_data(point.run.max(0) as usize).map_or(0, |run| {
            run.source_start as usize + byte(&run.text, point.char)
        })
    };
    let a = position(selection.anchor);
    let b = position(selection.focus);
    text[a.min(b).min(text.len())..a.max(b).min(text.len())].into()
}
