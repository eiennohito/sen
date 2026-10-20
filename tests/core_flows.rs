#[path = "flows/blocked.rs"]
mod blocked;
use line::client::error::{Refusal, SignOutReason};
use line::{
    backend::{Failure, FakeBackend, FakeRpc, LiveOp, Op},
    controller::AppState,
    engine::{Connection, Delta, Intent, StoreLocation, Timing, Worker},
    store::{ChatId, Delivery, Key, Prefs, Reaction, ReactionKind},
};
use std::time::{Duration, Instant};

fn take_once<B: line::backend::Backend + 'static>(backend: B) -> impl FnMut() -> anyhow::Result<B> {
    let mut backend = Some(backend);
    move || Ok(backend.take().expect("backend already opened"))
}
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct App {
    worker: Worker,
    state: AppState,
    notifications: usize,
}
impl App {
    fn from_worker(worker: Worker) -> Self {
        Self {
            worker,
            state: AppState::default(),
            notifications: 0,
        }
    }

    fn new(fake: impl line::backend::Backend + 'static, location: StoreLocation) -> Self {
        Self {
            worker: Worker::start(
                location,
                take_once(fake),
                line::engine::WorkerOptions {
                    timing: Timing {
                        retry_initial: Duration::from_millis(5),
                        retry_max: Duration::from_millis(50),
                        offline_after: Duration::from_millis(100),
                        health_now: Instant::now,
                    },
                    ..Default::default()
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
    fn apply(&mut self, delta: Delta) {
        if matches!(delta, Delta::Notify(_)) {
            self.notifications += 1;
        }
        assert!(
            !matches!(delta, Delta::Fatal | Delta::StorageFailed(_)),
            "engine failure"
        );
        self.state.apply(delta);
    }
    fn until(&mut self, condition: impl Fn(&AppState) -> bool) {
        if condition(&self.state) {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.until_delta(|_| true, deadline.saturating_duration_since(Instant::now()));
            if condition(&self.state) {
                return;
            }
        }
    }
    fn until_delta(&mut self, condition: impl Fn(&Delta) -> bool, timeout: Duration) -> Delta {
        let deadline = Instant::now() + timeout;
        loop {
            let delta = self
                .worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("engine progress");
            let done = condition(&delta);
            self.apply(delta.clone());
            if done {
                return delta;
            }
        }
    }
    fn fence(&mut self) {
        let size = self.state.prefs.text_size + 1;
        self.send(Intent::Prefs(Prefs {
            text_size: size,
            ..self.state.prefs.clone()
        }));
        self.until(|s| s.prefs.text_size == size);
    }
    fn online(&mut self) {
        self.until(|s| s.connection == Connection::Online && s.chats.len() == 2);
    }
    fn body_key(&mut self, body: &str) -> Key {
        self.until(|s| s.messages.values().any(|m| m.body.as_text() == body));
        self.state
            .messages
            .values()
            .find(|m| m.body.as_text() == body)
            .unwrap()
            .key
    }
}
#[test]
fn m13_send_switch_return_with_stale_page() {
    let fake = FakeBackend::canned();
    let (backend, entered, release) =
        blocked::Blocked::new(fake.clone(), FakeRpc::Messages(ChatId("alice".into())), 0);
    let mut app = App::new(backend, StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "durable send".into(),
        reply: None,
    });
    let key = app.body_key("durable send");
    app.select("weekend");
    app.body_key("Shall we meet at the station?");
    fake.set_page(
        ChatId("alice".into()),
        vec![FakeBackend::message("alice", 10, "stale history", false)],
    );
    app.select("alice");
    app.until(|s| s.messages.contains_key(&key));
    release.send(()).unwrap();
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    // A network result fences the stale page completion.
    app.send(Intent::CreateChat {
        name: "page fence".into(),
        targets: vec!["alice".into(), "weekend".into()],
    });
    app.until_delta(
        |d| matches!(d, Delta::CreatedChat(Ok(_))),
        Duration::from_secs(5),
    );
    assert_eq!(app.state.messages[&key].body.as_text(), "durable send");
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
            .is_some_and(|m| m.body.as_text() == "closed arrival")
    });
    let pages = fake
        .calls()
        .iter()
        .filter(|c| **c == FakeRpc::Messages(ChatId("alice".into())))
        .count();
    app.select("alice");
    app.body_key("closed arrival");
    app.until(|_| {
        fake.calls()
            .iter()
            .filter(|c| **c == FakeRpc::Messages(ChatId("alice".into())))
            .count()
            > pages
    });
    assert!(
        app.state
            .messages
            .values()
            .any(|m| m.body.as_text() == "closed arrival")
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
            reply: None,
        });
        let key = app.body_key("echo");
        app.until(|s| s.messages[&key].delivery == Delivery::Sent);
        if !echo_first {
            let m = &app.state.messages[&key];
            let mut echo = FakeBackend::message(
                "alice",
                m.server_id.as_ref().unwrap().as_str().parse().unwrap(),
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
                .filter(|m| m.body.as_text() == "echo")
                .count(),
            1
        );
        assert!(app.state.messages.contains_key(&key));
    }
}
#[test]
fn offline_cache_reconnect_resumes_without_duplicate_notifications() {
    let temp = TempDir::new("line-core-offline");
    let dir = temp.0.clone();
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
        assert_eq!(app.notifications, 1);
    }
    fake.offline(true);
    let mut app = App::new(fake.clone(), StoreLocation::Directory(dir.clone()));
    app.until(|s| {
        !s.chats.is_empty()
            && s.messages
                .values()
                .any(|m| m.body.as_text() == "checkpoint")
    });
    assert!(app.state.chats.values().all(|chat| chat.chat.unread_stale));
    fake.offline(false);
    app.until(|s| s.connection == Connection::Online); // Stream reconnect also resumes persisted revision.
    app.until(|_| fake.calls().contains(&FakeRpc::Stream(75)));
    assert_eq!(app.notifications, 0);
    for (id, text, replay) in [
        (75, "checkpoint", true),
        (76, "catch-up", true),
        (77, "fresh after reconnect", false),
    ] {
        fake.inject(LiveOp {
            revision: id,
            replay,
            op: Op::Message(FakeBackend::message("alice", id, text, false).into()),
        });
    }
    app.body_key("fresh after reconnect");
    assert_eq!(app.notifications, 1);
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|m| m.body.as_text() == "checkpoint")
            .count(),
        1
    );
    drop(app);
}
#[test]
fn send_failures_preserve_delivery_and_retry_identity() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    fake.fail_rpc(
        FakeRpc::Send(ChatId("alice".into())),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "retry text".into(),
        reply: None,
    });
    let key = app.body_key("retry text");
    app.until(|s| matches!(s.messages[&key].delivery, Delivery::Rejected { .. }));
    app.send(Intent::Copy(key));
    app.until(|s| s.composer == "retry text");
    app.send(Intent::Retry(key));
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    fake.fail_rpc(
        FakeRpc::Send(ChatId("alice".into())),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "discard text".into(),
        reply: None,
    });
    let key = app.body_key("discard text");
    app.until(|s| matches!(s.messages[&key].delivery, Delivery::Rejected { .. }));
    app.send(Intent::Discard(key));
    app.until(|s| !s.messages.contains_key(&key));
    let sends_before = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, FakeRpc::Send(_)))
        .count();
    fake.fail_rpc(FakeRpc::Send(ChatId("alice".into())), Failure::NotDelivered);
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "one delivery".into(),
        reply: None,
    });
    let key = app.body_key("one delivery");
    app.until(|state| state.messages[&key].delivery == Delivery::Sent);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| **call == FakeRpc::Send(ChatId("alice".into())))
            .count(),
        sends_before + 2
    );
    assert_eq!(
        fake.page(&ChatId("alice".into()))
            .iter()
            .filter(|row| row.body.as_text() == "one delivery")
            .count(),
        1
    );
    assert_eq!(app.state.connection, Connection::Online);
    let sends_before = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, FakeRpc::Send(_)))
        .count();
    fake.fail_rpc(FakeRpc::Send(ChatId("alice".into())), Failure::Unanswered);
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "unanswered".into(),
        reply: None,
    });
    let key = app.body_key("unanswered");
    app.until(|state| {
        state.messages[&key].delivery
            == Delivery::Rejected {
                reason: Refusal::Unanswered,
            }
    });
    // Finish another network job before delivering the late echo: Unanswered
    // must stay rejected rather than being resent by the scheduler.
    app.send(Intent::CreateChat {
        name: "unanswered fence".into(),
        targets: vec!["alice".into(), "weekend".into()],
    });
    app.until_delta(
        |d| matches!(d, Delta::CreatedChat(Ok(_))),
        Duration::from_secs(5),
    );
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Send(_)))
            .count(),
        sends_before + 1
    );
    let seq = app.state.messages[&key].request_seq;
    let mut echo = FakeBackend::message("alice", 901, "unanswered", true);
    echo.request_seq = seq;
    fake.inject(LiveOp {
        revision: 2001,
        replay: false,
        op: Op::Message(Box::new(echo)),
    });
    app.until(|state| state.messages[&key].delivery == Delivery::Sent);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| **call == FakeRpc::Send(ChatId("alice".into())))
            .count(),
        sends_before + 1
    );
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|row| row.body.as_text() == "unanswered")
            .count(),
        1
    );

    fake.fail_rpc(
        FakeRpc::Send(ChatId("alice".into())),
        Failure::Refused(Refusal::Internal),
    );
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "in flight".into(),
        reply: None,
    });
    let key = app.body_key("in flight");
    app.until(|s| s.messages[&key].delivery.is_failed());
    let (entered, release) = fake.block_send();
    let sends_before = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, FakeRpc::Send(_)))
        .count();
    app.send(Intent::Retry(key));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    app.until(|s| s.messages[&key].delivery == Delivery::Pending);
    let seq = app.state.messages[&key].request_seq;
    app.send(Intent::Retry(key));
    app.send(Intent::Discard(key));
    app.fence();
    assert_eq!(app.state.messages[&key].request_seq, seq);
    release.send(()).unwrap();
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    // A network result fences all earlier send jobs.
    app.send(Intent::CreateChat {
        name: "send fence".into(),
        targets: vec!["alice".into(), "weekend".into()],
    });
    app.until_delta(
        |d| matches!(d, Delta::CreatedChat(_)),
        Duration::from_secs(5),
    );
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Send(_)))
            .count(),
        sends_before + 1
    );
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|m| m.body.as_text() == "in flight")
            .count(),
        1
    );
}

#[test]
fn opening_unread_chat_centers_the_window_on_the_divider() {
    use line::store::{Anchor, ChatInput, ServerId};
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
        server_unread: 160,
        last_message_id: Some(ServerId::new("1240").unwrap()),
        ..Default::default()
    });
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.until(|s| s.unread_divider.is_some());
    let divider = app.state.unread_divider.unwrap();
    assert_eq!(
        app.state.messages[&divider].server_id,
        Some(ServerId::new("1081").unwrap())
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
fn rpc_failures_retry_the_intent_without_flapping_the_connection() {
    use line::{
        backend::{Failure, FakeRpc},
        store::{ChatInput, ServerId},
    };
    let fake = FakeBackend::canned();
    let chat = ChatId("alice".into());
    fake.add_chat(ChatInput {
        id: chat.clone(),
        ..Default::default()
    });
    let failures = [
        (FakeRpc::Chats, Failure::Unanswered),
        (
            FakeRpc::Read(chat.clone()),
            Failure::Refused(line::client::error::Refusal::Server(
                line::client::error::ServerRefusal::Gateway(13),
            )),
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
            .any(|m| m.server_id == Some(ServerId::new("10").unwrap()))
    });
    app.send(Intent::MarkRead {
        chat: chat.clone(),
        id: ServerId::new("10").unwrap(),
    });
    app.until(|s| {
        assert!(!matches!(s.connection, Connection::Offline));
        let calls = fake.calls();
        failures.iter().all(|(rpc, error)| {
            calls.iter().filter(|c| *c == rpc).count()
                >= if matches!(error, Failure::Refused(_)) {
                    1
                } else {
                    2
                }
        })
    });
    assert_eq!(app.state.connection, Connection::Online);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|r| **r == FakeRpc::Profile)
            .count(),
        1
    );
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Stream(_)))
            .count(),
        1
    );
}

#[test]
fn first_identity_keeps_local_work_and_sends_pending() {
    use line::store::{Draft, Store};
    use sha2::{Digest, Sha256};
    let temp = TempDir::new("line-first-identity");
    let root = temp.0.clone();
    let fake = FakeBackend::canned();
    fake.offline(true);
    let mut app = App::new(
        fake.clone(),
        StoreLocation::Account {
            root: root.clone(),
            mid: None,
        },
    );
    app.until(|s| matches!(s.connection, Connection::Connecting));
    app.select("alice");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "before identity".into(),
        reply: None,
    });
    let key = app.body_key("before identity");
    assert_eq!(app.state.messages[&key].delivery, Delivery::Pending);
    app.send(Intent::Draft {
        chat: ChatId("alice".into()),
        draft: Draft {
            reply: None,
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
            .any(|m| m.body.as_text() == "before identity" && m.delivery == Delivery::Sent)
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
    let unread = app.state.chats[&ChatId("alice".into())].unread_count;
    for curr in [
        Some(json!({"predefinedReactionType":2})),
        Some(json!({"predefinedReactionType":4})),
        Some(serde_json::Value::Null),
    ] {
        let mut payload = json!({"chatMid":"alice"});
        if let Some(curr) = &curr {
            payload["curr"] = curr.clone();
        }
        fake.inject_raw(json!({"type":139,"param1":"10","param2":payload.to_string()}));
        let revision = fake.configure(|s| s.revision);
        app.until(|s| s.messages[&key].reactions_rev >= revision);
        let entries = app.state.messages[&key].reactions.as_slice();
        assert_eq!(entries.iter().filter(|e| e.actor == "other").count(), 1);
        assert_eq!(
            entries.iter().filter(|e| e.actor == "self").count(),
            usize::from(
                curr.as_ref()
                    .and_then(|v| v.as_object())
                    .is_some_and(|c| !c.is_empty())
            )
        );
        assert_eq!(format!("{:?}", app.state.window_anchor), anchor);
        assert_eq!(
            app.state.chats[&ChatId("alice".into())].unread_count,
            unread
        );
    }
    fake.inject_raw(json!({"type":139,"param1":"10","param2":json!({"chatMid":"alice","curr":{"predefinedReactionType":"2"}}).to_string()}));
    app.until(|s| {
        s.messages[&key]
            .reactions
            .iter()
            .any(|r| r.actor == "self" && r.kind == ReactionKind::Predefined(2))
    });
    for curr in [
        None,
        Some(json!({})),
        Some(json!({"customReactionType":{"id":"new"}})),
    ] {
        let mut data = json!({"chatMid":"alice"});
        if let Some(curr) = curr {
            data["curr"] = curr;
        }
        fake.inject_raw(
            json!({"type":140,"param1":"10","param3":"self","param2":data.to_string()}),
        );
        fake.inject_raw(json!({"type":140,"param1":"10","param3":"other","param2":json!({"chatMid":"alice","curr":{"predefinedReactionType":7}}).to_string()}));
        let revision = fake.configure(|s| s.revision);
        app.until(|s| s.messages[&key].reactions_rev >= revision);
        assert!(
            app.state.messages[&key]
                .reactions
                .iter()
                .any(|r| r.actor == "self" && r.kind == ReactionKind::Predefined(2))
        );
    }
    let mut refreshed = FakeBackend::message("alice", 10, "react here", false);
    refreshed.rev = fake.revision();
    refreshed.reactions = vec![Reaction {
        actor: "other".into(),
        kind: ReactionKind::Predefined(5),
    }];
    fake.set_page(ChatId("alice".into()), vec![refreshed]);
    fake.inject_raw(json!({"type":140,"param1":"10","param2":json!({"chatMid":"alice","curr":{"predefinedReactionType":3}}).to_string()}));
    app.until(|s| s.messages[&key].reactions[0].kind == ReactionKind::Predefined(5));
    // Fence the FIFO before checking that the refresh emitted no notification.
    app.send(Intent::Prefs(Prefs {
        text_size: 31,
        ..Prefs::default()
    }));
    app.until(|s| s.prefs.text_size == 31);
    assert_eq!(app.notifications, 0);
    assert_eq!(format!("{:?}", app.state.window_anchor), anchor);
    assert_eq!(
        app.state.chats[&ChatId("alice".into())].unread_count,
        unread
    );
}

#[test]
fn unsend_success_echo_and_failure_keep_the_same_row() {
    use line::backend::{Failure, FakeRpc};
    use line::store::ServerId;
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
        FakeRpc::Unsend(ServerId::new("11").unwrap()),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    app.send(Intent::Unsend(key));
    app.until_delta(
        |d| matches!(d, Delta::UnsendFailed(k) if *k == key),
        Duration::from_secs(5),
    );
    assert!(!app.state.messages[&key].body.is_withdrawn());
    assert_eq!(app.state.messages[&key].body.as_text(), "own confirmed");
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
fn contact_actions_report_failure_then_update_in_place() {
    use line::{
        backend::ProfileAction,
        engine::ProfileFailure,
        store::{ContactProfile, Friend},
    };
    let chat = ChatId("weekend".into());
    for (action, failure) in [
        (
            ProfileAction::Rename {
                mid: "alice".into(),
                name: "Aoi 🌿".into(),
            },
            ProfileFailure::Rename,
        ),
        (
            ProfileAction::AddFriend {
                mid: "alice".into(),
                chat: "weekend".into(),
            },
            ProfileFailure::AddFriend,
        ),
        (
            ProfileAction::Mute {
                chat: chat.clone(),
                disabled: true,
            },
            ProfileFailure::Mute,
        ),
        (
            ProfileAction::Leave { chat: chat.clone() },
            ProfileFailure::Leave,
        ),
    ] {
        let fake = FakeBackend::canned();
        let relation = if matches!(action, ProfileAction::AddFriend { .. }) {
            2
        } else {
            0
        };
        fake.set_friends(vec![Friend {
            id: "alice".into(),
            name: Some("葵".into()),
            profile: ContactProfile {
                relation: Some(relation),
                ..Default::default()
            },
            ..Default::default()
        }]);
        let mut app = App::new(fake.clone(), StoreLocation::Memory);
        app.online();
        app.select("weekend");
        app.body_key("Shall we meet at the station?");
        app.send(Intent::Contact("alice".into()));
        app.until(|s| s.contacts.contains_key("alice"));
        app.send(Intent::GroupInfo(chat.clone()));
        app.until(|s| s.group_info.get(&chat).is_some_and(|info| info.is_object()));
        fake.fail_rpc(
            FakeRpc::Action(action.clone()),
            Failure::Refused(Refusal::Internal),
        );
        for success in [false, true] {
            app.state.action = Some(action.clone());
            app.send(Intent::ProfileAction(action.clone()));
            app.until(|s| s.action.is_none());
            assert_eq!(
                app.state.action_error,
                if success { None } else { Some(failure) }
            );
            match &action {
                ProfileAction::Rename { .. } => {
                    assert_eq!(
                        app.state.contacts["alice"].name.as_deref(),
                        Some(if success { "Aoi 🌿" } else { "葵" })
                    );
                    if success {
                        assert_eq!(
                            app.state.contacts["alice"].profile.original_name.as_deref(),
                            Some("葵")
                        );
                        // Finish the action's automatic refresh before requesting another.
                        app.until_delta(|d| matches!(d, Delta::Contact(_)), Duration::from_secs(5));
                        let before = fake
                            .calls()
                            .iter()
                            .filter(|c| **c == FakeRpc::Contact("alice".into()))
                            .count();
                        app.send(Intent::Contact("alice".into()));
                        // Consume both the cached contact and the refetched contact.
                        for _ in 0..2 {
                            app.until_delta(
                                |d| matches!(d, Delta::Contact(_)),
                                Duration::from_secs(5),
                            );
                        }
                        assert_eq!(
                            fake.calls()
                                .iter()
                                .filter(|c| **c == FakeRpc::Contact("alice".into()))
                                .count(),
                            before + 1
                        );
                        assert_eq!(
                            app.state.contacts["alice"]
                                .profile
                                .overridden_name
                                .as_deref(),
                            Some("Aoi 🌿")
                        );
                        assert_eq!(app.state.contacts["alice"].name.as_deref(), Some("Aoi 🌿"));
                        assert_eq!(
                            app.state.contacts["alice"].profile.original_name.as_deref(),
                            Some("葵")
                        );
                    }
                }
                ProfileAction::AddFriend { .. } => assert_eq!(
                    app.state.contacts["alice"].profile.relation,
                    Some(if success { 0 } else { 2 })
                ),
                ProfileAction::Mute { .. } => {
                    assert_eq!(app.state.chats[&chat].chat.muted, success);
                    assert_eq!(app.state.group_info[&chat]["notificationDisabled"], success);
                    assert!(app.state.group_info[&chat]["extra"]["groupExtra"].is_object());
                }
                ProfileAction::Leave { .. } => {
                    assert_eq!(app.state.chats[&chat].chat.left, success);
                    assert!(app.state.chat_order.contains(&chat));
                    if success {
                        app.state.composer = "still here".into();
                        assert!(app.state.submit().is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn compose_reaction_set_cancel_and_failure_revert() {
    use line::backend::{Failure, FakeRpc};
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
        Failure::Refused(Refusal::Internal),
    );
    app.send(Intent::React { key, kind: 2 });
    app.until_delta(
        |d| matches!(d, Delta::ReactionFailed(k) if *k == key),
        Duration::from_secs(5),
    );
    assert!(app.state.messages[&key].reactions.is_empty());
}

#[test]
fn compose_sticker_send_echo_dedup_and_recovery() {
    use line::store::Content;
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
        reply: None,
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
    fake.fail_rpc(
        FakeRpc::Send(ChatId("alice".into())),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    let other = app.state.sticker_packages[1].stickers[0].clone();
    app.send(Intent::SendSticker {
        chat: ChatId("alice".into()),
        sticker: Box::new(other.clone()),
        reply: None,
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
fn first_message_creates_a_friend_chat() {
    use line::{backend::FakeRpc, store::Friend};
    let fake = FakeBackend::canned();
    fake.set_friends(vec![Friend {
        id: "new-friend".into(),
        name: Some("New friend".into()),
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
        reply: None,
    });
    let key = app.body_key("First hello");
    app.until(|s| {
        s.messages[&key].delivery == Delivery::Sent
            && s.chats
                .get(&ChatId("new-friend".into()))
                .is_some_and(|c| c.chat.title == "New friend")
    });
}

#[test]
fn create_group_success_and_failure() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::CreateChat {
        name: "  Picnic  ".into(),
        targets: vec!["alice".into(), "new-friend".into(), "self".into()],
    });
    let Delta::CreatedChat(Ok(id)) = app.until_delta(
        |d| matches!(d, Delta::CreatedChat(Ok(_))),
        Duration::from_secs(5),
    ) else {
        unreachable!()
    };
    assert_eq!(app.state.chats[&id].chat.title, "Picnic");
    assert_eq!(app.state.chats[&id].chat.member_count, Some(3));
    app.select(&id.0);
    fake.fail_rpc(FakeRpc::CreateChat, Failure::Refused(Refusal::Internal));
    app.send(Intent::CreateChat {
        name: "Failure".into(),
        targets: vec!["alice".into(), "new-friend".into()],
    });
    app.until_delta(
        |d| matches!(d, Delta::CreatedChat(Err(_))),
        Duration::from_secs(5),
    );
    assert!(!app.state.chats.values().any(|c| c.chat.title == "Failure"));
}

#[test]
fn signed_out_account_intents_never_reach_the_backend() {
    let fake = FakeBackend::signed_out();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.until(|s| {
        s.connection
            == Connection::SignedOut {
                reason: SignOutReason::NoSession,
            }
    });
    for intent in [
        Intent::Retry(Key(999)),
        Intent::Send {
            chat: ChatId("unknown-chat-id".into()),
            text: "private text".into(),
            reply: None,
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
fn backend_open_failure_completes_sign_in_and_retries_after_repair() {
    use line::client::error::SignInFailure;
    let fake = FakeBackend::signed_out();
    let reason = SignInFailure::CryptoModule("/missing/crypto.wasm".into());
    fake.fail_open(Some(reason.clone()));
    let opener = fake.clone();
    let mut app = App::from_worker(Worker::start(
        StoreLocation::Memory,
        move || opener.open(),
        line::engine::WorkerOptions::default(),
    ));
    let failure = |app: &mut App| {
        let Delta::PairFailed { reason } = app.until_delta(
            |d| matches!(d, Delta::PairFailed { .. }),
            Duration::from_secs(3),
        ) else {
            unreachable!()
        };
        reason
    };
    assert_eq!(failure(&mut app), reason);
    app.send(Intent::SignIn);
    assert_eq!(failure(&mut app), reason);
    fake.fail_open(None);
    app.send(Intent::SignIn);
    app.until_delta(
        |d| {
            assert!(!matches!(d, Delta::PairFailed { .. }));
            matches!(d, Delta::Pair(line::backend::PairEvent::Qr(_)))
        },
        Duration::from_secs(3),
    );
    app.worker.sender.cancel_sign_in();
    assert_eq!(failure(&mut app), SignInFailure::Cancelled);
}

#[test]
fn cancel_before_pairing_job_runs_closes_immediately_and_fences_late_results() {
    use line::client::error::SignInFailure;
    use std::sync::mpsc;
    let fake = FakeBackend::signed_out();
    let (started, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut first = true;
    let mut app = App::from_worker(Worker::start(
        StoreLocation::Memory,
        move || {
            if std::mem::take(&mut first) {
                return Err(SignInFailure::CryptoModule("/missing/crypto.wasm".into()).into());
            }
            started.send(()).unwrap();
            resume.recv().unwrap();
            fake.open()
        },
        line::engine::WorkerOptions::default(),
    ));
    app.until_delta(
        |d| {
            matches!(
                d,
                Delta::Connection(Connection::SignedOut {
                    reason: SignOutReason::Unavailable(_)
                })
            )
        },
        Duration::from_secs(3),
    );
    app.send(Intent::SignIn);
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    app.worker.sender.cancel_sign_in();
    let Delta::PairFailed { reason } = app.until_delta(
        |d| {
            assert!(!matches!(d, Delta::Pair(line::backend::PairEvent::Qr(_))));
            matches!(d, Delta::PairFailed { .. })
        },
        Duration::from_secs(1),
    ) else {
        unreachable!()
    };
    assert_eq!(reason, SignInFailure::Cancelled);
    release.send(()).unwrap();
    // A second attempt must get its own token and deliver QR after the old
    // completion. No late cancellation may close this new attempt.
    app.send(Intent::SignIn);
    app.until_delta(
        |d| {
            assert!(
                !matches!(d, Delta::PairFailed { .. }),
                "late result escaped its generation"
            );
            matches!(d, Delta::Pair(line::backend::PairEvent::Qr(_)))
        },
        Duration::from_secs(3),
    );
    app.worker.sender.cancel_sign_in();
}

#[test]
fn panicking_open_completes_each_job_and_keeps_network_worker_alive() {
    use line::{client::error::SignInFailure, store::Store};
    let temp = TempDir::new("line-panicking-open");
    Store::open(&temp.0)
        .unwrap()
        .set_account(
            "self",
            line::store::Profile {
                mid: "self".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let opens = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let open_count = opens.clone();
    let mut app = App::from_worker(Worker::start::<FakeBackend>(
        StoreLocation::Directory(temp.0.clone()),
        move || {
            open_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            panic!("synthetic network worker crash");
        },
        line::engine::WorkerOptions::default(),
    ));
    app.until(|state| {
        state.connection
            == Connection::SignedOut {
                reason: SignOutReason::Unavailable(SignInFailure::Internal),
            }
    });
    assert!(
        !app.state
            .refresh_failures
            .contains_key(&line::engine::Refresh::Session)
    );
    assert_eq!(app.state.profile.mid, "self");
    for _ in 0..2 {
        app.send(Intent::SignIn);
        let Delta::PairFailed { reason } = app.until_delta(
            |d| matches!(d, Delta::PairFailed { .. }),
            Duration::from_secs(3),
        ) else {
            unreachable!()
        };
        assert_eq!(reason, SignInFailure::Internal);
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[test]
fn failed_probe_during_sign_in_restores_a_signed_out_account() {
    use line::{client::error::SignInFailure, store::Store};
    use std::sync::mpsc;
    for failure in [
        Failure::Bug("probe failed".into()),
        Failure::Refused(Refusal::Internal),
    ] {
        let temp = TempDir::new("line-probe-sign-in");
        Store::open(&temp.0)
            .unwrap()
            .set_account(
                "self",
                line::store::Profile {
                    mid: "self".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut first = true;
        let fake = FakeBackend::signed_out();
        fake.fail_sign_in(Some(SignInFailure::Internal));
        let mut app = App::from_worker(Worker::start(
            StoreLocation::Directory(temp.0.clone()),
            move || {
                if std::mem::take(&mut first) {
                    entered.send(()).unwrap();
                    resume.recv().unwrap();
                    return Err(failure.clone().into());
                }
                Ok(fake.clone())
            },
            Default::default(),
        ));
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        app.send(Intent::SignIn);
        app.until(|s| s.connection == Connection::SigningIn);
        release.send(()).unwrap();
        app.until_delta(
            |d| matches!(d, Delta::PairFailed { .. }),
            Duration::from_secs(3),
        );
        // Cancel presents the restored account and exposes a lingering Opening state.
        app.send(Intent::SignIn);
        app.until_delta(
            |d| matches!(d, Delta::Pair(line::backend::PairEvent::Qr(_))),
            Duration::from_secs(3),
        );
        app.worker.sender.cancel_sign_in();
        app.until_delta(
            |d| {
                matches!(
                    d,
                    Delta::PairFailed {
                        reason: SignInFailure::Cancelled
                    }
                )
            },
            Duration::from_secs(3),
        );
        assert!(matches!(
            app.state.connection,
            Connection::SignedOut {
                reason: SignOutReason::Unavailable(_)
            }
        ));
    }
}

#[test]
fn attachments_preserve_order_progress_and_retry_identity() {
    use line::{attachments::Attachment, store::Content};
    let temp = TempDir::new("line-attachments-flow");
    let root = temp.0.clone();
    std::fs::create_dir_all(&root).unwrap();
    let photo = root.join("First.png");
    image::RgbaImage::from_pixel(32, 24, image::Rgba([40, 120, 200, 255]))
        .save(&photo)
        .unwrap();
    let file = root.join("Second.txt");
    std::fs::write(&file, b"synthetic attachment").unwrap();
    let items = vec![
        Attachment::prepare(&photo).unwrap(),
        Attachment::prepare(&file).unwrap(),
    ];
    let fake = FakeBackend::canned();
    fake.configure(|s| s.upload_delay = Duration::from_millis(80));
    fake.fail_rpc(
        FakeRpc::Upload("Second.txt".into()),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    assert_eq!(
        fake.page(&ChatId("alice".into())).len(),
        1,
        "selection must not send"
    );
    app.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items,
    });
    app.until(|s| s.messages.values().filter(|m| m.outgoing).count() == 2);
    let mut rows = app
        .state
        .rows()
        .into_iter()
        .filter(|m| m.outgoing)
        .collect::<Vec<_>>();
    rows.sort_by_key(|m| m.key);
    let first = rows[0].key;
    let second = rows[1].key;
    assert!(matches!(rows[0].body, Content::Photo { .. }));
    assert!(
        matches!(&rows[1].body, Content::File { name: Some(name), .. } if name == "Second.txt")
    );
    app.until(|s| s.uploads.values().any(|p| *p > 0. && *p < 1.));
    app.until(|s| {
        s.messages[&first].delivery == Delivery::Sent && s.messages[&second].delivery.is_failed()
    });
    assert_eq!(fake.page(&ChatId("alice".into())).len(), 2);
    let old_seq = app.state.messages[&second].request_seq;
    app.send(Intent::Retry(second));
    app.until(|s| s.messages[&second].delivery == Delivery::Sent);
    assert_ne!(app.state.messages[&second].request_seq, old_seq);
    assert_eq!(app.state.messages[&first].key, first);
    assert_eq!(app.state.messages[&second].key, second);
    let sent = fake.page(&ChatId("alice".into()));
    assert_eq!(sent.len(), 3);
    assert!(matches!(sent[1].body, Content::Photo { .. }));
    assert!(
        matches!(&sent[2].body, Content::File { name: Some(name), .. } if name == "Second.txt")
    );
    assert!(app.state.messages[&first].sort < app.state.messages[&second].sort);
    drop(app);
}

#[test]
fn text_send_completes_while_upload_is_blocked() {
    use line::attachments::Attachment;
    let temp = TempDir::new("line-upload-blocked");
    let root = temp.0.clone();
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("blocked.txt");
    std::fs::write(&path, b"attachment").unwrap();
    let fake = FakeBackend::canned();
    let (entered, release) = fake.block_upload();
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![Attachment::prepare(&path).unwrap()],
    });
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("upload blocked");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "text during upload".into(),
        reply: None,
    });
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.body.as_text() == "text during upload" && m.delivery == Delivery::Sent)
    });
    assert!(
        app.state
            .messages
            .values()
            .any(|m| m.body.attachment().is_some() && m.delivery == Delivery::Pending)
    );
    release.send(()).unwrap();
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.body.attachment().is_some() && m.delivery == Delivery::Sent)
    });
    drop(app);
}

#[test]
fn attachment_send_success_after_sign_in_still_confirms_the_row() {
    use line::{attachments::Attachment, store::Store};
    let temp = TempDir::new("line-send-sign-in");
    let root = temp.0.clone();
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("clipboard.txt");
    std::fs::write(&path, b"attachment").unwrap();
    let fake = FakeBackend::canned();
    let (entered, release) = fake.block_send();
    let mut app = App::new(
        fake.clone(),
        StoreLocation::Account {
            root: root.join("accounts"),
            mid: Some("self".into()),
        },
    );
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    let mut item = Attachment::prepare(&path).unwrap();
    item.clipboard = true;
    app.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![item],
    });
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("send blocked after upload");
    app.until(|s| {
        s.messages
            .values()
            .any(|m| m.outgoing && m.delivery == Delivery::Pending)
    });
    let key = app
        .state
        .messages
        .values()
        .find(|m| m.outgoing)
        .unwrap()
        .key;
    app.send(Intent::SignIn);
    app.until(|s| s.connection == Connection::SigningIn);
    use sha2::{Digest, Sha256};
    let store = Store::open(
        &root
            .join("accounts")
            .join(format!("{:x}", Sha256::digest("self"))),
    )
    .unwrap();
    assert_eq!(store.message(key).unwrap().delivery, Delivery::Pending);
    release.send(()).unwrap();
    // Sign-in runs behind the send on the FIFO network queue. Its failure is a barrier
    // proving the stale send completion has been applied, without relying on UI updates.
    app.until(|s| {
        matches!(
            s.connection,
            Connection::SignedOut {
                reason: SignOutReason::Unavailable(_)
            }
        )
    });
    assert_eq!(store.message(key).unwrap().delivery, Delivery::Sent);
    assert_eq!(fake.page(&ChatId("alice".into())).len(), 2);
    assert!(!path.exists());
    drop(store);
    drop(app);
}

#[test]
fn expired_session_does_not_resume_an_upload_after_a_stream_success() {
    use line::{
        attachments::Attachment,
        backend::{Failure, FakeRpc},
    };
    let temp = TempDir::new("line-upload-session");
    let root = temp.0.clone();
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("blocked.txt");
    std::fs::write(&path, b"attachment").unwrap();
    let fake = FakeBackend::canned();
    let (entered, release) = fake.block_upload();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![Attachment::prepare(&path).unwrap()],
    });
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    app.until(|s| s.messages.values().any(|m| m.outgoing));
    let key = app
        .state
        .messages
        .values()
        .find(|m| m.outgoing)
        .unwrap()
        .key;
    fake.fail_rpc(
        FakeRpc::StickerPackages,
        Failure::SignedOut(SignOutReason::AuthenticationFailed),
    );
    app.send(Intent::StickerPackages);
    app.until(|s| {
        s.connection
            == Connection::SignedOut {
                reason: SignOutReason::AuthenticationFailed,
            }
    });
    // Upload attempts while authorization is unusable are ignored.
    app.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![Attachment::prepare(&path).unwrap()],
    });
    app.send(Intent::Prefs(Prefs {
        text_size: 29,
        ..Default::default()
    }));
    app.until(|s| s.prefs.text_size == 29);
    fake.offline(false);
    release.send(()).unwrap();
    app.until(|s| s.messages[&key].delivery.is_failed());
    assert_eq!(
        app.state.messages.values().filter(|m| m.outgoing).count(),
        1
    );
    assert_eq!(fake.page(&ChatId("alice".into())).len(), 1);
    assert!(!app.state.uploads.contains_key(&key));
    assert_eq!(
        app.state.connection,
        Connection::SignedOut {
            reason: SignOutReason::AuthenticationFailed
        }
    );
    drop(app);
}

#[test]
fn restart_recovers_pending_row_without_sending() {
    use line::{
        backend::FakeRpc,
        store::{Content, LocalId, Store},
    };
    let temp = TempDir::new("line-restart-sends");
    let root = temp.0.clone();
    let queued = {
        let mut store = Store::open(&root).unwrap();
        let (queued, _) = store
            .insert_pending(
                &ChatId("alice".into()),
                LocalId("queued".into()),
                2,
                Content::text("queued before restart"),
                2,
                None,
            )
            .unwrap();
        queued
    };
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Directory(root.clone()));
    app.online();
    app.select("alice");
    app.body_key("queued before restart");
    assert_eq!(
        app.state.messages[&queued].delivery,
        Delivery::Rejected {
            reason: Refusal::AppClosed
        }
    );
    assert!(
        !fake
            .calls()
            .iter()
            .any(|call| matches!(call, FakeRpc::Send(_)))
    );
    app.send(Intent::Retry(queued));
    app.until(|state| state.messages[&queued].delivery == Delivery::Sent);
    drop(app);
}

#[test]
fn reply_draft_survives_chat_switch_and_attaches_to_text_and_sticker() {
    use line::store::{Draft, ReplyReference};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    let key = app.body_key("Hello! 日本語も使えます。");
    let target = ReplyReference {
        message_id: app.state.messages[&key].server_id.as_ref().unwrap().clone(),
        service_code: Some(1),
    };
    app.send(Intent::Draft {
        chat: ChatId("alice".into()),
        draft: Draft {
            reply: Some(target.clone()),
            text: "reply text".into(),
            cursor: 10,
            anchor: 10,
            updated: 99,
        },
    });
    app.select("weekend");
    app.body_key("Shall we meet at the station?");
    app.select("alice");
    app.until(|s| s.composer_reply.is_some());
    assert_eq!(app.state.composer_reply.as_ref(), Some(&target));
    let intent = app.state.submit().unwrap();
    app.send(intent);
    let sent = app.body_key("reply text");
    app.until(|s| s.messages[&sent].delivery == Delivery::Sent);
    assert_eq!(app.state.messages[&sent].reply.as_deref(), Some(&target));
    app.send(Intent::Draft {
        chat: ChatId("alice".into()),
        draft: Draft {
            reply: Some(target.clone()),
            text: String::new(),
            cursor: 0,
            anchor: 0,
            updated: 100,
        },
    });
    let sticker = FakeBackend::demo_sticker_packages()[0].stickers[0].clone();
    app.send(Intent::SendSticker {
        chat: ChatId("alice".into()),
        sticker: Box::new(sticker),
        reply: Some(target.clone()),
    });
    app.until(|s| {
        s.messages.values().any(|m| {
            matches!(m.body, line::store::Content::Sticker { .. }) && m.delivery == Delivery::Sent
        })
    });
    let sent = app
        .state
        .messages
        .values()
        .find(|m| matches!(m.body, line::store::Content::Sticker { .. }))
        .unwrap();
    assert_eq!(sent.reply.as_deref(), Some(&target));
    app.until(|s| s.composer_reply.is_none());
    assert_eq!(
        fake.page(&ChatId("alice".into()))
            .last()
            .unwrap()
            .reply
            .as_deref(),
        Some(&target)
    );
}

#[test]
fn rich_parse_discards_a_malformed_reply_before_publication() {
    let fake = FakeBackend::canned();
    let mut message = FakeBackend::message("alice", 91002, "Body with a malformed reply", false);
    message.reply = line::client::desktop::rich::parse(&serde_json::json!({
        "messageRelationType": 3, "relatedMessageId": "01"
    }))
    .reply
    .map(Box::new);
    fake.set_page(ChatId("alice".into()), vec![message]);
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.select("alice");
    let key = app.body_key("Body with a malformed reply");
    assert!(app.state.messages[&key].reply.is_none());
    assert!(app.state.reply_targets.is_empty());
}

#[test]
fn missing_reply_original_loads_from_the_server_and_opens_in_history() {
    use line::store::{Anchor, PageKind, ReplyReference, ServerId};
    let fake = FakeBackend::canned();
    let original = FakeBackend::message("alice", 91001, "Original outside local history", false);
    let mut reply = FakeBackend::message("alice", 91002, "Reply to an older message", false);
    reply.reply = Some(Box::new(ReplyReference {
        message_id: line::store::ServerId::new("91001").unwrap(),
        service_code: Some(1),
    }));
    fake.set_page(ChatId("alice".into()), vec![reply.clone()]);
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Reply to an older message");
    assert!(
        !app.state
            .reply_targets
            .contains_key(&ServerId::new("91001").unwrap())
    );
    fake.set_page(ChatId("alice".into()), vec![original, reply]);
    app.send(Intent::Page {
        chat: ChatId("alice".into()),
        kind: PageKind::Around {
            id: ServerId::new("91001").unwrap(),
        },
    });
    app.until(|s| {
        s.reply_targets
            .contains_key(&ServerId::new("91001").unwrap())
    });
    let key = app.state.reply_targets[&ServerId::new("91001").unwrap()].key;
    let intent = app.state.subscribe_window(Anchor::Key(key), 80, 80);
    app.send(intent);
    app.until(|s| !s.window_loading && s.messages.contains_key(&key));
    assert_eq!(
        app.state.messages[&key].body.as_text(),
        "Original outside local history"
    );
}

#[test]
fn chat_list_pagination_survives_live_operations() {
    use line::backend::{Backend, ChatPage};
    let fake = FakeBackend::canned();
    let mut source = fake.clone();
    let chats = source.chats(None).unwrap().chats;
    let pages = vec![
        (
            None,
            ChatPage {
                chats: vec![chats[0].clone()],
                previews: vec![],
                more: true,
                cursor: Some("page-2".into()),
            },
        ),
        (
            Some("page-2".into()),
            ChatPage {
                chats: vec![chats[1].clone()],
                previews: vec![],
                more: false,
                cursor: None,
            },
        ),
    ];
    let (mut backend, entered, release) = blocked::Blocked::new(fake.clone(), FakeRpc::Chats, 1);
    let (observed, cursors) = std::sync::mpsc::channel();
    let mut pages = std::collections::VecDeque::from(pages);
    backend.chats = Some(Box::new(move |cursor| {
        let (expected, page) = pages.pop_front().expect("unexpected extra chat page");
        let _ = observed.send((cursor, expected));
        Ok(page)
    }));
    let mut app = App::new(backend, StoreLocation::Memory);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    app.until(|s| s.chats.len() == 1);
    let chat = &chats[0].id;
    fake.inject(LiveOp {
        revision: 90,
        replay: false,
        op: Op::Message(FakeBackend::message(&chat.0, 90, "during pagination", false).into()),
    });
    app.until(|s| {
        s.chats[chat]
            .preview
            .as_ref()
            .is_some_and(|m| m.body.as_text() == "during pagination")
    });
    release.send(()).unwrap();
    app.until_delta(
        |d| matches!(d, Delta::MoreChats(false)),
        Duration::from_secs(5),
    );
    app.until(|s| s.chats.len() == 2);
    for _ in 0..2 {
        let (actual, expected) = cursors.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            actual, expected,
            "live operations must preserve the list cursor"
        );
    }
    assert!(chats.iter().all(|c| app.state.chats.contains_key(&c.id)));
    assert_eq!(
        app.state.chats[chat]
            .preview
            .as_ref()
            .unwrap()
            .body
            .as_text(),
        "during pagination"
    );
    assert_eq!(app.notifications, 1);
}

#[test]
fn receipt_updates_the_existing_row_in_place() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "read me".into(),
        reply: None,
    });
    let key = app.body_key("read me");
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    let row = app.state.messages[&key].clone();
    let count = app.state.messages.len();
    fake.inject(LiveOp {
        revision: fake.revision(),
        replay: false,
        op: Op::Receipt {
            chat: ChatId("alice".into()),
            up_to: row.server_id.clone().unwrap(),
        },
    });
    app.until(|s| s.messages[&key].delivery == Delivery::Read);
    assert_eq!(app.state.messages.len(), count);
    assert_eq!(app.state.messages[&key].server_id, row.server_id);
    assert_eq!(app.state.messages[&key].body, row.body);
}

#[test]
fn restart_restores_unread_history_anchor_and_drafts_together() {
    use line::store::{ChatInput, Draft, ServerId};
    let temp = TempDir::new("line-anchor-draft");
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
        server_unread: 160,
        last_message_id: Some(ServerId::new("1240").unwrap()),
        ..Default::default()
    });
    let draft = Draft {
        text: "unfinished".into(),
        cursor: 7,
        anchor: 3,
        reply: None,
        updated: 1,
    };
    let divider = {
        let mut app = App::new(fake.clone(), StoreLocation::Directory(temp.0.clone()));
        app.online();
        app.select("alice");
        app.until(|s| s.unread_divider.is_some());
        let divider = app.state.unread_divider.unwrap();
        app.send(Intent::Draft {
            chat: ChatId("alice".into()),
            draft: draft.clone(),
        });
        app.fence();
        divider
    };
    // The persisted unread boundary is the supported history anchor on reopen.
    // Arbitrary scroll positions are not persisted by the current product.
    fake.offline(true);
    let mut app = App::new(fake, StoreLocation::Directory(temp.0.clone()));
    app.until(|s| s.composer == "unfinished" && s.unread_divider.is_some());
    assert_eq!(app.state.unread_divider, Some(divider));
    assert_eq!(
        app.state.messages[&divider].server_id,
        Some(ServerId::new("1081").unwrap())
    );
    assert!(!app.state.reaches_end);
    assert_eq!(
        app.state.chats[&ChatId("alice".into())].draft.as_ref(),
        Some(&draft)
    );
}

#[test]
fn raw_live_operations_refresh_contacts_and_groups() {
    use line::store::{Friend, Member};
    use serde_json::json;
    for kind in [4, 49, 41, 121, 122, 128, 127] {
        let fake = FakeBackend::canned();
        let mut app = App::new(fake.clone(), StoreLocation::Memory);
        app.online();
        app.select("weekend");
        app.body_key("Shall we meet at the station?");
        let group = ChatId("weekend".into());
        let renamed = Friend {
            id: "alice".into(),
            name: Some("Renamed elsewhere".into()),
            ..Default::default()
        };
        fake.set_contact(renamed.clone());
        fake.configure(|c| {
            if kind == 41 {
                c.chats.retain(|c| c.id != group);
            } else {
                c.chats.iter_mut().find(|c| c.id == group).unwrap().title = "Updated group".into();
            }
            c.members = Some(vec![Member {
                contact: renamed,
                is_self: false,
            }]);
        });
        fake.inject_raw(
            json!({"type":kind,"param1":if matches!(kind, 4 | 49) { "alice" } else { "weekend" }}),
        );
        match kind {
            4 | 49 => app.until(|s| {
                s.contacts
                    .get("alice")
                    .is_some_and(|c| c.name.as_deref() == Some("Renamed elsewhere"))
            }),
            41 => {
                app.until(|s| !s.chat_order.contains(&group));
                assert!(!app.state.chats.contains_key(&group));
            }
            127 => app.until(|s| s.chats[&group].chat.left),
            _ => app.until(|s| {
                s.chats[&group].chat.title == "Updated group"
                    && s.members.len() == 1
                    && s.members[0].contact.name.as_deref() == Some("Renamed elsewhere")
                    && s.group_info
                        .get(&group)
                        .is_some_and(|v| v["chatName"] == "Updated group")
            }),
        }
    }
}

#[test]
fn unusable_saved_session_allows_start_to_pair() {
    use line::client::error::SignInFailure;
    let fake = FakeBackend::signed_out();
    fake.fail_open(Some(SignInFailure::SavedSession));
    let opener = fake.clone();
    let mut app = App::from_worker(Worker::start(
        StoreLocation::Memory,
        move || opener.open(),
        Default::default(),
    ));
    app.until(|s| {
        s.connection
            == Connection::SignedOut {
                reason: SignOutReason::Unavailable(SignInFailure::SavedSession),
            }
    });
    app.send(Intent::SignIn);
    app.until_delta(
        |d| {
            assert!(
                !matches!(d, Delta::PairFailed { .. }),
                "broken saved session blocked pairing"
            );
            matches!(d, Delta::Pair(line::backend::PairEvent::Qr(_)))
        },
        Duration::from_secs(5),
    );
    assert!(fake.calls().contains(&FakeRpc::SignIn));
    app.worker.sender.cancel_sign_in();
    app.until_delta(
        |d| {
            matches!(
                d,
                Delta::PairFailed {
                    reason: SignInFailure::Cancelled
                }
            )
        },
        Duration::from_secs(5),
    );
}

#[test]
fn start_and_cancel_before_probe_still_uses_valid_saved_session() {
    use std::sync::mpsc;
    let fake = FakeBackend::canned();
    let opener = fake.clone();
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut app = App::from_worker(Worker::start(
        StoreLocation::Memory,
        move || {
            let _ = entered.send(());
            let _ = resume.recv();
            opener.open()
        },
        Default::default(),
    ));
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    app.send(Intent::SignIn);
    app.until(|s| s.connection == Connection::SigningIn);
    app.worker.sender.cancel_sign_in();
    app.until_delta(
        |d| {
            matches!(
                d,
                Delta::PairFailed {
                    reason: line::client::error::SignInFailure::Cancelled
                }
            )
        },
        Duration::from_secs(5),
    );
    release.send(()).unwrap();
    app.online();
    assert!(!fake.calls().contains(&FakeRpc::SignIn));
    app.select("alice");
    app.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "saved session works".into(),
        reply: None,
    });
    let key = app.body_key("saved session works");
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
}

#[test]
fn committed_write_clears_storage_banner_and_sending_still_works() {
    use line::engine::StorageFailure;
    let mut app = App::new(FakeBackend::canned(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    app.fence();
    app.until_delta(|d| matches!(d, Delta::StoreSaved), Duration::from_secs(5));
    app.state.apply(Delta::StorageFailed(StorageFailure::Full));
    assert_eq!(app.state.error, Some(StorageFailure::Full));
    app.fence();
    app.until_delta(|d| matches!(d, Delta::StoreSaved), Duration::from_secs(5));
    assert!(app.state.error.is_none());
    app.state.apply(Delta::StorageFailed(StorageFailure::Full));
    app.state.composer = "send despite storage banner".into();
    let intent = app
        .state
        .submit()
        .expect("storage banner must not block sending");
    app.send(intent);
    let key = app.body_key("send despite storage banner");
    app.until(|s| s.messages[&key].delivery == Delivery::Sent);
    assert!(app.state.error.is_none());
}

#[test]
fn chat_preview_resolves_system_actor_before_opening_conversation() {
    let fake = FakeBackend::canned();
    fake.set_contact(line::store::Friend {
        id: "preview-actor".into(),
        name: Some("葵".into()),
        ..Default::default()
    });
    let mut preview = FakeBackend::message("alice", 99001, "", false);
    // The event actor is distinct from the envelope sender, as in the demo seed.
    preview.body = line::store::Content::System {
        kind: Some("C_PI".into()),
        params: Some("preview-actor".into()),
        text: None,
    };
    fake.set_page(ChatId("alice".into()), vec![preview]);
    let mut app = App::new(fake, StoreLocation::Memory);
    app.online();
    app.until(|s| {
        s.chats
            .get(&ChatId("alice".into()))
            .and_then(|c| c.preview.as_ref())
            .is_some()
            && s.contact("preview-actor").is_some()
    });
    assert!(app.state.selected.is_none());
    assert_eq!(
        app.state.contact("preview-actor").unwrap().name.as_deref(),
        Some("葵")
    );
}

#[test]
fn shutdown_saves_the_final_queued_draft() {
    use line::store::{Draft, Store};
    use std::{sync::mpsc, thread};
    let temp = TempDir::new("line-stop-draft");
    let mut app = App::new(
        FakeBackend::canned(),
        StoreLocation::Directory(temp.0.clone()),
    );
    app.online();
    let draft = Draft {
        text: "final draft".into(),
        cursor: 11,
        anchor: 11,
        updated: 123,
        reply: None,
    };
    app.send(Intent::Draft {
        chat: ChatId("alice".into()),
        draft: draft.clone(),
    });
    let (finished, done) = mpsc::channel();
    let worker = thread::spawn(move || {
        drop(app);
        finished.send(()).unwrap();
    });
    done.recv_timeout(Duration::from_secs(3))
        .expect("engine shutdown stayed blocked");
    worker.join().unwrap();
    let saved = Store::open(&temp.0)
        .unwrap()
        .load_draft(&ChatId("alice".into()))
        .unwrap()
        .unwrap();
    assert_eq!(saved.text, draft.text);
    assert_eq!((saved.cursor, saved.anchor), (draft.cursor, draft.anchor));
}
