use line_wasm_poc::{
    backend::{FakeBackend, LiveOp, Op},
    controller::AppState,
    engine::{Connection, Delta, Intent, StoreLocation, Worker},
    store::{ChatId, Delivery, Key},
};
use std::time::{Duration, Instant};
struct App {
    worker: Worker,
    state: AppState,
}
impl App {
    fn new(fake: FakeBackend, location: StoreLocation) -> Self {
        Self {
            worker: Worker::start(fake, location),
            state: AppState::default(),
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
            if let Delta::Error(e) = &d {
                panic!("engine: {e}");
            }
            self.state.apply(d);
        }
    }
    fn online(&mut self) {
        self.until(|s| s.connection == Connection::Online && s.chats.len() == 2);
    }
    fn body_key(&mut self, body: &str) -> Key {
        self.until(|s| s.messages.values().any(|m| m.body["body"] == body));
        self.state
            .messages
            .values()
            .find(|m| m.body["body"] == body)
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
    app.send(Intent::Pref {
        key: "stale_page_applied".into(),
        value: true.into(),
    });
    app.until(|s| s.prefs.contains_key("stale_page_applied"));
    assert_eq!(app.state.messages[&key].body["body"], "durable send");
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
        op: Op::Message(FakeBackend::message("alice", 50, "closed arrival", false)),
    });
    app.until(|s| {
        s.chats[&ChatId("alice".into())]
            .preview
            .as_ref()
            .is_some_and(|m| m.body["body"] == "closed arrival")
    });
    let pages = fake.page_requests("alice");
    app.select("alice");
    app.body_key("closed arrival");
    app.until(|_| fake.page_requests("alice") > pages);
    assert!(
        app.state
            .messages
            .values()
            .any(|m| m.body["body"] == "closed arrival")
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
                op: Op::Message(echo),
            });
            app.until(|s| s.messages[&key].body["sender_name"] == "葵");
        }
        assert_eq!(
            app.state
                .messages
                .values()
                .filter(|m| m.body["body"] == "echo")
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
            op: Op::Message(FakeBackend::message("alice", 75, "checkpoint", false)),
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
            && s.messages.values().any(|m| m.body["body"] == "checkpoint")
    });
    assert!(app.state.chats.values().all(|c| c.chat.unread_stale));
    fake.offline(false);
    app.until(|s| s.connection == Connection::Online); // Stream reconnect also resumes persisted revision.
    let deadline = Instant::now() + Duration::from_secs(4);
    while !fake.resumed_revisions().contains(&75) && Instant::now() < deadline {
        if let Ok(d) = app.worker.deltas.recv_timeout(Duration::from_millis(100)) {
            app.state.apply(d);
        }
    }
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
    app.state.apply(Delta::Error("store write failed".into()));
    assert!(app.state.can_send());
    app.send(Intent::Pref {
        key: "saved".into(),
        value: true.into(),
    });
    app.until(|s| s.prefs.contains_key("saved") && s.error.is_none());
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
        callbacks.push(|state| state.panel = Panel::Members);
        assert_eq!(state.panel, Panel::Settings);
    });
    actions.drain(&mut app.state);
    assert_eq!(app.state.panel, Panel::Members);
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
            Error::Rejected("read failed".into()),
        ),
    ];
    for (rpc, error) in &failures {
        fake.fail_rpc(rpc.clone(), error.clone());
    }
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.select("alice");
    let mut read = false;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let delta = app
            .worker
            .deltas
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("RPC retries finish");
        assert!(!matches!(
            delta,
            Delta::Connection(Connection::Offline { .. }) | Delta::Error(_)
        ));
        app.state.apply(delta);
        if !read
            && app
                .state
                .messages
                .values()
                .any(|m| m.server_id == Some(ServerId("10".into())))
        {
            app.send(Intent::MarkRead {
                chat: chat.clone(),
                id: ServerId("10".into()),
            });
            read = true;
        }
        let calls = fake.calls();
        if failures
            .iter()
            .all(|(rpc, _)| calls.iter().filter(|c| *c == rpc).count() >= 2)
        {
            break;
        }
    }
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
        store.set_pref("last_chat", "alice".into()).unwrap();
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
