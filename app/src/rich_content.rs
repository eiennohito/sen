//! Rich text uses native measured runs; the original body remains the copy source.
use crate::*;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn runs(body: &str, emoji: &[account::EmojiView]) -> ModelRc<RichRun> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for item in emoji {
        if rows.len() > 2048 {
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
            ..Default::default()
        });
        offset = span.end_byte;
    }
    text_runs(&body[offset..], &mut rows);
    if rows.last().is_some_and(|run| run.newline) {
        rows.push(RichRun::default());
    }
    // A pathological rich body falls back to the complete selectable body.
    if rows.len() > 2048 {
        rows.clear();
    }
    ModelRc::new(VecModel::from(rows))
}
fn text_runs(text: &str, rows: &mut Vec<RichRun>) {
    // Small chunks permit wrapping long URLs and Japanese without guessing fonts.
    // Actual widths come from Slint's shaping engine, including fallback fonts.
    for ch in text
        .graphemes(true)
        .take(2049usize.saturating_sub(rows.len()))
    {
        rows.push(RichRun {
            text: ch.to_string().into(),
            newline: ch == "\n",
            ..Default::default()
        });
    }
}

pub(crate) fn measure(
    rows: &ModelRc<RichRun>,
    index: usize,
    width: f32,
    height: f32,
    available: f32,
) {
    let Some(model) = rows.as_any().downcast_ref::<VecModel<RichRun>>() else {
        return;
    };
    let Some(mut run) = model.row_data(index) else {
        return;
    };
    if (run.measured_width - width).abs() < 0.25
        && (run.measured_height - height).abs() < 0.25
        && (run.available - available).abs() < 0.25
    {
        return;
    }
    run.measured_width = width.max(0.0);
    run.measured_height = height.max(1.0);
    run.available = available;
    model.set_row_data(index, run);
    // All native run measurements in one event-loop turn share one reflow.
    // Initializing/resizing 2048 runs therefore performs O(n) model work.
    let key = model as *const VecModel<RichRun> as usize;
    let schedule = PENDING_REFLOWS.with(|pending| pending.borrow_mut().insert(key));
    if schedule {
        let rows = rows.clone();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            reflow(&rows, available);
            PENDING_REFLOWS.with(|pending| {
                pending.borrow_mut().remove(&key);
            });
        });
    }
}
thread_local! { static PENDING_REFLOWS: RefCell<std::collections::HashSet<usize>> = RefCell::new(std::collections::HashSet::new()); }
fn reflow(rows: &ModelRc<RichRun>, available: f32) {
    let Some(model) = rows.as_any().downcast_ref::<VecModel<RichRun>>() else {
        return;
    };
    let widths: Vec<_> = (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect();
    let available = widths.first().map(|row| row.available).unwrap_or(available);
    if available < 1.0 || widths.iter().any(|row| row.measured_height < 1.0) {
        return;
    }
    let positions = pack(
        &widths
            .iter()
            .map(|row| (row.measured_width, row.measured_height, row.newline))
            .collect::<Vec<_>>(),
        available,
    );
    for (index, (x, y)) in positions.into_iter().enumerate() {
        let mut row = model.row_data(index).unwrap();
        if row.x != x || row.y != y {
            row.x = x;
            row.y = y;
            model.set_row_data(index, row);
        }
    }
}

fn pack(runs: &[(f32, f32, bool)], available: f32) -> Vec<(f32, f32)> {
    let mut result = Vec::with_capacity(runs.len());
    let (mut x, mut y, mut line_height) = (0.0, 0.0, 1.0_f32);
    for &(width, height, newline) in runs {
        if !newline && x > 0.0 && x + width > available.max(1.0) {
            y += line_height;
            x = 0.0;
            line_height = 1.0;
        }
        result.push((x, y));
        line_height = line_height.max(height);
        if newline {
            y += line_height;
            x = 0.0;
            line_height = 1.0;
        } else {
            x += width;
        }
    }
    result
}
pub(crate) fn duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
pub(crate) fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(crate) fn install(ui: &AppWindow, state: Rc<RefCell<View>>) {
    ui.global::<State>()
        .on_rich_measure(move |rows, index, width, height, available| {
            measure(&rows, index.max(0) as usize, width, height, available);
        });
    let weak = ui.as_weak();
    ui.global::<State>().on_save_file(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if ui.global::<State>().get_file_saving() {
            return;
        }
        let mut view = state.borrow_mut();
        ui.global::<State>().set_file_save_id(id.clone());
        ui.global::<State>().set_file_save_status("".into());
        ui.global::<State>().set_file_saving(true);
        if view.demo {
            save_download(
                &ui,
                "資料.txt".into(),
                std::sync::Arc::new(b"Synthetic attachment\n".to_vec()),
                true,
            );
            return;
        }
        view.serial += 1;
        view.file_request = view.serial;
        let command = account::Command::SaveFile {
            chat: view.current.clone(),
            message_id: id.to_string(),
            request: view.file_request,
        };
        if view
            .commands
            .as_ref()
            .is_none_or(|tx| tx.try_send(command).is_err())
        {
            ui.global::<State>().set_file_saving(false);
            ui.global::<State>()
                .set_file_save_status("Could not start download. Try again.".into());
        }
    });
}

pub(crate) fn save_download(
    ui: &AppWindow,
    name: String,
    bytes: std::sync::Arc<Vec<u8>>,
    demo: bool,
) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = crate::viewer::save_bytes(&bytes, &name, demo);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<State>().set_file_saving(false);
            ui.global::<State>().set_file_save_status(
                match result {
                    Ok(Some(_)) => "Saved".into(),
                    Ok(None) => String::new(),
                    Err(error) => error,
                }
                .into(),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_flow_wraps_without_font_width_guesses() {
        let positions = pack(
            &[
                (10., 20., false),
                (24., 24., false),
                (9., 20., false),
                (0., 20., true),
                (12., 20., false),
            ],
            35.,
        );
        assert_eq!(
            positions,
            [(0., 0.), (10., 0.), (0., 24.), (9., 24.), (0., 44.)]
        );
        assert_eq!(
            pack(&[(99., 20., false), (1., 20., false)], 20.),
            [(0., 0.), (0., 20.)]
        );
    }
    #[test]
    fn grapheme_tokens_preserve_fallback_and_multiline_copy_source() {
        let body = "日本👩🏽‍💻[笑]\nlongunbrokentext";
        let start = body.find("[笑]").unwrap();
        let end = start + "[笑]".len();
        let span = line_wasm_poc::client::desktop::rich::InlineEmoji {
            start_utf16: 0,
            end_utf16: 0,
            start_byte: start,
            end_byte: end,
            fallback: "[笑]".into(),
            product_id: "synthetic".into(),
            sticon_id: "one".into(),
            version: None,
            animated: false,
        };
        let rows = runs(
            body,
            &[account::EmojiView {
                span,
                image_key: "image-key".into(),
            }],
        );
        let tokens: Vec<_> = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .collect();
        assert_eq!(
            tokens.iter().map(|t| t.text.as_str()).collect::<String>(),
            body
        );
        assert!(tokens.iter().any(|t| t.text == "👩🏽‍💻"));
        assert_eq!(tokens.iter().filter(|t| t.emoji).count(), 1);
        assert_eq!(tokens.iter().filter(|t| t.newline).count(), 1);
        assert_eq!(runs(&"a".repeat(2049), &[]).row_count(), 0);
    }
    #[test]
    fn maximum_rich_flow_measures_and_reflows_at_narrow_large_text() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let state = Rc::new(RefCell::new(View::default()));
            install(&ui, state);
            let body = format!("{}[笑]{}", "a".repeat(1023), "日".repeat(1024));
            let start = 1023;
            let end = start + "[笑]".len();
            let span = line_wasm_poc::client::desktop::rich::InlineEmoji {
                start_utf16: 0,
                end_utf16: 0,
                start_byte: start,
                end_byte: end,
                fallback: "[笑]".into(),
                product_id: "synthetic".into(),
                sticon_id: "one".into(),
                version: None,
                animated: false,
            };
            let runs = runs(
                &body,
                &[account::EmojiView {
                    span,
                    image_key: "synthetic".into(),
                }],
            );
            assert_eq!(runs.row_count(), 2048);
            ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                id: "stress".into(),
                body: body.into(),
                rich_runs: runs.clone(),
                ..Default::default()
            }])));
            ui.set_show_conversation(true);
            ui.show().unwrap();
            let started = std::time::Instant::now();
            let path =
                std::env::temp_dir().join(format!("line-rich-stress-{}.ppm", std::process::id()));
            snapshot::save(&window, path.to_str().unwrap(), (980, 720)).unwrap();
            let before = runs.row_data(2047).unwrap().y;
            assert!(before > 0.0, "native run measurements did not initialize");
            ui.set_text_size(24);
            snapshot::save(&window, path.to_str().unwrap(), (360, 520)).unwrap();
            let last = runs.row_data(2047).unwrap();
            assert!(last.y > before, "narrow large text did not reflow");
            for index in 0..runs.row_count() {
                let run = runs.row_data(index).unwrap();
                assert!(
                    run.x + run.measured_width <= run.available + 1.0,
                    "rich run overflowed the pane"
                );
            }
            eprintln!(
                "synthetic2048run cold render+resize: {:?}",
                started.elapsed()
            );
            ui.hide().unwrap();
            std::fs::remove_file(path).unwrap();
        })
        .join()
        .unwrap();
    }
    #[test]
    fn metadata_labels_are_explicit() {
        assert_eq!(duration(125000), "2:05");
        assert_eq!(file_size(1024), "1.0 KB");
        assert_eq!(file_size(12), "12 B");
    }
}
