use super::*;

fn emoji_message() -> account::MessageView {
    let body = "日😀[笑]\n again [笑] end";
    let mut m = message("preview-emoji", body);
    m.outgoing = true;
    m.emoji = body
        .match_indices("[笑]")
        .map(|(start, text)| account::EmojiView {
            span: line_wasm_poc::client::desktop::rich::InlineEmoji {
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

fn signature(row: &ChatRow) -> Vec<(String, String, bool, bool)> {
    row.preview_runs
        .iter()
        .map(|r| {
            (
                r.text.to_string(),
                r.image_key.to_string(),
                r.emoji,
                r.has_picture,
            )
        })
        .collect()
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
    let image = demo::avatar(0);
    preview::image(&row, "emoji-key", Some(&image), 15);
    assert!(
        row.preview_runs
            .iter()
            .filter(|r| r.emoji)
            .all(|r| r.has_picture)
    );
    preview::set(&mut row, &message, 15);
    assert!(
        row.preview_runs
            .iter()
            .filter(|r| r.emoji)
            .all(|r| r.has_picture)
    );
    preview::image(&row, "emoji-key", None, 15);
    assert!(
        row.preview_runs
            .iter()
            .filter(|r| r.emoji)
            .all(|r| !r.has_picture && r.text == "[笑]")
    );
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
fn initial_and_refresh_preview_models_agree_and_visible_assets_are_bounded() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        rich_content::install(&ui, Rc::new(RefCell::new(View::default())));
        let mut view = View {
            demo: true,
            ..Default::default()
        };
        view.update(
            &ui,
            account::Update::Chats {
                chats: vec![account::ChatView {
                    id: "chat".into(),
                    title: "葵".into(),
                    preview: emoji_message().body,
                    preview_message: Some(emoji_message()),
                    timestamp: 1,
                    avatar_key: String::new(),
                    unread: Some(0),
                    muted: None,
                    last_read_message: None,
                    last_message_id: None,
                    member_count: None,
                    outgoing_preview: true,
                    group: false,
                }],
                more: false,
            },
        );
        let initial = view.rows[0].clone();
        view.refresh_preview(&ui, "chat", &[message_row(emoji_message())]);
        assert_eq!(view.rows[0].preview, initial.preview);
        assert_eq!(view.rows[0].outgoing_preview, initial.outgoing_preview);
        assert_eq!(signature(&view.rows[0]), signature(&initial));
        view.update(
            &ui,
            account::Update::ChatMeta {
                chat: "chat".into(),
                title: "葵".into(),
                message: Box::new(emoji_message()),
                unread: Some(0),
                muted: None,
            },
        );
        assert_eq!(signature(&view.rows[0]), signature(&initial));
        view.messages = vec![message_row(emoji_message())];
        view.update(
            &ui,
            account::Update::Image {
                key: "emoji-key".into(),
                pixels: std::sync::Arc::new(line_wasm_poc::client::desktop::assets::Pixels {
                    width: 1,
                    height: 1,
                    rgba: vec![255, 180, 0, 255],
                }),
            },
        );
        assert!(
            view.rows[0]
                .preview_runs
                .iter()
                .filter(|run| run.emoji)
                .all(|run| run.has_picture)
        );
        assert!(
            ui.get_messages()
                .iter()
                .flat_map(|row| row.rich_runs.iter().collect::<Vec<_>>())
                .filter(|run| run.emoji)
                .all(|run| run.has_picture)
        );
        view.update(
            &ui,
            account::Update::ImageFailed {
                key: "emoji-key".into(),
                failure: line_wasm_poc::client::desktop::assets::Failure::Decode,
            },
        );
        assert!(
            view.rows[0]
                .preview_runs
                .iter()
                .filter(|run| run.emoji)
                .all(|run| !run.has_picture)
        );
        assert!(
            ui.get_messages()
                .iter()
                .flat_map(|row| row.rich_runs.iter().collect::<Vec<_>>())
                .filter(|run| run.emoji)
                .all(|run| !run.has_picture)
        );
        for i in 1..500 {
            view.ids.push(i.to_string());
            view.rows.push(ChatRow {
                index: i,
                preview_runs: initial.preview_runs.clone(),
                ..Default::default()
            });
        }
        view.filter(&ui);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        let visible = view.preview_keys(&ui);
        assert!(!visible.is_empty());
        assert!(visible.len() < 30);
        ui.set_show_conversation(true);
        ui.global::<State>().set_narrow(true);
        assert!(view.preview_keys(&ui).is_empty());
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
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
        let text = message_text(&m);
        let message = message_row(m);
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
        preview::image(&row, "media-key", Some(&demo::avatar(0)), 15);
        assert!(
            preview::layout(&row.preview_runs, "", 100., 1., measure)
                .iter()
                .any(|r| r.thumbnail)
        );
        preview::image(&row, "media-key", None, 15);
        assert_eq!(row.preview, text);
    }
}

#[test]
fn scrolling_photo_previews_releases_buffers_resizes_and_requests_cache_hits() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        let mut view = View {
            demo: true,
            commands: Some(tx.into()),
            ..Default::default()
        };
        for i in 0..500 {
            view.ids.push(i.to_string());
            let mut row = ChatRow {
                index: i,
                title: format!("Photo {i:03}").into(),
                ..Default::default()
            };
            preview::set(
                &mut row,
                &MessageRow {
                    kind: MessageKind::Photo,
                    body: "▧ Photo".into(),
                    image_key: format!("photo-{i}").into(),
                    ..Default::default()
                },
                15,
            );
            view.rows.push(row);
        }
        view.filter(&ui);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        let picture = demo::thumbnail(false);
        for i in (0..500).step_by(5).chain(std::iter::once(0)) {
            ui.set_list_y(-(i as f32 * ui.get_chat_row_height()));
            view.read_exposure(&ui, Vec::new());
            let commands = rx.try_iter().collect::<Vec<_>>();
            let requested: Vec<_> = commands
                .into_iter()
                .filter_map(|command| match command {
                    account::Command::PreviewAssets { keys } => Some(keys),
                    _ => None,
                })
                .flatten()
                .collect();
            assert!(!requested.is_empty());
            for key in requested {
                view.preview_image(&ui, &key, Some(&picture));
            }
            let allowed = view.preview_window(&ui, 2);
            let mut bytes = 0;
            for row in &view.rows {
                for run in row.preview_runs.iter().filter(|run| run.has_picture) {
                    assert!(allowed.contains(&row.index));
                    let size = run.picture.size();
                    assert!(size.width <= 42 && size.height <= 42);
                    bytes += size.width as usize * size.height as usize * 4;
                }
            }
            assert!(bytes <= preview::PICTURE_BYTES);
            assert!(bytes > 0);
        }
        view.preview_image(&ui, "photo-499", Some(&picture));
        assert!(
            !view.rows[499]
                .preview_runs
                .iter()
                .any(|run| run.has_picture)
        );
        ui.set_text_size(24);
        view.preview_image(&ui, "photo-0", Some(&picture));
        assert_eq!(
            view.rows[0]
                .preview_runs
                .row_data(0)
                .unwrap()
                .picture
                .size()
                .width,
            69
        );
        // Repeated images share their allocation, including repeated source runs.
        let source = view.rows[0].preview_runs.row_data(0).unwrap();
        view.rows[0].preview_runs = ModelRc::new(VecModel::from(vec![source; 1000]));
        view.filter(&ui);
        view.retain_previews(&ui);
        let pictures: Vec<_> = view.rows[0]
            .preview_runs
            .iter()
            .filter(|run| run.has_picture)
            .map(|run| run.picture)
            .collect();
        assert!(!pictures.is_empty());
        assert!(pictures.iter().all(|picture| *picture == pictures[0]));
        assert!(view.preview_budget(&ui) <= preview::PICTURE_BYTES);
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn emoji_loading_keeps_preview_geometry_and_media_labels_drop_only_the_icon() {
    let mut row = ChatRow::default();
    preview::set(&mut row, &message_row(emoji_message()), 15);
    let measure = |s: &str| s.chars().count() as f32 * 7.;
    let before = preview::layout(&row.preview_runs, "You: ", 200., 14., measure);
    preview::image(&row, "emoji-key", Some(&demo::avatar(0)), 15);
    let after = preview::layout(&row.preview_runs, "You: ", 200., 14., measure);
    let geometry = |runs: ModelRc<RichRun>| {
        runs.iter()
            .map(|run| (run.x, run.measured_width))
            .collect::<Vec<_>>()
    };
    assert_eq!(geometry(before), geometry(after));
    for chat in demo::preview_examples().into_iter().skip(5) {
        let message = &chat.messages[0];
        preview::set(&mut row, message, 15);
        if !message.image_key.is_empty() {
            assert_eq!(
                row.preview_runs.row_data(0).unwrap().contain,
                message.kind == MessageKind::Sticker
            );
            let loaded = preview::layout(&row.preview_runs, "", 1000., 14., measure);
            let text = loaded
                .iter()
                .map(|run| run.text.to_string())
                .collect::<String>();
            if message.has_picture {
                assert_eq!(text, message.body.split_once(' ').unwrap().1);
            } else {
                assert_eq!(text, message.body.as_str());
            }
        }
        assert_eq!(
            message.time,
            display_time(message.sort_key.parse().unwrap())
        );
    }
}

#[test]
fn demo_preview_history_uses_the_list_timestamp_after_a_delay() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let timestamp = now_millis() - 5 * 60_000;
        let mut view = View {
            demo: true,
            demo_previews: true,
            ids: vec!["0".into()],
            rows: vec![ChatRow {
                title: "One LINE emoji".into(),
                activity: order_key(timestamp).into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        view.filter(&ui);
        view.select(&ui, 0);
        assert_eq!(view.messages[0].sort_key, order_key(timestamp));
        assert_eq!(view.messages[0].time, display_time(timestamp));
        assert_eq!(view.messages[0].delivered_time, view.messages[0].sort_key);
    })
    .join()
    .unwrap();
}

#[test]
fn scrolling_previews_request_only_missing_pictures() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        let mut view = View { demo: true, commands: Some(tx.into()), ..Default::default() };
        for i in 0..500 {
            view.ids.push(i.to_string());
            let mut row = ChatRow { index: i, title: format!("Photo {i:03}").into(), ..Default::default() };
            preview::set(&mut row, &MessageRow { kind: MessageKind::Photo, body: "▧ Photo".into(), image_key: format!("photo-{i}").into(), ..Default::default() }, 15);
            view.rows.push(row);
        }
        view.filter(&ui);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        let picture = demo::thumbnail(false);
        let mut requested = 0;
        let mut loaded = 0;
        for step in 0..=40 {
            ui.set_list_y(-(step as f32 * ui.get_chat_row_height()));
            view.read_exposure(&ui, Vec::new());
            for command in rx.try_iter() {
                if let account::Command::PreviewAssets { keys } = command {
                    for key in keys {
                        if step > 0 {
                            requested += 1;
                            loaded += usize::from(view.rows.iter().any(|row| row.preview_runs.iter().any(|run| run.image_key == key && run.has_picture)));
                        }
                        view.preview_image(&ui, &key, Some(&picture));
                    }
                }
            }
        }
        eprintln!("500 photos, 40 one-row steps: {requested} requests ({:.2}/step), {loaded} already loaded", requested as f32 / 40.);
        assert_eq!(loaded, 0);
        assert_eq!(requested, 40);
        view.read_exposure(&ui, Vec::new());
        assert!(!rx.try_iter().any(|command| matches!(command, account::Command::PreviewAssets { .. })));
        // Overlapping requests stay in flight until completion, even while scrolling.
        ui.set_list_y(-(41. * ui.get_chat_row_height()));
        view.read_exposure(&ui, Vec::new());
        let pending: Vec<_> = rx.try_iter().filter_map(|command| match command { account::Command::PreviewAssets { keys } => Some(keys), _ => None }).flatten().collect();
        assert_eq!(pending.len(), 1);
        ui.set_list_y(-(42. * ui.get_chat_row_height()));
        view.read_exposure(&ui, Vec::new());
        let next: Vec<_> = rx.try_iter().filter_map(|command| match command { account::Command::PreviewAssets { keys } => Some(keys), _ => None }).flatten().collect();
        assert_eq!(next.len(), 1);
        assert!(!next.contains(&pending[0]));
        ui.hide().unwrap();
    }).join().unwrap();
}

#[test]
fn tall_previews_share_emoji_pictures_and_keep_every_visible_row() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.set_text_size(24);
        let mut view = View {
            demo: true,
            ..Default::default()
        };
        for i in 0..500 {
            view.ids.push(i.to_string());
            let mut runs = (0..3)
                .flat_map(|j| {
                    let run = RichRun {
                        emoji: true,
                        text: "[笑]".into(),
                        image_key: format!("{i}-{j}").into(),
                        ..Default::default()
                    };
                    [run.clone(), run]
                })
                .collect::<Vec<_>>();
            runs.push(RichRun {
                emoji: true,
                image_key: "shared".into(),
                ..Default::default()
            });
            // Reverse canonical order to exercise viewport-order retention.
            view.rows.push(ChatRow {
                index: i,
                activity: format!("{i:04}").into(),
                title: format!("Emoji {i:03}").into(),
                preview_runs: ModelRc::new(VecModel::from(runs)),
                ..Default::default()
            });
        }
        view.filter(&ui);
        ui.show().unwrap();
        snapshot::render(&window, (980, 2160)).unwrap();
        ui.set_list_y(-(10. * ui.get_chat_row_height()));
        let visible = view.preview_window(&ui, 0);
        assert!(visible.len() >= 20);
        let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(384, 384));
        let window = view.preview_window(&ui, 2);
        for index in &window {
            for j in 0..3 {
                view.preview_image(&ui, &format!("{index}-{j}"), Some(&picture));
            }
        }
        view.preview_image(&ui, "shared", Some(&picture));
        view.retain_previews(&ui);
        let mut pictures = HashMap::new();
        for row in &view.rows {
            for run in row.preview_runs.iter() {
                if window.contains(&row.index) {
                    assert!(
                        run.has_picture,
                        "dropped {} in row {}",
                        run.image_key, row.index
                    );
                    assert_eq!(run.picture.size().width, 69);
                    if let Some(shared) =
                        pictures.insert(run.image_key.clone(), run.picture.clone())
                    {
                        assert_eq!(shared, run.picture);
                    }
                } else {
                    assert!(!run.has_picture);
                }
            }
        }
        let bytes: usize = pictures
            .values()
            .map(|picture| {
                let size = picture.size();
                size.width as usize * size.height as usize * 4
            })
            .sum();
        assert!(bytes > 1024 * 1024);
        eprintln!(
            "tall text-24 window: {} visible rows, {} shared images, {bytes} bytes, {} byte budget",
            visible.len(),
            pictures.len(),
            view.preview_budget(&ui)
        );
        assert!(bytes <= view.preview_budget(&ui));
        assert!(view.preview_budget(&ui) <= preview::PICTURE_BYTES);
        assert!(view.preview_keys(&ui).is_empty());
        ui.set_list_y(-(11. * ui.get_chat_row_height()));
        view.retain_previews(&ui);
        assert!(view.preview_keys(&ui).is_empty());
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
