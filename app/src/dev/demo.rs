//! Synthetic data enters through the backend and follows the normal commit path.
use super::*;
use line::backend::{Failure, FakeRpc, LiveOp, Op};
const REFUSED: Failure = Failure::Refused(line::client::error::Refusal::Server(
    line::client::error::ServerRefusal::Gateway(13),
));

pub(crate) struct Args {
    pub enabled: bool,
    pub background: bool,
    pub snapshot_path: Option<String>,
    pub size: (u32, u32),
    pub text_size: i32,
    pub scene: String,
    pub case: Option<String>,
    pub dark: bool,
    pub snapshot_profile: bool,
}
impl Default for Args {
    fn default() -> Self {
        Self {
            enabled: false,
            background: false,
            snapshot_path: None,
            size: (980, 720),
            text_size: 15,
            scene: String::new(),
            case: None,
            dark: false,
            snapshot_profile: false,
        }
    }
}
impl Args {
    pub fn parse(
        &mut self,
        arg: &str,
        args: &mut impl Iterator<Item = String>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        match arg {
            #[cfg(feature = "ui-debug")]
            "--debug-background" => self.background = true,
            "--demo" => self.enabled = true,
            "--demo-dark" => {
                self.enabled = true;
                self.dark = true;
            }
            "--snapshot-profile" => self.snapshot_profile = true,
            "--snapshot-case" => self.case = Some(args.next().ok_or("missing snapshot case")?),
            "--snapshot" => {
                self.snapshot_path = Some(args.next().ok_or("missing snapshot path")?);
                self.enabled = true;
            }
            "--snapshot-size" | "--demo-size" => {
                let size = args.next().ok_or("missing size")?;
                let (w, h) = size.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                self.size = (w.parse()?, h.parse()?);
            }
            "--snapshot-text-size" => {
                self.text_size = args.next().ok_or("missing text size")?.parse()?
            }
            "--snapshot-scene" => self.scene = args.next().ok_or("missing scene")?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

pub(super) fn scenario(ui: &AppWindow, d: &mut Desktop, fake: &FakeBackend, kind: DemoScenario) {
    let chat = d.state.selected.clone().unwrap_or(ChatId("alice".into()));
    use DemoScenario::*;
    match kind {
        SignInCrypto | SignInState | SignInKeyring | SignInNetwork | SignInServer | SignInPhone
        | SignInCancelled => {
            use line::client::error::SignInFailure::*;
            fake.fail_sign_in(Some(match kind {
                SignInCrypto => CryptoModule("/demo/crypto.wasm".into()),
                SignInState => StateDirectory("/demo/state".into()),
                SignInKeyring => Keyring,
                SignInNetwork => Network,
                SignInServer => Server,
                SignInPhone => Phone,
                _ => Cancelled,
            }));
        }
        Attachments => {
            let directory = directory().join("attachments");
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
                    callbacks::attachments::drop_files(ui, d, &paths);
                }
            }
        }

        Protocol => {
            let rows = vec![
                FakeBackend::message("protocol", 6300, "Unsend success fixture", true),
                FakeBackend::message("protocol", 6301, "Unsend failure fixture", true),
                FakeBackend::official_image("protocol", 6302),
            ];
            fake.fail_rpc(FakeRpc::Unsend(ServerId::new("6301").unwrap()), REFUSED);
            fixture(ui, d, fake, "protocol", "Protocol fixtures", rows);
        }
        Reaction => {
            if let Some(row) = d
                .state
                .rows()
                .into_iter()
                .find(|m| m.server_id.is_some() && !m.body.is_withdrawn())
            {
                fake.inject_raw(serde_json::json!({"type":139,"param1":row.server_id.as_ref().unwrap().as_str(),
                    "param2":serde_json::json!({"chatMid":chat.0,"curr":{"predefinedReactionType":2}}).to_string()}));
                let curr = serde_json::json!({"paidReactionType":{"productId":"demo", "emojiId":"reaction"}});
                if let Some(ReactionKind::Paid(Some(asset))) =
                    line::client::desktop::reaction_kind(&curr)
                {
                    let pixels = demo_data::avatar(1).to_rgba8().unwrap();
                    fake.set_image(
                        &asset,
                        line::client::desktop::assets::Pixels {
                            width: pixels.width(),
                            height: pixels.height(),
                            rgba: pixels.as_bytes().to_vec(),
                        },
                    );
                }
                for actor in ["alice", "unknown-reactor"] {
                    fake.inject_raw(serde_json::json!({"type":140,"param1":row.server_id.as_ref().unwrap().as_str(), "param3":actor,
                        "param2":serde_json::json!({"chatMid":chat.0,"curr":curr}).to_string()}));
                }
            }
        }
        Rich => {
            fixture(
                ui,
                d,
                fake,
                "rich",
                "Rich content with a very long conversation name 日本語",
                demo_data::rich_fixture(fake, "rich", 2000),
            );
        }
        History => {
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
            ui.global::<ConversationState>()
                .invoke_show_history(0., true);
        }
        Reading => fixture(
            ui,
            d,
            fake,
            "reading",
            "Reading fixtures",
            demo_data::reading(fake, "reading"),
        ),
        Unread => {
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
                snapshot.server_unread = d.state.chats[&target].unread_count + count as u32;
                snapshot.last_message_id = rows.last().map(|row| row.server_id.clone());
                snapshot.last_message_ms = rows.last().map(|row| row.created_ms);
                fake.add_chat(snapshot);
                fake.set_page(target, rows);
            }
            d.sender.send(Intent::MoreChats);
        }
        ChatList => {
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
        Arrivals => {
            for i in 1..=3 {
                let id = fake.revision();
                fake.inject(LiveOp {
                    revision: id,
                    replay: false,
                    op: Op::Message(
                        (FakeBackend::message(&chat.0, id, &format!("到着 {i} · {id}"), false))
                            .into(),
                    ),
                });
            }
        }
        Photo => fixture(
            ui,
            d,
            fake,
            "photos",
            "Photo preview",
            demo_data::photos(fake, "photos"),
        ),
        Sticker => {
            let row = demo_data::sticker(fake, "stickers");
            fixture(ui, d, fake, "stickers", "Sticker preview", vec![row]);
        }
        Actions => {
            fixture(
                ui,
                d,
                fake,
                "action-group",
                "Group actions",
                demo_data::action_geometry(fake, "action-group", 9900),
            );
            fixture(
                ui,
                d,
                fake,
                "actions",
                "Message actions",
                demo_data::action_geometry(fake, "actions", 9800),
            );
        }
        Group => fixture(
            ui,
            d,
            fake,
            "groups",
            "Group preview",
            demo_data::group(fake, "groups", 5200),
        ),
        MediaTimeout | MediaNetwork | MediaNotFound | MediaForbidden | MediaServer
        | MediaUnsupported | MediaPath | MediaLarge | MediaDecode | MediaCancelled => {
            let row = media_row(&chat.0, fake.revision(), media_failure(kind));
            fake.set_page(chat.clone(), vec![row]);
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Recent,
            });
            ui.global::<ConversationState>().set_following(true);
        }
        Expired => fake.auth_expired(),
        Unanswered => {
            fake.fail_rpc(FakeRpc::Send(ChatId("alice".into())), Failure::Unanswered);
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
        Rejected | RejectNext => {
            fake.fail_rpc(FakeRpc::Send(chat.clone()), REFUSED);
            if kind == Rejected {
                d.sender.send(Intent::Send {
                    chat,
                    text: "Rejected message".into(),
                    reply: None,
                });
            }
        }
        Offline => fake.offline(true),
        Reconnect => fake.offline(false),
        MediaMenu => {}
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
    fake.configure(|s| s.upload_delay = std::time::Duration::from_millis(1800));
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
        let kind = match class {
            "timeout" => DemoScenario::MediaTimeout,
            "network" => DemoScenario::MediaNetwork,
            "404" => DemoScenario::MediaNotFound,
            "403" => DemoScenario::MediaForbidden,
            "500" => DemoScenario::MediaServer,
            "unsupported" => DemoScenario::MediaUnsupported,
            "path" => DemoScenario::MediaPath,
            "large" => DemoScenario::MediaLarge,
            "decode" => DemoScenario::MediaDecode,
            "cancelled" => DemoScenario::MediaCancelled,
            _ => panic!("unknown media failure case: {class}"),
        };
        let row = media_row("alice", 900, media_failure(kind));
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
        *super::image_mut(&mut row.body) = Some(demo_data::image_asset(
            fake,
            "photo-landscape",
            demo_data::photo(false),
        ));
        fake.set_page(ChatId("alice".into()), vec![row]);
    }
}
pub fn install(ui: &AppWindow, actions: Runtime, fake: FakeBackend, session: &Rc<dev::Session>) {
    let weak = ui.as_weak();
    let links = actions.clone();
    ui.global::<ContentState>().on_open_link(move |url| {
        let weak = weak.clone();
        links.push(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<ContentState>()
                    .set_link_feedback(format!("Preview opened: {url}").into());
                ui.global::<DemoState>().set_last_opened_link(url);
            }
        });
    });
    #[cfg(feature = "ui-debug")]
    automation::install_reset(ui, &actions, session);
    #[cfg(not(feature = "ui-debug"))]
    let _ = session;
    let weak = ui.as_weak();
    ui.global::<DemoState>().on_demo_event(move |kind| {
        let weak = weak.clone();
        let fake = fake.clone();
        actions.push(move |d| {
            if let Some(ui) = weak.upgrade() {
                scenario(&ui, d, &fake, kind);
            }
        });
    });
}

fn media_row(chat: &str, id: i64, failure: line::client::desktop::assets::Failure) -> MessageInput {
    let mut row = FakeBackend::message(chat, id, "", false);
    demo_data::rich(
        &mut row,
        line::client::desktop::rich::RichContent {
            content_type: 1,
            ..Default::default()
        },
    );
    let mut asset = serde_json::to_value(FakeBackend::asset(true)).unwrap();
    asset["failure"] = serde_json::to_value(failure).unwrap();
    *super::image_mut(&mut row.body) = Some(serde_json::from_value(asset).unwrap());
    row
}
fn media_failure(kind: DemoScenario) -> line::client::desktop::assets::Failure {
    use DemoScenario::*;
    use line::client::desktop::assets::Failure::*;
    match kind {
        MediaTimeout => Timeout,
        MediaNetwork => Network,
        MediaNotFound => Http(404),
        MediaForbidden => Http(403),
        MediaServer => Http(500),
        MediaUnsupported => UnsupportedHost,
        MediaPath => RejectedPath,
        MediaLarge => TooLarge,
        MediaDecode => Decode,
        MediaCancelled => Cancelled,
        _ => unreachable!("not a media failure scenario"),
    }
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
    let is_group = matches!(id, "groups" | "action-group");
    fake.add_chat(ChatInput {
        id: chat.clone(),
        title: title.into(),
        is_group,
        member_count: is_group.then_some(25),
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
            cursor: ui.global::<ComposerState>().get_editor_caret() as usize,
            anchor: ui.global::<ComposerState>().get_editor_anchor() as usize,
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
    ui.global::<ConversationState>()
        .invoke_show_history(0., false);
}

pub(crate) fn stage(
    ui: &AppWindow,
    desktop: &Runtime,
    fake: Option<&FakeBackend>,
    scene: &str,
    snapshot_profile: bool,
) {
    ui.global::<ShellState>().set_show_conversation(
        scene != "chats" && scene != "signed-out" && !scene.starts_with("friends"),
    );
    if scene == "settings" {
        desktop.update(|d| {
            d.state.panel = Panel::Settings;
            d.render(ui);
            ui.invoke_focus_settings_close();
        });
    }
    if scene == "reply-review"
        && let Some(id) = desktop.update(|d| d.state.rows().first().map(|m| m.key.0.to_string()))
    {
        ui.global::<ComposerState>().invoke_reply(id.into());
    }
    if scene == "friends" {
        ui.global::<ShellState>().invoke_browse(Section::Friends);
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
        ui.global::<DetailState>().invoke_open_members();
    }
    if snapshot_profile || scene == "member-profile" {
        ui.global::<ShellState>().invoke_browse(Section::Friends);
    }
    if scene.starts_with("message-selected")
        && let Some(key) = desktop.update(|d| d.state.rows().first().map(|m| m.key))
    {
        ui.global::<ConversationState>()
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
                fake.fail_rpc(FakeRpc::Send(chat), REFUSED);
            }
        });
        ui.global::<ComposerState>()
            .set_draft("See you soon! 🐈".into());
        ui.global::<ComposerState>().invoke_send();
    }
}

pub(crate) fn directory() -> std::path::PathBuf {
    std::env::var_os("LINE_DEMO_DIRECTORY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub(crate) fn settle(ui: &AppWindow, desktop: &Runtime, worker: &Worker) {
    for _ in 0..20 {
        desktop.update(|_| {});
        desktop.update(|d| d.apply_batch(ui, worker.deltas.try_iter()));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

pub(crate) fn worker(
    fake: FakeBackend,
    text_size: i32,
    directory: &std::path::Path,
) -> (Worker, tempfile::TempDir) {
    std::fs::create_dir_all(directory).expect("create demo directory");
    let root = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(directory)
        .expect("create empty demo store");
    let mut store = line::store::Store::open(root.path()).expect("open demo store");
    store
        .set_prefs(&Prefs {
            text_size,
            ..Default::default()
        })
        .expect("seed demo preferences");
    drop(store);
    let worker = Worker::start(
        StoreLocation::Directory(root.path().to_owned()),
        move || fake.open(),
        line::engine::WorkerOptions {
            timing: line::engine::Timing {
                offline_after: std::time::Duration::from_millis(300),
                ..Default::default()
            },
        },
    );
    (worker, root)
}

pub(crate) fn prepare(ui: &AppWindow) {
    ui.global::<DemoState>().set_demo(true);
    ui.global::<ShellState>()
        .set_title_override("LINE (preview · F12: scenarios)".into());
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_demo_workers_start_empty_and_remove_their_store() {
        let directory = tempfile::Builder::new()
            .prefix("demo-session-")
            .tempdir_in(directory())
            .unwrap();
        for _ in 0..2 {
            let (worker, store_directory) = worker(FakeBackend::signed_out(), 14, directory.path());
            let mut state = AppState::default();
            while !matches!(state.connection, Connection::SignedOut { .. }) {
                state.apply(
                    worker
                        .deltas
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap(),
                );
            }
            assert!(state.chats.is_empty());
            let paths: Vec<_> = std::fs::read_dir(directory.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(paths.len(), 1);
            let mut store = line::store::Store::open(&paths[0]).unwrap();
            assert!(store.chat_list(10, 0).unwrap().is_empty());
            store
                .upsert_chats(
                    &[ChatInput {
                        id: ChatId("stale-chat".into()),
                        ..Default::default()
                    }],
                    false,
                )
                .unwrap();
            drop(store);
            drop(worker);
            drop(store_directory);
            assert!(!paths[0].exists(), "demo store survived its session");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }
}
