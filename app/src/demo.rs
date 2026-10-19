//! Synthetic data enters through the backend and follows the normal commit path.
use crate::*;
use line::backend::{Failure, FakeRpc, LiveOp, Op};

pub(crate) struct Args {
    pub enabled: bool,
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
            "--demo" | "--demo-light" => self.enabled = true,
            "--demo-dark" => {
                self.enabled = true;
                self.dark = true;
            }
            "--demo-paging" | "--demo-previews" | "--demo-signed-out" | "--demo-rich"
            | "--demo-photos" | "--demo-groups" | "--demo-friends" => {
                self.enabled = true;
                self.scene = arg.trim_start_matches("--demo-").into();
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
            "--snapshot-text-size" | "--demo-text-size" => {
                self.text_size = args.next().ok_or("missing text size")?.parse()?
            }
            "--snapshot-scene" => self.scene = args.next().ok_or("missing scene")?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

pub(super) fn scenario(ui: &AppWindow, d: &mut Desktop, fake: &FakeBackend, kind: i32) {
    let chat = d.state.selected.clone().unwrap_or(ChatId("alice".into()));
    match kind {
        40..=46 => {
            use line::client::error::SignInFailure::*;
            fake.fail_sign_in(Some(match kind {
                40 => CryptoModule("/demo/crypto.wasm".into()),
                41 => StateDirectory("/demo/state".into()),
                42 => Keyring,
                43 => Network,
                44 => Server,
                45 => Phone,
                _ => Cancelled,
            }));
        }
        34 => {
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
                FakeRpc::Unsend(ServerId::new("6301").unwrap()),
                Failure::Refused(line::client::error::Refusal::Server(
                    line::client::error::ServerRefusal::Gateway(13),
                )),
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
                snapshot.server_unread = d.state.chats[&target].unread_count + count as u32;
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
                        (FakeBackend::message(&chat.0, id, &format!("到着 {i} · {id}"), false))
                            .into(),
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
        47 => {
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
        10 | 31 => {
            fake.fail_rpc(
                FakeRpc::Send(chat.clone()),
                Failure::Refused(line::client::error::Refusal::Server(
                    line::client::error::ServerRefusal::Gateway(13),
                )),
            );
            if kind == 10 {
                d.sender.send(Intent::Send {
                    chat,
                    text: "Rejected message".into(),
                    reply: None,
                });
            }
        }
        30 => fake.fail_rpc(
            FakeRpc::Send(chat.clone()),
            Failure::Refused(line::client::error::Refusal::Server(
                line::client::error::ServerRefusal::Gateway(13),
            )),
        ),
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
pub fn install(
    ui: &AppWindow,
    actions: Runtime,
    fake: FakeBackend,
    workers: Rc<RefCell<Worker>>,
    text_size: i32,
) {
    let fake = Rc::new(RefCell::new(fake));
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
    #[cfg(feature = "ui-debug")]
    {
        let weak = ui.as_weak();
        let fake = fake.clone();
        let actions = actions.clone();
        let reset_worker = workers;
        ui.global::<State>().set_demo_reset_enabled(true);
        ui.global::<State>()
            .set_demo_instance(std::env::var("LINE_UI_INSTANCE").unwrap_or_default().into());
        ui.global::<State>().on_demo_automation(move |hook| {
            if hook != line_desktop_ui::DemoAutomation::Reset {
                return;
            }
            let weak = weak.clone();
            let fake = fake.clone();
            let reset_worker = reset_worker.clone();
            actions.push(move |d| {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_focus_search();
                    ui.global::<State>().invoke_forget_detail_focus();
                    ui.global::<State>().invoke_cancel_attachments();
                    let fresh = FakeBackend::canned();
                    configure(&fresh, "", None);
                    let worker = worker(fresh.clone(), text_size);
                    worker.on_deltas(event_loop::wake);
                    d.sender = worker.sender.clone();
                    reset_worker.replace(worker);
                    *fake.borrow_mut() = fresh;
                    d.state = AppState::default();
                    d.history = reading::Reading {
                        opening_history: true,
                        ..Default::default()
                    };
                    d.assets = assets::Assets::default();
                    d.presenter = presenter::Presenter::default();
                    d.shown_composer.clear();
                    d.query.clear();
                    d.category = 0;
                    d.reveal = None;
                    d.viewer_asset = None;
                    d.viewer_source = None;
                    // Preserve geometry/derived bindings and instance identity; reset
                    // writable presentation state from the same defaults as a fresh UI.
                    let defaults = AppWindow::new().expect("create demo reset defaults");
                    let state = ui.global::<State>();
                    let fresh_state = defaults.global::<State>();
                    state.set_composer_reply(fresh_state.get_composer_reply());
                    state.set_attachment_injection_paths(
                        fresh_state.get_attachment_injection_paths(),
                    );
                    state.set_attachments_open(fresh_state.get_attachments_open());
                    state.set_attachments_chat(fresh_state.get_attachments_chat());
                    state.set_attachments_error(fresh_state.get_attachments_error());
                    state.set_attachments_count(fresh_state.get_attachments_count());
                    state.set_attachment_items(fresh_state.get_attachment_items());
                    state.set_paste_text_serial(fresh_state.get_paste_text_serial());
                    state.set_history_virtual(fresh_state.get_history_virtual());
                    state.set_virtual_history_height(fresh_state.get_virtual_history_height());
                    state.set_plain_history_height(fresh_state.get_plain_history_height());
                    state.set_account_initial(fresh_state.get_account_initial());
                    state.set_chat_id(fresh_state.get_chat_id());
                    state.set_chat_initial(fresh_state.get_chat_initial());
                    state.set_chat_avatar(fresh_state.get_chat_avatar());
                    state.set_panel(fresh_state.get_panel());
                    state.set_preview_demo(fresh_state.get_preview_demo());
                    state.set_unread_conversations(fresh_state.get_unread_conversations());
                    state.set_unread_stale(fresh_state.get_unread_stale());
                    state.set_unread_divider(fresh_state.get_unread_divider());
                    state.set_unread_locating(fresh_state.get_unread_locating());
                    state.set_unread_boundary_unknown(fresh_state.get_unread_boundary_unknown());
                    state.set_section(fresh_state.get_section());
                    state.set_category(fresh_state.get_category());
                    state.set_reaction_error_id(fresh_state.get_reaction_error_id());
                    state.set_sticker_tabs(fresh_state.get_sticker_tabs());
                    state.set_sticker_items(fresh_state.get_sticker_items());
                    state.set_sticker_package(fresh_state.get_sticker_package());
                    state.set_new_chat_open(fresh_state.get_new_chat_open());
                    state.set_new_chat_busy(fresh_state.get_new_chat_busy());
                    state.set_new_chat_uncertain(fresh_state.get_new_chat_uncertain());
                    state.set_new_chat_error(fresh_state.get_new_chat_error());
                    state.set_new_chat_query(fresh_state.get_new_chat_query());
                    state.set_new_chat_name(fresh_state.get_new_chat_name());
                    state.set_new_chat_placeholder(fresh_state.get_new_chat_placeholder());
                    state.set_new_chat_count(fresh_state.get_new_chat_count());
                    state.set_new_chat_friends(fresh_state.get_new_chat_friends());
                    state.set_new_chat_selected(fresh_state.get_new_chat_selected());
                    state.set_friends(fresh_state.get_friends());
                    state.set_friend_result(fresh_state.get_friend_result());
                    state.set_friend_row_height(fresh_state.get_friend_row_height());
                    state.set_friend_query(fresh_state.get_friend_query());
                    state.set_groups_expanded(fresh_state.get_groups_expanded());
                    state.set_friends_expanded(fresh_state.get_friends_expanded());
                    state.set_friends_y(fresh_state.get_friends_y());
                    state.set_member_result(fresh_state.get_member_result());
                    state.set_detail_depth(fresh_state.get_detail_depth());
                    state.set_detail_page(fresh_state.get_detail_page());
                    state.set_detail_busy(fresh_state.get_detail_busy());
                    state.set_detail_running(fresh_state.get_detail_running());
                    state.set_viewer_from_detail(fresh_state.get_viewer_from_detail());
                    state.set_detail_error(fresh_state.get_detail_error());
                    state.set_conversation_error(fresh_state.get_conversation_error());
                    state.set_friends_error(fresh_state.get_friends_error());
                    state.set_chat_list_error(fresh_state.get_chat_list_error());
                    state.set_profile_friend(fresh_state.get_profile_friend());
                    state.set_profile_addable(fresh_state.get_profile_addable());
                    state.set_profile_own(fresh_state.get_profile_own());
                    state.set_profile_favourite(fresh_state.get_profile_favourite());
                    state.set_profile_original(fresh_state.get_profile_original());
                    state.set_profile_notice(fresh_state.get_profile_notice());
                    state.set_profile_name_runs(fresh_state.get_profile_name_runs());
                    state.set_profile_status_runs(fresh_state.get_profile_status_runs());
                    state.set_group_room(fresh_state.get_group_room());
                    state.set_group_muted(fresh_state.get_group_muted());
                    state.set_group_left(fresh_state.get_group_left());
                    state.set_group_writable(fresh_state.get_group_writable());
                    state.set_group_count(fresh_state.get_group_count());
                    state.set_invitee_count(fresh_state.get_invitee_count());
                    state.set_chat_left(fresh_state.get_chat_left());
                    state.set_detail_dialog(fresh_state.get_detail_dialog());
                    state.set_members(fresh_state.get_members());
                    state.set_member_query(fresh_state.get_member_query());
                    state.set_chat_group(fresh_state.get_chat_group());
                    state.set_member_count(fresh_state.get_member_count());
                    state.set_browser_focus_hidden(fresh_state.get_browser_focus_hidden());
                    state.set_profile_initial(fresh_state.get_profile_initial());
                    state.set_profile_name(fresh_state.get_profile_name());
                    state.set_profile_id(fresh_state.get_profile_id());
                    state.set_profile_status(fresh_state.get_profile_status());
                    state.set_profile_group(fresh_state.get_profile_group());
                    state.set_profile_avatar(fresh_state.get_profile_avatar());
                    state.set_profile_available(fresh_state.get_profile_available());
                    state.set_ime_active(fresh_state.get_ime_active());
                    state.set_text_size(fresh_state.get_text_size());
                    state.set_chats(fresh_state.get_chats());
                    state.set_messages(fresh_state.get_messages());
                    state.set_selected_chat(fresh_state.get_selected_chat());
                    state.set_chat_title(fresh_state.get_chat_title());
                    state.set_chat_subtitle(fresh_state.get_chat_subtitle());
                    state.set_draft(fresh_state.get_draft());
                    state.set_enter_sends(fresh_state.get_enter_sends());
                    state.set_editor_caret(fresh_state.get_editor_caret());
                    state.set_editor_anchor(fresh_state.get_editor_anchor());
                    state.set_composer_focused(fresh_state.get_composer_focused());
                    state.set_unsend_feedback_id(fresh_state.get_unsend_feedback_id());
                    state.set_recovery_kind(fresh_state.get_recovery_kind());
                    state.set_recovery_id(fresh_state.get_recovery_id());
                    state.set_recovery_focus_id(fresh_state.get_recovery_focus_id());
                    state.set_recovery_focus_action(fresh_state.get_recovery_focus_action());
                    state.set_send_pending(fresh_state.get_send_pending());
                    state.set_send_reason(fresh_state.get_send_reason());
                    state.set_list_pressed(fresh_state.get_list_pressed());
                    state.set_list_release_pending(fresh_state.get_list_release_pending());
                    state.set_background_mode(fresh_state.get_background_mode());
                    state.set_tray_available(fresh_state.get_tray_available());
                    state.set_autostart(fresh_state.get_autostart());
                    state.set_background_status(fresh_state.get_background_status());
                    state.set_autostart_status(fresh_state.get_autostart_status());
                    state.set_notifications_paused(fresh_state.get_notifications_paused());
                    state.set_notifications_enabled(fresh_state.get_notifications_enabled());
                    state.set_notification_preview(fresh_state.get_notification_preview());
                    state.set_notification_sound(fresh_state.get_notification_sound());
                    state.set_notification_mention_exception(
                        fresh_state.get_notification_mention_exception(),
                    );
                    state
                        .set_notification_group_events(fresh_state.get_notification_group_events());
                    state.set_notification_available(fresh_state.get_notification_available());
                    state.set_notification_status(fresh_state.get_notification_status());
                    state.set_transient_notice(fresh_state.get_transient_notice());
                    state.set_viewer_image(fresh_state.get_viewer_image());
                    state.set_viewer_status(fresh_state.get_viewer_status());
                    state.set_viewer_name(fresh_state.get_viewer_name());
                    state.set_viewer_error(fresh_state.get_viewer_error());
                    state.set_viewer_retryable(fresh_state.get_viewer_retryable());
                    state.set_viewer_loading(fresh_state.get_viewer_loading());
                    state.set_viewer_key(fresh_state.get_viewer_key());
                    state.set_viewer_zoom(fresh_state.get_viewer_zoom());
                    state.set_viewer_has_previous(fresh_state.get_viewer_has_previous());
                    state.set_viewer_has_next(fresh_state.get_viewer_has_next());
                    state.set_viewer_save_status(fresh_state.get_viewer_save_status());
                    state.set_viewer_saving(fresh_state.get_viewer_saving());
                    state.set_viewer_return_key(fresh_state.get_viewer_return_key());
                    state.set_viewer_return_serial(fresh_state.get_viewer_return_serial());
                    state.set_connection(fresh_state.get_connection());
                    state.set_auth_cancel_confirm(fresh_state.get_auth_cancel_confirm());
                    state.set_auth_cancelling(fresh_state.get_auth_cancelling());
                    state.set_auth_qr(fresh_state.get_auth_qr());
                    state.set_auth_pin(fresh_state.get_auth_pin());
                    state.set_auth_stage(fresh_state.get_auth_stage());
                    state.set_auth_error(fresh_state.get_auth_error());
                    state.set_session_cause(fresh_state.get_session_cause());
                    state
                        .set_auth_prerequisite_missing(fresh_state.get_auth_prerequisite_missing());
                    state.set_title_override(fresh_state.get_title_override());
                    state.set_link_feedback(fresh_state.get_link_feedback());
                    state.set_demo_scenarios_open(fresh_state.get_demo_scenarios_open());
                    state.set_more_chats(fresh_state.get_more_chats());
                    state.set_has_chat(fresh_state.get_has_chat());
                    state.set_storage_error(fresh_state.get_storage_error());
                    state.set_desktop_warning(fresh_state.get_desktop_warning());
                    state.set_connection_status(fresh_state.get_connection_status());
                    state.set_has_account(fresh_state.get_has_account());
                    state.set_account_mid(fresh_state.get_account_mid());
                    state.set_account_name(fresh_state.get_account_name());
                    state.set_show_conversation(fresh_state.get_show_conversation());
                    state.set_following(fresh_state.get_following());
                    state.set_new_count(fresh_state.get_new_count());
                    state.set_history_y(fresh_state.get_history_y());
                    state.set_history_dragging(fresh_state.get_history_dragging());
                    state.set_query(fresh_state.get_query());
                    state.set_list_y(fresh_state.get_list_y());
                    state.set_editor_y(fresh_state.get_editor_y());
                    state.set_search_list_y(fresh_state.get_search_list_y());
                    state.set_searching(fresh_state.get_searching());
                    state.set_result_focus(fresh_state.get_result_focus());
                    state.set_history_extent(fresh_state.get_history_extent());
                    state.set_file_save_id(fresh_state.get_file_save_id());
                    state.set_file_save_status(fresh_state.get_file_save_status());
                    state.set_file_saving(fresh_state.get_file_saving());
                    state.set_last_opened_link(fresh_state.get_last_opened_link());
                    state.set_history_cover_inset(fresh_state.get_history_cover_inset());
                    state.set_older_start_known(fresh_state.get_older_start_known());
                    state.set_older_status(fresh_state.get_older_status());
                    state.set_highlight_message(fresh_state.get_highlight_message());
                    state.set_return_visible(fresh_state.get_return_visible());
                    state.set_history_needs_refresh(fresh_state.get_history_needs_refresh());
                    state.set_history_budget_warning(fresh_state.get_history_budget_warning());
                    state.set_history_can_follow(fresh_state.get_history_can_follow());
                    state.set_history_interacting(fresh_state.get_history_interacting());
                    d.presenter.install(&ui);
                    ui.set_text_size(text_size);
                    ui.invoke_focus_search();
                    d.render(&ui);
                    state.set_demo_reset_generation(state.get_demo_reset_generation() + 1);
                }
            });
        });
    }
    #[cfg(not(feature = "ui-debug"))]
    let _ = (workers, text_size);
    let weak = ui.as_weak();
    ui.on_demo_event(move |kind| {
        let weak = weak.clone();
        let fake = fake.clone();
        actions.push(move |d| {
            if let Some(ui) = weak.upgrade() {
                scenario(&ui, d, &fake.borrow(), kind);
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
                fake.fail_rpc(
                    FakeRpc::Send(chat),
                    Failure::Refused(line::client::error::Refusal::Server(
                        line::client::error::ServerRefusal::Gateway(13),
                    )),
                );
            }
        });
        ui.set_draft("See you soon! 🐈".into());
        ui.invoke_send();
    }
}

fn directory() -> std::path::PathBuf {
    std::env::var_os("LINE_DEMO_DIRECTORY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub(crate) fn destination() -> viewer::Destination {
    viewer::Destination::Directory(directory().join("exports"))
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
    let worker = Worker::start(
        StoreLocation::Memory,
        move || fake.open(),
        line::engine::WorkerOptions {
            timing: line::engine::Timing {
                offline_after: std::time::Duration::from_millis(300),
                ..Default::default()
            },
            project: std::sync::Arc::new(move |delta| match delta {
                Delta::Prefs(mut prefs) if initial.swap(false, Ordering::Relaxed) => {
                    prefs.text_size = text_size;
                    Delta::Prefs(prefs)
                }
                other => other,
            }),
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
