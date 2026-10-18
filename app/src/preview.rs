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

pub(crate) fn set(row: &mut ChatRow, message: &MessageRow, size: i32) {
    row.preview = match message.kind {
        MessageKind::Sticker => "Sticker".into(),
        MessageKind::Photo => "Photo".into(),
        MessageKind::Video => "Video".into(),
        MessageKind::Voice => "Voice".into(),
        MessageKind::File => "File".into(),
        MessageKind::Location => "Location".into(),
        MessageKind::Contact => "Contact".into(),
        _ => single_line(&message.body).into(),
    };
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
            thumbnail_text: row.preview.clone(),
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
