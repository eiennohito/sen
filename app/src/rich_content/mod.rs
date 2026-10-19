//! Rich text uses native measured runs; the original body remains the copy source.
use crate::*;
pub(crate) const MAX_RUNS: usize = 2048;

pub(crate) fn runs(body: &str, emoji: &[presenter::format::EmojiView]) -> ModelRc<RichRun> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for item in emoji {
        if rows.len() > crate::rich_content::MAX_RUNS {
            break;
        }
        let span = &item.span;
        if span.start_byte < offset
            || span.start_byte >= span.end_byte
            || !body.is_char_boundary(span.start_byte)
            || !body.is_char_boundary(span.end_byte)
            || span.end_byte > body.len()
        {
            continue;
        }
        text_runs(&body[offset..span.start_byte], &mut rows);
        rows.push(RichRun {
            text: body[span.start_byte..span.end_byte].into(),
            image_key: item.image_key.clone().into(),
            emoji: true,
            break_before: true,
            ..Default::default()
        });
        offset = span.end_byte;
    }
    text_runs(&body[offset..], &mut rows);
    if rows.last().is_some_and(|run| run.newline) {
        rows.push(RichRun::default());
    }
    // A pathological rich body falls back to the complete selectable body.
    if rows.len() > crate::rich_content::MAX_RUNS {
        rows.clear();
    }
    ModelRc::new(VecModel::from(rows))
}
fn text_runs(text: &str, rows: &mut Vec<RichRun>) {
    let mut start = 0;
    for (end, _) in unicode_linebreak::linebreaks(text) {
        for segment in text[start..end].split_inclusive('\n') {
            let content = segment.strip_suffix('\n').unwrap_or(segment);
            if !content.is_empty() {
                rows.push(RichRun {
                    text: content.into(),
                    break_before: true,
                    ..Default::default()
                });
            }
            if segment.ends_with('\n') {
                rows.push(RichRun {
                    text: "\n".into(),
                    newline: true,
                    ..Default::default()
                });
            }
            if rows.len() > crate::rich_content::MAX_RUNS {
                return;
            }
        }
        start = end;
    }
}

pub(crate) mod flow;
pub(crate) mod links;
pub(crate) mod preview;
pub(crate) mod spans;

pub(crate) fn install(ui: &AppWindow) {
    ui.global::<State>().on_rich_source(|rows| {
        rows.model_tracker().track_row_count_changes();
        for i in 0..rows.row_count() {
            rows.model_tracker().track_row_data_changes(i);
        }
        // A new observer handle triggers reflow without copying the rows.
        ModelRc::new(rows)
    });
    let metrics = Rc::new(RefCell::new(flow::Measurements::default()));
    let preview_metrics = metrics.clone();
    let preview_weak = ui.as_weak();
    ui.global::<State>()
        .on_preview_layout(move |rows, prefix, width, size| {
            let Some(ui) = preview_weak.upgrade() else {
                return ModelRc::default();
            };
            crate::rich_content::preview::layout(&rows, &prefix, width, size, |text| {
                let key = (text.into(), size.to_bits());
                if let Some(value) = preview_metrics.borrow_mut().get(&key) {
                    return value.0;
                }
                let value = ui.invoke_measure_rich(text.into(), size);
                preview_metrics
                    .borrow_mut()
                    .insert(key, (value.width, value.height));
                value.width
            })
        });
    let weak = ui.as_weak();
    ui.global::<State>()
        .on_rich_layout(move |rows, width, size, previous| {
            weak.upgrade()
                .map(|ui| flow::reuse(previous, flow::layout(&ui, &rows, width, size, &metrics)))
                .unwrap_or_default()
        });
}
