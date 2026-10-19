use super::*;
use crate::{client::error::classify, controller::AppState};

struct App {
    worker: Worker,
    state: AppState,
    fake: FakeBackend,
}
impl App {
    fn new() -> Self {
        Self::at(StoreLocation::Memory)
    }
    fn at(location: StoreLocation) -> Self {
        let fake = FakeBackend::canned();
        let worker = Worker::start_timed(
            fake.clone(),
            location,
            Timing {
                retry_initial: Duration::from_millis(1),
                retry_max: Duration::from_millis(10),
                poll: Duration::from_millis(1),
                offline_after: Duration::from_millis(100),
            },
        );
        let mut app = Self {
            worker,
            state: AppState::default(),
            fake,
        };
        app.until(|state| state.connection == Connection::Online && state.chats.len() == 2);
        app.worker
            .sender
            .send(app.state.select(ChatId("alice".into())));
        app.until(|state| !state.messages.is_empty());
        app.settle();
        app
    }
    fn until(&mut self, ready: impl Fn(&AppState) -> bool) {
        while !ready(&self.state) {
            self.state.apply(
                self.worker
                    .deltas
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap(),
            );
        }
    }
    fn on_engine(&self, run: impl FnOnce(&mut Engine) + Send + 'static) {
        let (done, wait) = mpsc::channel();
        upload_completed(
            &self.worker.sender.0,
            Box::new(move |engine| {
                run(engine);
                done.send(()).unwrap();
                Ok(())
            }),
        );
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline, "engine did not settle");
            let (done, wait) = mpsc::channel();
            self.on_engine(move |engine| {
                let queued = engine.inbox.lanes.lock().unwrap().refresh.is_empty();
                done.send(engine.net_pending == 0 && queued).unwrap();
            });
            if wait.recv().unwrap() {
                break;
            }
            thread::yield_now();
        }
        for delta in self.worker.deltas.try_iter() {
            self.state.apply(delta);
        }
    }
    fn retry(&self, code: i64) {
        let error = classify(
            &crate::client::RpcError {
                gateway: 0,
                method: "stream".into(),
                http: 200,
                code,
                status: 0,
            }
            .into(),
        );
        self.worker
            .sender
            .0
            .live(StreamEvent::Retry { attempt: 1, error });
    }
}

#[test]
fn three_transient_stream_failures_have_no_visible_effect() {
    let mut app = App::new();
    let messages = app.state.messages.clone();
    for _ in 0..3 {
        app.worker.sender.0.live(StreamEvent::Retry {
            attempt: 1,
            error: Failure::Unanswered,
        });
    }
    app.on_engine(|_| {});
    let deltas: Vec<_> = app.worker.deltas.try_iter().collect();
    assert!(
        deltas.is_empty(),
        "transient stream failures must publish nothing"
    );
    assert_eq!(app.state.connection, Connection::Online);
    assert_eq!(app.state.messages, messages);
    app.settle();
}

#[test]
fn persistent_stream_loss_shows_offline_and_success_clears_it() {
    let mut app = App::new();
    app.worker.sender.0.live(StreamEvent::Retry {
        attempt: 1,
        error: Failure::Unanswered,
    });
    app.until(|state| state.connection == Connection::Offline);
    let response =
        crate::client::error::decode_response("/api/test", 503, b"unavailable").unwrap_err();
    app.worker.sender.0.live(StreamEvent::Retry {
        attempt: 2,
        error: classify(&response),
    });
    app.until(|state| state.connection == Connection::Online);
}

#[test]
fn code_one_ends_the_session_and_cancels_queued_work_without_changing_the_reason() {
    let mut app = App::new();
    let (entered, release) = app.fake.block_send();
    app.worker.sender.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "in flight".into(),
    });
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    app.fake.fail_rpc(
        FakeRpc::StickerPackages,
        Failure::SignedOut(SignOutReason::NotAuthorizedDevice),
    );
    app.worker.sender.send(Intent::StickerPackages);
    app.worker.sender.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "queued".into(),
    });
    app.retry(1);
    app.until(|state| {
        matches!(
            state.connection,
            Connection::SignedOut {
                reason: SignOutReason::AuthenticationFailed
            }
        )
    });
    app.until(|state| {
        state.messages.values().any(|row| {
            row.body.as_text() == "queued"
                && row.delivery
                    == Delivery::Rejected {
                        reason: Refusal::SignedOut,
                    }
        })
    });
    release.send(()).unwrap();
    app.until(|state| {
        state
            .messages
            .values()
            .any(|row| row.body.as_text() == "in flight" && row.delivery == Delivery::Sent)
    });
    app.on_engine(|_| {});
    assert!(!app.fake.calls().contains(&FakeRpc::StickerPackages));
    assert_eq!(
        app.state.connection,
        Connection::SignedOut {
            reason: SignOutReason::AuthenticationFailed
        }
    );
}

#[test]
fn refresh_503_backoff_does_not_hold_a_send() {
    let mut app = App::new();
    app.on_engine(|engine| {
        engine.timing.retry_initial = Duration::from_secs(3600);
        engine.timing.retry_max = Duration::from_secs(3600);
    });
    let chat = ChatId("alice".into());
    let before = app
        .fake
        .calls()
        .iter()
        .filter(|call| **call == FakeRpc::Messages(chat.clone()))
        .count();
    let error =
        crate::client::error::decode_response("/api/test", 503, b"unavailable").unwrap_err();
    app.fake
        .fail_rpc(FakeRpc::Messages(chat.clone()), classify(&error));
    app.worker.sender.send(Intent::Page {
        chat: chat.clone(),
        kind: PageKind::Recent,
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            Instant::now() < deadline,
            "failed refresh did not return to its scheduler"
        );
        let (done, wait) = mpsc::channel();
        let request = Refresh::Recent(chat.clone());
        app.on_engine(move |engine| {
            done.send(engine.retry_after.contains_key(&request))
                .unwrap();
        });
        if wait.recv().unwrap() {
            break;
        }
        thread::yield_now();
    }
    app.worker.sender.send(Intent::Send {
        chat: chat.clone(),
        text: "during backoff".into(),
    });
    app.until(|state| {
        state
            .messages
            .values()
            .any(|row| row.body.as_text() == "during backoff" && row.delivery == Delivery::Sent)
    });
    assert_eq!(
        app.fake
            .calls()
            .iter()
            .filter(|call| **call == FakeRpc::Messages(chat.clone()))
            .count(),
        before + 1
    );
}

#[test]
fn live_frames_decode_in_arrival_order_before_refreshes() {
    let mut app = App::new();
    let chat = ChatId("alice".into());
    let rows = vec![
        FakeBackend::message("alice", 500, "first", false),
        FakeBackend::message("alice", 501, "second", false),
    ];
    app.fake.set_page(chat.clone(), rows);
    app.worker.sender.send(Intent::Page {
        chat,
        kind: PageKind::Recent,
    });
    app.until(|state| {
        state
            .messages
            .values()
            .any(|row| row.body.as_text() == "second")
    });
    app.fake
        .inject_raw(serde_json::json!({"type":64,"param2":"501"}));
    app.fake
        .inject_raw(serde_json::json!({"type":64,"param2":"500"}));
    app.until(|state| {
        ["500", "501"].iter().all(|id| {
            state.messages.values().any(|row| {
                row.server_id.as_ref().is_some_and(|server| server.0 == *id)
                    && row.body.is_withdrawn()
            })
        })
    });
}

#[test]
fn stream_exit_restarts_and_subsequent_operations_arrive() {
    let mut app = App::new();
    let starts = app.fake.resumed_revisions().len();
    app.worker.sender.0.live(StreamEvent::Retry {
        attempt: 1,
        error: Failure::Unanswered,
    });
    app.worker.sender.0.live(StreamEvent::Exited);
    app.settle();
    assert!(app.fake.resumed_revisions().len() > starts);
    app.fake.inject(LiveOp {
        revision: 9001,
        replay: false,
        op: Op::Message(Box::new(FakeBackend::message(
            "alice",
            9001,
            "after reconnect",
            false,
        ))),
    });
    app.until(|state| {
        state
            .messages
            .values()
            .any(|row| row.body.as_text() == "after reconnect")
    });
}
