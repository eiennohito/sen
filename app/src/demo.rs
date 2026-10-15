use super::*;
pub(super) struct Conversation {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub preview: &'static str,
    pub initial: &'static str,
    pub messages: Vec<MessageRow>,
}

fn message(body: &str, time: &str, outgoing: bool) -> MessageRow {
    MessageRow {
        id: format!("demo-{time}-{body}").into(),
        sort_key: order_key(
            chrono::NaiveDateTime::parse_from_str(time, "%Y/%m/%d %H:%M")
                .ok()
                .or_else(|| {
                    chrono::NaiveTime::parse_from_str(time, "%H:%M")
                        .ok()
                        .map(|time| {
                            chrono::NaiveDate::from_ymd_opt(2026, 10, 15)
                                .unwrap()
                                .and_time(time)
                        })
                })
                .map(|time| time.and_utc().timestamp_millis())
                .unwrap_or(0),
        )
        .into(),
        body: body.into(),
        time: if time.len() == 5 {
            format!("2026/10/15 {time}").into()
        } else {
            time.into()
        },
        outgoing,
        ..Default::default()
    }
}

fn group_fixture() -> Vec<MessageRow> {
    [
        ("Morning!", "2026/10/15 09:18", "aoi", "葵", false),
        ("Shall we walk?", "2026/10/15 09:19", "aoi", "葵", false),
        ("Yes!", "2026/10/15 09:20", "ren", "蓮", false),
        (
            "The riverside route looks good.",
            "2026/10/15 09:21",
            "self",
            "",
            true,
        ),
        ("See you there 🐈", "2026/10/15 09:22", "self", "", true),
        ("Bring a jacket.", "2026/10/15 09:27", "self", "", true),
        ("On my way!", "2026/10/16 09:18", "aoi", "葵", false),
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
            subtitle: "Example conversation",
            preview: "また明日、駅の前で。",
            initial: "葵",
            messages: vec![
                message("明日は何時に待ち合わせる？", "10:41", false),
                message("11時でどう？ コーヒーも飲もう。", "10:42", true),
                message("いいね！\nまた明日、駅の前で。", "10:43", false),
            ],
        },
        Conversation {
            title: "Weekend plans",
            subtitle: "Example group · 3 members",
            preview: "The riverside route looks good.",
            initial: "W",
            messages: group_fixture(),
        },
        Conversation {
            title: "Notes to self",
            subtitle: "Example conversation",
            preview: "A quiet place for a thought.",
            initial: "N",
            messages: vec![message(
                "A quiet place for a thought.",
                "2026/10/14 18:00",
                true,
            )],
        },
    ]
}

/// All scenario events are local fixtures, even when completions are delayed.
pub(super) fn scenario(state: &Rc<RefCell<View>>, ui: &AppWindow, kind: i32) {
    if !state.borrow().demo {
        return;
    }
    if kind == 7 {
        let mut view = state.borrow_mut();
        let current = view.current.clone();
        view.saved.outbox.retain(|item| item.chat != current);
        view.messages = group_fixture();
        view.render(ui);
        ui.invoke_show_history(0., false);
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
            time: "2026/10/15 12:09".into(),
            ..Default::default()
        }];
        view.render(ui);
        ui.invoke_show_history(0., false);
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
                time: "2026/10/15 12:05".into(),
                sort_key: order_key(100).into(),
                ..Default::default()
            },
            MessageRow {
                sort_key: order_key(200).into(),
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
                time: "2026/10/15 12:07".into(),
                sort_key: order_key(300).into(),
                ..Default::default()
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
    if kind == 4 {
        state
            .borrow_mut()
            .update(ui, account::Update::ConnectionFailed);
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
                            &format!("到着 {} · 🐈 synthetic burst {}", i + 1, view.serial),
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
    } else if kind == 3 {
        let mut view = state.borrow_mut();
        view.serial += 1;
        let local_id = format!("synthetic-outgoing-{}", view.serial);
        view.saved.outbox.push(storage::Outgoing {
            local_id: local_id.clone(),
            timestamp_ms: i64::MAX,
            chat: chat.clone(),
            text: "Synthetic submission · no network request".into(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        view.render(ui);
        ui.invoke_latest();
        drop(view);
        let weak = ui.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
            if let Some(ui) = weak.upgrade() {
                state
                    .borrow_mut()
                    .update(&ui, account::Update::SendFailed { id: chat, local_id });
            }
        });
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
