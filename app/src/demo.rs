use super::*;
pub(super) struct Conversation {
    pub title: &'static str,
    pub preview: &'static str,
    pub initial: &'static str,
    pub messages: Vec<MessageRow>,
}

fn fixture_time(time: &str) -> i64 {
    fixture_time_at(
        time,
        chrono::DateTime::from_timestamp_millis(now_millis())
            .unwrap()
            .with_timezone(&chrono::Local),
    )
}

pub(super) fn fixture_time_at<Tz: chrono::TimeZone>(time: &str, now: chrono::DateTime<Tz>) -> i64 {
    use chrono::Timelike;
    let reference = chrono::NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
    let parsed =
        chrono::NaiveDateTime::parse_from_str(time, "%Y/%m/%d %H:%M").unwrap_or_else(|_| {
            reference.and_time(chrono::NaiveTime::parse_from_str(time, "%H:%M").unwrap())
        });
    let today = now.date_naive();
    let timezone = now.timezone();
    let timestamp = now.timestamp_millis();
    if parsed.date() == reference {
        // A midnight gap starts the day at its first valid local minute.
        let start = (0..=now.hour() * 60 + now.minute())
            .find_map(|minute| {
                let local = today.and_hms_opt(minute / 60, minute % 60, 0)?;
                timezone
                    .from_local_datetime(&local)
                    .earliest()
                    .map(|time| time.timestamp_millis())
            })
            .unwrap_or(timestamp);
        let minute = parsed.hour() as i64 * 60 + parsed.minute() as i64;
        let age = (12 * 60 + 12 - minute).max(1) * 60_000;
        let span = (174 * 60_000).max(age);
        return timestamp - age * (timestamp - start).min(span) / span;
    }
    let days = parsed.date() - reference;
    let date = today + days;
    timezone
        .from_local_datetime(&date.and_time(parsed.time()))
        .earliest()
        .map(|time| time.timestamp_millis())
        .unwrap_or(timestamp + days.num_days() * 86_400_000)
}

fn message(body: &str, time: &str, outgoing: bool) -> MessageRow {
    let timestamp = fixture_time(time);
    MessageRow {
        id: format!("demo-{time}-{body}").into(),
        sort_key: order_key(timestamp).into(),
        delivered_time: timestamp.to_string().into(),
        body: body.into(),
        time: display_time(timestamp).into(),
        outgoing,
        ..Default::default()
    }
}

fn group_fixture() -> Vec<MessageRow> {
    [
        ("Morning!", "2026/10/17 09:18", "aoi", "葵", false),
        ("Shall we walk?", "2026/10/17 09:19", "aoi", "葵", false),
        ("Yes!", "2026/10/17 09:20", "ren", "蓮", false),
        (
            "The riverside route looks good.",
            "2026/10/17 09:21",
            "self",
            "",
            true,
        ),
        ("See you there 🐈", "2026/10/17 09:22", "self", "", true),
        ("Bring a jacket.", "2026/10/17 09:27", "self", "", true),
        ("On my way!", "2026/10/18 09:18", "aoi", "葵", false),
    ]
    .into_iter()
    .map(|(body, time, id, name, outgoing)| MessageRow {
        sender_id: id.into(),
        sender_name: name.into(),
        sender_avatar: avatar(if id == "aoi" { 0 } else { 1 }),
        has_sender_avatar: !outgoing,
        ..message(body, time, outgoing)
    })
    .collect()
}

pub(super) fn examples() -> Vec<Conversation> {
    vec![
        Conversation {
            title: "葵",
            preview: "また明日、駅の前で。",
            initial: "葵",
            messages: vec![
                message("去年の秋の散歩、楽しかったね。", "2025/10/18 15:20", false),
                message("今週も川沿いのカフェに行こう。", "2026/10/15 18:30", true),
                message(
                    "メニューはこちら: https://example.org/cafe",
                    "2026/10/17 19:10",
                    false,
                ),
                message(
                    "駅前のカフェは11時からだって。窓際の席で本を読みながら待っているね。If it rains, we can spend the afternoon there and take the riverside walk another day. ☕",
                    "10:36",
                    false,
                ),
                message("明日は何時に待ち合わせる？", "10:41", false),
                message("11時でどう？ コーヒーも飲もう。", "10:42", true),
                message("いいね！\nまた明日、駅の前で。", "10:43", false),
            ],
        },
        Conversation {
            title: "Weekend plans",
            preview: "The riverside route looks good.",
            initial: "W",
            messages: group_fixture(),
        },
        Conversation {
            title: "Notes to self",
            preview: "A quiet place for a thought.",
            initial: "N",
            messages: vec![message(
                "A quiet place for a thought.",
                "2025/10/18 18:00",
                true,
            )],
        },
    ]
}

/// All scenario events are local fixtures, even when completions are delayed.
pub(super) fn scenario(state: &Rc<RefCell<View>>, ui: &AppWindow, kind: i32) {
    // Switching synthetic fixtures must not inherit the paging fixture's range.
    if !matches!(kind, 1 | 2 | 3 | 4 | 9 | 10 | 16 | 17 | 18 | 19) {
        let mut view = state.borrow_mut();
        if view.demo_paging {
            view.demo_paging = false;
            let current = view.current.clone();
            view.paging.borrow_mut().remove(&current);
            view.recent.remove(&current);
            ui.global::<State>().set_paging_demo(false);
            ui.global::<State>().set_history_needs_refresh(false);
        }
    }

    if !state.borrow().demo {
        return;
    }
    if (20..30).contains(&kind) {
        let mut view = state.borrow_mut();
        view.messages = vec![media_failure((kind - 20) as usize)];
        view.render(ui);
        ui.invoke_show_history(0., true);
        return;
    }
    if kind == 15 {
        let mut view = state.borrow_mut();
        for number in 0..40 {
            let id = format!("browser-{number}");
            if view.ids.contains(&id) {
                continue;
            }
            let index = view.ids.len() as i32;
            view.ids.push(id);
            view.rows.push(ChatRow {
                index,
                title: format!("Browser friend {number:02}").into(),
                time: "New".into(),
                preview: "No messages yet".into(),
                initial: "B".into(),
                ..Default::default()
            });
        }
        view.filter(ui);
        return;
    }
    if kind == 14 {
        let mut view = state.borrow_mut();
        let chat = view.current.clone();
        view.unread.chats.remove(&chat);
        view.unread.server(&chat, Some(0), None);
        let ids = ["90001", "90002", "90003"];
        view.unread
            .incoming(&chat, ids.into_iter().map(str::to_owned));
        for id in ids {
            if !view.messages.iter().any(|m| m.id == id) {
                view.messages.push(MessageRow {
                    id: id.into(),
                    body: "新しいメッセージ 🐈".into(),
                    ..message("新しいメッセージ 🐈", "12:10", false)
                });
            }
        }
        if let Some(other) = view.ids.iter().find(|id| **id != chat).cloned() {
            view.unread.server(&other, Some(0), None);
            view.unread
                .incoming(&other, ["80001", "80002"].into_iter().map(str::to_owned));
        }
        ui.set_following(false);
        ui.set_new_count(3);
        view.unread_page(ui);
        view.render(ui);
        return;
    }
    if kind == 12 {
        let mut view = state.borrow_mut();
        view.messages = rich_fixture();
        view.render(ui);
        ui.invoke_show_history(0., false);
        return;
    }
    if kind == 9 {
        ui.set_loading_history(!ui.get_loading_history());
        ui.set_history_error("".into());
        return;
    }
    if kind == 8 {
        let mut view = state.borrow_mut();
        let current = view.current.clone();
        view.saved.outbox.retain(|item| item.chat != current);
        view.messages = [
            MessageKind::Text,
            MessageKind::Photo,
            MessageKind::Sticker,
            MessageKind::Video,
            MessageKind::Voice,
            MessageKind::File,
            MessageKind::Location,
            MessageKind::Contact,
            MessageKind::System,
            MessageKind::Withdrawn,
            MessageKind::Undecryptable,
            MessageKind::Unknown,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| MessageRow {
            id: format!("reading-{i}").into(),
            kind,
            body: crate::links::placeholder(kind)
                .unwrap_or("参照「https://example.org/reading」。またはwww.example.jp、どう？")
                .into(),
            sender_name: "葵".into(),
            ..message("", &format!("12:{i:02}"), false)
        })
        .collect();
        view.render(ui);
        ui.invoke_show_history(0., false);
        return;
    }
    if kind == 7 {
        let mut view = state.borrow_mut();
        let current = view.current.clone();
        view.saved.outbox.retain(|item| item.chat != current);
        view.messages = group_fixture();
        view.render(ui);
        ui.invoke_show_history(0., true);
        return;
    }
    if kind == 6 {
        let key = "synthetic-sticker".to_string();
        let mut view = state.borrow_mut();
        let current = view.current.clone();
        view.saved.outbox.retain(|item| item.chat != current);
        view.messages = vec![MessageRow {
            id: key.clone().into(),
            sticker: true,
            body: "[sticker]".into(),
            image_key: key.clone().into(),
            ..message("[sticker]", "12:09", false)
        }];
        view.render(ui);
        ui.invoke_show_history(0., true);
        drop(view);
        let weak = ui.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().update(
                    &ui,
                    account::Update::Image {
                        key,
                        pixels: line_wasm_poc::client::desktop::assets::Pixels {
                            width: 2,
                            height: 1,
                            rgba: vec![54, 130, 88, 255, 196, 220, 173, 255],
                        }
                        .into(),
                    },
                );
            }
        });
        return;
    }
    if kind == 5 {
        let mut view = state.borrow_mut();
        view.messages = vec![
            MessageRow {
                id: "demo-photo-wide".into(),
                photo: true,
                body: "Photo".into(),
                image_key: "demo-photo-wide".into(),
                picture: thumbnail(false),
                has_picture: true,
                outgoing: true,
                ..message("Photo", "12:05", true)
            },
            MessageRow {
                ..message(
                    "写真の下に時間が表示されます。\nA message following the photo.",
                    "12:06",
                    false,
                )
            },
            MessageRow {
                id: "demo-photo-portrait".into(),
                photo: true,
                body: "Photo".into(),
                image_key: "demo-photo-portrait".into(),
                picture: thumbnail(true),
                has_picture: true,
                ..message("Photo", "12:07", false)
            },
        ];
        let current = view.current.clone();
        view.saved
            .outbox
            .retain(|outgoing| outgoing.chat != current);
        view.render(ui);
        ui.invoke_show_history(0., false);
        return;
    }
    if matches!(kind, 4 | 16 | 17 | 18 | 19) {
        use line_wasm_poc::client::desktop::sync::SyncState;
        let event = match kind {
            4 => account::Update::ConnectionFailed(account::ConnectionFailure::Session),
            16 => account::Update::Sync(SyncState::Reconnecting { attempt: 1 }),
            17 => account::Update::Sync(SyncState::Offline),
            19 => account::Update::Sync(SyncState::AuthExpired),
            _ => account::Update::ChatsFailed(account::ListRequest::Refresh),
        };
        if kind == 16 {
            state
                .borrow_mut()
                .update(ui, account::Update::Sync(SyncState::Connected));
        }
        state.borrow_mut().update(ui, event);
        if kind == 16 {
            let weak = ui.as_weak();
            let state = state.clone();
            slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                if let Some(ui) = weak.upgrade()
                    && ui.get_retrying_connection()
                    && !ui.get_connection_failed()
                {
                    state
                        .borrow_mut()
                        .update(&ui, account::Update::Sync(SyncState::Connected));
                }
            });
        }
        return;
    }
    if kind == 0 {
        let mut view = state.borrow_mut();
        view.messages = (0..40).map(|i| {
            let text = match i % 4 {
                0 => format!("{i}: 日本語の長いメッセージ 🐈\n二行目を選択してコピーできます。\nA final line with emoji 👩🏽‍💻"),
                1 => format!("{i}: https://example.invalid/{}", "averylongunbrokenpath".repeat(8)),
                _ => format!("{i}: compact message"),
            };
            message(&text, "2026/10/15 12:00", i % 3 == 0)
        }).collect();
        view.render(ui);
        ui.invoke_show_history(0., true);
        return;
    }
    let chat = state.borrow().current.clone();
    if kind == 1 {
        let weak = ui.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(350), move || {
            if let Some(ui) = weak.upgrade() {
                let mut view = state.borrow_mut();
                view.serial += 1;
                let rows = (0..3)
                    .map(|i| {
                        message(
                            &format!("到着 {} · コーヒーを買っていくね 🐈 {}", i + 1, view.serial),
                            "12:01",
                            false,
                        )
                    })
                    .collect::<Vec<_>>();
                if view.current == chat {
                    view.messages.extend(rows);
                    if !ui.get_following() {
                        ui.set_new_count(ui.get_new_count() + 3);
                    }
                    view.render(&ui);
                } else if let Some(history) = view.history.get_mut(&chat) {
                    history.messages.extend(rows);
                }
            }
        });
    } else if kind == 2 {
        let mut view = state.borrow_mut();
        view.serial += 1;
        let key = format!("synthetic-media-{}", view.serial);
        // Complete media on an existing message above the reader, rather than
        // disguising a fourth arrival as part of the three-message burst.
        let Some(message) = view.messages.first_mut() else {
            return;
        };
        message.image_key = key.clone().into();
        message.photo = true;
        message.has_picture = false;
        message.image_failed = false;
        message.picture = Default::default();
        view.render(ui);
        drop(view);
        let weak = ui.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().update(
                    &ui,
                    account::Update::Image {
                        key,
                        pixels: line_wasm_poc::client::desktop::assets::Pixels {
                            width: 2,
                            height: 1,
                            rgba: vec![54, 130, 88, 255, 196, 220, 173, 255],
                        }
                        .into(),
                    },
                );
            }
        });
    } else if matches!(kind, 3 | 10 | 30 | 31) {
        let mut view = state.borrow_mut();
        view.demo_send_failure.store(
            if matches!(kind, 10 | 31) { 2 } else { 1 },
            std::sync::atomic::Ordering::Relaxed,
        );
        if kind < 30 {
            let draft = ui.get_draft();
            ui.set_draft("駅に着いたら連絡してね。See you soon! 🐈".into());
            if !view.submit(ui) {
                view.demo_send_failure
                    .store(0, std::sync::atomic::Ordering::Relaxed);
            }
            ui.set_draft(draft);
            view.persist(ui);
        }
    }
}

/// High-resolution local fixture; source pixels remain visible when zoomed.
pub(super) fn photo(portrait: bool) -> slint::Image {
    let (width, height) = if portrait { (800, 1200) } else { (1200, 800) };
    render_photo(width, height)
}
pub(super) fn thumbnail(portrait: bool) -> slint::Image {
    let (width, height) = if portrait { (256, 384) } else { (384, 256) };
    render_photo(width, height)
}
fn render_photo(width: u32, height: u32) -> slint::Image {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(width, height);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let (source_width, source_height) = if width > height {
                (1200, 800)
            } else {
                (800, 1200)
            };
            let source_x = x * source_width / width as usize;
            let source_y = y * source_height / height as usize;
            let grid = source_x % 100 < 3 || source_y % 100 < 3;
            pixels.make_mut_slice()[y * width as usize + x] = slint::Rgba8Pixel {
                r: if grid {
                    255
                } else {
                    (source_x * 200 / source_width + 30) as u8
                },
                g: if grid {
                    255
                } else {
                    (source_y * 180 / source_height + 40) as u8
                },
                b: 100,
                a: 255,
            };
        }
    }
    slint::Image::from_rgba8(pixels)
}

pub(super) fn avatar(index: usize) -> slint::Image {
    let mut rgba = vec![0; 80 * 120 * 4];
    for y in 0..120usize {
        for x in 0..80usize {
            let i = (y * 80 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[
                80 + index as u8 * 40,
                (80 + x) as u8,
                (50 + y) as u8,
                255,
            ]);
        }
    }
    let pixels = line_wasm_poc::client::desktop::assets::circular_avatar(
        &line_wasm_poc::client::desktop::assets::Pixels {
            width: 80,
            height: 120,
            rgba,
        },
    );
    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &pixels.rgba,
        pixels.width,
        pixels.height,
    ))
}

pub(crate) fn rich_fixture() -> Vec<MessageRow> {
    let mut target = message(
        "前のメッセージ — 日本語と emoji 🌸",
        "2026/10/17 12:00",
        false,
    );
    target.id = "rich-target".into();
    let body = "日😀[笑]本🌸 longunbrokenabcdefghijklmnopqrstuvwxyz0123456789\n二行目のテキスト";
    let span = line_wasm_poc::client::desktop::rich::InlineEmoji {
        start_utf16: 3,
        end_utf16: 6,
        start_byte: 7,
        end_byte: 12,
        fallback: "[笑]".into(),
        product_id: "synthetic".into(),
        sticon_id: "synthetic".into(),
        version: None,
        animated: false,
    };
    let mut row = message(body, "2026/10/17 12:01", false);
    row.id = "rich-emoji".into();
    row.rich_runs = crate::rich_content::runs(
        body,
        &[account::EmojiView {
            span,
            image_key: "demo-inline".into(),
        }],
    );
    if let Some(runs) = row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>() {
        for i in 0..runs.row_count() {
            let mut run = runs.row_data(i).unwrap();
            if run.emoji {
                run.picture = avatar(0);
                run.has_picture = true;
                runs.set_row_data(i, run);
            }
        }
    }
    row.reply_id = target.id.clone();
    row.reply_preview = "葵: 前のメッセージ".into();
    let mut rows = vec![target, row];
    for (kind, body, detail) in [
        (MessageKind::File, "資料.txt", "2.0 KB"),
        (MessageKind::Video, "Video", "2:05"),
        (MessageKind::Voice, "Voice message", "0:12"),
        (MessageKind::Contact, "Contact · 葵", ""),
        (MessageKind::System, "葵 joined the group", ""),
        (MessageKind::Withdrawn, "Message withdrawn", ""),
    ] {
        let mut row = message(body, "2026/10/17 12:02", false);
        row.id = format!("rich-{kind:?}").into();
        row.kind = kind;
        row.detail = detail.into();
        rows.push(row);
    }
    rows
}

/// Generate only the requested synthetic page, never a retained thousand-row model.
pub(super) fn paged_rows(range: std::ops::Range<usize>) -> Vec<MessageRow> {
    range
        .map(|n| {
            let timestamp = fixture_time("12:00") - (1000 - n) as i64 * 60_000;
            MessageRow {
                id: format!("paged-{n:04}").into(),
                body: format!("History item {n:04} · bounded page\n日本語 🐈").into(),
                sort_key: order_key(timestamp).into(),
                delivered_time: timestamp.to_string().into(),
                time: display_time(timestamp).into(),
                ..Default::default()
            }
        })
        .collect()
}

pub(crate) const MEDIA_FAILURES: [(&str, line_wasm_poc::client::desktop::assets::Failure); 10] = {
    use line_wasm_poc::client::desktop::assets::Failure::*;
    [
        ("timeout", Timeout),
        ("network", Network),
        ("404", Http(404)),
        ("403", Http(403)),
        ("500", Http(500)),
        ("unsupported", UnsupportedHost),
        ("path", RejectedPath),
        ("large", TooLarge),
        ("decode", Decode),
        ("cancelled", Cancelled),
    ]
};

pub(crate) fn media_failure(index: usize) -> MessageRow {
    let (name, failure) = MEDIA_FAILURES[index];
    MessageRow {
        id: format!("media-{name}").into(),
        image_key: format!("media-{name}").into(),
        kind: MessageKind::Photo,
        photo: true,
        image_failed: true,
        image_failure: format!("{failure:?}").into(),
        image_error: failure.image_text().into(),
        image_retryable: failure.retriable(),
        ..message("Photo", "12:00", false)
    }
}
