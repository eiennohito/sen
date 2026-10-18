//! Synthetic data enters through the backend and follows the normal commit path.
use crate::*;
use line_wasm_poc::backend::{LiveOp, Op};
pub(super) fn scenario(ui: &AppWindow, d: &mut Desktop, fake: &FakeBackend, kind: i32) {
    let chat = d.state.selected.clone().unwrap_or(ChatId("alice".into()));
    match kind {
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
            d.opening_history = false;
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
            for (target, count) in [(chat.clone(), 3), (ChatId("weekend".into()), 2)] {
                let mut rows = fake.page(&target);
                // New ids must sort after everything already read.
                let newest = rows
                    .iter()
                    .filter_map(|m| m.server_id.0.parse::<i64>().ok())
                    .max()
                    .unwrap_or(0);
                for n in 1..=count {
                    let id = newest + n;
                    let row = FakeBackend::message(&target.0, id, "新しいメッセージ 🐈", false);
                    rows.push(row.clone());
                    fake.inject(LiveOp {
                        revision: fake.revision(),
                        replay: false,
                        op: Op::Message(row),
                    });
                }
                fake.set_page(target, rows);
            }
        }
        15 => {
            for i in 0..40 {
                fake.add_chat(ChatInput {
                    id: ChatId(format!("browser-{i}")),
                    title: format!("Browser friend {i:02}"),
                    ..Default::default()
                });
            }
            d.sender.send(Intent::MoreChats);
        }
        1 => {
            for i in 1..=3 {
                let id = fake.revision();
                fake.inject(LiveOp {
                    revision: id,
                    replay: false,
                    op: Op::Message(FakeBackend::message(
                        &chat.0,
                        id,
                        &format!("到着 {i}"),
                        false,
                    )),
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
            row.content_type = 1;
            row.body["rich"] =
                serde_json::to_value(line_wasm_poc::client::desktop::rich::RichContent {
                    content_type: 1,
                    ..Default::default()
                })
                .unwrap();
            let mut asset = serde_json::to_value(FakeBackend::asset(true)).unwrap();
            asset["failure"] = serde_json::to_value(media_failure((kind - 20) as usize)).unwrap();
            row.body["image"] = asset;
            fake.set_page(chat.clone(), vec![row]);
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Recent,
            });
            ui.set_following(true);
        }
        19 => fake.auth_expired(),
        3 => {
            fake.uncertain_send();
            d.sender.send(Intent::Send {
                chat: chat.clone(),
                text: "駅に着いたら連絡してね。See you soon! 🐈".into(),
            });
            d.sender.send(Intent::Draft {
                chat,
                draft: Draft {
                    text: d.state.composer.clone(),
                    cursor: 0,
                    anchor: 0,
                    updated: line_wasm_poc::engine::now_ms(),
                },
            });
        }
        10 | 31 => {
            fake.fail_send();
            if kind == 10 {
                d.sender.send(Intent::Send {
                    chat,
                    text: "Rejected message".into(),
                });
            }
        }
        30 => fake.fail_send(),
        17 => fake.offline(true),
        16 => fake.offline(false),
        _ => {}
    }
}

pub fn configure(fake: &FakeBackend, scene: &str, case: Option<&str>) {
    demo_data::seed(fake);
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
        row.content_type = 1;
        row.body["rich"] =
            serde_json::to_value(line_wasm_poc::client::desktop::rich::RichContent {
                content_type: 1,
                ..Default::default()
            })
            .unwrap();
        let mut asset = serde_json::to_value(FakeBackend::asset(true)).unwrap();
        asset["failure"] = serde_json::to_value(media_failure(index)).unwrap();
        row.body["image"] = asset;
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
    if scene == "groups" {
        fake.set_page(ChatId("alice".into()), demo_data::group(fake, "alice", 900));
    }
    if scene == "photos" {
        let mut row = FakeBackend::message("alice", 900, "Riverside walk", false);
        row.content_type = 1;
        row.body["rich"] =
            serde_json::to_value(line_wasm_poc::client::desktop::rich::RichContent {
                content_type: 1,
                ..Default::default()
            })
            .unwrap();
        row.body["image"] = serde_json::to_value(demo_data::image_asset(
            fake,
            "photo-landscape",
            demo_data::photo(false),
        ))
        .unwrap();
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

fn media_failure(index: usize) -> line_wasm_poc::client::desktop::assets::Failure {
    use line_wasm_poc::client::desktop::assets::Failure::*;
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
        kind: if id == "groups" { 1 } else { 0 },
        member_count: (id == "groups").then_some(25),
        ..Default::default()
    });
    fake.set_page(chat.clone(), rows.clone());
    fake.inject(LiveOp {
        revision: fake.revision(),
        replay: true,
        op: Op::Message(rows[0].clone()),
    });
    d.capture_draft(ui);
    // A fixture changes the history only; the composer text stays. The draft row
    // would otherwise hide the new chat from detail loading, so load the chat list.
    d.sender.send(Intent::Draft {
        chat: chat.clone(),
        draft: Draft {
            text: d.state.composer.clone(),
            cursor: ui.get_editor_caret() as usize,
            anchor: ui.get_editor_anchor() as usize,
            updated: line_wasm_poc::engine::now_ms(),
        },
    });
    d.sender.send(Intent::MoreChats);
    let intent = d.state.select(chat.clone());
    d.opening_history = false;
    d.restore_anchor = None;
    d.geometry.borrow_mut().clear();
    d.sender.send(intent);
    d.sender.send(Intent::Page {
        chat,
        kind: PageKind::Recent,
    });
    d.render(ui);
    ui.invoke_show_history(0., false);
}
