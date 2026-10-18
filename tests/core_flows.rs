use line_wasm_poc::client::error::RejectReason;
use line_wasm_poc::{
    backend::{FakeBackend, LiveOp, Op},
    controller::AppState,
    engine::{Connection, Delta, Intent, StoreLocation, Timing, Worker},
    store::{ChatId, Delivery, Key, Prefs, Reaction, ReactionKind},
};
use std::time::{Duration, Instant};
fn message_text(row: &line_wasm_poc::store::StoredMessage) -> &str {
    row.body.as_text()
}
struct App {
    worker: Worker,
    state: AppState,
    notifications: usize,
}
impl App {
    fn new(fake: FakeBackend, location: StoreLocation) -> Self {
        Self {
            worker: Worker::start_timed(
                fake,
                location,
                Timing {
                    retry_initial: Duration::from_millis(5),
                    retry_max: Duration::from_millis(50),
                    poll: Duration::from_millis(1),
                },
            ),
            state: AppState::default(),
            notifications: 0,
        }
    }
    fn send(&self, i: Intent) {
        self.worker.sender.send(i);
    }
    fn select(&mut self, chat: &str) {
        let i = self.state.select(ChatId(chat.into()));
        self.send(i);
    }
    fn until(&mut self, condition: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition(&self.state) {
            let d = self
                .worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("engine progress");
            if matches!(d, Delta::Notify(_)) {
                self.notifications += 1;
            }
            if let Delta::StorageFailed(e) = &d {
                panic!("engine: {e:?}");
            }
            self.state.apply(d);
        }
    }
    fn online(&mut self) {
        self.until(|s| s.connection == Connection::Online && s.chats.len() == 2);
    }
    fn body_key(&mut self, body: &str) -> Key {
        self.until(|s| s.messages.values().any(|m| message_text(m) == body));
        self.state
            .messages
            .values()
            .find(|m| message_text(m) == body)
            .unwrap()
            .key
    }
}
#[test]
fn m13_send_switch_return_with_stale_page() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "durable send".into(),
    });
    let key = app.body_key("durable send");
    app.until(|s| {
        s.messages
            .get(&key)
            .is_some_and(|m| m.delivery == Delivery::Sent)
    });
    app.select("weekend");
    app.body_key("Shall we meet at the station?");
    fake.set_page(
        ChatId("alice".into()),
        vec![FakeBackend::message("alice", 10, "stale history", false)],
    );
    app.select("alice");
    app.until(|s| s.messages.contains_key(&key));
    app.send(Intent::Page {
        chat: ChatId("alice".into()),
        kind: line_wasm_poc::store::PageKind::Recent,
    });
    app.send(Intent::Prefs(Prefs {
        text_size: 38,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 38);
    assert_eq!(message_text(&app.state.messages[&key]), "durable send");
}
#[test]
fn m14_closed_chat_incoming_then_open_stale_history() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("weekend");
    app.body_key("Shall we meet at the station?");
    fake.inject(LiveOp {
        revision: 50,
        replay: false,
        op: Op::Message((FakeBackend::message("alice", 50, "closed arrival", false)).into()),
    });
    app.until(|s| {
        s.chats[&ChatId("alice".into())]
            .preview
            .as_ref()
            .is_some_and(|m| message_text(m) == "closed arrival")
    });
    let pages = fake.page_requests("alice");
    app.select("alice");
    app.body_key("closed arrival");
    app.until(|_| fake.page_requests("alice") > pages);
    assert!(
        app.state
            .messages
            .values()
            .any(|m| message_text(m) == "closed arrival")
    );
}
#[test]
fn echo_and_send_result_keep_one_identity_in_both_orders() {
    for echo_first in [true, false] {
        let fake = FakeBackend::canned();
        fake.echo_first(echo_first);
        let mut app = App::new(fake.clone(), StoreLocation::Memory);
        app.online();
        app.select("alice");
        app.body_key("Hello! 日本語も使えます。");
        app.send(Intent::Send {
            chat: ChatId("alice".into()),
            text: "echo".into(),
        });
        let key = app.body_key("echo");
        app.until(|s| s.messages[&key].delivery == Delivery::Sent);
        if !echo_first {
            let m = &app.state.messages[&key];
            let mut echo = FakeBackend::message(
                "alice",
                m.server_id.as_ref().unwrap().0.parse().unwrap(),
                "echo",
                true,
            );
            echo.request_seq = m.request_seq;
            fake.inject(LiveOp {
                revision: 102,
                replay: false,
                op: Op::Message((echo).into()),
            });
            app.until(|s| s.messages[&key].sender_id == "self");
        }
        assert_eq!(
            app.state
                .messages
                .values()
                .filter(|m| message_text(m) == "echo")
                .count(),
            1
        );
        assert!(app.state.messages.contains_key(&key));
    }
}
#[test]
fn offline_start_shows_cached_chats_and_window() {
    let dir = std::env::temp_dir().join(format!("line-core-offline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let fake = FakeBackend::canned();
    {
        let mut app = App::new(fake.clone(), StoreLocation::Directory(dir.clone()));
        app.online();
        app.select("alice");
        app.body_key("Hello! 日本語も使えます。");
        fake.inject(LiveOp {
            revision: 75,
            replay: false,
            op: Op::Message((FakeBackend::message("alice", 75, "checkpoint", false)).into()),
        });
        app.body_key("checkpoint");
    }
    fake.offline(true);
    let mut app = App::new(fake.clone(), StoreLocation::Directory(dir.clone()));
    app.state.selected = Some(ChatId("alice".into()));
    app.state.window_sub = 1;
    app.until(|s| {
        matches!(s.connection, Connection::Offline { .. })
            && !s.chats.is_empty()
            && s.messages.values().any(|m| message_text(m) == "checkpoint")
    });
    assert!(app.state.chats.values().all(|c| c.chat.unread_stale));
    fake.offline(false);
    app.until(|s| s.connection == Connection::Online); // Stream reconnect also resumes persisted revision.
    app.until(|_| fake.resumed_revisions().contains(&75));
    assert!(fake.resumed_revisions().contains(&75));
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn failed_send_retry_discard_and_copy() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    fake.fail_send();
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "retry text".into(),
    });
    let key = app.body_key("retry text");
    app.until(|s| matches!(s.messages[&key].delivery, Delivery::Rejected { .. }));
    app.send(Intent::Copy(key));
    app.until(|s| s.composer == "retry text");
    app.send(Intent::Retry(key));
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    fake.fail_send();
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "discard text".into(),
    });
    let key = app.body_key("discard text");
    app.until(|s| matches!(s.messages[&key].delivery, Delivery::Rejected { .. }));
    app.send(Intent::Discard(key));
    app.until(|s| !s.messages.contains_key(&key));
}

#[test]
fn unrelated_fetch_failure_does_not_block_send() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    assert!(!app.state.can_send());
    fake.fail_friends();
    app.send(Intent::Friends);
    app.until(|s| !s.friends.is_empty()); // The engine retries the failed fetch silently.
    assert_eq!(fake.friends_calls(), 2);
    assert!(app.state.error.is_none());
    app.state.composer = "after unrelated failure".into();
    assert!(app.state.can_send());
    app.select("alice"); // Reselecting the open chat preserves its composer snapshot.
    assert_eq!(app.state.composer, "after unrelated failure");
    assert!(app.state.can_send());
    // Even a storage banner does not disable an unrelated operation. A committed
    // write clears it; no controller reset or manual retry is needed.
    app.state.apply(Delta::StorageFailed(
        line_wasm_poc::engine::StorageFailure::Full,
    ));
    assert!(app.state.can_send());
    app.send(Intent::Prefs(Prefs {
        text_size: 37,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 37 && s.error.is_none());
    let intent = app.state.submit().unwrap();
    app.send(intent);
    let key = app.body_key("after unrelated failure");
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    assert!(app.state.composer.is_empty());
    assert!(!app.state.can_send());
}
#[test]
fn opening_unread_chat_centers_the_window_on_the_divider() {
    use line_wasm_poc::store::{Anchor, ChatInput, ServerId};
    let fake = FakeBackend::canned();
    fake.set_page(
        ChatId("alice".into()),
        (1001..=1240)
            .map(|id| FakeBackend::message("alice", id, &format!("history {id}"), false))
            .collect(),
    );
    fake.add_chat(ChatInput {
        id: ChatId("alice".into()),
        title: "葵".into(),
        unread_count: 160,
        last_message_id: Some(ServerId("1240".into())),
        ..Default::default()
    });
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.until(|s| s.unread_divider.is_some());
    let divider = app.state.unread_divider.unwrap();
    assert_eq!(
        app.state.messages[&divider].server_id,
        Some(ServerId("1081".into()))
    );
    assert!(matches!(app.state.window_anchor, Anchor::Unread));
    assert!(!app.state.reaches_end); // The unread boundary lies outside the latest 80.
    assert!(
        app.state
            .rows()
            .iter()
            .any(|m| m.sort < app.state.messages[&divider].sort)
    );
}

#[test]
fn reentrant_panel_requests_are_queued_without_borrowing_the_controller() {
    use line_wasm_poc::controller::{ActionQueue, Panel};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    let actions = ActionQueue::<AppState>::default();
    let callbacks = actions.clone();
    actions.push(move |state| {
        state.panel = Panel::Settings;
        // A presenter-triggered callback only records another action.
        callbacks.push(|state| state.panel = Panel::Detail);
        assert_eq!(state.panel, Panel::Settings);
    });
    actions.drain(&mut app.state);
    assert_eq!(app.state.panel, Panel::Detail);
}

#[test]
fn rpc_failures_retry_the_intent_without_flapping_the_connection() {
    use line_wasm_poc::{
        backend::{Error, FakeRpc},
        store::{ChatInput, ServerId},
    };
    let fake = FakeBackend::canned();
    let chat = ChatId("alice".into());
    fake.add_chat(ChatInput {
        id: chat.clone(),
        ..Default::default()
    });
    let failures = [
        (FakeRpc::Chats, Error::Server(500)),
        (FakeRpc::Messages(chat.clone()), Error::Decode),
        (FakeRpc::Details(chat.clone()), Error::Crypto),
        (FakeRpc::Receipts(chat.clone()), Error::Server(503)),
        (
            FakeRpc::Read(chat.clone()),
            Error::Rejected(RejectReason::ReadFailed),
        ),
    ];
    for (rpc, error) in &failures {
        fake.fail_rpc(rpc.clone(), error.clone());
    }
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.select("alice");
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.server_id == Some(ServerId("10".into())))
    });
    app.send(Intent::MarkRead {
        chat: chat.clone(),
        id: ServerId("10".into()),
    });
    app.until(|s| {
        assert!(!matches!(s.connection, Connection::Offline { .. }));
        let calls = fake.calls();
        failures
            .iter()
            .all(|(rpc, _)| calls.iter().filter(|c| *c == rpc).count() >= 2)
    });
    assert_eq!(app.state.connection, Connection::Online);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|r| **r == FakeRpc::Profile)
            .count(),
        1
    );
    assert_eq!(fake.resumed_revisions().len(), 1);
}

#[test]
fn opened_chat_history_and_reads_run_before_missing_details_sweep() {
    use line_wasm_poc::{
        backend::FakeRpc,
        store::{ChatInput, ServerId, Store},
    };
    let dir = std::env::temp_dir().join(format!("line-core-priority-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let chat = ChatId("alice".into());
    {
        let mut store = Store::open(&dir).unwrap();
        store
            .upsert_chats(
                &[ChatInput {
                    id: chat.clone(),
                    title: "葵".into(),
                    ..Default::default()
                }],
                false,
            )
            .unwrap();
        store.mark_read(&chat, ServerId("1".into())).unwrap();
        store
            .set_prefs(&Prefs {
                last_chat: Some(ChatId("alice".into())),
                ..Prefs::default()
            })
            .unwrap();
    }
    let fake = FakeBackend::canned();
    for i in 0..40 {
        fake.add_chat(ChatInput {
            id: ChatId(format!("missing-{i}")),
            ..Default::default()
        });
    }
    let mut app = App::new(fake.clone(), StoreLocation::Directory(dir.clone()));
    app.until(|_| {
        fake.calls()
            .iter()
            .any(|r| matches!(r, FakeRpc::Details(_)))
    });
    let calls = fake.calls();
    let details = calls
        .iter()
        .position(|r| matches!(r, FakeRpc::Details(_)))
        .unwrap();
    assert!(
        calls
            .iter()
            .position(|r| *r == FakeRpc::Read(chat.clone()))
            .unwrap()
            < details
    );
    assert!(
        calls
            .iter()
            .position(|r| *r == FakeRpc::Messages(chat.clone()))
            .unwrap()
            < details
    );
    assert!(!calls.contains(&FakeRpc::Details(chat)));
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn first_identity_keeps_local_work_and_sends_pending() {
    use line_wasm_poc::store::{Draft, Store};
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!("line-first-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let fake = FakeBackend::canned();
    fake.offline(true);
    let mut app = App::new(
        fake.clone(),
        StoreLocation::Account {
            root: root.clone(),
            mid: None,
        },
    );
    app.until(|s| matches!(s.connection, Connection::Offline { .. }));
    app.select("alice");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "before identity".into(),
    });
    let key = app.body_key("before identity");
    assert_eq!(app.state.messages[&key].delivery, Delivery::Pending);
    app.send(Intent::Draft {
        chat: ChatId("alice".into()),
        draft: Draft {
            text: "keep draft".into(),
            cursor: 4,
            anchor: 2,
            updated: 123,
        },
    });
    app.send(Intent::Prefs(Prefs {
        text_size: 19,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 19);
    fake.offline(false);
    app.online();
    app.until(|s| {
        s.messages
            .values()
            .any(|m| message_text(m) == "before identity" && m.delivery == Delivery::Sent)
    });
    drop(app);
    let store = Store::open(&root.join(format!("{:x}", Sha256::digest("self")))).unwrap();
    let draft = store.load_draft(&ChatId("alice".into())).unwrap().unwrap();
    assert_eq!(
        (draft.text.as_str(), draft.cursor, draft.anchor),
        ("keep draft", 4, 2)
    );
    assert_eq!(store.prefs().unwrap().text_size, 19);
    assert!(store.all_pending().unwrap().is_empty());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn traced_reactions_replace_remove_and_refresh_quietly() {
    use serde_json::json;
    let fake = FakeBackend::canned();
    let mut row = FakeBackend::message("alice", 10, "react here", false);
    row.reactions = vec![Reaction {
        actor: "other".into(),
        kind: ReactionKind::Predefined(3),
    }];
    fake.set_page(ChatId("alice".into()), vec![row]);
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    let key = app.body_key("react here");
    let anchor = format!("{:?}", app.state.window_anchor);
    let unread = app.state.chats[&ChatId("alice".into())].chat.unread_count;
    for curr in [
        json!({"predefinedReactionType":2}),
        json!({"predefinedReactionType":4}),
        json!({}),
        serde_json::Value::Null,
    ] {
        let mut payload = json!({"chatMid":"alice"});
        if !curr.is_null() {
            payload["curr"] = curr.clone();
        }
        fake.inject_raw(json!({"type":139,"param1":"10","param2":payload.to_string()}));
        let revision = fake.revision() - 1;
        app.until(|s| s.messages[&key].reactions_rev >= revision);
        let entries = app.state.messages[&key].reactions.as_slice();
        assert_eq!(entries.iter().filter(|e| e.actor == "other").count(), 1);
        assert_eq!(
            entries.iter().filter(|e| e.actor == "self").count(),
            usize::from(curr.as_object().is_some_and(|c| !c.is_empty()))
        );
        assert_eq!(format!("{:?}", app.state.window_anchor), anchor);
        assert_eq!(
            app.state.chats[&ChatId("alice".into())].chat.unread_count,
            unread
        );
    }
    fake.inject_raw(json!({"type":140,"param1":"10","param2":json!({"chatMid":"alice","fromUserMid":"other","curr":{"predefinedReactionType":7}}).to_string()}));
    app.until(|s| s.messages[&key].reactions[0].kind == ReactionKind::Predefined(7));
    let calls = fake.page_requests("alice");
    let mut refreshed = fake.page(&ChatId("alice".into()));
    refreshed[0].reactions = vec![Reaction {
        actor: "unknown-before-refresh".into(),
        kind: ReactionKind::Predefined(3),
    }];
    fake.set_page(ChatId("alice".into()), refreshed);
    fake.inject_raw(json!({"type":140,"param1":"10","param2":json!({"chatMid":"alice","curr":{"predefinedReactionType":3}}).to_string()}));
    app.until(|_| fake.page_requests("alice") > calls);
    app.until(|s| s.messages[&key].reactions[0].actor == "unknown-before-refresh");
    assert_eq!(app.notifications, 0);
    // A FIFO user intent fences the refresh publication and checks for alerts.
    app.send(Intent::Prefs(Prefs {
        text_size: 35,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 35);
    assert_eq!(format!("{:?}", app.state.window_anchor), anchor);
    assert_eq!(
        app.state.chats[&ChatId("alice".into())].chat.unread_count,
        unread
    );
}

#[test]
fn unsend_success_echo_and_failure_keep_the_same_row() {
    use line_wasm_poc::backend::{Error, FakeRpc};
    use line_wasm_poc::store::ServerId;
    let fake = FakeBackend::canned();
    fake.set_page(
        ChatId("alice".into()),
        vec![FakeBackend::message("alice", 11, "own confirmed", true)],
    );
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    let key = app.body_key("own confirmed");
    fake.fail_rpc(
        FakeRpc::Unsend(ServerId("11".into())),
        Error::Rejected(RejectReason::SyntheticUnsend),
    );
    app.send(Intent::Unsend(key));
    let mut failures = 0;
    loop {
        let delta = app
            .worker
            .deltas
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        if let Delta::UnsendFailed(k) = delta {
            assert_eq!(k, key);
            failures += 1;
            break;
        }
        assert!(!matches!(delta, Delta::StorageFailed(_)));
        app.state.apply(delta);
    }
    assert_eq!(failures, 1);
    assert!(!app.state.messages[&key].body.is_withdrawn());
    assert_eq!(message_text(&app.state.messages[&key]), "own confirmed");
    app.send(Intent::Unsend(key));
    app.until(|s| s.messages[&key].body.is_withdrawn());
    assert!(app.state.messages[&key].body.is_withdrawn());
    fake.inject_raw(serde_json::json!({"type":64,"param1":"alice","param2":"11"}));
    app.send(Intent::Prefs(Prefs {
        text_size: 34,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 34);
    assert!(app.state.messages[&key].body.is_withdrawn());
    assert_eq!(app.state.messages.len(), 1);
    app.send(Intent::Unsend(key));
    app.send(Intent::Prefs(Prefs {
        text_size: 33,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 33);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Unsend(_)))
            .count(),
        2
    );
}

#[test]
fn cached_intents_complete_while_backend_rpc_is_blocked() {
    use line_wasm_poc::{backend::*, engine::WindowSub, store::*};
    use std::sync::{Arc, atomic::AtomicBool};
    struct Blocked(
        FakeBackend,
        Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        std::sync::mpsc::Sender<()>,
    );
    impl Blocked {
        fn wait(&self) {
            let _ = self.2.send(());
            let mut released = self.1.0.lock().unwrap();
            while !*released {
                released = self.1.1.wait(released).unwrap();
            }
        }
    }
    impl Backend for Blocked {
        fn contact(&mut self, mid: &str) -> Result<line_wasm_poc::store::Friend> {
            self.0.contact(mid)
        }
        fn notice(&mut self, mid: &str) -> Result<serde_json::Value> {
            self.0.notice(mid)
        }
        fn full_chat(&mut self, chat: &ChatId) -> Result<serde_json::Value> {
            self.0.full_chat(chat)
        }
        fn profile_action(
            &mut self,
            action: &line_wasm_poc::backend::ProfileAction,
            full: serde_json::Value,
            seq: u32,
        ) -> Result<()> {
            self.0.profile_action(action, full, seq)
        }

        fn sign_in(&mut self, c: Arc<AtomicBool>, d: PairDeliver) -> Result<()> {
            self.0.sign_in(c, d)
        }
        fn friends(&mut self) -> Result<Vec<line_wasm_poc::store::Friend>> {
            self.wait();
            self.0.friends()
        }
        fn members(&mut self, c: &ChatId) -> Result<Vec<line_wasm_poc::store::Member>> {
            self.wait();
            self.0.members(c)
        }
        fn start_assets(&mut self, s: Arc<AtomicBool>, d: Deliver) {
            self.0.start_assets(s, d)
        }
        fn authorize(
            &mut self,
            a: &line_wasm_poc::client::desktop::assets::Asset,
        ) -> std::result::Result<Option<String>, line_wasm_poc::client::desktop::assets::Failure>
        {
            self.0.authorize(a)
        }
        fn retry_asset(&mut self, k: &str) {
            self.0.retry_asset(k)
        }
        fn media(
            &mut self,
            a: &line_wasm_poc::client::desktop::assets::Asset,
            p: line_wasm_poc::client::desktop::assets::Purpose,
        ) -> Result<Option<line_wasm_poc::client::desktop::assets::Payload>> {
            self.0.media(a, p)
        }
        fn profile(&mut self) -> Result<Profile> {
            self.wait();
            self.0.profile()
        }
        fn chats(&mut self, n: Option<String>) -> Result<ChatPage> {
            self.wait();
            self.0.chats(n)
        }
        fn details(&mut self, c: &ChatId) -> Result<Vec<ChatInput>> {
            self.wait();
            self.0.details(c)
        }
        fn messages(&mut self, c: &ChatId, k: &PageKind) -> Result<Page> {
            self.wait();
            self.0.messages(c, k)
        }
        fn send(&mut self, c: &ChatId, t: &str, s: u32) -> Result<MessageInput> {
            self.wait();
            self.0.send(c, t, s)
        }
        fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
            self.0.react(id, kind, seq)
        }
        fn sticker_packages(&mut self) -> Result<Vec<line_wasm_poc::store::StickerPackage>> {
            self.0.sticker_packages()
        }
        fn send_sticker(
            &mut self,
            chat: &ChatId,
            sticker: &line_wasm_poc::store::Content,
            seq: u32,
        ) -> Result<MessageInput> {
            self.0.send_sticker(chat, sticker, seq)
        }
        fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput> {
            self.0.create_chat(name, targets, seq)
        }
        fn unsend(&mut self, i: &ServerId, s: u32) -> Result<()> {
            self.wait();
            self.0.unsend(i, s)
        }
        fn receipts(&mut self, c: &ChatId) -> Result<Vec<Option<ServerId>>> {
            self.wait();
            self.0.receipts(c)
        }
        fn mark_read(&mut self, c: &ChatId, i: &ServerId, seq: u32) -> Result<()> {
            self.wait();
            self.0.mark_read(c, i, seq)
        }
        fn stream(
            &mut self,
            r: i64,
            s: Arc<AtomicBool>,
            d: Deliver,
        ) -> Result<Box<dyn FnOnce() + Send>> {
            self.0.stream(r, s, d)
        }
        fn decode(&mut self, r: serde_json::Value) -> Result<LiveOp> {
            self.0.decode(r)
        }
        fn refresh_token(&mut self, stop: &AtomicBool) -> Result<()> {
            self.0.refresh_token(stop)
        }
    }

    let root = std::env::temp_dir().join(format!("line-blocked-rpc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    use sha2::{Digest, Sha256};
    let account = root.join(format!("{:x}", Sha256::digest("self")));
    let mut store = Store::open(&account).unwrap();
    store.set_account("self", Profile::default()).unwrap();
    std::fs::write(root.join("last-mid"), "self").unwrap();
    let chats = (0..150)
        .map(|i| ChatInput {
            id: ChatId(format!("cached-{i}")),
            title: format!("Stored chat {i}"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    store.upsert_chats(&chats, true).unwrap();
    let rows = (1..=300)
        .map(|i| FakeBackend::message("cached-0", i, "stored history", false))
        .collect::<Vec<_>>();
    store
        .apply_page(&chats[0].id, PageKind::Recent, &rows, true)
        .unwrap();
    store
        .set_prefs(&Prefs {
            last_chat: Some(ChatId("cached-0".into())),
            ..Prefs::default()
        })
        .unwrap();
    drop(store);
    let released = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    // Always release the net thread, including on a failed assertion.
    struct Release(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    let release = Release(released.clone());
    let (entered, rpc) = std::sync::mpsc::channel();
    let fake = FakeBackend::default();
    fake.set_chats(chats.clone());
    let (initialize, initialized) = std::sync::mpsc::channel();
    let worker = Worker::start_with(
        StoreLocation::Account {
            root: root.clone(),
            mid: None,
        },
        move || {
            initialized.recv()?;
            Ok(Blocked(fake, released, entered))
        },
    );
    let receive = |predicate: &dyn Fn(&Delta) -> bool| {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let delta = worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("cached work must finish before RPC is released");
            assert!(!matches!(delta, Delta::Fatal | Delta::StorageFailed(_)));
            if predicate(&delta) {
                return delta;
            }
        }
    };
    receive(&|d| matches!(d, Delta::Window { sub: 1, .. }));
    // The cached account and window arrive even while backend initialization waits.
    initialize.send(()).unwrap();
    rpc.recv_timeout(Duration::from_secs(2))
        .expect("profile RPC entered and blocked");
    let mut state = AppState::default();
    state.members.push(line_wasm_poc::store::Member {
        resolved: true,
        contact: line_wasm_poc::store::Friend {
            profile: Default::default(),
            id: "old".into(),
            name: "Old chat".into(),
            status: String::new(),
            group: false,
            avatar: None,
            member_count: None,
        },
        is_self: false,
    });
    state.select(chats[0].id.clone());
    assert!(state.window_loading);
    assert!(state.members.is_empty());
    state.apply(Delta::Window {
        sub: state.window_sub,
        upsert: Vec::new(),
        remove: Vec::new(),
        gaps: Vec::new(),
        reaches_start: true,
        reaches_end: true,
        replies: Vec::new(),
        unread_anchor: None,
    });
    assert!(!state.window_loading);
    for (sub, chat) in [(2, "cached-1"), (3, "cached-0")] {
        worker.sender.send(Intent::Window(WindowSub {
            sub,
            chat: ChatId(chat.into()),
            anchor: Anchor::Latest,
            before: 20,
            after: 0,
        }));
        receive(&|d| matches!(d, Delta::Window { sub: s, .. } if *s == sub));
    }
    worker.sender.send(Intent::Friends);
    receive(&|d| matches!(d, Delta::Friends(_)));
    worker.sender.send(Intent::Members(chats[0].id.clone()));
    receive(&|d| matches!(d, Delta::Members { .. }));
    worker.sender.send(Intent::Asset(FakeBackend::asset(false)));
    receive(&|d| matches!(d, Delta::Asset { .. }));
    worker.sender.send(Intent::Page {
        chat: chats[0].id.clone(),
        kind: PageKind::Older {
            before: Cursor {
                sort_ts: rows[280].created_ms,
                sort_id: format!("{:020}", 281),
            },
        },
    });
    worker.sender.send(Intent::Window(WindowSub {
        sub: 4,
        chat: chats[0].id.clone(),
        anchor: Anchor::Latest,
        before: 100,
        after: 0,
    }));
    let delta = receive(&|d| matches!(d, Delta::Window { sub: 4, .. }));
    assert!(matches!(delta, Delta::Window { upsert, .. } if upsert.len() == 101));
    worker.sender.send(Intent::MoreChats);
    assert!(
        matches!(receive(&|d| matches!(d, Delta::Chats { .. })), Delta::Chats { order, .. } if order.len() == 150)
    );
    worker.sender.send(Intent::SearchChats {
        sub: 9,
        query: "Stored chat 149".into(),
    });
    assert!(
        matches!(receive(&|d| matches!(d, Delta::Chats { sub:9, .. })), Delta::Chats { upsert, .. } if upsert.len() == 1 && upsert[0].chat.id == chats[149].id)
    );
    worker.sender.send(Intent::ChatList { sub: 10, limit: 10 });
    receive(&|d| matches!(d, Delta::Chats { sub: 10, .. }));
    worker.sender.send(Intent::Draft {
        chat: chats[149].id.clone(),
        draft: Draft {
            text: "local draft".into(),
            cursor: 11,
            anchor: 11,
            updated: 0,
        },
    });
    assert!(
        matches!(receive(&|d| matches!(d, Delta::Chats { .. })), Delta::Chats { upsert, .. } if upsert.len() == 1 && upsert[0].chat.id == chats[149].id)
    );
    worker.sender.send(Intent::Prefs(Prefs {
        text_size: 32,
        ..Prefs::default()
    }));
    loop {
        let delta = receive(&|_| true);
        assert!(!matches!(delta, Delta::Window { .. }));
        if matches!(delta, Delta::Prefs(_)) {
            break;
        }
    }
    drop(worker);
    drop(release);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeated_retry_and_discard_preserve_in_flight_send() {
    use line_wasm_poc::{backend::*, store::*};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };

    let mut store = Store::open_in_memory().unwrap();
    let (key, _) = store
        .insert_pending(
            &ChatId("alice".into()),
            LocalId("guard".into()),
            1,
            Content::text("guard"),
            1,
        )
        .unwrap();
    assert!(store.retry(key, 2).is_err());
    assert!(store.discard(key).is_err());
    assert_eq!(store.message(key).unwrap().request_seq, Some(1));

    struct BlockedSend {
        fake: FakeBackend,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }
    impl Backend for BlockedSend {
        fn contact(&mut self, mid: &str) -> Result<line_wasm_poc::store::Friend> {
            self.fake.contact(mid)
        }
        fn notice(&mut self, mid: &str) -> Result<serde_json::Value> {
            self.fake.notice(mid)
        }
        fn full_chat(&mut self, chat: &ChatId) -> Result<serde_json::Value> {
            self.fake.full_chat(chat)
        }
        fn profile_action(
            &mut self,
            action: &line_wasm_poc::backend::ProfileAction,
            full: serde_json::Value,
            seq: u32,
        ) -> Result<()> {
            self.fake.profile_action(action, full, seq)
        }

        fn sign_in(&mut self, c: Arc<AtomicBool>, d: PairDeliver) -> Result<()> {
            self.fake.sign_in(c, d)
        }
        fn friends(&mut self) -> Result<Vec<line_wasm_poc::store::Friend>> {
            self.fake.friends()
        }
        fn members(&mut self, c: &ChatId) -> Result<Vec<line_wasm_poc::store::Member>> {
            self.fake.members(c)
        }
        fn start_assets(&mut self, s: Arc<AtomicBool>, d: Deliver) {
            self.fake.start_assets(s, d)
        }
        fn authorize(
            &mut self,
            a: &line_wasm_poc::client::desktop::assets::Asset,
        ) -> std::result::Result<Option<String>, line_wasm_poc::client::desktop::assets::Failure>
        {
            self.fake.authorize(a)
        }
        fn retry_asset(&mut self, k: &str) {
            self.fake.retry_asset(k)
        }
        fn media(
            &mut self,
            a: &line_wasm_poc::client::desktop::assets::Asset,
            p: line_wasm_poc::client::desktop::assets::Purpose,
        ) -> Result<Option<line_wasm_poc::client::desktop::assets::Payload>> {
            self.fake.media(a, p)
        }
        fn profile(&mut self) -> Result<Profile> {
            self.fake.profile()
        }
        fn chats(&mut self, n: Option<String>) -> Result<ChatPage> {
            self.fake.chats(n)
        }
        fn details(&mut self, c: &ChatId) -> Result<Vec<ChatInput>> {
            self.fake.details(c)
        }
        fn messages(&mut self, c: &ChatId, k: &PageKind) -> Result<Page> {
            self.fake.messages(c, k)
        }
        fn send(&mut self, c: &ChatId, t: &str, s: u32) -> Result<MessageInput> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.calls.load(std::sync::atomic::Ordering::Relaxed) > 1 {
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.fake.send(c, t, s)
        }
        fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
            self.fake.react(id, kind, seq)
        }
        fn sticker_packages(&mut self) -> Result<Vec<line_wasm_poc::store::StickerPackage>> {
            self.fake.sticker_packages()
        }
        fn send_sticker(
            &mut self,
            chat: &ChatId,
            sticker: &line_wasm_poc::store::Content,
            seq: u32,
        ) -> Result<MessageInput> {
            self.fake.send_sticker(chat, sticker, seq)
        }
        fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput> {
            self.fake.create_chat(name, targets, seq)
        }
        fn unsend(&mut self, i: &ServerId, s: u32) -> Result<()> {
            self.fake.unsend(i, s)
        }
        fn receipts(&mut self, c: &ChatId) -> Result<Vec<Option<ServerId>>> {
            self.fake.receipts(c)
        }
        fn mark_read(&mut self, c: &ChatId, i: &ServerId, seq: u32) -> Result<()> {
            self.fake.mark_read(c, i, seq)
        }
        fn stream(
            &mut self,
            r: i64,
            s: Arc<AtomicBool>,
            d: Deliver,
        ) -> Result<Box<dyn FnOnce() + Send>> {
            self.fake.stream(r, s, d)
        }
        fn decode(&mut self, r: serde_json::Value) -> Result<LiveOp> {
            self.fake.decode(r)
        }
        fn refresh_token(&mut self, stop: &AtomicBool) -> Result<()> {
            self.fake.refresh_token(stop)
        }
    }
    let fake = FakeBackend::canned();
    fake.fail_send();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut app = App {
        worker: Worker::start(
            BlockedSend {
                fake: fake.clone(),
                entered: entered_tx,
                release: release_rx,
                calls: calls.clone(),
            },
            StoreLocation::Memory,
        ),
        state: AppState::default(),
        notifications: 0,
    };
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "one retry".into(),
    });
    let key = app.body_key("one retry");
    app.until(|s| matches!(s.messages[&key].delivery, Delivery::Rejected { .. }));
    app.send(Intent::Retry(key));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    app.until(|s| s.messages[&key].delivery == Delivery::Pending);
    let seq = app.state.messages[&key].request_seq;
    app.send(Intent::Retry(key));
    app.send(Intent::Discard(key));
    app.send(Intent::Prefs(Prefs {
        text_size: 31,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 31);
    assert_eq!(app.state.messages[&key].request_seq, seq);
    release.send(()).unwrap();
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    let friends_calls = fake.friends_calls();
    app.send(Intent::Friends);
    app.until(|_| fake.friends_calls() > friends_calls);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|m| message_text(m) == "one retry")
            .count(),
        1
    );
}

fn profile_action(app: &mut App, action: line_wasm_poc::backend::ProfileAction) {
    app.state.action = Some(action.clone());
    app.send(Intent::ProfileAction(action));
    app.until(|s| s.action.is_none());
}
#[test]
fn rename_contact_failure_then_success_updates_the_store_page() {
    use line_wasm_poc::backend::{Error, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::Friends);
    app.until(|s| !s.friends.is_empty());
    let action = ProfileAction::Rename {
        mid: "alice".into(),
        name: "Aoi 🌿".into(),
    };
    fake.fail_rpc(FakeRpc::Action(action.clone()), Error::Decode);
    profile_action(&mut app, action.clone());
    assert_eq!(app.state.contacts["alice"].name, "葵");
    assert_eq!(
        app.state.action_error,
        Some(line_wasm_poc::engine::ProfileFailure::Rename)
    );
    profile_action(&mut app, action);
    assert_eq!(app.state.contacts["alice"].name, "Aoi 🌿");
    assert_eq!(
        app.state.contacts["alice"].profile.original_name.as_deref(),
        Some("葵")
    );
    assert!(app.state.action_error.is_none());
    app.send(Intent::Contact("alice".into()));
    app.until(|s| s.contacts["alice"].profile.overridden_name.as_deref() == Some("Aoi 🌿"));
}
#[test]
fn add_friend_failure_then_success_changes_relation_in_place() {
    use line_wasm_poc::backend::{Error, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut members = fake.clone();
    use line_wasm_poc::backend::Backend;
    let mut rows = members.members(&ChatId("weekend".into())).unwrap();
    rows[0].contact.profile = line_wasm_poc::store::ContactProfile {
        relation: Some(2),
        ..Default::default()
    };
    let mid = rows[0].contact.id.clone();
    fake.set_members(rows);
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::Contact(mid.clone()));
    app.until(|s| s.contacts.contains_key(&mid));
    // Synthetic contact source prefers the friend record, so provide the observed non-friend snapshot.
    let mut row = app.state.contacts[&mid].clone();
    row.profile = line_wasm_poc::store::ContactProfile {
        relation: Some(2),
        ..Default::default()
    };
    fake.set_friends(vec![row]);
    app.send(Intent::Contact(mid.clone()));
    app.until(|s| s.contacts[&mid].profile.relation == Some(2));
    let action = ProfileAction::AddFriend {
        mid: mid.clone(),
        chat: "weekend".into(),
    };
    fake.fail_rpc(FakeRpc::Action(action.clone()), Error::Decode);
    profile_action(&mut app, action.clone());
    assert_eq!(app.state.contacts[&mid].profile.relation, Some(2));
    assert_eq!(
        app.state.action_error,
        Some(line_wasm_poc::engine::ProfileFailure::AddFriend)
    );
    profile_action(&mut app, action);
    assert_eq!(app.state.contacts[&mid].profile.relation, Some(0));
    assert!(app.state.action_error.is_none());
}
#[test]
fn mute_failure_then_success_preserves_full_chat_and_updates_notifications() {
    use line_wasm_poc::backend::{Error, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    let chat = ChatId("weekend".into());
    app.send(Intent::GroupInfo(chat.clone()));
    app.until(|s| {
        s.group_info
            .get(&ChatId("weekend".into()))
            .is_some_and(|v| v.is_object())
    });
    let action = ProfileAction::Mute {
        chat: chat.clone(),
        disabled: true,
    };
    fake.fail_rpc(FakeRpc::Action(action.clone()), Error::Decode);
    profile_action(&mut app, action.clone());
    assert!(!app.state.chats[&chat].chat.muted);
    assert_eq!(
        app.state.action_error,
        Some(line_wasm_poc::engine::ProfileFailure::Mute)
    );
    profile_action(&mut app, action);
    assert!(app.state.chats[&chat].chat.muted);
    assert_eq!(app.state.group_info[&chat]["notificationDisabled"], true);
    assert!(app.state.group_info[&chat]["extra"]["groupExtra"].is_object());
    assert!(app.state.action_error.is_none());
}
#[test]
fn leave_failure_then_success_retains_a_read_only_chat_in_the_list() {
    use line_wasm_poc::backend::{Error, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("weekend");
    app.until(|s| !s.window_loading);
    let chat = ChatId("weekend".into());
    let action = ProfileAction::Leave { chat: chat.clone() };
    fake.fail_rpc(FakeRpc::Action(action.clone()), Error::Decode);
    profile_action(&mut app, action.clone());
    assert!(!app.state.chats[&chat].chat.left);
    assert_eq!(
        app.state.action_error,
        Some(line_wasm_poc::engine::ProfileFailure::Leave)
    );
    profile_action(&mut app, action);
    assert!(app.state.chats[&chat].chat.left);
    assert!(app.state.chat_order.contains(&chat));
    app.state.composer = "still here".into();
    assert!(!app.state.can_send());
    assert!(app.state.action_error.is_none());
}
#[test]
fn detail_stack_pop_and_close_use_the_recorded_opener() {
    use line_wasm_poc::controller::DetailPage;
    let mut state = AppState::default();
    let chat = ChatId("weekend".into());
    state.push_detail(DetailPage::GroupInfo(chat.clone()), "header".into());
    state.push_detail(DetailPage::Members(chat), "members row".into());
    state.push_detail(DetailPage::Profile("alice".into()), "alice row".into());
    assert_eq!(state.pop_detail(false).as_deref(), Some("alice row"));
    assert!(matches!(state.detail_page(), Some(DetailPage::Members(_))));
    assert_eq!(state.pop_detail(true).as_deref(), Some("header"));
    assert!(state.detail.is_empty());
}

#[test]
fn traced_contact_and_chat_operations_refresh_the_affected_identity() {
    use line_wasm_poc::backend::{Backend, FakeRpc};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    for kind in [4, 49] {
        let mut source = fake.clone();
        let mut rows = source.friends().unwrap();
        rows[0].name = format!("Changed {kind}");
        rows[0].profile = line_wasm_poc::store::ContactProfile {
            relation: Some(0),
            ..Default::default()
        };
        fake.set_friends(rows);
        fake.inject_raw(serde_json::json!({"type":kind,"param1":"alice"}));
        app.until(|s| {
            s.contacts
                .get("alice")
                .is_some_and(|f| f.name == format!("Changed {kind}"))
        });
    }
    let chat = ChatId("weekend".into());
    for kind in [121, 122, 127, 128] {
        fake.add_chat(line_wasm_poc::store::ChatInput {
            id: chat.clone(),
            title: format!("Group {kind}"),
            is_group: true,
            ..Default::default()
        });
        fake.inject_raw(serde_json::json!({"type":kind,"param1":"weekend"}));
        app.until(|s| {
            s.chats
                .get(&ChatId("weekend".into()))
                .is_some_and(|c| c.chat.title == format!("Group {kind}"))
        });
    }
    fake.inject_raw(serde_json::json!({"type":41,"param1":"weekend"}));
    app.until(|s| !s.chats.contains_key(&chat));
    assert!(fake.calls().contains(&FakeRpc::Contact("alice".into())));
    assert!(fake.calls().contains(&FakeRpc::Details(chat)));
    assert_eq!(app.notifications, 0);
}

#[test]
fn compose_reaction_set_cancel_and_failure_revert() {
    use line_wasm_poc::backend::{Error, FakeRpc};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    let key = app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::React { key, kind: 3 });
    app.until(|s| s.messages[&key].reactions_rev > 0 && s.messages[&key].reactions.len() == 1);
    assert_eq!(app.state.messages[&key].reactions[0].actor, "self");
    app.send(Intent::React { key, kind: 3 });
    app.until(|s| s.messages[&key].reactions.is_empty() && s.messages[&key].reactions_rev > 1001);
    fake.fail_rpc(
        FakeRpc::React(app.state.messages[&key].server_id.clone().unwrap(), Some(2)),
        Error::Rejected(RejectReason::NotAccepted),
    );
    app.send(Intent::React { key, kind: 2 });
    app.until(|s| s.reaction_failure == Some(key));
    assert!(app.state.messages[&key].reactions.is_empty());
}

#[test]
fn compose_sticker_send_echo_dedup_and_recovery() {
    use line_wasm_poc::store::Content;
    let fake = FakeBackend::canned();
    fake.echo_first(true);
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::StickerPackages);
    app.until(|s| s.sticker_packages.len() == 2);
    let sticker = app.state.sticker_packages[0].stickers[0].clone();
    app.send(Intent::SendSticker {
        chat: ChatId("alice".into()),
        sticker: Box::new(sticker.clone()),
    });
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.body == sticker && m.local_id.is_some() && m.delivery == Delivery::Sent)
    });
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|m| matches!(m.body, Content::Sticker { .. }))
            .count(),
        1
    );
    fake.fail_send();
    let other = app.state.sticker_packages[1].stickers[0].clone();
    app.send(Intent::SendSticker {
        chat: ChatId("alice".into()),
        sticker: Box::new(other.clone()),
    });
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.body == other && matches!(m.delivery, Delivery::Rejected { .. }))
    });
    let key = app
        .state
        .messages
        .values()
        .find(|m| m.body == other)
        .unwrap()
        .key;
    app.send(Intent::Retry(key));
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|m| m.body == other)
            .count(),
        1
    );
}

#[test]
fn compose_new_chat_local_friend_then_group_success_and_failure() {
    use line_wasm_poc::{
        backend::{Error, FakeRpc},
        store::Friend,
    };
    let fake = FakeBackend::canned();
    fake.set_friends(vec![Friend {
        id: "new-friend".into(),
        name: "New friend".into(),
        ..Default::default()
    }]);
    fake.echo_first(true);
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::Friends);
    app.until(|s| s.friends.iter().any(|f| f.id == "new-friend"));
    app.select("new-friend");
    app.send(Intent::Prefs(Prefs {
        text_size: 17,
        ..Default::default()
    }));
    app.until(|s| s.prefs.text_size == 17);
    assert!(!app.state.chats.contains_key(&ChatId("new-friend".into())));
    assert!(
        !fake
            .calls()
            .contains(&FakeRpc::Details(ChatId("new-friend".into())))
    );
    app.send(Intent::Send {
        chat: ChatId("new-friend".into()),
        text: "First hello".into(),
    });
    let key = app.body_key("First hello");
    app.until(|s| {
        s.messages[&key].delivery == Delivery::Sent
            && s.chats
                .get(&ChatId("new-friend".into()))
                .is_some_and(|c| c.chat.title == "New friend")
    });
    app.send(Intent::CreateChat {
        name: "  Picnic  ".into(),
        targets: vec!["alice".into(), "new-friend".into(), "self".into()],
    });
    app.until(|s| matches!(s.created_chat, Some(Ok(_))));
    let id = app.state.created_chat.take().unwrap().unwrap();
    assert_eq!(app.state.chats[&id].chat.title, "Picnic");
    assert_eq!(app.state.chats[&id].chat.member_count, Some(3));
    app.select(&id.0);
    fake.fail_rpc(
        FakeRpc::CreateChat,
        Error::Rejected(RejectReason::NotAccepted),
    );
    app.send(Intent::CreateChat {
        name: "Failure".into(),
        targets: vec!["alice".into(), "new-friend".into()],
    });
    app.until(|s| matches!(s.created_chat, Some(Err(_))));
    assert!(!app.state.chats.values().any(|c| c.chat.title == "Failure"));
}

#[test]
fn signed_out_account_intents_never_reach_the_backend() {
    let fake = FakeBackend::signed_out();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.until(|s| s.connection == Connection::AuthExpired);
    for intent in [
        Intent::Friends,
        Intent::Contact("unknown-account-id".into()),
        Intent::GroupInfo(ChatId("unknown-chat-id".into())),
        Intent::Members(ChatId("unknown-chat-id".into())),
        Intent::StickerPackages,
        Intent::Retry(Key(999)),
        Intent::Send {
            chat: ChatId("unknown-chat-id".into()),
            text: "private text".into(),
        },
    ] {
        app.send(intent);
    }
    // A preference acknowledgment is a FIFO barrier after all account intents.
    app.send(Intent::Prefs(Prefs {
        text_size: 29,
        ..Default::default()
    }));
    app.until(|s| s.prefs.text_size == 29);
    assert!(fake.calls().is_empty());
    assert!(app.state.error.is_none());
    assert!(app.state.messages.is_empty());
}

#[test]
fn pairing_reasons_survive_the_engine() {
    use line_wasm_poc::client::error::SignInFailure::*;
    let fake = FakeBackend::signed_out();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.until(|s| s.connection == Connection::AuthExpired);
    for reason in [
        CryptoModule("/missing/crypto.wasm".into()),
        StateDirectory("/readonly/state".into()),
        Keyring,
        Network,
        Server,
        Phone,
        Cancelled,
    ] {
        fake.fail_sign_in(reason.clone());
        app.send(Intent::SignIn);
        loop {
            let delta = app
                .worker
                .deltas
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if let Delta::PairFailed { reason: actual } = delta {
                assert_eq!(actual, reason);
                break;
            }
            assert!(!matches!(
                delta,
                Delta::Fatal | Delta::StorageFailed(_) | Delta::InternalError(_)
            ));
            app.state.apply(delta);
        }
    }
    assert!(fake.calls().is_empty());
}

#[test]
fn invariant_errors_are_never_storage_failures() {
    use line_wasm_poc::{engine::StorageFailure, store::StoreError};
    for error in [
        StoreError::Invalid("impossible state".into()),
        StoreError::Sql(rusqlite::Error::QueryReturnedNoRows),
    ] {
        assert_eq!(StorageFailure::classify(&error), None);
    }
    for (code, expected) in [
        (rusqlite::ffi::SQLITE_FULL, StorageFailure::Full),
        (rusqlite::ffi::SQLITE_READONLY, StorageFailure::ReadOnly),
        (rusqlite::ffi::SQLITE_PERM, StorageFailure::Permission),
        (rusqlite::ffi::SQLITE_IOERR, StorageFailure::Io),
    ] {
        let error = StoreError::Sql(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ));
        assert_eq!(StorageFailure::classify(&error), Some(expected));
    }
}

#[test]
fn unknown_key_reports_a_bug_and_keeps_the_worker_running() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.send(Intent::Copy(Key(i64::MAX)));
    loop {
        let delta = app
            .worker
            .deltas
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        if let Delta::InternalError(kind) = delta {
            assert_eq!(kind, "unknown key");
            break;
        }
        assert!(!matches!(delta, Delta::StorageFailed(_) | Delta::Fatal));
        app.state.apply(delta);
    }
    assert!(app.state.error.is_none());
    app.send(Intent::Prefs(Prefs {
        text_size: 31,
        ..Default::default()
    }));
    app.until(|s| s.prefs.text_size == 31);
}
