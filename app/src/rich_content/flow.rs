use super::links::url_break;
use crate::*;

pub(crate) const METRIC_ENTRIES: usize = 4096;
pub(crate) const METRIC_TEXT_BYTES: usize = 1024 * 1024;
type MetricKey = (slint::SharedString, u32);

#[derive(Default)]
pub(crate) struct Measurements {
    pub(crate) values: std::collections::HashMap<MetricKey, ((f32, f32), u64)>,
    pub(crate) recent: std::collections::BTreeSet<(u64, MetricKey)>,
    pub(crate) text_bytes: usize,
    clock: u64,
}
impl Measurements {
    pub(crate) fn get(&mut self, key: &MetricKey) -> Option<(f32, f32)> {
        let (value, stamp) = self.values.get_mut(key)?;
        self.recent.remove(&(*stamp, key.clone()));
        self.clock += 1;
        *stamp = self.clock;
        self.recent.insert((*stamp, key.clone()));
        Some(*value)
    }
    pub(crate) fn insert(&mut self, key: MetricKey, value: (f32, f32)) {
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

pub(crate) fn layout(
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
        let (width, height) = super::measure(ui, metrics, &run.text, size);
        (width, height.max(size * 1.5))
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
        let line_width = width(line).min(available);
        let mut x = 0.;
        for mut run in line.drain(..) {
            run.line_width = line_width;
            if output.len() == crate::rich_content::MAX_RUNS {
                return false;
            }
            run.measured_width = run.measured_width.min((available - x).max(0.));
            run.measured_height = height;
            run.x = x;
            run.y = y;
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
    if output.len() > crate::rich_content::MAX_RUNS {
        return ModelRc::default();
    }
    if let Some(first) = output.first_mut() {
        first.natural_width = natural_width;
    }
    ModelRc::new(VecModel::from(output))
}
/// Updates the previous layout in place so the runs' delegates keep their
/// identity. `previous` is this function's own earlier output; anything else
/// (the initial empty model) just takes `next`.
pub(crate) fn reuse(previous: ModelRc<RichRun>, next: ModelRc<RichRun>) -> ModelRc<RichRun> {
    let Some(model) = previous.as_any().downcast_ref::<VecModel<RichRun>>() else {
        return next;
    };
    for i in (next.row_count()..model.row_count()).rev() {
        model.remove(i);
    }
    for (i, row) in next.iter().enumerate() {
        if i == model.row_count() {
            model.push(row);
        } else if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
    previous
}
