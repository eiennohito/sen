//! Rich text uses native measured runs; the original body remains the copy source.
use crate::*;

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
    if rows.len() > 2048 {
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
            if rows.len() > 2048 {
                return;
            }
        }
        start = end;
    }
}

fn url_break(left: char, right: char) -> bool {
    !("/:.?&=-_#".contains(left) && "/:.?&=-_#".contains(right))
}

pub(crate) fn decorate_links(row: &mut MessageRow) {
    use std::collections::BTreeSet;
    row.has_inline_emoji = row.rich_runs.iter().any(|run| run.emoji);
    if row.links.row_count() == 0 || row.rich_fallback {
        return;
    }
    let mut spans = Vec::new();
    let mut offset = 0;
    for (index, link) in row.links.iter().enumerate() {
        if let Some(start) = row.body[offset..].find(link.label.as_str()) {
            let start = offset + start;
            offset = start + link.label.len();
            spans.push((start, offset, link.url, index as i32 + 1));
        }
    }
    let body = row.body.as_str();
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

const METRIC_ENTRIES: usize = 4096;
const METRIC_TEXT_BYTES: usize = 1024 * 1024;
type MetricKey = (slint::SharedString, u32);

#[derive(Default)]
struct Measurements {
    values: std::collections::HashMap<MetricKey, ((f32, f32), u64)>,
    recent: std::collections::BTreeSet<(u64, MetricKey)>,
    text_bytes: usize,
    clock: u64,
}
impl Measurements {
    fn get(&mut self, key: &MetricKey) -> Option<(f32, f32)> {
        let (value, stamp) = self.values.get_mut(key)?;
        self.recent.remove(&(*stamp, key.clone()));
        self.clock += 1;
        *stamp = self.clock;
        self.recent.insert((*stamp, key.clone()));
        Some(*value)
    }
    fn insert(&mut self, key: MetricKey, value: (f32, f32)) {
        if key.0.len() > METRIC_TEXT_BYTES {
            return;
        }
        while self.values.len() >= METRIC_ENTRIES
            || self.text_bytes + key.0.len() > METRIC_TEXT_BYTES
        {
            let (_, oldest) = self.recent.pop_first().unwrap();
            self.text_bytes -= oldest.0.len();
            self.values.remove(&oldest);
        }
        self.clock += 1;
        self.text_bytes += key.0.len();
        self.recent.insert((self.clock, key.clone()));
        self.values.insert(key, (value, self.clock));
    }
}

fn append(line: &mut Vec<RichRun>, runs: &[RichRun]) {
    for run in runs {
        if let Some(last) = line.last_mut()
            && !last.emoji
            && !run.emoji
            && last.link_id == run.link_id
        {
            last.text = format!("{}{}", last.text, run.text).into();
            last.measured_width += run.measured_width;
            last.measured_height = last.measured_height.max(run.measured_height);
        } else {
            line.push(run.clone());
        }
    }
}

fn slice(runs: &[RichRun], start: usize, end: usize) -> Vec<RichRun> {
    let mut offset = 0;
    let mut result = Vec::new();
    for run in runs {
        let next = offset + run.text.len();
        if start < next && offset < end {
            let mut part = run.clone();
            part.text =
                run.text[start.saturating_sub(offset)..(end - offset).min(run.text.len())].into();
            result.push(part);
        }
        offset = next;
        if offset >= end {
            break;
        }
    }
    result
}

fn fallback_break(left: char, right: char) -> bool {
    use unicode_linebreak::{BreakClass::*, break_property};
    !matches!(
        break_property(right as u32),
        ClosePunctuation | CloseParenthesis | Exclamation | InfixSeparator | NonStarter
    ) && break_property(left as u32) != OpenPunctuation
        && url_break(left, right)
}

fn layout(
    ui: &AppWindow,
    rows: &ModelRc<RichRun>,
    available: f32,
    size: f32,
    metrics: &RefCell<Measurements>,
) -> ModelRc<RichRun> {
    if available < 1.0 {
        return ModelRc::default();
    }
    let measure = |run: &RichRun| {
        if run.emoji {
            return (size * 1.4, size * 1.5);
        }
        if run.text.is_empty() || run.newline {
            return (0., size * 1.5);
        }
        let key = (run.text.clone(), size.to_bits());
        if let Some(measured) = metrics.borrow_mut().get(&key) {
            return measured;
        }
        let measured = ui.invoke_measure_rich(run.text.clone(), size);
        let measured = (measured.width, measured.height.max(size * 1.5));
        metrics.borrow_mut().insert(key, measured);
        measured
    };
    let whitespace = |text: &str| {
        measure(&RichRun {
            text: text.into(),
            ..Default::default()
        })
        .0
    };
    let width = |runs: &[RichRun]| {
        runs.iter().map(|r| r.measured_width).sum::<f32>()
            - runs
                .last()
                .filter(|r| !r.emoji)
                .map_or(0., |r| whitespace(&r.text[r.text.trim_end().len()..]))
    };
    let shaped = |line: &[RichRun], part: &[RichRun]| {
        let mut candidate = line.to_vec();
        append(&mut candidate, part);
        for run in &mut candidate {
            (run.measured_width, run.measured_height) = measure(run);
        }
        candidate
    };
    let source: Vec<_> = rows.iter().collect();
    let mut groups: Vec<Vec<RichRun>> = Vec::new();
    let (mut natural_width, mut unwrapped) = (0.0_f32, 0.0_f32);
    for mut run in source {
        (run.measured_width, run.measured_height) = measure(&run);
        if run.newline {
            natural_width = natural_width.max(unwrapped);
            unwrapped = 0.;
        } else {
            unwrapped += run.measured_width;
        }
        if groups.is_empty()
            || run.break_before
            || run.newline
            || groups.last().unwrap().last().unwrap().newline
        {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(run);
    }
    natural_width = natural_width.max(unwrapped);
    let mut output = Vec::new();
    let mut line: Vec<RichRun> = Vec::new();
    let mut y = 0.;
    let mut first_links = std::collections::HashSet::new();
    let mut finish = |line: &mut Vec<RichRun>| {
        while let Some(first) = line.first_mut() {
            if first.emoji {
                break;
            }
            let trimmed = first.text.trim_start();
            first.measured_width = (first.measured_width
                - whitespace(&first.text[..first.text.len() - trimmed.len()]))
            .max(0.);
            first.text = trimmed.into();
            if !first.text.is_empty() {
                break;
            }
            line.remove(0);
        }
        while let Some(last) = line.last_mut() {
            if last.emoji {
                break;
            }
            let trimmed = last.text.trim_end();
            last.measured_width =
                (last.measured_width - whitespace(&last.text[trimmed.len()..])).max(0.);
            last.text = trimmed.into();
            if !last.text.is_empty() {
                break;
            }
            line.pop();
        }
        if line.is_empty() {
            line.push(RichRun {
                newline: true,
                ..Default::default()
            });
        }
        let height = line
            .iter()
            .map(|r| r.measured_height)
            .fold(size * 1.5, f32::max);
        let mut x = 0.;
        for mut run in line.drain(..) {
            if output.len() == 2048 {
                return false;
            }
            run.measured_width = run.measured_width.min((available - x).max(0.));
            run.measured_height = height;
            run.available = available;
            run.x = x;
            run.y = y;
            run.positioned = true;
            run.link_start = run.link_id != 0;
            run.link_first = run.link_start && first_links.insert(run.link_id);
            run.link_width = if run.link_start {
                run.measured_width
            } else {
                0.
            };
            x += run.measured_width;
            output.push(run);
        }
        y += height;
        true
    };
    for group in groups {
        if group[0].newline {
            if !finish(&mut line) {
                return ModelRc::default();
            }
            continue;
        }
        if line.iter().map(|r| r.measured_width).sum::<f32>() + width(&group) <= available {
            append(&mut line, &group);
            continue;
        }
        if width(&group) <= available {
            if !finish(&mut line) {
                return ModelRc::default();
            }
            append(&mut line, &group);
            continue;
        }
        let body: String = group.iter().map(|r| r.text.as_str()).collect();
        use unicode_segmentation::UnicodeSegmentation;
        let mut boundaries = Vec::new();
        for (i, _) in body.grapheme_indices(true).skip(1) {
            if fallback_break(
                body[..i].chars().next_back().unwrap(),
                body[i..].chars().next().unwrap(),
            ) {
                boundaries.push(i);
            }
        }
        boundaries.push(body.len());
        let mut start = 0;
        while start < body.len() {
            let choices = boundaries.partition_point(|&end| end <= start);
            let mut lo = choices;
            let mut hi = boundaries.len();
            while lo < hi {
                let mid = (lo + hi) / 2;
                let candidate = shaped(&line, &slice(&group, start, boundaries[mid]));
                if width(&candidate) <= available {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            if lo == choices && !line.is_empty() {
                if !finish(&mut line) {
                    return ModelRc::default();
                }
                continue;
            }
            let end = boundaries[lo.saturating_sub(1).max(choices)];
            line = shaped(&line, &slice(&group, start, end));
            start = end;
            if start < body.len() && !finish(&mut line) {
                return ModelRc::default();
            }
        }
    }
    if !finish(&mut line) {
        return ModelRc::default();
    }
    if output.len() > 2048 {
        return ModelRc::default();
    }
    if let Some(first) = output.first_mut() {
        first.natural_width = natural_width;
    }
    ModelRc::new(VecModel::from(output))
}
fn reuse(previous: ModelRc<RichRun>, next: ModelRc<RichRun>) -> ModelRc<RichRun> {
    let Some(model) = previous.as_any().downcast_ref::<VecModel<RichRun>>() else {
        return next;
    };
    for index in 0..model.row_count().min(next.row_count()) {
        if model.row_data(index) != next.row_data(index) {
            model.set_row_data(index, next.row_data(index).unwrap());
        }
    }
    while model.row_count() > next.row_count() {
        model.remove(model.row_count() - 1);
    }
    for index in model.row_count()..next.row_count() {
        model.push(next.row_data(index).unwrap());
    }
    previous
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
    ui.global::<State>().on_rich_source(|rows| {
        rows.model_tracker().track_row_count_changes();
        for i in 0..rows.row_count() {
            rows.model_tracker().track_row_data_changes(i);
        }
        // A new observer handle triggers reflow without copying the rows.
        ModelRc::new(rows)
    });
    let metrics = RefCell::new(Measurements::default());
    let weak = ui.as_weak();
    ui.global::<State>()
        .on_rich_layout(move |rows, width, size, previous| {
            weak.upgrade()
                .map(|ui| reuse(previous, layout(&ui, &rows, width, size, &metrics)))
                .unwrap_or_default()
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
    slint::slint! {
        import { TextMetrics } from "../ui/text-metrics.slint";
        export component MetricsProbe inherits Window {
            in-out property <string> sample;
            in-out property <float> size: 15;
            in-out property <float> measured-width;
            out property <float> plain-width: plain.preferred-width / 1px;
            background: #ffffff;
            metrics := TextMetrics {}
            public function measure() { root.measured-width = metrics.measure(root.sample, root.size).width; }
            public function measure-part(content: string) -> float { return metrics.measure(content, root.size).width; }
            plain := Text { x: 20px; y: 20px; width: parent.width - 40px; height: 40px; text: root.sample; font-size: root.size * 1px; color: #008c52; wrap: no-wrap; }
            Text { x: 20px; y: 100px; width: root.measured-width * 1px; height: 40px; text: root.sample; font-size: root.size * 1px; color: #008c52; wrap: no-wrap; overflow: clip; }
        }
    }

    use slint::SharedString;

    fn layout(ui: &AppWindow, rows: &ModelRc<RichRun>, width: f32, size: f32) -> ModelRc<RichRun> {
        super::layout(
            ui,
            rows,
            width,
            size,
            &RefCell::new(Measurements::default()),
        )
    }

    fn watch_layout(ui: &AppWindow) -> Rc<RefCell<ModelRc<RichRun>>> {
        let observed = Rc::new(RefCell::new(ModelRc::default()));
        let recorded = observed.clone();
        let weak = ui.as_weak();
        let metrics = RefCell::new(Measurements::default());
        ui.global::<State>()
            .on_rich_layout(move |rows, width, size, previous| {
                let output = reuse(
                    previous,
                    super::layout(&weak.unwrap(), &rows, width, size, &metrics),
                );
                *recorded.borrow_mut() = output.clone();
                output
            });
        observed
    }

    #[test]
    fn measurement_lru_bounds_text_and_entries_and_separates_font_sizes() {
        let mut cache = Measurements::default();
        for i in 0..METRIC_ENTRIES {
            cache.insert(
                (format!("word{i}").into(), 15_f32.to_bits()),
                (i as f32, 20.),
            );
        }
        let oldest = ("word0".into(), 15_f32.to_bits());
        assert_eq!(cache.get(&oldest), Some((0., 20.)));
        cache.insert(("new".into(), 15_f32.to_bits()), (1., 20.));
        assert!(cache.get(&oldest).is_some());
        assert!(cache.get(&("word1".into(), 15_f32.to_bits())).is_none());
        assert!(cache.get(&("word0".into(), 18_f32.to_bits())).is_none());
        assert_eq!(cache.values.len(), METRIC_ENTRIES);
        assert_eq!(cache.recent.len(), METRIC_ENTRIES);
        cache.insert(
            ("x".repeat(METRIC_TEXT_BYTES).into(), 15_f32.to_bits()),
            (1., 20.),
        );
        assert_eq!(cache.text_bytes, METRIC_TEXT_BYTES);
        assert_eq!(cache.values.len(), 1);
        cache.insert(
            (
                "too large".repeat(METRIC_TEXT_BYTES).into(),
                15_f32.to_bits(),
            ),
            (1., 20.),
        );
        assert_eq!(cache.text_bytes, METRIC_TEXT_BYTES);
        assert_eq!(cache.values.len(), 1);
    }

    #[test]
    fn public_scaled_metrics_match_painted_segments() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let probe = MetricsProbe::new().unwrap();
            probe.show().unwrap();
            let mut max_rounding = 0.0_f32;
            let mut max_ink = 0;
            let mut max_sum = 0.0_f32;
            let mut blank = 0;
            for sample in [
                "AVATAR office ffi", "To Wa fi fl ffi", "affinity waffle", "https://www.example.jp",
                "日本語の文字列", "漢字かなカナ。", "参照「リンク」、次は", "👩🏽‍💻", "👨‍👩‍👧‍👦", "🏳️‍🌈", "A👩🏽‍💻Z", "A👨‍👩‍👧‍👦Z", "A🏳️‍🌈Z",
                "한글 메시지", "한국어 테스트", "ภาษาไทยทดสอบ", "ข้อความภาษาไทย",
            ] {
                probe.set_sample(sample.into());
                let mut row = MessageRow { body: sample.into(), ..Default::default() };
                links::decorate_message(&mut row, "c");
                let pieces = if row.rich_runs.row_count() > 0 { row.rich_runs } else { runs(sample, &[]) };
                for size in [12., 15., 18., 22., 24.] {
                    probe.set_size(size);
                    probe.invoke_measure();
                    let measured = probe.get_measured_width();
                    let rounded = probe.get_plain_width();
                    let error = rounded - measured;
                    assert!((-0.001..=1.0).contains(&error), "{sample} at {size}: measured {measured}, public width {rounded}");
                    max_rounding = max_rounding.max(error);
                    let summed = pieces.iter().map(|run| probe.invoke_measure_part(run.text)).sum::<f32>();
                    let delta = (summed - measured).abs();
                    assert!(delta <= 1., "{sample} at {size}: summed widths differ by {delta}px");
                    max_sum = max_sum.max(delta);
                    probe.set_measured_width(summed);
                    let pixels = render(&window, (980, 180));
                    let ink = |top| {
                        let xs: Vec<_> = pixels.enumerate_pixels().filter(|(_, y, p)| *y >= top && *y < top + 40 && p.0 != [255, 255, 255]).map(|(x, _, _)| x).collect();
                        xs.iter().min().zip(xs.iter().max()).map(|(left, right)| (*left, *right))
                    };
                    let plain = ink(20);
                    let clipped = ink(100);
                    if plain.is_none() && clipped.is_none() { blank += 1; continue; }
                    let (plain_left, plain_right) = plain.expect("native reference did not paint");
                    let (clipped_left, clipped_right) = clipped.expect("measured segment lost all painted ink");
                    let delta = (plain_right - plain_left).abs_diff(clipped_right - clipped_left);
                    assert!(delta <= 1, "{sample} at {size}: measured segment clipped painted ink by {delta}px");
                    max_ink = max_ink.max(delta);
                }
            }
            eprintln!("public64 metrics: max preferred-width rounding {max_rounding}px; max summed-width difference {max_sum}px; max painted ink difference {max_ink}px; {blank} native blank samples (17 strings × 5 sizes)");
            probe.hide().unwrap();
        }).join().unwrap();
    }

    #[test]
    fn layout_failure_keeps_the_native_body_selectable() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            install(&ui, Rc::new(RefCell::new(View::default())));
            let mut row = MessageRow {
                body: "Before https://example.org/a after".into(),
                outgoing: true,
                ..Default::default()
            };
            links::decorate_message(&mut row, "contact");
            assert_eq!(row.links.row_count(), 1);
            let oversized = ModelRc::new(VecModel::from(vec![RichRun {
                text: "a".repeat(2050).into(),
                ..Default::default()
            }]));
            assert_eq!(layout(&ui, &oversized, 1., 24.).row_count(), 0);
            ui.global::<State>()
                .on_rich_layout(|_, _, _, _| ModelRc::default());
            ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.show().unwrap();
            let pixels = render(&window, (980, 720));
            let (x, y, _) = pixels
                .enumerate_pixels()
                .find(|(_, _, p)| p.0 == [24, 41, 30])
                .expect("fallback body did not paint");
            let position = slint::LogicalPosition::new(x as f32, y as f32);
            for pressed in [true, false] {
                ui.window().dispatch_event(if pressed {
                    slint::platform::WindowEvent::PointerPressed {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    }
                } else {
                    slint::platform::WindowEvent::PointerReleased {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    }
                });
            }
            for (text, pressed) in [
                ("\u{11}", true),
                ("a", true),
                ("a", false),
                ("\u{11}", false),
            ] {
                ui.window().dispatch_event(if pressed {
                    slint::platform::WindowEvent::KeyPressed { text: text.into() }
                } else {
                    slint::platform::WindowEvent::KeyReleased { text: text.into() }
                });
            }
            let selected = render(&window, (980, 720));
            assert!(
                selected.pixels().filter(|p| p.0 == [36, 89, 166]).count() > 100,
                "fallback did not expose native selection"
            );
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn break_tokens_preserve_fallback_and_multiline_copy_source() {
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
        assert_eq!(runs(&"日".repeat(2049), &[]).row_count(), 0);
    }
    #[test]
    fn maximum_rich_flow_measures_and_reflows_at_narrow_large_text() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let state = Rc::new(RefCell::new(View::default()));
            install(&ui, state);
            let observed = watch_layout(&ui);
            let letter = |i: u8| char::from(if i < 26 { b'a' + i } else { b'A' + i - 26 });
            let words = (0..2047)
                .map(|i| format!("{}{} ", letter((i / 52) as u8), letter((i % 52) as u8)))
                .collect::<String>();
            let start = words.len();
            let body = format!("{words}[笑]");
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
            render(&window, (980, 720));
            let cold = started.elapsed();
            let before = observed.borrow().iter().last().unwrap().y;
            let resized = std::time::Instant::now();
            assert!(before > 0.0, "native run measurements did not initialize");
            ui.set_text_size(24);
            render(&window, (360, 520));
            let last = observed.borrow().iter().last().unwrap();
            assert!(last.y > before, "narrow large text did not reflow");
            for run in observed.borrow().iter() {
                assert!(
                    run.x + run.measured_width <= run.available + 1.0,
                    "rich run overflowed the pane"
                );
            }
            eprintln!(
                "synthetic2048run cold: {cold:?}; resize: {:?}",
                resized.elapsed()
            );
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }
    fn render(
        window: &slint::platform::software_renderer::MinimalSoftwareWindow,
        size: (u32, u32),
    ) -> image::RgbImage {
        use slint::platform::WindowAdapter;
        window.set_size(slint::PhysicalSize::new(size.0, size.1));
        window.window().request_redraw();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.0, size.1);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            window.draw_if_needed(|renderer| {
                renderer.render(pixels.make_mut_slice(), size.0 as usize);
            });
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        image::RgbImage::from_raw(size.0, size.1, pixels.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn unbroken_bodies_and_inline_links_stay_inside_narrow_bubbles() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            install(&ui, Rc::new(RefCell::new(View::default())));
            let observed = watch_layout(&ui);
            let opened = Rc::new(RefCell::new(Vec::new()));
            let recorded = opened.clone();
            ui.global::<State>()
                .on_open_link(move |url| recorded.borrow_mut().push(url));
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.show().unwrap();
            for size in [(980, 720), (360, 520)] {
                for text_size in [15, 24] {
                    ui.set_text_size(text_size);
                    for linked in [false, true] {
                        let mut row = MessageRow {
                            id: "long-run".into(),
                            body: if linked {
                                format!(
                                    "Before https://example.org/{} after",
                                    "abcdefghij".repeat(8)
                                )
                            } else {
                                "abcdefghij".repeat(10)
                            }
                            .into(),
                            outgoing: true,
                            ..Default::default()
                        };
                        links::decorate_message(&mut row, "contact");
                        ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
                        let pixels = render(&window, size);
                        let runs = observed.borrow().clone();
                        if linked {
                            for run in runs.iter() {
                                assert!(run.measured_height > 0.0);
                                assert!(
                                    run.x + run.measured_width <= run.available + 0.25,
                                    "inline URL overflowed"
                                );
                                if run.link_start {
                                    assert!(run.x + run.link_width <= run.available + 0.25);
                                }
                            }
                            assert!(runs.row_count() < 20);
                            assert_eq!(runs.iter().filter(|r| r.link_first).count(), 1);
                        }
                        let green: Vec<_> = pixels
                            .enumerate_pixels()
                            .filter(|(_, _, p)| p.0 == [216, 243, 220])
                            .map(|(x, y, _)| (x, y))
                            .collect();
                        assert!(!green.is_empty());
                        let left = green.iter().map(|p| p.0).min().unwrap();
                        let right = green.iter().map(|p| p.0).max().unwrap();
                        if linked && size.0 == 980 && text_size == 24 {
                            let link = runs.iter().find(|r| r.link_start).unwrap();
                            let top = green.iter().map(|p| p.1).min().unwrap();
                            let position = slint::LogicalPosition::new(
                                left as f32 + 9.0 + link.x + link.link_width * 0.8,
                                top as f32 + 9.0 + link.y + link.measured_height / 2.0,
                            );
                            ui.window().dispatch_event(
                                slint::platform::WindowEvent::PointerPressed {
                                    position,
                                    button: slint::platform::PointerEventButton::Left,
                                },
                            );
                            ui.window().dispatch_event(
                                slint::platform::WindowEvent::PointerReleased {
                                    position,
                                    button: slint::platform::PointerEventButton::Left,
                                },
                            );
                            assert_eq!(
                                opened.borrow().last(),
                                Some(&link.url),
                                "inline link hit area did not cover its text"
                            );
                        }
                        let ink: Vec<_> = pixels
                            .enumerate_pixels()
                            .filter(|(_, _, p)| p.0 == [24, 41, 30])
                            .map(|(x, y, _)| (x, y))
                            .collect();
                        assert!(!ink.is_empty());
                        assert!(
                            ink.iter().all(|p| p.0 > left && p.0 < right),
                            "body painted outside the bubble"
                        );
                        let top = green.iter().map(|p| p.1).min().unwrap();
                        let bottom = green.iter().map(|p| p.1).max().unwrap();
                        let mut lines: Vec<_> = ink.iter().map(|p| p.1).collect();
                        if linked {
                            lines.extend(
                                pixels
                                    .enumerate_pixels()
                                    .filter(|(x, y, p)| {
                                        *x > left
                                            && *x < right
                                            && *y >= top
                                            && *y <= bottom
                                            && p.0 == [0, 95, 184]
                                    })
                                    .map(|(_, y, _)| y),
                            );
                        }
                        assert!(
                            lines.iter().max().unwrap() - lines.iter().min().unwrap()
                                > text_size as u32,
                            "long run did not wrap: {size:?} font {text_size} linked {linked}; layout {:?}", runs.iter().map(|r| (r.text, r.x, r.y, r.measured_width, r.available)).collect::<Vec<_>>()
                        );
                    }
                }
            }
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn emoji_click_keeps_images() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            install(&ui, Rc::new(RefCell::new(View::default())));
            let mut row = demo::rich_fixture().remove(1);
            row.reply_id = "".into();
            let runs = row.rich_runs.clone();
            let model = runs.as_any().downcast_ref::<VecModel<RichRun>>().unwrap();
            let emoji_index = runs.iter().position(|r| r.emoji).unwrap();
            let mut emoji = model.row_data(emoji_index).unwrap();
            let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(8, 8);
            buffer.make_mut_slice().fill(slint::Rgb8Pixel {
                r: 227,
                g: 0,
                b: 113,
            });
            emoji.picture = slint::Image::from_rgb8(buffer.clone());
            model.set_row_data(emoji_index, emoji);
            ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.show().unwrap();
            for size in [(980, 720), (360, 520)] {
                for text_size in [15, 24] {
                    ui.set_text_size(text_size);
                    let before = render(&window, size);
                    let red: Vec<_> = before
                        .enumerate_pixels()
                        .filter(|(_, _, p)| p.0 == [227, 0, 113])
                        .map(|(x, y, _)| (x, y))
                        .collect();
                    assert!(!red.is_empty(), "emoji did not paint");
                    let position = slint::LogicalPosition::new(
                        red[red.len() / 2].0 as f32,
                        red[red.len() / 2].1 as f32,
                    );
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    let after = render(&window, size);
                    assert_eq!(
                        after.pixels().filter(|p| p.0 == [227, 0, 113]).count(),
                        red.len(),
                        "single click entered emoji fallback mode"
                    );
                }
            }
            let mut updated = model.row_data(emoji_index).unwrap();
            buffer.make_mut_slice().fill(slint::Rgb8Pixel {
                r: 111,
                g: 0,
                b: 227,
            });
            updated.picture = slint::Image::from_rgb8(buffer);
            model.set_row_data(emoji_index, updated);
            let updated = render(&window, (360, 520));
            assert!(
                updated.pixels().any(|p| p.0 == [111, 0, 227]),
                "asset update did not repaint the measured flow"
            );
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn outgoing_selection_contrasts_in_both_color_schemes() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                id: "selection".into(),
                body: "Select this message".into(),
                outgoing: true,
                ..Default::default()
            }])));
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.set_text_size(24);
            ui.show().unwrap();
            for (theme, scheme, ink, selection) in [
                (
                    "light",
                    slint::language::ColorScheme::Light,
                    [24, 41, 30],
                    [36, 89, 166],
                ),
                (
                    "dark",
                    slint::language::ColorScheme::Dark,
                    [228, 246, 233],
                    [168, 201, 255],
                ),
            ] {
                ui.global::<Palette>().set_color_scheme(scheme);
                for size in [(980, 720), (360, 520)] {
                    ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                        id: format!("selection-{theme}-{}", size.0).into(),
                        body: "Select this message".into(),
                        outgoing: true,
                        ..Default::default()
                    }])));
                    ui.invoke_focus_composer();
                    let before = render(&window, size);
                    let (x, y, _) = before
                        .enumerate_pixels()
                        .find(|(_, _, p)| p.0 == ink)
                        .expect("outgoing text did not paint");
                    let position = slint::LogicalPosition::new(x as f32, y as f32);
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    for (text, pressed) in [
                        ("\u{11}", true),
                        ("a", true),
                        ("a", false),
                        ("\u{11}", false),
                    ] {
                        ui.window().dispatch_event(if pressed {
                            slint::platform::WindowEvent::KeyPressed { text: text.into() }
                        } else {
                            slint::platform::WindowEvent::KeyReleased { text: text.into() }
                        });
                    }
                    let selected = render(&window, size);
                    assert!(
                        selected.pixels().filter(|p| p.0 == selection).count() > 100,
                        "outgoing selection was not visible in {theme}"
                    );
                }
            }
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn inline_links_replace_matching_text_without_changing_the_body() {
        let mut row = MessageRow {
            body: "参照「https://example.org/a」。again https://example.org/a and www.example.jp"
                .into(),
            ..Default::default()
        };
        links::decorate_message(&mut row, "contact");
        assert_eq!(
            row.rich_runs
                .iter()
                .map(|r| r.text.to_string())
                .collect::<String>(),
            row.body.as_str()
        );
        assert_eq!(row.links.row_count(), 3);
        for link in row.links.iter() {
            assert!(row.rich_runs.iter().any(|r| r.url == link.url));
        }
    }

    #[test]
    fn failure_marks_stay_beside_the_bubble() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            ui.set_demo(true);
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.show().unwrap();
            for size in [(980, 720), (360, 520)] {
                for text_size in [15, 24] {
                    ui.set_text_size(text_size);
                    for delivery in ["Not sent", "Unconfirmed"] {
                        ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                            id: "failed".into(),
                            body: "Meet at eleven.".into(),
                            outgoing: true,
                            recoverable: true,
                            delivery: delivery.into(),
                            time: "12:01".into(),
                            clock: "12:01".into(),
                            ..Default::default()
                        }])));
                        ui.invoke_show_history(0., false);
                        let pixels = render(&window, size);
                        let green: Vec<_> = pixels
                            .enumerate_pixels()
                            .filter(|(_, _, p)| p.0 == [216, 243, 220])
                            .map(|(x, y, _)| (x, y))
                            .collect();
                        let right = green.iter().map(|p| p.0).max().unwrap();
                        let top = green.iter().map(|p| p.1).min().unwrap();
                        let bottom = green.iter().map(|p| p.1).max().unwrap();
                        assert!(
                            pixels.enumerate_pixels().any(|(x, y, p)| x > right
                                && y >= top
                                && y <= bottom
                                && p.0 == [148, 56, 36]),
                            "failure mark moved below the bubble"
                        );
                    }
                }
            }
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn coarse_runs_keep_words_and_cjk_break_opportunities() {
        let body = "Before https://example.org/a?b=c after words 日本語 https://www.example.jp";
        let mut row = MessageRow {
            body: body.into(),
            ..Default::default()
        };
        links::decorate_message(&mut row, "contact");
        let rows: Vec<_> = row.rich_runs.iter().collect();
        assert!(rows.len() < 25);
        assert!(rows.iter().any(|r| r.text == "after "));
        assert!(rows.iter().any(|r| r.text == "words "));
        assert!(rows.iter().any(|r| r.text == "日"));
        assert!(rows.iter().any(|r| r.text == "example."));
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<String>(),
            body
        );
    }

    #[test]
    fn link_edges_preserve_closing_punctuation_at_every_width() {
        std::thread::spawn(|| {
            let _window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            for body in [
                "参照「https://example.org/a」。次は",
                "参照 https://example.org/a、次は",
                "参照 https://example.org/a」次は",
            ] {
                let mut row = MessageRow {
                    body: body.into(),
                    ..Default::default()
                };
                links::decorate_message(&mut row, "contact");
                assert!(
                    row.rich_runs
                        .iter()
                        .filter(|r| r.text.starts_with(['」', '。', '、']))
                        .all(|r| !r.break_before)
                );
                assert!(row.rich_runs.iter().any(|r| r.text == "https://"));
                for size in [15., 24.] {
                    for width in 1..=600 {
                        let lines = layout(&ui, &row.rich_runs, width as f32, size);
                        assert!(lines.row_count() > 0);
                        let mut previous_y = -1.;
                        for run in lines.iter() {
                            if run.y != previous_y {
                                assert!(
                                    !run.text.starts_with(['」', '。', '、']),
                                    "closing punctuation at width {width}, size {size}: {}",
                                    run.text
                                );
                                assert!(!run.text.starts_with(char::is_whitespace));
                            }
                            previous_y = run.y;
                            assert!(run.x + run.measured_width <= width as f32 + 0.01);
                        }
                    }
                }
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn oversized_links_share_the_last_line_and_measure_whole_segments() {
        std::thread::spawn(|| {
            let _window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let mut row = MessageRow {
                body: format!("https://example.org/{} after", "abcdefghij".repeat(8)).into(),
                ..Default::default()
            };
            links::decorate_message(&mut row, "contact");
            let lines = layout(&ui, &row.rich_runs, 260., 24.);
            let links: Vec<_> = lines.iter().filter(|r| r.link_start).collect();
            assert!(links.len() > 3);
            assert_eq!(links.iter().filter(|r| r.link_first).count(), 1);
            assert!(
                links
                    .iter()
                    .all(|r| r.link_width == r.measured_width && r.measured_height == 36.)
            );
            let last = links.last().unwrap();
            let after = lines.iter().find(|r| r.text.contains("after")).unwrap();
            assert_eq!(
                last.y, after.y,
                "following text did not share the final link line"
            );
            assert!(after.x > 0.);
            let mut rounded = row.clone();
            rounded.body = "https://a.b.c.d.e.f.g.h.i.j.k.l/m?n=o&p=q。".into();
            links::decorate_message(&mut rounded, "contact");
            let segments = layout(&ui, &rounded.rich_runs, 1000., 24.);
            let link = segments.iter().find(|r| r.link_start).unwrap();
            let plain = layout(
                &ui,
                &ModelRc::new(VecModel::from(vec![RichRun {
                    text: link.text.clone(),
                    ..Default::default()
                }])),
                1000.,
                24.,
            );
            assert!((link.measured_width - plain.row_data(0).unwrap().measured_width).abs() <= 1.);
            let punctuation = segments.iter().find(|r| r.text == "。").unwrap();
            assert_eq!(punctuation.x, link.x + link.link_width);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn pathological_link_bodies_keep_the_original_and_menu_links() {
        let body = format!("https://example.org/{}", "a/".repeat(2049));
        let mut row = MessageRow {
            body: body.clone().into(),
            ..Default::default()
        };
        links::decorate_message(&mut row, "contact");
        assert_eq!(row.rich_runs.row_count(), 0);
        assert!(row.rich_fallback);
        assert_eq!(row.body, body);
        assert_eq!(row.links.row_count(), 1);
        links::decorate_message(&mut row, "contact");
        assert_eq!(row.rich_runs.row_count(), 0);
        let mut row = MessageRow {
            body: "https://example.org/".into(),
            rich_fallback: true,
            ..Default::default()
        };
        links::decorate_message(&mut row, "contact");
        assert_eq!(
            row.rich_runs.row_count(),
            0,
            "emoji fallback was rebuilt as an uncapped link flow"
        );
        assert_eq!(row.links.row_count(), 1);
    }

    #[test]
    fn shared_link_strings_are_budgeted_once_per_message() {
        let url: SharedString = "https://example.org/a".into();
        let mut row = MessageRow {
            rich_runs: ModelRc::new(VecModel::from(vec![
                RichRun {
                    url: url.clone(),
                    ..Default::default()
                };
                2
            ])),
            links: ModelRc::new(VecModel::from(vec![LinkRow {
                url,
                ..Default::default()
            }])),
            ..Default::default()
        };
        let before = paging::bytes(&row);
        let mut runs: Vec<_> = row.rich_runs.iter().collect();
        runs[1].url = "https://example.org/b".into();
        row.rich_runs = ModelRc::new(VecModel::from(runs));
        assert_eq!(paging::bytes(&row) - before, "https://example.org/b".len());
    }

    #[test]
    fn first_frame_never_paints_unpositioned_or_overlapping_runs() {
        for size in [(980, 720, 15), (360, 520, 24)] {
            for emoji in [false, true] {
                std::thread::spawn(move || {
                    use slint::platform::WindowAdapter;
                    let window = snapshot::install().unwrap();
                    window.set_size(slint::PhysicalSize::new(size.0, size.1));
                    let ui = create_app_window().unwrap();
                    install(&ui, Rc::new(RefCell::new(View::default())));
                    ui.set_has_chat(true);
                    ui.set_show_conversation(true);
                    ui.set_text_size(size.2);
                    let mut row = if emoji {
                        demo::rich_fixture().remove(1)
                    } else {
                        MessageRow {
                            body: "Before https://example.org/a after words".into(),
                            ..Default::default()
                        }
                    };
                    row.outgoing = true;
                    row.reply_id = "".into();
                    links::decorate_message(&mut row, "contact");
                    let observed = watch_layout(&ui);
                    ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
                    ui.show().unwrap();
                    let mut first_ink = Vec::new();
                    for frame in 0..2 {
                        if frame > 0 {
                            slint::platform::update_timers_and_animations();
                        }
                        window.window().request_redraw();
                        let mut pixels =
                            slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.0, size.1);
                        window.draw_if_needed(|renderer| {
                            renderer.render(pixels.make_mut_slice(), size.0 as usize);
                        });
                        let ink: Vec<_> = pixels
                            .as_slice()
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| p.r == 24 && p.g == 41 && p.b == 30)
                            .map(|(i, _)| i)
                            .collect();
                        assert!(!ink.is_empty(), "first frame has no message ink");
                        if frame == 0 {
                            first_ink = ink;
                        } else {
                            assert_eq!(ink, first_ink, "message ink moved after the first frame");
                        }
                        let rows: Vec<_> = observed.borrow().iter().collect();
                        let positioned = !rows.is_empty()
                            && rows.iter().all(|r| r.positioned && r.measured_height > 0.0);
                        if !positioned {
                            assert!(
                                !pixels
                                    .as_slice()
                                    .iter()
                                    .any(|p| p.r == 24 && p.g == 41 && p.b == 30),
                                "first frame painted stacked runs"
                            );
                        } else {
                            for (i, a) in rows.iter().enumerate().filter(|(_, r)| !r.newline) {
                                for b in rows.iter().skip(i + 1).filter(|r| !r.newline) {
                                    assert!(
                                        a.x + a.measured_width <= b.x + 0.25
                                            || b.x + b.measured_width <= a.x + 0.25
                                            || a.y + a.measured_height <= b.y + 0.25
                                            || b.y + b.measured_height <= a.y + 0.25,
                                        "first frame contains overlapping runs"
                                    );
                                }
                            }
                        }
                        eprintln!(
                            "{}px emoji={emoji} frame {frame}: positioned={positioned}",
                            size.0
                        );
                        if !emoji || frame == 1 {
                            assert!(positioned, "layout added another unpositioned frame");
                        }
                    }
                    ui.hide().unwrap();
                })
                .join()
                .unwrap();
            }
        }
    }

    #[test]
    fn metadata_labels_are_explicit() {
        assert_eq!(duration(125000), "2:05");
        assert_eq!(file_size(1024), "1.0 KB");
        assert_eq!(file_size(12), "12 B");
    }
}
