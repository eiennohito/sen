use crate::*;

pub(crate) const PICTURE_BYTES: usize = 16 * 1024 * 1024;
const MARGIN: usize = 2;

fn small(image: &slint::Image, size: i32) -> slint::Image {
    let edge = ((size.clamp(12, 24) - 1) * 3) as u32;
    let dimensions = image.size();
    if dimensions.width <= edge && dimensions.height <= edge {
        return image.clone();
    }
    let Some(pixels) = image.to_rgba8() else {
        return slint::Image::default();
    };
    let source =
        image::RgbaImage::from_raw(pixels.width(), pixels.height(), pixels.as_bytes().to_vec())
            .unwrap();
    let source = image::DynamicImage::ImageRgba8(source);
    let scaled = source.thumbnail(edge, edge).to_rgba8();
    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        scaled.as_raw(),
        scaled.width(),
        scaled.height(),
    ))
}

pub(crate) fn set(row: &mut ChatRow, message: &MessageRow, size: i32) {
    row.preview = single_line(&message.body).into();
    row.outgoing_preview = message.outgoing;
    let mut runs = Vec::new();
    let mut pictures = HashMap::new();
    if !message.image_key.is_empty()
        && matches!(
            message.kind,
            MessageKind::Photo | MessageKind::Sticker | MessageKind::Video
        )
    {
        runs.push(RichRun {
            image_key: message.image_key.clone(),
            picture: small(&message.picture, size),
            has_picture: message.has_picture && !message.image_failed,
            thumbnail: true,
            contain: message.kind == MessageKind::Sticker,
            ..Default::default()
        });
        runs.push(RichRun {
            text: row.preview.clone(),
            thumbnail_text: row
                .preview
                .strip_prefix(
                    links::placeholder(message.kind)
                        .and_then(|text| text.find(' ').map(|end| &text[..end + 1]))
                        .unwrap_or(""),
                )
                .unwrap_or(&row.preview)
                .into(),
            ..Default::default()
        });
    } else if message.rich_runs.iter().any(|run| run.emoji) && !message.rich_fallback {
        let mut space = false;
        let mut started = false;
        for mut run in message.rich_runs.iter() {
            if run.emoji {
                if space {
                    if let Some(last) = runs.last_mut().filter(|last| !last.emoji) {
                        last.text = format!("{} ", last.text).into();
                    } else {
                        runs.push(RichRun {
                            text: " ".into(),
                            ..Default::default()
                        });
                    }
                }
                space = false;
                started = true;
                if run.has_picture {
                    run.picture = pictures
                        .entry(run.image_key.clone())
                        .or_insert_with(|| small(&run.picture, size))
                        .clone();
                }
                runs.push(run);
                continue;
            }
            let mut text = String::new();
            for ch in run.text.chars() {
                if ch.is_whitespace() {
                    space = started;
                } else {
                    if space {
                        text.push(' ');
                    }
                    space = false;
                    started = true;
                    text.push(ch);
                }
            }
            run.text = text.into();
            run.newline = false;
            run.url = "".into();
            if !run.text.is_empty() {
                if let Some(last) = runs.last_mut().filter(|last| !last.emoji) {
                    last.text = format!("{}{}", last.text, run.text).into();
                } else {
                    runs.push(run);
                }
            }
        }
    }
    for run in &mut runs {
        if !run.has_picture
            && !run.image_key.is_empty()
            && let Some(old) = row
                .preview_runs
                .iter()
                .find(|old| old.image_key == run.image_key && old.has_picture)
        {
            run.picture = old.picture;
            run.has_picture = true;
        }
    }
    row.preview_runs = ModelRc::new(VecModel::from(runs));
}

pub(crate) fn image(row: &ChatRow, key: &str, image: Option<&slint::Image>, size: i32) -> bool {
    let Some(runs) = row
        .preview_runs
        .as_any()
        .downcast_ref::<VecModel<RichRun>>()
    else {
        return false;
    };
    let picture = image.map(|image| small(image, size)).unwrap_or_default();
    apply_image(runs, key, &picture, image.is_some())
}

fn apply_image(runs: &VecModel<RichRun>, key: &str, picture: &slint::Image, loaded: bool) -> bool {
    let mut changed = false;
    for i in 0..runs.row_count() {
        let mut run = runs.row_data(i).unwrap();
        if run.image_key == key && (run.has_picture != loaded || run.picture != *picture) {
            run.picture = picture.clone();
            run.has_picture = loaded;
            runs.set_row_data(i, run);
            changed = true;
        }
    }
    changed
}

pub(crate) fn layout(
    rows: &ModelRc<RichRun>,
    prefix: &str,
    width: f32,
    size: f32,
    measure: impl Fn(&str) -> f32,
) -> ModelRc<RichRun> {
    use unicode_segmentation::UnicodeSegmentation;
    if width < 1. || size < 1. {
        return ModelRc::default();
    }
    let mut source = Vec::new();
    if !prefix.is_empty() {
        source.push(RichRun {
            text: prefix.into(),
            ..Default::default()
        });
    }
    let thumbnail = rows.iter().any(|run| run.thumbnail && run.has_picture);
    source.extend(
        rows.iter()
            .filter(|run| !run.thumbnail || run.has_picture)
            .map(|mut run| {
                if thumbnail && !run.thumbnail_text.is_empty() {
                    run.text = run.thumbnail_text.clone();
                }
                run
            }),
    );
    let run_width = |run: &RichRun| {
        if run.thumbnail {
            size * 1.5 + 4.
        } else if run.emoji {
            size * 1.4
        } else {
            measure(&run.text)
        }
    };
    let total: f32 = source.iter().map(&run_width).sum();
    let elided = total > width;
    let limit = if elided {
        (width - measure("…")).max(0.)
    } else {
        width
    };
    let mut result = Vec::new();
    let mut x = 0.;
    for mut run in source {
        let mut w = run_width(&run);
        let truncated = x + w > limit;
        if x + w > limit {
            if !run.emoji && !run.thumbnail {
                let mut end = 0;
                for (i, ch) in run.text.grapheme_indices(true) {
                    let next = i + ch.len();
                    if x + measure(&run.text[..next]) > limit {
                        break;
                    }
                    end = next;
                }
                run.text = run.text[..end].into();
                w = measure(&run.text);
            } else {
                break;
            }
        }
        run.x = x;
        run.measured_width = w;
        x += w;
        result.push(run);
        if truncated || x >= limit {
            break;
        }
    }
    if elided {
        result.push(RichRun {
            text: "…".into(),
            x,
            measured_width: measure("…").min(width),
            ..Default::default()
        });
    }
    ModelRc::new(VecModel::from(result))
}

impl View {
    pub(crate) fn preview_window(&self, ui: &AppWindow, margin: usize) -> Vec<i32> {
        let state = ui.global::<State>();
        if state.get_section() != 0 || (state.get_narrow() && ui.get_show_conversation()) {
            return Vec::new();
        }
        let height = ui.get_chat_row_height().max(1.);
        let first = (-ui.get_list_y() / height).floor().max(0.) as usize;
        let start = first.saturating_sub(margin);
        let count = (ui.get_chat_list_height() / height).ceil().max(0.) as usize + 1;
        ui.get_chats()
            .iter()
            .skip(start)
            .take(first - start + count + margin)
            .filter(|row| !row.draft)
            .map(|row| row.index)
            .collect()
    }
    fn preview_columns(&self, ui: &AppWindow) -> usize {
        let state = ui.global::<State>();
        let width = if state.get_narrow() {
            state.get_window_width().max(state.get_browser_width())
        } else {
            state.get_browser_width()
        };
        (width.max(1.) / ((ui.get_text_size().clamp(12, 24) - 1) as f32 * 1.4)).ceil() as usize
    }
    pub(crate) fn preview_budget(&self, ui: &AppWindow) -> usize {
        let rows = (ui.get_chat_list_height().max(0.) / ui.get_chat_row_height().max(1.)).ceil()
            as usize
            + 1;
        let edge = ((ui.get_text_size().clamp(12, 24) - 1) * 3) as usize;
        (rows + MARGIN * 2)
            .saturating_mul(self.preview_columns(ui))
            .saturating_mul(edge * edge * 4)
            .min(PICTURE_BYTES)
    }
    pub(crate) fn retain_previews(&self, ui: &AppWindow) {
        let visible = self.preview_window(ui, 0);
        let window = self.preview_window(ui, MARGIN);
        let ranks: HashMap<_, _> = window
            .iter()
            .enumerate()
            .map(|(position, index)| {
                let distance = window
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| visible.contains(i))
                    .map(|(i, _)| i.abs_diff(position))
                    .min()
                    .unwrap_or(usize::MAX);
                (*index, (distance, position))
            })
            .collect();
        let mut ordered: Vec<_> = self
            .rows
            .iter()
            .filter(|row| ranks.contains_key(&row.index))
            .collect();
        ordered.sort_by_key(|row| ranks[&row.index]);
        let columns = self.preview_columns(ui);
        let budget = self.preview_budget(ui);
        let mut pictures = HashMap::new();
        let mut used = 0;
        for row in ordered {
            for run in row
                .preview_runs
                .iter()
                .filter(|run| !run.image_key.is_empty())
                .take(columns)
            {
                if !run.has_picture || pictures.contains_key(&run.image_key) {
                    continue;
                }
                let picture = small(&run.picture, ui.get_text_size());
                let size = picture.size();
                let bytes = size.width as usize * size.height as usize * 4;
                if used + bytes <= budget {
                    pictures.insert(run.image_key, picture);
                    used += bytes;
                }
            }
        }
        for row in &self.rows {
            let Some(runs) = row
                .preview_runs
                .as_any()
                .downcast_ref::<VecModel<RichRun>>()
            else {
                continue;
            };
            let mut images = 0;
            for i in 0..runs.row_count() {
                let mut run = runs.row_data(i).unwrap();
                if run.image_key.is_empty() {
                    continue;
                }
                images += 1;
                let picture = if window.contains(&row.index) && images <= columns {
                    pictures.get(&run.image_key)
                } else {
                    None
                };
                let loaded = picture.is_some();
                let picture = picture.cloned().unwrap_or_default();
                if run.has_picture != loaded || run.picture != picture {
                    run.picture = picture;
                    run.has_picture = loaded;
                    runs.set_row_data(i, run);
                }
            }
        }
    }
    pub(crate) fn preview_image(&self, ui: &AppWindow, key: &str, image: Option<&slint::Image>) {
        let window = self.preview_window(ui, MARGIN);
        let mut picture = None;
        for row in &self.rows {
            if (image.is_none() || window.contains(&row.index))
                && row.preview_runs.iter().any(|run| run.image_key == key)
            {
                let picture = picture.get_or_insert_with(|| {
                    image
                        .map(|image| small(image, ui.get_text_size()))
                        .unwrap_or_default()
                });
                self::image(row, key, image.map(|_| &*picture), ui.get_text_size());
            }
        }
        self.retain_previews(ui);
    }
    pub(crate) fn preview_priority_keys(&self, ui: &AppWindow) -> Vec<String> {
        self.preview_asset_keys(ui, false)
    }
    pub(crate) fn preview_keys(&self, ui: &AppWindow) -> Vec<String> {
        self.preview_asset_keys(ui, true)
    }
    fn preview_asset_keys(&self, ui: &AppWindow, missing: bool) -> Vec<String> {
        let window = self.preview_window(ui, 0);
        let columns = self.preview_columns(ui);
        let mut keys = Vec::new();
        for row in self.rows.iter().filter(|row| window.contains(&row.index)) {
            for run in row
                .preview_runs
                .iter()
                .filter(|run| !run.image_key.is_empty())
                .take(columns)
            {
                if (!missing || !run.has_picture) && !keys.contains(&run.image_key.to_string()) {
                    keys.push(run.image_key.to_string());
                }
                if keys.len() == 256 {
                    return keys;
                }
            }
        }
        keys
    }
}
