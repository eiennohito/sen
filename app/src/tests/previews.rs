use super::*;

fn emoji_message() -> StoredMessage {
    let body = "日😀[笑]\n again [笑] end";
    let mut m = message(body);
    m.outgoing = true;
    let spans = body
        .match_indices("[笑]")
        .map(|(start, text)| sen::store::InlineEmoji {
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
    let mut message = message_row(&emoji_message(), None, &presenter::Images::default());
    rich_content::links::decorate_message(&mut message);
    let row = presenter::chats::chat_row(
        presenter::ChatInputs {
            item: ChatListItem {
                preview: Some(view),
                ..chat_item(ChatInput::default())
            },
            title: "葵".into(),
            avatar: None,
            system_text: None,
            index: 0,
        },
        &presenter::Images::default(),
    );
    assert_eq!(row.preview, "日😀[笑] again [笑] end");
    assert_eq!(row.preview_mode, PreviewMode::Outgoing);
    assert_eq!(row.accessible_label, "葵, You: 日😀[笑] again [笑] end");
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
    let line = rich_content::preview::layout(&row.preview_runs, "You: ", 100., 1., measure);
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
    let short = rich_content::preview::layout(&rows, "", 2., 1., measure);
    assert_eq!(
        short.iter().map(|r| r.text.to_string()).collect::<String>(),
        "a…"
    );
    assert!(!short.iter().any(|r| r.emoji));
    let long = rich_content::preview::layout(&rows, "", 5.5, 1., measure);
    assert!(long.iter().any(|r| r.emoji));
    assert_eq!(
        long.iter().map(|r| r.text.to_string()).collect::<String>(),
        "a[笑]e\u{301}…"
    );
    assert!(long.iter().all(|r| r.x + r.measured_width <= 5.5));
    assert_eq!(
        rich_content::preview::layout(&rows, "", 0., 1., measure).row_count(),
        0
    );
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
        let mut m = message("");
        m.body = content;
        let asset = sen::client::desktop::assets::Asset::avatar("/synthetic").unwrap();
        let media_key = asset.key();
        match &mut m.body {
            Content::Photo { asset: a, .. } | Content::Sticker { asset: a, .. } => *a = Some(asset),
            Content::Video { preview, .. } => *preview = Some(asset),
            _ => {}
        }
        let images = presenter::Images::default();
        let view = m;
        let mut row = ChatRow::default();
        rich_content::preview::set(&mut row, &view.body, None, &images);
        assert_eq!(row.preview, text);
        if kind == MessageKind::File {
            assert_eq!(row.preview_runs.row_count(), 0);
            continue;
        }
        assert!(row.preview_runs.row_data(0).unwrap().thumbnail);
        let measure = |s: &str| s.chars().count() as f32;
        let fallback = rich_content::preview::layout(&row.preview_runs, "", 100., 1., measure);
        assert!(!fallback.iter().any(|run| run.thumbnail));
        let picture = green_image();
        images.insert(media_key.clone(), crate::tests::image_file(&picture));
        rich_content::preview::set(&mut row, &view.body, None, &images);
        assert!(
            rich_content::preview::layout(&row.preview_runs, "", 100., 1., measure)
                .iter()
                .any(|r| r.thumbnail)
        );
        images.failed(media_key, sen::client::desktop::assets::Failure::Decode);
        rich_content::preview::set(&mut row, &view.body, None, &images);
        assert_eq!(row.preview, text);
        assert!(!row.preview_runs.row_data(0).unwrap().has_picture);
    }
}

pub(crate) fn label_cases() -> [(Content, &'static str, &'static str); 9] {
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
            "notes.txt",
        ),
        (
            Content::Contact {
                mid: "mid".into(),
                name: Some("Alice".into()),
            },
            "Contact · Alice",
            "Alice",
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
fn photo_previews_and_reply_quotes_load_the_small_disk_variant() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let fake = sen::backend::FakeBackend::canned();
        let asset = sen::backend::FakeBackend::asset(true);
        let path = fake.set_image(
            &asset,
            sen::client::desktop::assets::Pixels {
                width: 384,
                height: 256,
                rgba: [20, 30, 40, 255].repeat(384 * 256),
            },
        );
        let mut presenter = presenter::Presenter::default();
        presenter.images.insert(asset.key(), path);
        let mut target = message("");
        target.body = Content::Photo {
            asset: Some(asset),
            size: Some((1200, 800)),
            album_count: None,
            caption: None,
        };
        let mut row = ChatRow::default();
        rich_content::preview::set(&mut row, &target.body, None, &presenter.images);
        assert_eq!(
            row.preview_runs.row_data(0).unwrap().picture.size().width,
            64
        );
        let ui = window::create_app_window().unwrap();
        presenter.install(&ui);
        let mut state = AppState::default();
        let id = sen::store::ServerId::new("42").unwrap();
        target.server_id = Some(id.clone());
        state.messages.insert(target.key, target.clone());
        state.composer_reply = Some(sen::store::ReplyReference {
            message_id: id,
            service_code: None,
        });
        state.selected = Some(target.chat_id.clone());
        let mut reply = message("reply");
        reply.key = Key(2);
        reply.reply = state.composer_reply.clone().map(Box::new);
        state.messages.insert(reply.key, reply);
        presenter.present(&ui, &state, ChatCategory::All);
        assert_eq!(
            ui.global::<ComposerState>()
                .get_composer_reply()
                .picture
                .size()
                .width,
            64
        );
        let reply = ui
            .global::<ConversationState>()
            .get_messages()
            .iter()
            .find(|row| row.id == "2")
            .unwrap();
        assert_eq!(reply.reply_picture.size().width, 64);
        assert_eq!(
            presenter.images.picture(target.body.image()).size().width,
            384
        );
    })
    .join()
    .unwrap();
}

#[test]
fn content_labels_agree_across_rows_previews_and_accessibility() {
    for (content, compact, full) in label_cases() {
        let mut m = message("");
        m.body = content;
        let mut chat = ChatRow::default();
        rich_content::preview::set(&mut chat, &m.body, None, &presenter::Images::default());
        let mut row = message_row(&m, None, &presenter::Images::default());
        rich_content::links::decorate_message(&mut row);
        assert_eq!(row.body, full);
        assert_eq!(chat.preview, compact);
        assert!(row.accessible_label.ends_with(full));
    }
}

#[test]
fn unread_counts_remain_visible_in_every_connection_state() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        for (id, unread) in [("read", 0), ("unread", 3), ("arrivals", 2)] {
            let id = ChatId(id.into());
            state.chat_order.push(id.clone());
            state.chats.insert(
                id.clone(),
                ChatListItem {
                    unread_count: unread,
                    ..chat_item(ChatInput {
                        id,
                        ..Default::default()
                    })
                },
            );
        }
        for connection in [
            Connection::Local,
            Connection::Connecting,
            Connection::Offline,
            Connection::Online,
            Connection::SigningIn,
            Connection::SignedOut {
                reason: sen::client::error::SignOutReason::NoSession,
            },
            Connection::Fatal,
        ] {
            state.connection = connection;
            presenter.present(&ui, &state, ChatCategory::All);
            let rows: Vec<_> = ui.global::<ChatsState>().get_chats().iter().collect();
            let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap();

            assert_eq!(row("unread").unread, 3);
            assert_eq!(row("arrivals").badge, "2");
            assert!(row("arrivals").accessible_label.contains("2 unread"));
            assert_eq!(row("read").unread, 0);
            assert!(!row("read").accessible_label.contains(" unread"));
            assert_eq!(row("unread").badge, "3");
            assert!(row("unread").accessible_label.contains("3 unread"));
            assert_eq!(ui.global::<ShellState>().get_unread_badge(), "2");
        }
    })
    .join()
    .unwrap();
}
