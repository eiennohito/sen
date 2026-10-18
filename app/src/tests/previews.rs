use super::*;

fn emoji_message() -> account::MessageView {
    let body = "日😀[笑]\n again [笑] end";
    let mut m = message("preview-emoji", body);
    m.outgoing = true;
    m.emoji = body
        .match_indices("[笑]")
        .map(|(start, text)| account::EmojiView {
            span: line_wasm_poc::store::InlineEmoji {
                start_utf16: body[..start].encode_utf16().count(),
                end_utf16: body[..start + text.len()].encode_utf16().count(),
                start_byte: start,
                end_byte: start + text.len(),
                fallback: text.into(),
                product_id: "synthetic".into(),
                sticon_id: "smile".into(),
                version: None,
                animated: false,
            },
            image_key: "emoji-key".into(),
        })
        .collect();
    m
}

#[test]
fn preview_conversion_keeps_spans_text_fallback_and_prefix() {
    let mut message = message_row(emoji_message());
    links::decorate_message(&mut message, "葵");
    let mut row = ChatRow::default();
    preview::set(&mut row, &message, 15);
    assert_eq!(row.preview, "日😀[笑] again [笑] end");
    assert!(row.outgoing_preview);
    assert!(message.accessible_label.ends_with(message.body.as_str()));
    assert_eq!(row.preview_runs.iter().filter(|r| r.emoji).count(), 2);
    assert_eq!(
        row.preview_runs
            .iter()
            .map(|r| r.text.to_string())
            .collect::<String>(),
        row.preview.as_str()
    );
    let measure = |s: &str| s.chars().count() as f32;
    let line = preview::layout(&row.preview_runs, "You: ", 100., 1., measure);
    assert_eq!(
        line.iter().map(|r| r.text.to_string()).collect::<String>(),
        format!("You: {}", row.preview)
    );
    assert!(
        line.iter()
            .filter(|r| r.emoji)
            .all(|r| !r.has_picture && r.text == "[笑]")
    );
    assert_eq!(message_text(&emoji_message()), message.body.as_str());
}

#[test]
fn preview_elision_keeps_images_atomic_and_text_on_grapheme_boundaries() {
    let rows = ModelRc::new(VecModel::from(vec![
        RichRun {
            text: "a".into(),
            ..Default::default()
        },
        RichRun {
            text: "[笑]".into(),
            emoji: true,
            has_picture: true,
            ..Default::default()
        },
        RichRun {
            text: "e\u{301}xyz".into(),
            ..Default::default()
        },
    ]));
    let measure = |s: &str| s.chars().count() as f32;
    let short = preview::layout(&rows, "", 2., 1., measure);
    assert_eq!(
        short.iter().map(|r| r.text.to_string()).collect::<String>(),
        "a…"
    );
    assert!(!short.iter().any(|r| r.emoji));
    let long = preview::layout(&rows, "", 5.5, 1., measure);
    assert!(long.iter().any(|r| r.emoji));
    assert_eq!(
        long.iter().map(|r| r.text.to_string()).collect::<String>(),
        "a[笑]e\u{301}…"
    );
    assert!(long.iter().all(|r| r.x + r.measured_width <= 5.5));
    assert_eq!(preview::layout(&rows, "", 0., 1., measure).row_count(), 0);
}

#[test]
fn media_preview_missing_or_failed_has_the_same_text_and_no_retry() {
    for kind in [1, 2, 7, 14] {
        let mut m = message("media", "caption");
        m.rich.content_type = kind;
        if kind == 1 {
            m.rich.album_count = Some(4);
            m.rich.caption = Some("walk".into());
        }
        m.image_key = if kind == 14 { "" } else { "media-key" }.into();
        let text = match kind {
            1 => "Photo",
            2 => "Video",
            7 => "Sticker",
            14 => "File",
            _ => unreachable!(),
        };
        let mut message = message_row(m);
        let mut row = ChatRow::default();
        preview::set(&mut row, &message, 15);
        assert_eq!(row.preview, text);
        if kind == 14 {
            assert_eq!(row.preview_runs.row_count(), 0);
            continue;
        }
        assert!(row.preview_runs.row_data(0).unwrap().thumbnail);
        let measure = |s: &str| s.chars().count() as f32;
        let fallback = preview::layout(&row.preview_runs, "", 100., 1., measure);
        assert!(!fallback.iter().any(|run| run.thumbnail));
        message.picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &[0, 180, 100, 255],
            1,
            1,
        ));
        message.has_picture = true;
        preview::set(&mut row, &message, 15);
        assert!(
            preview::layout(&row.preview_runs, "", 100., 1., measure)
                .iter()
                .any(|r| r.thumbnail)
        );
        message.image_failed = true;
        preview::set(&mut row, &message, 15);
        assert_eq!(row.preview, text);
    }
}
