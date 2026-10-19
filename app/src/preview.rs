use crate::*;

pub(crate) fn small(image: &slint::Image, size: i32) -> slint::Image {
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

pub(crate) fn preview_runs(
    text: &str,
    metadata: &[account::EmojiView],
    images: &HashMap<String, slint::Image>,
    size: i32,
) -> ModelRc<RichRun> {
    let source = rich_content::runs(text, metadata);
    if !source.iter().any(|r| r.emoji) {
        return ModelRc::default();
    }
    let mut runs: Vec<RichRun> = Vec::new();
    let mut pictures = HashMap::new();
    let mut space = false;
    let mut started = false;
    for mut run in source.iter() {
        if run.emoji {
            if space {
                if let Some(last) = runs.last_mut().filter(|r| !r.emoji) {
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
            if let Some(image) = images.get(run.image_key.as_str()) {
                run.picture = pictures
                    .entry(run.image_key.clone())
                    .or_insert_with(|| small(image, size))
                    .clone();
                run.has_picture = true;
            }
            runs.push(run);
        } else {
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
                if let Some(last) = runs.last_mut().filter(|r| !r.emoji) {
                    last.text = format!("{}{}", last.text, run.text).into();
                } else {
                    runs.push(run);
                }
            }
        }
    }
    ModelRc::new(VecModel::from(runs))
}

pub(crate) fn set(
    row: &mut ChatRow,
    view: &account::MessageView,
    images: &HashMap<String, slint::Image>,
    size: i32,
) {
    let kind = links::kind(
        view.rich.content_type,
        view.rich.withdrawn,
        view.decryption_failed,
    );
    row.preview = match kind {
        MessageKind::Sticker => "Sticker".into(),
        MessageKind::Photo => "Photo".into(),
        MessageKind::Video => "Video".into(),
        MessageKind::Voice => "Voice".into(),
        MessageKind::File => "File".into(),
        MessageKind::Location => "Location".into(),
        MessageKind::Contact => "Contact".into(),
        _ => single_line(&message_text(view)).into(),
    };
    row.outgoing_preview = view.outgoing;
    row.preview_runs = if !view.image_key.is_empty()
        && matches!(
            kind,
            MessageKind::Photo | MessageKind::Sticker | MessageKind::Video
        ) {
        let image = images.get(&view.image_key);
        ModelRc::new(VecModel::from(vec![
            RichRun {
                image_key: view.image_key.clone().into(),
                picture: image.map(|i| small(i, size)).unwrap_or_default(),
                has_picture: image.is_some(),
                thumbnail: true,
                contain: kind == MessageKind::Sticker,
                ..Default::default()
            },
            RichRun {
                text: row.preview.clone(),
                thumbnail_text: row.preview.clone(),
                ..Default::default()
            },
        ]))
    } else {
        preview_runs(&view.body, &view.emoji, images, size)
    };
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
