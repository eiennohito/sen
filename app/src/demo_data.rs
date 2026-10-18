//! Original demo content, supplied as decoded RPC rows and backend assets.
use crate::*;
use line_wasm_poc::client::desktop::{
    assets::{Asset, Pixels},
    rich::RichContent,
};
fn fixture_time(time: &str) -> i64 {
    fixture_time_at(
        time,
        chrono::DateTime::from_timestamp_millis(now_millis())
            .unwrap()
            .with_timezone(&chrono::Local),
    )
}

fn fixture_time_at<Tz: chrono::TimeZone>(time: &str, now: chrono::DateTime<Tz>) -> i64 {
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

pub(super) fn photo(portrait: bool) -> slint::Image {
    let (width, height) = if portrait { (800, 1200) } else { (1200, 800) };
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
    let pixels = std::thread::spawn(move || {
        crate::assets::circular_avatar(&line_wasm_poc::client::desktop::assets::Pixels {
            width: 80,
            height: 120,
            rgba,
        })
    })
    .join()
    .unwrap();
    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &pixels.rgba,
        pixels.width,
        pixels.height,
    ))
}

pub fn image_asset(fake: &FakeBackend, name: &str, image: slint::Image) -> Asset {
    let mut value = serde_json::to_value(FakeBackend::asset(true)).unwrap();
    value["url"] = format!("https://obs.line-apps.com/r/talk/m/{name}/preview").into();
    value["name"] = name.into();
    let asset: Asset = serde_json::from_value(value).unwrap();
    let pixels = image.to_rgba8().unwrap();
    fake.set_pixels(
        &asset,
        Pixels {
            width: pixels.width(),
            height: pixels.height(),
            rgba: pixels.as_bytes().to_vec(),
        },
    );
    if let Some(original) = asset.original() {
        fake.set_pixels(
            &original,
            Pixels {
                width: pixels.width(),
                height: pixels.height(),
                rgba: pixels.as_bytes().to_vec(),
            },
        );
    }
    asset
}

fn message(
    fake: &FakeBackend,
    chat: &str,
    id: i64,
    text: &str,
    time: &str,
    outgoing: bool,
) -> MessageInput {
    let mut row = FakeBackend::message(chat, id, text, outgoing);
    row.created_ms = fixture_time(time);
    row.delivered_ms = Some(row.created_ms);
    if !outgoing {
        fake.set_contact(Friend {
            id: row.sender_id.clone(),
            profile: ContactProfile {
                relation: Some(0),
                ..Default::default()
            },
            name: "葵".into(),
            avatar: Some(image_asset(fake, "avatar-aoi", avatar(0))),
            ..Default::default()
        });
    }
    row
}
pub(super) fn rich(row: &mut MessageInput, rich: RichContent) {
    let text = row.body.as_text().to_owned();
    row.reply = rich.reply.map(Box::new);
    row.body = if rich.withdrawn {
        Content::Withdrawn
    } else {
        match rich.content_type {
            0 => Content::Text {
                text,
                spans: rich.emoji,
            },
            1 => Content::Photo {
                asset: None,
                size: None,
                album_count: rich.album_count,
                caption: rich.caption,
            },
            2 => Content::Video {
                duration: rich.duration_ms,
                asset: None,
                preview: None,
                album_count: rich.album_count,
                caption: rich.caption,
            },
            3 => Content::Voice {
                duration: rich.duration_ms,
                asset: None,
            },
            7 => Content::Sticker {
                pkg: String::new(),
                id: String::new(),
                ver: None,
                asset: None,
                text: (!text.is_empty()).then_some(text),
            },
            13 => Content::Contact {
                mid: String::new(),
                name: rich.contact_name,
            },
            14 => Content::File {
                name: rich.file_name,
                size: rich.file_size,
                asset: None,
            },
            15 => Content::Location {
                title: None,
                lat: 0.,
                lon: 0.,
            },
            18 => Content::System {
                kind: rich.system_key,
                params: rich.system_args,
                text: rich.system_text,
            },
            content_type => Content::Unknown {
                content_type: content_type as i32,
            },
        }
    };
}
pub fn group(fake: &FakeBackend, chat: &str, base: i64) -> Vec<MessageInput> {
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
    .enumerate()
    .map(|(i, (body, time, sender, name, outgoing))| {
        let mut row = message(fake, chat, base + i as i64, body, time, outgoing);
        row.sender_id = sender.into();
        if !outgoing {
            fake.set_contact(Friend {
                id: sender.into(),
                name: name.into(),
                avatar: Some(image_asset(
                    fake,
                    &format!("avatar-{sender}"),
                    avatar(if sender == "aoi" { 0 } else { 1 }),
                )),
                ..Default::default()
            });
        }
        row
    })
    .collect()
}
pub fn rich_fixture(fake: &FakeBackend, chat: &str, base: i64) -> Vec<MessageInput> {
    let target = message(
        fake,
        chat,
        base,
        "前のメッセージ — 日本語と emoji 🌸",
        "2026/10/17 12:00",
        false,
    );
    let mut emoji = message(
        fake,
        chat,
        base + 1,
        "日😀[笑]本🌸 longunbrokenabcdefghijklmnopqrstuvwxyz0123456789\n二行目のテキスト",
        "2026/10/17 12:01",
        false,
    );
    let span = InlineEmoji {
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
    let asset = Asset::inline_emoji(&span).unwrap();
    let pixels = avatar(0).to_rgba8().unwrap();
    fake.set_pixels(
        &asset,
        Pixels {
            width: pixels.width(),
            height: pixels.height(),
            rgba: pixels.as_bytes().to_vec(),
        },
    );
    rich(
        &mut emoji,
        RichContent {
            emoji: vec![span],
            reply: Some(ReplyReference {
                message_id: target.server_id.0.clone(),
                service_code: None,
            }),
            ..Default::default()
        },
    );
    let mut rows = vec![target, emoji];
    for (i, (content_type, body)) in [
        (14, "資料.txt"),
        (2, "Video"),
        (3, "Voice message"),
        (13, "Contact · 葵"),
        (18, "葵 joined the group"),
        (1000, "Message unsent"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut row = message(
            fake,
            chat,
            base + 2 + i as i64,
            body,
            "2026/10/17 12:02",
            false,
        );
        rich(
            &mut row,
            RichContent {
                content_type,
                file_name: (content_type == 14).then(|| body.into()),
                file_size: (content_type == 14).then_some(2048),
                duration_ms: match content_type {
                    2 => Some(125000),
                    3 => Some(12000),
                    _ => None,
                },
                contact_name: (content_type == 13).then(|| "葵".into()),
                system_text: (content_type == 18).then(|| body.into()),
                withdrawn: content_type == 1000,
                ..Default::default()
            },
        );
        if let Content::File { asset, .. } = &mut row.body {
            *asset = Some(FakeBackend::asset(true));
        }
        rows.push(row);
    }
    rows
}
pub fn seed(fake: &FakeBackend) {
    let alice=[
        ("去年の秋の散歩、楽しかったね。","2025/10/18 15:20",false),
        ("今週も川沿いのカフェに行こう。","2026/10/15 18:30",true),
        ("メニューはこちら: https://example.org/cafe","2026/10/17 19:10",false),
        ("駅前のカフェは11時からだって。窓際の席で本を読みながら待っているね。If it rains, we can spend the afternoon there and take the riverside walk another day. ☕","10:36",false),
        ("明日は何時に待ち合わせる？","10:41",false),
        ("11時でどう？ コーヒーも飲もう。","10:42",true),
        ("いいね！\nまた明日、駅の前で。","10:43",false),
    ].into_iter().enumerate().map(|(i,(body,time,outgoing))|message(fake,"alice",100+i as i64,body,time,outgoing)).collect::<Vec<_>>();
    let pages = [
        ("alice", "葵", alice),
        ("weekend", "Weekend plans", group(fake, "weekend", 200)),
        (
            "notes",
            "Notes to self",
            vec![message(
                fake,
                "notes",
                300,
                "A quiet place for a thought.",
                "2025/10/18 18:00",
                true,
            )],
        ),
    ];
    let mut chats = Vec::new();
    for (i, (chat, title, rows)) in pages.iter().enumerate() {
        let last = rows.last().unwrap();
        if let Some(sent) = rows.iter().rev().find(|m| m.outgoing) {
            fake.set_receipt(ChatId((*chat).into()), sent.server_id.clone());
        }
        chats.push(ChatInput {
            id: ChatId((*chat).into()),
            title: (*title).into(),
            is_group: i == 1,
            member_count: (i == 1).then_some(25),
            avatar: Some(image_asset(fake, &format!("chat-avatar-{i}"), avatar(i))),
            last_message_id: Some(last.server_id.clone()),
            last_message_ms: Some(last.created_ms),
            ..Default::default()
        });
    }
    let mut friends = chats
        .iter()
        .enumerate()
        .map(|(i, c)| line_wasm_poc::store::Friend {
            profile: Default::default(),
            id: c.id.0.clone(),
            name: c.title.clone(),
            status: [
                "散歩とコーヒーが好き 🌿",
                "Saturday by the river",
                "Ideas for a quiet afternoon",
            ][i]
                .into(),
            group: c.is_group,
            avatar: c.avatar.clone(),
            member_count: c.member_count.map(|n| n as usize),
        })
        .collect::<Vec<_>>();
    friends.push(line_wasm_poc::store::Friend {
        profile: Default::default(),
        id: "new-friend".into(),
        name: "新しい友だち 🌸".into(),
        status: "まだ会話のない友だち".into(),
        group: false,
        avatar: None,
        member_count: None,
    });
    fake.set_friends(friends);
    fake.set_members(
        (0..26)
            .map(|i| line_wasm_poc::store::Member {
                resolved: true,
                is_self: i == 0,
                contact: line_wasm_poc::store::Friend {
                    profile: ContactProfile {
                        relation: Some(if i == 1 { 0 } else { 2 }),
                        invited: i == 25,
                        ..Default::default()
                    },
                    id: match i {
                        0 => "self".into(),
                        1 => "alice".into(),
                        _ => format!("member-{i}"),
                    },
                    name: match i {
                        0 => "Offline preview".into(),
                        1 => "葵".into(),
                        2 => "蓮".into(),
                        24 => "Unknown member".into(),
                        _ => format!("Weekend walker {i:02}"),
                    },
                    status: String::new(),
                    group: false,
                    avatar: (i != 24)
                        .then(|| image_asset(fake, &format!("member-avatar-{i}"), avatar(i % 3))),
                    member_count: None,
                },
            })
            .collect(),
    );
    fake.set_chats(chats);
    for (chat, _, rows) in pages {
        fake.set_page(ChatId(chat.into()), rows);
    }
}

pub fn reading(fake: &FakeBackend, chat: &str) -> Vec<MessageInput> {
    [0, 1, 7, 2, 3, 14, 15, 13, 18, 18, 0, 999]
        .into_iter()
        .enumerate()
        .map(|(i, content_type)| {
            let mut row = message(
                fake,
                chat,
                700 + i as i64,
                if i == 0 {
                    "参照「https://example.org/reading」。またはwww.example.jp、どう？"
                } else {
                    ""
                },
                &format!("12:{i:02}"),
                false,
            );
            rich(
                &mut row,
                RichContent {
                    content_type,
                    withdrawn: i == 9,
                    ..Default::default()
                },
            );
            if i == 10 {
                row.body = Content::Undecryptable;
            }
            row
        })
        .collect()
}
pub fn history(fake: &FakeBackend, chat: &str) -> Vec<MessageInput> {
    (0..40).map(|i| {
        let text=match i%4 {0=>format!("{i}: 日本語の長いメッセージ 🐈\n二行目を選択してコピーできます。\nA final line with emoji 👩🏽‍💻"),1=>format!("{i}: https://example.invalid/{}","averylongunbrokenpath".repeat(8)),_=>format!("{i}: compact message")};
        message(fake,chat,4000+i,&text,"2026/10/15 12:00",i%3==0)
    }).collect()
}
pub fn photos(fake: &FakeBackend, chat: &str) -> Vec<MessageInput> {
    let text = message(
        fake,
        chat,
        5001,
        "写真の下に時間が表示されます。\nA message following the photo.",
        "12:06",
        false,
    );
    let mut rows: Vec<_> = [(false, "12:05"), (true, "12:07")]
        .into_iter()
        .enumerate()
        .map(|(i, (portrait, time))| {
            let mut row = message(fake, chat, 5000 + 2 * i as i64, "", time, !portrait);
            rich(
                &mut row,
                RichContent {
                    content_type: 1,
                    ..Default::default()
                },
            );
            *row.body.image_mut() = Some(image_asset(
                fake,
                if portrait {
                    "photo-portrait"
                } else {
                    "photo-landscape"
                },
                photo(portrait),
            ));
            row
        })
        .collect();
    rows.insert(1, text);
    rows
}
pub fn previews(fake: &FakeBackend) {
    let mut chats = Vec::new();
    let mut pages = Vec::new();
    for (i, (title, body)) in [
        ("One LINE emoji", "[笑]"),
        ("Several LINE emoji", "[笑][笑][笑]"),
        ("Mixed LINE emoji", "日😀[笑]本🌸 [笑] See you soon"),
        (
            "Long LINE emoji",
            "Hi [笑] a long preview [笑] with enough text to elide at the row width [笑] end",
        ),
        ("Missing LINE emoji", "Waiting [笑] for you"),
    ]
    .into_iter()
    .enumerate()
    {
        let chat = format!("preview-emoji-{i}");
        let mut row = message(fake, &chat, 6000 + i as i64, body, "10:43", i == 2);
        row.created_ms -= i as i64 * 60000;
        let emoji = body
            .match_indices("[笑]")
            .map(|(start, text)| InlineEmoji {
                start_utf16: body[..start].encode_utf16().count(),
                end_utf16: body[..start + text.len()].encode_utf16().count(),
                start_byte: start,
                end_byte: start + text.len(),
                fallback: text.into(),
                product_id: "synthetic".into(),
                sticon_id: if i == 4 { "missing" } else { "smile" }.into(),
                version: None,
                animated: false,
            })
            .collect::<Vec<_>>();
        for span in &emoji {
            let asset = Asset::inline_emoji(span).unwrap();
            if i == 4 {
                fake.set_asset_failure(
                    &asset,
                    line_wasm_poc::client::desktop::assets::Failure::Http(404),
                );
            } else {
                let image = avatar(0).to_rgba8().unwrap();
                fake.set_pixels(
                    &asset,
                    Pixels {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.as_bytes().to_vec(),
                    },
                );
            }
        }
        rich(
            &mut row,
            RichContent {
                emoji,
                ..Default::default()
            },
        );
        chats.push(ChatInput {
            id: ChatId(chat.clone()),
            title: title.into(),
            last_message_id: Some(row.server_id.clone()),
            last_message_ms: Some(row.created_ms),
            ..Default::default()
        });
        pages.push((ChatId(chat), vec![row]));
    }
    for (i, (title, content_type)) in [
        ("Photo preview", 1),
        ("Album preview", 1),
        ("Sticker preview", 7),
        ("Animated sticker preview", 7),
        ("Missing photo preview", 1),
        ("File preview", 14),
        ("Video preview", 2),
    ]
    .into_iter()
    .enumerate()
    {
        let chat = format!("preview-media-{i}");
        let mut row = message(
            fake,
            &chat,
            6100 + i as i64,
            match content_type {
                7 => "Nice!",
                14 => "walk.pdf",
                _ => "",
            },
            "10:43",
            false,
        );
        row.created_ms -= (5 + i) as i64 * 60000;
        rich(
            &mut row,
            RichContent {
                content_type,
                album_count: (i == 1).then_some(4),
                caption: match i {
                    0 => Some("Riverside walk".into()),
                    6 => Some("Weekend walk".into()),
                    _ => None,
                },
                ..Default::default()
            },
        );
        if content_type != 14 {
            let mut asset =
                serde_json::to_value(image_asset(fake, &chat, photo(content_type == 7))).unwrap();
            if i == 4 {
                asset["failure"] = serde_json::to_value(
                    line_wasm_poc::client::desktop::assets::Failure::Http(404),
                )
                .unwrap();
            }
            *row.body.image_mut() = Some(serde_json::from_value(asset).unwrap());
        }
        chats.push(ChatInput {
            id: ChatId(chat.clone()),
            title: title.into(),
            last_message_id: Some(row.server_id.clone()),
            last_message_ms: Some(row.created_ms),
            ..Default::default()
        });
        pages.push((ChatId(chat), vec![row]));
    }
    fake.set_chats(chats);
    for (chat, rows) in pages {
        fake.set_page(chat, rows);
    }
}

pub fn paged_rows(
    fake: &FakeBackend,
    chat: &str,
    range: std::ops::Range<usize>,
) -> Vec<MessageInput> {
    range
        .map(|n| {
            let mut row = message(
                fake,
                chat,
                10000 + n as i64,
                &format!("History item {n:04} · bounded page\n日本語 🐈"),
                "12:00",
                false,
            );
            row.created_ms -= (1000 - n) as i64 * 60000;
            row
        })
        .collect()
}
/// A sticker whose image arrives a second after its "[sticker]" fallback.
pub fn sticker(fake: &FakeBackend, chat: &str) -> MessageInput {
    let mut row = message(fake, chat, 5100, "[sticker]", "12:09", false);
    rich(
        &mut row,
        RichContent {
            content_type: 7,
            ..Default::default()
        },
    );
    let asset = image_asset(fake, "sticker", photo(true));
    fake.delay_asset(&asset, std::time::Duration::from_secs(1));
    *row.body.image_mut() = Some(asset);
    row
}
