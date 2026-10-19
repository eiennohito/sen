//! Synthetic data enters through the backend and follows the normal commit path.
use crate::*;
use line::backend::{LiveOp, Op};
pub(super) fn scenario(ui: &AppWindow, d: &mut Desktop, fake: &FakeBackend, kind: i32) {
    let chat = d.state.selected.clone().unwrap_or(ChatId("alice".into()));
    match kind {
        40..=46 => {
            use line::client::error::SignInFailure::*;
            fake.fail_sign_in(match kind {
                40 => CryptoModule("/demo/crypto.wasm".into()),
                41 => StateDirectory("/demo/state".into()),
                42 => Keyring,
                43 => Network,
                44 => Server,
                45 => Phone,
                _ => Cancelled,
            });
        }
        34 => {
            let directory =
                std::env::temp_dir().join(format!("line-demo-attachments-{}", std::process::id()));
            if std::fs::create_dir_all(&directory).is_ok() {
                let photo = directory.join("Holiday.png");
                let file = directory.join("Notes.txt");
                let image =
                    image::RgbaImage::from_pixel(160, 120, image::Rgba([58, 145, 191, 255]));
                if image.save(&photo).is_ok()
                    && std::fs::write(&file, b"Synthetic demo file").is_ok()
                {
                    let paths = [photo, file]
                        .iter()
                        .filter_map(|path| url::Url::from_file_path(path).ok())
                        .map(|url| url.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.global::<State>().invoke_drop_attachment(paths.into());
                }
            }
        }

        32 => {
            let rows = vec![
                FakeBackend::message("protocol", 6300, "Unsend success fixture", true),
                FakeBackend::message("protocol", 6301, "Unsend failure fixture", true),
                FakeBackend::official_image("protocol", 6302),
            ];
            fake.fail_rpc(
                line::backend::FakeRpc::Unsend(ServerId::new("6301").unwrap()),
                line::backend::Failure::Refused(line::client::error::Refusal::Gateway(13)),
            );
            fixture(ui, d, fake, "protocol", "Protocol fixtures", rows);
        }
        33 => {
            if let Some(row) = d
                .state
                .rows()
                .into_iter()
                .find(|m| m.server_id.is_some() && !m.body.is_withdrawn())
            {
                fake.inject_raw(serde_json::json!({"type":139,"param1":row.server_id.as_ref().unwrap().as_str(),
                    "param2":serde_json::json!({"chatMid":chat.0,"curr":{"predefinedReactionType":2}}).to_string()}));
            }
        }
        12 => {
            fixture(
                ui,
                d,
                fake,
                "rich",
                "Rich content",
                demo_data::rich_fixture(fake, "rich", 2000),
            );
        }
        0 => {
            let mut rows = fake.page(&chat);
            rows.extend(demo_data::history(fake, &chat.0));
            rows.sort_by_key(|m| m.created_ms);
            rows.dedup_by(|a, b| a.server_id == b.server_id);
            fake.set_page(chat.clone(), rows);
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Recent,
            });
            let intent = d.state.subscribe_window(Anchor::Latest, 80, 0);
            d.sender.send(intent);
            d.history.opening_history = false;
            ui.invoke_show_history(0., true);
        }
        8 => fixture(
            ui,
            d,
            fake,
            "reading",
            "Reading fixtures",
            demo_data::reading(fake, "reading"),
        ),
        14 => {
            demo_data::hidden_events(fake);
            for (target, count) in [(chat.clone(), 3), (ChatId("weekend".into()), 2)] {
                let mut rows = fake.page(&target);
                // New ids must sort after everything already read.
                let newest = rows
                    .iter()
                    .filter_map(|m| m.server_id.as_str().parse::<i64>().ok())
                    .max()
                    .unwrap_or(0);
                for n in 1..=count {
                    let id = newest + n;
                    let row = FakeBackend::message(&target.0, id, "新しいメッセージ 🐈", false);
                    rows.push(row.clone());
                    fake.inject(LiveOp {
                        revision: fake.revision(),
                        replay: false,
                        op: Op::Message((row).into()),
                    });
                }
                let mut snapshot = d.state.chats[&target].chat.clone();
                snapshot.server_unread = snapshot.unread_count + count as u32;
                snapshot.last_message_id = rows.last().map(|row| row.server_id.clone());
                snapshot.last_message_ms = rows.last().map(|row| row.created_ms);
                fake.add_chat(snapshot);
                fake.set_page(target, rows);
            }
            d.sender.send(Intent::MoreChats);
        }
        15 => {
            for i in 0..40 {
                fake.add_chat(ChatInput {
                    id: ChatId(format!("browser-{i}")),
                    title: format!("Browser friend {i:02}"),
                    ..Default::default()
                });
            }
            fake.add_chat(ChatInput {
                id: ChatId("new-friend".into()),
                title: "新しい友だち 🌸".into(),
                ..Default::default()
            });
            d.sender.send(Intent::MoreChats);
        }
        1 => {
            for i in 1..=3 {
                let id = fake.revision();
                fake.inject(LiveOp {
                    revision: id,
                    replay: false,
                    op: Op::Message(
                        (FakeBackend::message(&chat.0, id, &format!("到着 {i}"), false)).into(),
                    ),
                });
            }
        }
        5 => fixture(
            ui,
            d,
            fake,
            "photos",
            "Photo preview",
            demo_data::photos(fake, "photos"),
        ),
        6 => {
            let row = demo_data::sticker(fake, "stickers");
            fixture(ui, d, fake, "stickers", "Sticker preview", vec![row]);
        }
        7 => fixture(
            ui,
            d,
            fake,
            "groups",
            "Group preview",
            demo_data::group(fake, "groups", 5200),
        ),
        20..=29 => {
            let mut row = FakeBackend::message(&chat.0, fake.revision(), "", false);

            demo_data::rich(
                &mut row,
                line::client::desktop::rich::RichContent {
                    content_type: 1,
                    ..Default::default()
                },
            );
            let mut asset = serde_json::to_value(FakeBackend::asset(true)).unwrap();
            asset["failure"] = serde_json::to_value(media_failure((kind - 20) as usize)).unwrap();
            *row.body.image_mut() = Some(serde_json::from_value(asset).unwrap());
            fake.set_page(chat.clone(), vec![row]);
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Recent,
            });
            ui.set_following(true);
        }
        19 => fake.auth_expired(),
        3 => {
            fake.unanswered_send();
            d.sender.send(Intent::Send {
                chat: chat.clone(),
                text: "駅に着いたら連絡してね。See you soon! 🐈".into(),
                reply: None,
            });
            d.sender.send(Intent::Draft {
                chat,
                draft: Draft {
                    reply: None,
                    text: d.state.composer.clone(),
                    cursor: 0,
                    anchor: 0,
                    updated: line::engine::now_ms(),
                },
            });
        }
        10 | 31 => {
            fake.fail_send(chat.clone());
            if kind == 10 {
                d.sender.send(Intent::Send {
                    chat,
                    text: "Rejected message".into(),
                    reply: None,
                });
            }
        }
        30 => fake.fail_send(chat.clone()),
        17 => fake.offline(true),
        16 => fake.offline(false),
        _ => {}
    }
}

pub fn configure(fake: &FakeBackend, scene: &str, case: Option<&str>) {
    if scene == "signed-out" {
        if case == Some("missing-module") {
            fake.fail_open(Some(line::client::error::SignInFailure::CryptoModule(
                "/demo/crypto.wasm".into(),
            )));
        }
        return;
    }
    demo_data::seed(fake);
    fake.upload_delay(std::time::Duration::from_millis(1800));
    if scene == "previews" {
        demo_data::previews(fake);
    }
    if scene == "paging" {
        fake.set_page(
            ChatId("alice".into()),
            demo_data::paged_rows(fake, "alice", 0..1000),
        );
    }
    if let Some(case) = case
        && let Some(class) = case.strip_prefix("media-")
    {
        let index = [
            "timeout",
            "network",
            "404",
            "403",
            "500",
            "unsupported",
            "path",
            "large",
            "decode",
            "cancelled",
        ]
        .iter()
        .position(|v| *v == class)
        .unwrap();
        let mut row = FakeBackend::message("alice", 900, "", false);

        demo_data::rich(
            &mut row,
            line::client::desktop::rich::RichContent {
                content_type: 1,
                ..Default::default()
            },
        );
        let mut asset = serde_json::to_value(FakeBackend::asset(true)).unwrap();
        asset["failure"] = serde_json::to_value(media_failure(index)).unwrap();
        *row.body.image_mut() = Some(serde_json::from_value(asset).unwrap());
        fake.set_page(ChatId("alice".into()), vec![row]);
        return;
    }
    if let Some(case) = case {
        let body=match case {"long-url"=>format!("https://example.org/{}","averylongunbrokenpath".repeat(8)),"mixed"=>"Before https://www.example.jp after ordinary words. Next: https://example.org/a?b=c&name=one_two#part after.".into(),"cjk"=>"日本語の前置き https://www.example.jp。続きの日本語と https://example.org/長い/道?名前=値。終わり".into(),_=>"日😀[笑]本🌸".into()};
        fake.set_page(
            ChatId("alice".into()),
            vec![FakeBackend::message("alice", 900, &body, true)],
        );
    }
    if scene.starts_with("message-selected") {
        fake.set_page(
            ChatId("alice".into()),
            vec![FakeBackend::message(
                "alice",
                900,
                "選択したメッセージ\nSelected message",
                scene.ends_with("outgoing"),
            )],
        );
    }
    if scene == "rich" {
        fake.set_page(
            ChatId("alice".into()),
            demo_data::rich_fixture(fake, "alice", 900),
        );
    }
    if scene == "run-review" {
        fake.set_page(
            ChatId("weekend".into()),
            demo_data::run_review(fake, "weekend"),
        );
    }
    if scene == "quote-review" || scene == "reply-review" {
        fake.set_page(
            ChatId("alice".into()),
            demo_data::quote_review(fake, "alice"),
        );
    }
    if scene == "groups" {
        fake.set_page(ChatId("alice".into()), demo_data::group(fake, "alice", 900));
    }
    if scene == "photos" {
        let mut row = FakeBackend::message("alice", 900, "Riverside walk", false);

        demo_data::rich(
            &mut row,
            line::client::desktop::rich::RichContent {
                content_type: 1,
                ..Default::default()
            },
        );
        *row.body.image_mut() = Some(demo_data::image_asset(
            fake,
            "photo-landscape",
            demo_data::photo(false),
        ));
        fake.set_page(ChatId("alice".into()), vec![row]);
    }
}
pub fn install(ui: &AppWindow, actions: Runtime, fake: FakeBackend) {
    let weak = ui.as_weak();
    let links = actions.clone();
    ui.global::<State>().on_open_link(move |url| {
        let weak = weak.clone();
        links.push(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<State>()
                    .set_link_feedback(format!("Preview opened: {url}").into());
                ui.global::<State>().set_last_opened_link(url);
            }
        });
    });
    let weak = ui.as_weak();
    ui.on_demo_event(move |kind| {
        let weak = weak.clone();
        let fake = fake.clone();
        actions.push(move |d| {
            if let Some(ui) = weak.upgrade() {
                scenario(&ui, d, &fake, kind);
            }
        });
    });
}

fn media_failure(index: usize) -> line::client::desktop::assets::Failure {
    use line::client::desktop::assets::Failure::*;
    [
        Timeout,
        Network,
        Http(404),
        Http(403),
        Http(500),
        UnsupportedHost,
        RejectedPath,
        TooLarge,
        Decode,
        Cancelled,
    ][index]
}

fn fixture(
    ui: &AppWindow,
    d: &mut Desktop,
    fake: &FakeBackend,
    id: &str,
    title: &str,
    rows: Vec<MessageInput>,
) {
    let chat = ChatId(id.into());
    fake.add_chat(ChatInput {
        id: chat.clone(),
        title: title.into(),
        is_group: id == "groups",
        member_count: (id == "groups").then_some(25),
        ..Default::default()
    });
    fake.set_page(chat.clone(), rows.clone());
    fake.inject(LiveOp {
        revision: fake.revision(),
        replay: true,
        op: Op::Message((rows[0].clone()).into()),
    });
    d.capture_draft(ui);
    // A fixture changes the history only; the composer text stays. The draft row
    // would otherwise hide the new chat from detail loading, so load the chat list.
    d.sender.send(Intent::Draft {
        chat: chat.clone(),
        draft: Draft {
            reply: None,
            text: d.state.composer.clone(),
            cursor: ui.get_editor_caret() as usize,
            anchor: ui.get_editor_anchor() as usize,
            updated: line::engine::now_ms(),
        },
    });
    d.sender.send(Intent::MoreChats);
    let intent = d.state.select(chat.clone());
    d.history.opening_history = false;
    d.history.restore_anchor = None;
    d.history.geometry.clear();
    d.sender.send(intent);
    d.sender.send(Intent::Page {
        chat,
        kind: PageKind::Recent,
    });
    d.render(ui);
    ui.invoke_show_history(0., false);
}

pub(crate) fn stage(
    ui: &AppWindow,
    desktop: &Runtime,
    fake: Option<&FakeBackend>,
    scene: &str,
    snapshot_profile: bool,
) {
    ui.set_show_conversation(
        scene != "chats" && scene != "signed-out" && !scene.starts_with("friends"),
    );
    if scene == "settings" {
        desktop.update(|d| {
            d.state.panel = Panel::Settings;
            d.render(ui);
        });
    }
    if scene == "reply-review"
        && let Some(id) = desktop.update(|d| d.state.rows().first().map(|m| m.key.0.to_string()))
    {
        ui.global::<State>().invoke_reply(id.into());
    }
    if scene == "friends" {
        ui.invoke_browse(1);
    }
    if matches!(
        scene,
        "members" | "member-profile" | "group-header" | "room-header"
    ) {
        desktop.update(|d| d.select(ui, 1));
    }
    if scene == "run-review" {
        desktop.update(|d| {
            if let Some(index) = d.state.chat_order.iter().position(|id| id.0 == "weekend") {
                d.select(ui, index);
            }
        });
    }
    if scene.starts_with("members") || scene.starts_with("member-profile") {
        ui.invoke_open_members();
    }
    if snapshot_profile || scene == "member-profile" {
        ui.invoke_browse(1);
    }
    if scene.starts_with("message-selected")
        && let Some(key) = desktop.update(|d| d.state.rows().first().map(|m| m.key))
    {
        ui.global::<State>()
            .set_highlight_message(key.0.to_string().into());
    }
    if scene == "session-expired"
        && let Some(fake) = fake
    {
        fake.auth_expired();
    }
    if scene.ends_with("offline")
        && let Some(fake) = fake
    {
        fake.offline(true);
    }
    if scene == "failed-send"
        && let Some(fake) = fake
    {
        desktop.update(|d| {
            if let Some(chat) = d.state.selected.clone() {
                fake.fail_send(chat);
            }
        });
        ui.set_draft("See you soon! 🐈".into());
        ui.invoke_send();
    }
}

pub(crate) fn destination() -> viewer::Destination {
    viewer::Destination::Directory(std::env::temp_dir().join("line-desktop-demo-exports"))
}

pub(crate) fn settle(ui: &AppWindow, desktop: &Runtime, worker: &Worker) {
    for _ in 0..20 {
        desktop.update(|_| {});
        desktop.update(|d| d.apply_batch(ui, worker.deltas.try_iter()));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

pub(crate) fn worker(fake: FakeBackend, text_size: i32) -> Worker {
    use std::sync::atomic::{AtomicBool, Ordering};
    let initial = AtomicBool::new(true);
    let worker = Worker::start_projected(
        StoreLocation::Memory,
        move || fake.open(),
        line::engine::Timing {
            offline_after: std::time::Duration::from_millis(300),
            ..Default::default()
        },
        move |delta| match delta {
            Delta::Prefs(mut prefs) if initial.swap(false, Ordering::Relaxed) => {
                prefs.text_size = text_size;
                Delta::Prefs(prefs)
            }
            other => other,
        },
    );
    worker.sender.send(Intent::Prefs(Prefs {
        text_size,
        ..Default::default()
    }));
    worker
}

pub(crate) fn prepare(ui: &AppWindow, scene: &str, dark: bool) {
    ui.set_demo(true);
    ui.global::<State>()
        .set_title_override("LINE (preview · F12: scenarios)".into());
    ui.global::<State>().set_preview_demo(scene == "previews");
    if dark {
        ui.invoke_preview_dark(true);
    }
}
pub(crate) fn profile(ui: &AppWindow, scene: &str, snapshot_profile: bool) {
    if snapshot_profile || scene == "member-profile" {
        ui.invoke_open_profile("alice".into());
    }
}
