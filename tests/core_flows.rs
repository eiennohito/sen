use line::client::error::{Refusal, SignOutReason};
use line::client::http::CancelToken;
use line::{
    backend::{Failure, FakeBackend, FakeRpc, LiveOp, Op},
    controller::AppState,
    engine::{Connection, Delta, Intent, StoreLocation, Timing, Worker},
    store::{ChatId, Delivery, Key, Prefs, Reaction, ReactionKind},
};
use std::time::{Duration, Instant};
fn message_text(row: &line::store::StoredMessage) -> &str {
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
                    offline_after: Duration::from_millis(100),
                    health_now: Instant::now,
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
    fn until_delta(&mut self, condition: impl Fn(&Delta) -> bool) -> Delta {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let delta = self
                .worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("engine progress");
            let done = condition(&delta);
            self.state.apply(delta.clone());
            if done {
                return delta;
            }
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
        reply: None,
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
        kind: line::store::PageKind::Recent,
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
        !s.chats.is_empty() && s.messages.values().any(|m| message_text(m) == "checkpoint")
    });
    assert!(app.state.chats.values().all(|chat| chat.chat.unread_stale));
    fake.offline(false);
    app.until(|s| s.connection == Connection::Online); // Stream reconnect also resumes persisted revision.
    app.until(|_| fake.calls().contains(&FakeRpc::Stream(75)));
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
}

#[test]
fn unrelated_fetch_failure_does_not_block_send() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
    assert!(app.state.submit().is_none());
    fake.fail_rpc(
        FakeRpc::Friends,
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
    app.send(Intent::Friends);
    app.until(|s| {
        s.refresh_failures
            .contains_key(&line::engine::Refresh::Friends)
    });
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Friends))
            .count(),
        1
    );
    assert!(app.state.error.is_none());
    assert_eq!(app.state.connection, Connection::Online);
    app.state.composer = "after unrelated failure".into();
    assert!(app.state.submit().is_some());
    app.state.submitting = None;
    app.select("alice"); // Reselecting the open chat preserves its composer snapshot.
    assert_eq!(app.state.composer, "after unrelated failure");
    assert!(app.state.submit().is_some());
    app.state.submitting = None;
    // Even a storage banner does not disable an unrelated operation. A committed
    // write clears it; no controller reset or manual retry is needed.
    app.state
        .apply(Delta::StorageFailed(line::engine::StorageFailure::Full));
    assert!(app.state.submit().is_some());
    app.state.submitting = None;
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
    assert!(app.state.submit().is_none());
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
fn opened_chat_history_and_reads_run_before_missing_details_sweep() {
    use line::{
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
        store.mark_read(&chat, ServerId::new("1").unwrap()).unwrap();
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
    use line::store::{Draft, Store};
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
    app.until(|s| matches!(s.connection, Connection::Connecting { .. }));
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
        Some(json!({"predefinedReactionType":2})),
        Some(json!({"predefinedReactionType":4})),
        Some(json!({})),
        Some(serde_json::Value::Null),
        None,
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
            app.state.chats[&ChatId("alice".into())].chat.unread_count,
            unread
        );
    }
    fake.inject_raw(json!({"type":140,"param1":"10","param3":"other","param2":json!({"chatMid":"alice","curr":{"predefinedReactionType":7}}).to_string()}));
    app.until(|s| s.messages[&key].reactions[0].kind == ReactionKind::Predefined(7));
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
        app.state.chats[&ChatId("alice".into())].chat.unread_count,
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
    use line::{backend::*, engine::WindowSub, store::*};
    use std::sync::Arc;
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
        fn finish_attachment(
            &mut self,
            chat: &ChatId,
            uploaded: serde_json::Value,
            seq: u32,
        ) -> Result<MessageInput> {
            self.0.finish_attachment(chat, uploaded, seq)
        }
        fn contact(&mut self, mid: &str) -> Result<Option<line::store::Friend>> {
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
            action: &line::backend::ProfileAction,
            full: serde_json::Value,
            seq: u32,
        ) -> Result<()> {
            self.0.profile_action(action, full, seq)
        }

        fn sign_in(&mut self, c: Arc<CancelToken>, d: PairDeliver) -> Result<()> {
            self.0.sign_in(c, d)
        }
        fn friends(&mut self) -> Result<Vec<line::store::Friend>> {
            self.wait();
            self.0.friends()
        }
        fn members(&mut self, c: &ChatId) -> Result<Vec<line::store::Member>> {
            self.wait();
            self.0.members(c)
        }
        fn start_assets(&mut self, s: Arc<CancelToken>, d: Deliver) {
            self.0.start_assets(s, d)
        }
        fn authorize(
            &mut self,
            a: &line::client::desktop::assets::Asset,
        ) -> Result<Option<String>> {
            self.0.authorize(a)
        }
        fn session(&self) -> Result<bool> {
            self.0.session()
        }
        fn cached_media(
            &mut self,
            mid: &str,
            asset: &line::client::desktop::assets::Asset,
            purpose: line::client::desktop::assets::Purpose,
        ) -> Result<Option<line::client::desktop::assets::Payload>> {
            self.0.cached_media(mid, asset, purpose)
        }
        fn prepare_attachment(
            &mut self,
            chat: ChatId,
            item: line::attachments::Attachment,
            seq: u32,
        ) -> Result<line::backend::UploadJob> {
            self.0.prepare_attachment(chat, item, seq)
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
        fn send(
            &mut self,
            c: &ChatId,
            t: &str,
            s: u32,
            reply: Option<&line::store::ReplyReference>,
        ) -> Result<MessageInput> {
            self.wait();
            self.0.send(c, t, s, reply)
        }
        fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
            self.0.react(id, kind, seq)
        }
        fn sticker_packages(&mut self) -> Result<Vec<line::store::StickerPackage>> {
            self.0.sticker_packages()
        }
        fn send_sticker(
            &mut self,
            chat: &ChatId,
            sticker: &line::store::Content,
            seq: u32,
            reply: Option<&line::store::ReplyReference>,
        ) -> Result<MessageInput> {
            self.0.send_sticker(chat, sticker, seq, reply)
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
            s: Arc<CancelToken>,
            d: Deliver,
        ) -> Result<Box<dyn FnOnce() + Send>> {
            self.0.stream(r, s, d)
        }
        fn decode(&mut self, r: serde_json::Value) -> Result<LiveOp> {
            self.0.decode(r)
        }
        fn refresh_token(&mut self) -> Result<()> {
            self.0.refresh_token()
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
    fake.configure(|s| s.chats = chats.clone());
    let (initialize, initialized) = std::sync::mpsc::channel();
    let worker = Worker::start_with(
        StoreLocation::Account {
            root: root.clone(),
            mid: None,
        },
        move || {
            initialized.recv()?;
            Ok(Blocked(fake.clone(), released.clone(), entered.clone()))
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
    state.members.push(line::store::Member {
        resolved: true,
        contact: line::store::Friend {
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
    worker.sender.send(Intent::ChatList { sub: 10, limit: 10 });
    receive(&|d| matches!(d, Delta::Chats { sub: 10, .. }));
    worker.sender.send(Intent::Draft {
        chat: chats[149].id.clone(),
        draft: Draft {
            reply: None,
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
    use line::{backend::*, store::*};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
            None,
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
        fn finish_attachment(
            &mut self,
            chat: &ChatId,
            uploaded: serde_json::Value,
            seq: u32,
        ) -> Result<MessageInput> {
            self.fake.finish_attachment(chat, uploaded, seq)
        }
        fn contact(&mut self, mid: &str) -> Result<Option<line::store::Friend>> {
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
            action: &line::backend::ProfileAction,
            full: serde_json::Value,
            seq: u32,
        ) -> Result<()> {
            self.fake.profile_action(action, full, seq)
        }

        fn sign_in(&mut self, c: Arc<CancelToken>, d: PairDeliver) -> Result<()> {
            self.fake.sign_in(c, d)
        }
        fn friends(&mut self) -> Result<Vec<line::store::Friend>> {
            self.fake.friends()
        }
        fn members(&mut self, c: &ChatId) -> Result<Vec<line::store::Member>> {
            self.fake.members(c)
        }
        fn start_assets(&mut self, s: Arc<CancelToken>, d: Deliver) {
            self.fake.start_assets(s, d)
        }
        fn authorize(
            &mut self,
            a: &line::client::desktop::assets::Asset,
        ) -> Result<Option<String>> {
            self.fake.authorize(a)
        }
        fn session(&self) -> Result<bool> {
            self.fake.session()
        }
        fn cached_media(
            &mut self,
            mid: &str,
            asset: &line::client::desktop::assets::Asset,
            purpose: line::client::desktop::assets::Purpose,
        ) -> Result<Option<line::client::desktop::assets::Payload>> {
            self.fake.cached_media(mid, asset, purpose)
        }
        fn prepare_attachment(
            &mut self,
            chat: ChatId,
            item: line::attachments::Attachment,
            seq: u32,
        ) -> Result<line::backend::UploadJob> {
            self.fake.prepare_attachment(chat, item, seq)
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
        fn send(
            &mut self,
            c: &ChatId,
            t: &str,
            s: u32,
            reply: Option<&line::store::ReplyReference>,
        ) -> Result<MessageInput> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.calls.load(std::sync::atomic::Ordering::Relaxed) > 1 {
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.fake.send(c, t, s, reply)
        }
        fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
            self.fake.react(id, kind, seq)
        }
        fn sticker_packages(&mut self) -> Result<Vec<line::store::StickerPackage>> {
            self.fake.sticker_packages()
        }
        fn send_sticker(
            &mut self,
            chat: &ChatId,
            sticker: &line::store::Content,
            seq: u32,
            reply: Option<&line::store::ReplyReference>,
        ) -> Result<MessageInput> {
            self.fake.send_sticker(chat, sticker, seq, reply)
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
            s: Arc<CancelToken>,
            d: Deliver,
        ) -> Result<Box<dyn FnOnce() + Send>> {
            self.fake.stream(r, s, d)
        }
        fn decode(&mut self, r: serde_json::Value) -> Result<LiveOp> {
            self.fake.decode(r)
        }
        fn refresh_token(&mut self) -> Result<()> {
            self.fake.refresh_token()
        }
    }
    let fake = FakeBackend::canned();
    fake.fail_rpc(
        FakeRpc::Send(ChatId("alice".into())),
        Failure::Refused(line::client::error::Refusal::Server(
            line::client::error::ServerRefusal::Gateway(13),
        )),
    );
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
        reply: None,
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
    let friends_calls = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, FakeRpc::Friends))
        .count();
    app.send(Intent::Friends);
    app.until(|_| {
        fake.calls()
            .iter()
            .filter(|c| matches!(c, FakeRpc::Friends))
            .count()
            > friends_calls
    });
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

fn profile_action(app: &mut App, action: line::backend::ProfileAction) {
    app.state.action = Some(action.clone());
    app.send(Intent::ProfileAction(action));
    app.until(|s| s.action.is_none());
}
#[test]
fn rename_contact_failure_then_success_updates_the_store_page() {
    use line::backend::{Failure, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::Friends);
    app.until(|s| !s.friends.is_empty());
    let action = ProfileAction::Rename {
        mid: "alice".into(),
        name: "Aoi 🌿".into(),
    };
    fake.fail_rpc(
        FakeRpc::Action(action.clone()),
        Failure::Bug("decode".into()),
    );
    profile_action(&mut app, action.clone());
    assert_eq!(app.state.contacts["alice"].name, "葵");
    assert_eq!(
        app.state.action_error,
        Some(line::engine::ProfileFailure::Rename)
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
    use line::backend::{Failure, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut members = fake.clone();
    use line::backend::Backend;
    let mut rows = members.members(&ChatId("weekend".into())).unwrap();
    rows[0].contact.profile = line::store::ContactProfile {
        relation: Some(2),
        ..Default::default()
    };
    let mid = rows[0].contact.id.clone();
    fake.configure(|s| s.members = Some(rows));
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.send(Intent::Contact(mid.clone()));
    app.until(|s| s.contacts.contains_key(&mid));
    // Synthetic contact source prefers the friend record, so provide the observed non-friend snapshot.
    let mut row = app.state.contacts[&mid].clone();
    row.profile = line::store::ContactProfile {
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
    fake.fail_rpc(
        FakeRpc::Action(action.clone()),
        Failure::Bug("decode".into()),
    );
    profile_action(&mut app, action.clone());
    assert_eq!(app.state.contacts[&mid].profile.relation, Some(2));
    assert_eq!(
        app.state.action_error,
        Some(line::engine::ProfileFailure::AddFriend)
    );
    profile_action(&mut app, action);
    assert_eq!(app.state.contacts[&mid].profile.relation, Some(0));
    assert!(app.state.action_error.is_none());
}
#[test]
fn mute_failure_then_success_preserves_full_chat_and_updates_notifications() {
    use line::backend::{Failure, FakeRpc, ProfileAction};
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
    fake.fail_rpc(
        FakeRpc::Action(action.clone()),
        Failure::Bug("decode".into()),
    );
    profile_action(&mut app, action.clone());
    assert!(!app.state.chats[&chat].chat.muted);
    assert_eq!(
        app.state.action_error,
        Some(line::engine::ProfileFailure::Mute)
    );
    profile_action(&mut app, action);
    assert!(app.state.chats[&chat].chat.muted);
    assert_eq!(app.state.group_info[&chat]["notificationDisabled"], true);
    assert!(app.state.group_info[&chat]["extra"]["groupExtra"].is_object());
    assert!(app.state.action_error.is_none());
}
#[test]
fn leave_failure_then_success_retains_a_read_only_chat_in_the_list() {
    use line::backend::{Failure, FakeRpc, ProfileAction};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("weekend");
    app.until(|s| !s.window_loading);
    let chat = ChatId("weekend".into());
    let action = ProfileAction::Leave { chat: chat.clone() };
    fake.fail_rpc(
        FakeRpc::Action(action.clone()),
        Failure::Bug("decode".into()),
    );
    profile_action(&mut app, action.clone());
    assert!(!app.state.chats[&chat].chat.left);
    assert_eq!(
        app.state.action_error,
        Some(line::engine::ProfileFailure::Leave)
    );
    profile_action(&mut app, action);
    assert!(app.state.chats[&chat].chat.left);
    assert!(app.state.chat_order.contains(&chat));
    app.state.composer = "still here".into();
    assert!(app.state.submit().is_none());
    assert!(app.state.action_error.is_none());
}
#[test]
fn detail_stack_pop_and_close_use_the_recorded_opener() {
    use line::controller::DetailPage;
    let mut state = AppState::default();
    let chat = ChatId("weekend".into());
    state.push_detail(DetailPage::GroupInfo(chat.clone()), "header".into());
    state.push_detail(DetailPage::Members(chat), "members row".into());
    state.push_detail(DetailPage::Profile("alice".into()), "alice row".into());
    assert_eq!(state.pop_detail().as_deref(), Some("alice row"));
    assert!(matches!(state.detail_page(), Some(DetailPage::Members(_))));
    assert_eq!(state.close_detail().as_deref(), Some("header"));
    assert!(state.detail.is_empty());
}

#[test]
fn traced_contact_and_chat_operations_refresh_the_affected_identity() {
    use line::backend::{Backend, FakeRpc};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    for kind in [4, 49] {
        let mut source = fake.clone();
        let mut rows = source.friends().unwrap();
        rows[0].name = format!("Changed {kind}");
        rows[0].profile = line::store::ContactProfile {
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
        fake.add_chat(line::store::ChatInput {
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
        Failure::Refused(Refusal::NotAccepted),
    );
    app.send(Intent::React { key, kind: 2 });
    app.until_delta(|d| matches!(d, Delta::ReactionFailed(k) if *k == key));
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
fn compose_new_chat_local_friend_then_group_success_and_failure() {
    use line::{
        backend::{Failure, FakeRpc},
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
        reply: None,
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
    let Delta::CreatedChat(Ok(id)) = app.until_delta(|d| matches!(d, Delta::CreatedChat(Ok(_))))
    else {
        unreachable!()
    };
    assert_eq!(app.state.chats[&id].chat.title, "Picnic");
    assert_eq!(app.state.chats[&id].chat.member_count, Some(3));
    app.select(&id.0);
    fake.fail_rpc(FakeRpc::CreateChat, Failure::Refused(Refusal::NotAccepted));
    app.send(Intent::CreateChat {
        name: "Failure".into(),
        targets: vec!["alice".into(), "new-friend".into()],
    });
    app.until_delta(|d| matches!(d, Delta::CreatedChat(Err(_))));
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
        Intent::Friends,
        Intent::Contact("unknown-account-id".into()),
        Intent::GroupInfo(ChatId("unknown-chat-id".into())),
        Intent::Members(ChatId("unknown-chat-id".into())),
        Intent::StickerPackages,
        Intent::Retry(Key(999)),
        Intent::Send {
            chat: ChatId("unknown-chat-id".into()),
            text: "private text".into(),
            reply: None,
        },
        Intent::SendAttachments {
            chat: ChatId("unknown-chat-id".into()),
            items: vec![line::attachments::Attachment {
                path: "/unread-signed-out-attachment".into(),
                name: "private.txt".into(),
                size: 1,
                photo: None,
                preview: vec![],
                clipboard: false,
            }],
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
    use line::client::error::SignInFailure::*;
    let fake = FakeBackend::signed_out();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.until(|s| {
        s.connection
            == Connection::SignedOut {
                reason: SignOutReason::NoSession,
            }
    });
    for reason in [
        CryptoModule("/missing/crypto.wasm".into()),
        ExtensionMissing,
        CryptoChanged {
            version: "9.8.7".into(),
            missing: "E2EEChannel.decryptV2".into(),
        },
        StateDirectory("/readonly/state".into()),
        Keyring,
        Network,
        Server,
        Phone,
        Cancelled,
    ] {
        fake.fail_sign_in(Some(reason.clone()));
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
    assert_eq!(fake.calls(), vec![FakeRpc::SignIn; 9]);
}

#[test]
fn invariant_errors_are_never_storage_failures() {
    use line::{engine::StorageFailure, store::StoreError};
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

#[test]
fn backend_open_failure_completes_sign_in_and_retries_after_repair() {
    use line::client::error::SignInFailure;
    let fake = FakeBackend::signed_out();
    let reason = SignInFailure::CryptoModule("/missing/crypto.wasm".into());
    fake.fail_open(Some(reason.clone()));
    let opener = fake.clone();
    let worker = Worker::start_with(StoreLocation::Memory, move || opener.open());
    let failure = || loop {
        let delta = worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!matches!(delta, Delta::Fatal));
        if let Delta::PairFailed { reason } = delta {
            break reason;
        }
    };
    assert_eq!(failure(), reason);
    worker.sender.send(Intent::SignIn);
    assert_eq!(failure(), reason);
    fake.fail_open(None);
    worker.sender.send(Intent::SignIn);
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::Pair(line::backend::PairEvent::Qr(_)) => break,
            Delta::Fatal | Delta::PairFailed { .. } => panic!("repaired open did not recover"),
            _ => {}
        }
    }
    worker.sender.cancel_sign_in();
    assert_eq!(failure(), SignInFailure::Cancelled);
}

#[test]
fn cancel_before_pairing_job_runs_closes_immediately_and_fences_late_results() {
    use line::client::error::SignInFailure;
    use std::sync::mpsc;
    let fake = FakeBackend::signed_out();
    let (started, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut first = true;
    let worker = Worker::start_with(StoreLocation::Memory, move || {
        if std::mem::take(&mut first) {
            return Err(SignInFailure::CryptoModule("/missing/crypto.wasm".into()).into());
        }
        started.send(()).unwrap();
        resume.recv().unwrap();
        fake.open()
    });
    loop {
        if matches!(
            worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap(),
            Delta::Connection(Connection::SignedOut {
                reason: SignOutReason::Unavailable(_)
            })
        ) {
            break;
        }
    }
    worker.sender.send(Intent::SignIn);
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.sender.cancel_sign_in();
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(1)).unwrap() {
            Delta::PairFailed { reason } => {
                assert_eq!(reason, SignInFailure::Cancelled);
                break;
            }
            Delta::Pair(line::backend::PairEvent::Stage(
                line::client::pairing::Stage::WaitingKeyring,
            )) => {}
            Delta::Pair(_) | Delta::Fatal => panic!("pairing must not start"),
            _ => {}
        }
    }
    release.send(()).unwrap();
    // A second attempt must get its own token and deliver QR after the old
    // completion. No late cancellation may close this new attempt.
    worker.sender.send(Intent::SignIn);
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::Pair(line::backend::PairEvent::Qr(_)) => break,
            Delta::PairFailed { .. } | Delta::Fatal => panic!("late result escaped its generation"),
            _ => {}
        }
    }
    worker.sender.cancel_sign_in();
}

#[test]
fn cancel_during_initial_open_does_not_wait_for_credentials() {
    use line::client::error::SignInFailure;
    use std::sync::mpsc;
    let (started, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let worker = Worker::start_with(StoreLocation::Memory, move || {
        started.send(()).unwrap();
        resume.recv().unwrap();
        Ok(FakeBackend::signed_out())
    });
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.sender.send(Intent::Prefs(Prefs::default()));
    worker.sender.send(Intent::SignIn);
    worker.sender.cancel_sign_in();
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(1)).unwrap() {
            Delta::PairFailed { reason } => {
                assert_eq!(reason, SignInFailure::Cancelled);
                break;
            }
            Delta::Pair(line::backend::PairEvent::Stage(
                line::client::pairing::Stage::WaitingKeyring,
            )) => {}
            Delta::Pair(_) | Delta::Fatal => panic!("pairing must not start"),
            _ => {}
        }
    }
    release.send(()).unwrap();
    // The startup fact belongs to the prior opening state. A fresh attempt
    // must still reach QR without the cancelled attempt closing its screen.
    worker.sender.send(Intent::SignIn);
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::Pair(line::backend::PairEvent::Qr(_)) => break,
            Delta::PairFailed { .. } | Delta::Fatal => {
                panic!("stale startup escaped its generation")
            }
            _ => {}
        }
    }
    worker.sender.cancel_sign_in();
}

#[test]
fn panicking_open_completes_each_job_and_keeps_network_worker_alive() {
    use line::client::error::SignInFailure;
    let worker = Worker::start_with::<FakeBackend>(StoreLocation::Memory, || {
        panic!("synthetic network worker crash")
    });
    loop {
        if matches!(
            worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap(),
            Delta::PairFailed { .. }
        ) {
            break;
        }
    }
    for _ in 0..2 {
        worker.sender.send(Intent::SignIn);
        loop {
            if let Delta::PairFailed { reason } =
                worker.deltas.recv_timeout(Duration::from_secs(1)).unwrap()
            {
                assert_eq!(reason, SignInFailure::Internal);
                break;
            }
        }
    }
}

#[test]
fn unusable_saved_session_allows_start_to_pair() {
    use line::client::error::SignInFailure;
    let fake = FakeBackend::signed_out();
    let reason = SignInFailure::SavedSession;
    fake.fail_open(Some(reason.clone()));
    let opener = fake.clone();
    let worker = Worker::start_with(StoreLocation::Memory, move || opener.open());
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::PairFailed { reason: actual } => {
                assert_eq!(actual, reason);
                break;
            }
            Delta::Fatal => panic!("saved session prevented backend startup"),
            _ => {}
        }
    }
    worker.sender.send(Intent::SignIn);
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::Pair(line::backend::PairEvent::Qr(_)) => break,
            Delta::PairFailed { .. } | Delta::Fatal => panic!("unusable session blocked pairing"),
            _ => {}
        }
    }
    assert_eq!(fake.calls(), vec![FakeRpc::SignIn; 1]);
    worker.sender.cancel_sign_in();
}

#[test]
fn start_and_cancel_before_probe_still_uses_valid_saved_session() {
    use std::sync::mpsc;
    let fake = FakeBackend::canned();
    let (started, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let worker = Worker::start_with(StoreLocation::Memory, move || {
        started.send(()).unwrap();
        resume.recv().unwrap();
        Ok(fake.clone())
    });
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.sender.send(Intent::SignIn);
    worker.sender.cancel_sign_in();
    loop {
        if let Delta::PairFailed { reason } =
            worker.deltas.recv_timeout(Duration::from_secs(1)).unwrap()
        {
            assert_eq!(reason, line::client::error::SignInFailure::Cancelled);
            break;
        }
    }
    release.send(()).unwrap();
    loop {
        match worker.deltas.recv_timeout(Duration::from_secs(3)).unwrap() {
            Delta::Connection(Connection::Online) => break,
            Delta::Pair(line::backend::PairEvent::Qr(_))
            | Delta::Fatal
            | Delta::PairFailed { .. } => panic!("valid session lost after cancellation"),
            _ => {}
        }
    }
}

#[test]
fn attachments_preserve_order_progress_and_retry_identity() {
    use line::{attachments::Attachment, store::Content};
    let root = std::env::temp_dir().join(format!("line-attachments-flow-{}", std::process::id()));
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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn text_send_completes_while_upload_is_blocked() {
    use line::attachments::Attachment;
    let root = std::env::temp_dir().join(format!("line-upload-blocked-{}", std::process::id()));
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
            .any(|m| message_text(m) == "text during upload" && m.delivery == Delivery::Sent)
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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn attachment_send_success_after_sign_in_still_confirms_the_row() {
    use line::{attachments::Attachment, store::Store};
    let root = std::env::temp_dir().join(format!("line-send-sign-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn expired_session_does_not_resume_an_upload_after_a_stream_success() {
    use line::{
        attachments::Attachment,
        backend::{Failure, FakeRpc},
    };
    let root = std::env::temp_dir().join(format!("line-upload-session-{}", std::process::id()));
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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn not_written_send_retries_in_session_and_is_delivered_once() {
    use line::backend::{Failure, FakeRpc};
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
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
        2
    );
    assert_eq!(
        fake.page(&ChatId("alice".into()))
            .iter()
            .filter(|row| row.body.as_text() == "one delivery")
            .count(),
        1
    );
    assert_eq!(app.state.connection, Connection::Online);
}

#[test]
fn written_unanswered_send_never_resends_and_a_late_echo_confirms_it() {
    let fake = FakeBackend::canned();
    let mut app = App::new(fake.clone(), StoreLocation::Memory);
    app.online();
    app.select("alice");
    app.body_key("Hello! 日本語も使えます。");
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
        1
    );
    assert_eq!(
        app.state
            .messages
            .values()
            .filter(|row| row.body.as_text() == "unanswered")
            .count(),
        1
    );
}

#[test]
fn restart_recovers_pending_row_without_sending() {
    use line::{
        backend::FakeRpc,
        store::{Content, LocalId, Store},
    };
    let root = std::env::temp_dir().join(format!("line-restart-sends-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    std::fs::remove_dir_all(root).unwrap();
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
        message_id: app.state.messages[&key]
            .server_id
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned(),
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
fn missing_reply_original_loads_from_the_server_and_opens_in_history() {
    use line::store::{Anchor, PageKind, ReplyReference, ServerId};
    let fake = FakeBackend::canned();
    let original = FakeBackend::message("alice", 91001, "Original outside local history", false);
    let mut reply = FakeBackend::message("alice", 91002, "Reply to an older message", false);
    reply.reply = Some(Box::new(ReplyReference {
        message_id: "91001".into(),
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
