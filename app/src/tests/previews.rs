use super::*;

fn emoji_message() -> StoredMessage {
    let body = "日😀[笑]\n again [笑] end";
    let mut m = message("preview-emoji", body);
    m.outgoing = true;
    let spans = body
        .match_indices("[笑]")
        .map(|(start, text)| line::store::InlineEmoji {
            start_utf16: body[..start].encode_utf16().count(),
            end_utf16: body[..start + text.len()].encode_utf16().count(),
            start_byte: start,
            end_byte: start + text.len(),
            fallback: text.into(),
            product_id: "synthetic".into(),
            sticon_id: "smile".into(),
            version: None,
            animated: false,
        })
        .collect();
    m.body = Content::Text {
        text: body.into(),
        spans,
    };
    m
}

#[test]
fn preview_conversion_keeps_spans_text_fallback_and_prefix() {
    let view = emoji_message();
    let mut message = message_row(&emoji_message(), None);
    links::decorate_message(&mut message, "葵");
    let mut row = ChatRow::default();
    preview::set(
        &mut row,
        &view.body,
        view.outgoing,
        None,
        &HashMap::new(),
        15,
    );
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
    assert_eq!(message_text(&emoji_message().body).1, message.body.as_str());
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
    for (content, text, _) in label_cases() {
        let kind = kind_label(&content).0;
        if !matches!(
            kind,
            MessageKind::Photo | MessageKind::Video | MessageKind::Sticker | MessageKind::File
        ) {
            continue;
        }
        let mut m = message("media", "");
        m.body = content;
        let asset = line::client::desktop::assets::Asset::avatar("/synthetic").unwrap();
        let media_key = asset.key();
        match &mut m.body {
            Content::Photo { asset: a, .. } | Content::Sticker { asset: a, .. } => *a = Some(asset),
            Content::Video { preview, .. } => *preview = Some(asset),
            _ => {}
        }
        let mut images = HashMap::new();
        let view = m;
        let mut row = ChatRow::default();
        preview::set(&mut row, &view.body, view.outgoing, None, &images, 15);
        assert_eq!(row.preview, text);
        if kind == MessageKind::File {
            assert_eq!(row.preview_runs.row_count(), 0);
            continue;
        }
        assert!(row.preview_runs.row_data(0).unwrap().thumbnail);
        let measure = |s: &str| s.chars().count() as f32;
        let fallback = preview::layout(&row.preview_runs, "", 100., 1., measure);
        assert!(!fallback.iter().any(|run| run.thumbnail));
        let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &[0, 180, 100, 255],
            1,
            1,
        ));
        images.insert(media_key.clone(), picture);
        preview::set(&mut row, &view.body, view.outgoing, None, &images, 15);
        assert!(
            preview::layout(&row.preview_runs, "", 100., 1., measure)
                .iter()
                .any(|r| r.thumbnail)
        );
        images.remove(&media_key);
        preview::set(&mut row, &view.body, view.outgoing, None, &images, 15);
        assert_eq!(row.preview, text);
    }
}

fn label_cases() -> [(Content, &'static str, &'static str); 9] {
    [
        (
            Content::Photo {
                asset: None,
                size: None,
                album_count: Some(4),
                caption: Some("walk".into()),
            },
            "Photo",
            "Album · 4 items · walk",
        ),
        (
            Content::Sticker {
                pkg: "pkg".into(),
                id: "id".into(),
                ver: None,
                asset: None,
                text: Some("Hello".into()),
            },
            "Sticker",
            "Sticker · Hello",
        ),
        (
            Content::File {
                name: Some("notes.txt".into()),
                size: None,
                asset: None,
            },
            "File · notes.txt",
            "File · notes.txt",
        ),
        (
            Content::Contact {
                mid: "mid".into(),
                name: Some("Alice".into()),
            },
            "Contact · Alice",
            "Contact · Alice",
        ),
        (
            Content::Voice {
                duration: Some(1200),
                asset: None,
            },
            "Voice message",
            "Voice message",
        ),
        (
            Content::Undecryptable,
            "Couldn't decrypt this message",
            "Couldn't decrypt this message",
        ),
        (
            Content::Unknown { content_type: 7 },
            "This message type isn't supported yet",
            "This message type isn't supported yet",
        ),
        (Content::Withdrawn, "Message unsent", "Message unsent"),
        (
            Content::Video {
                duration: None,
                asset: None,
                preview: None,
                album_count: None,
                caption: None,
            },
            "Video",
            "Video",
        ),
    ]
}

#[test]
fn content_labels_agree_across_rows_previews_and_accessibility() {
    for (content, compact, full) in label_cases() {
        let mut m = message("label", "");
        m.body = content;
        let mut chat = ChatRow::default();
        preview::set(&mut chat, &m.body, m.outgoing, None, &HashMap::new(), 15);
        let mut row = message_row(&m, None);
        links::decorate_message(&mut row, "Alice");
        assert_eq!(kind_label(&m.body).1, compact);
        assert_eq!(message_text(&m.body).1, full);
        assert_eq!(row.body, full);
        assert_eq!(chat.preview, compact);
        assert!(row.accessible_label.ends_with(full));
    }
}
