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
        app.retry(115);
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
    app.retry(115);
    app.on_engine(|engine| {
        // Advance the health clock; the production timer still decides Offline.
        engine.health.failed_since = engine
            .health
            .failed_since
            .map(|since| since - Duration::from_secs(15));
    });
    app.until(|state| state.connection == Connection::Offline);
    app.worker.sender.0.live(StreamEvent::Connection(Ok(())));
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
fn late_success_after_sign_out_updates_its_own_account_store() {
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!("line-completion-account-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::at(StoreLocation::Account {
        root: root.clone(),
        mid: Some("self".into()),
    });
    let (entered, release) = app.fake.block_send();
    app.worker.sender.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "old account".into(),
    });
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    app.retry(1);
    app.until(|state| matches!(state.connection, Connection::SignedOut { .. }));
    let original_key = app
        .state
        .messages
        .values()
        .find(|row| row.body.as_text() == "old account")
        .unwrap()
        .key;
    let target = root.join(format!("{:x}", Sha256::digest("other")));
    // Give the active store a colliding row key before the real completion arrives.
    let directory = target.clone();
    app.on_engine(move |engine| {
        let mut other = Store::open(&directory).unwrap();
        other
            .set_account(
                "other",
                Profile {
                    mid: "other".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        for index in 1..original_key.0 {
            let seq = index as u32 + 100;
            other
                .insert_pending(
                    &ChatId("alice".into()),
                    LocalId(format!("history-{index}")),
                    seq,
                    Content::text("other history"),
                    index,
                )
                .unwrap();
            other
                .link_sent(seq, ServerId(format!("{}", index + 1000)), index)
                .unwrap();
        }
        let (key, _) = other
            .insert_pending(
                &ChatId("alice".into()),
                LocalId("other-row".into()),
                1,
                Content::text("other account"),
                1,
            )
            .unwrap();
        assert_eq!(key, original_key);
        other
            .mark_failed(
                key,
                Delivery::Rejected {
                    reason: Refusal::AppClosed,
                },
            )
            .unwrap();
        engine.store = other;
        engine.store_mid = Some("other".into());
    });
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "send completion did not return");
        let (done, wait) = mpsc::channel();
        app.on_engine(move |engine| {
            done.send(engine.net_pending == 0).unwrap();
        });
        if wait.recv().unwrap() {
            break;
        }
        thread::yield_now();
    }
    let own = Store::open(&root.join(format!("{:x}", Sha256::digest("self")))).unwrap();
    let rows = own
        .window(&ChatId("alice".into()), Anchor::Latest, 100, 0)
        .unwrap();
    assert!(
        rows.rows
            .iter()
            .any(|row| row.body.as_text() == "old account" && row.delivery == Delivery::Sent)
    );
    let other = Store::open(&target).unwrap();
    let rows = other.all_pending().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body.as_text(), "other account");
    assert_eq!(
        rows[0].delivery,
        Delivery::Rejected {
            reason: Refusal::AppClosed
        }
    );
    drop((own, other, app));
    std::fs::remove_dir_all(root).unwrap();
}
