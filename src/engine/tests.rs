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
        let worker = Worker::start(
            location,
            {
                let mut backend = Some(fake.clone());
                move || Ok(backend.take().expect("backend already opened"))
            },
            WorkerOptions {
                timing: Timing {
                    retry_initial: Duration::from_millis(1),
                    retry_max: Duration::from_millis(10),
                    poll: Duration::from_millis(1),
                    offline_after: Duration::from_millis(100),
                    health_now: Instant::now,
                },
                ..Default::default()
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
    fn expire_session(&self) {
        let error = crate::client::error::classify(
            &crate::client::decode_response("/test", 200, br#"{"code":10051,"data":{"code":1}}"#)
                .unwrap_err(),
        );
        self.worker
            .sender
            .0
            .live(StreamEvent::Retry { attempt: 1, error });
    }
}

#[test]
fn three_transient_stream_failures_have_no_visible_effect() {
    static FROZEN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let mut app = App::new();
    let messages = app.state.messages.clone();
    app.on_engine(|engine| {
        engine.timing.health_now = || *FROZEN.get_or_init(Instant::now);
    });
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
    let response = crate::client::decode_response("/api/test", 503, b"unavailable").unwrap_err();
    app.worker.sender.0.live(StreamEvent::Retry {
        attempt: 2,
        error: classify(&response),
    });
    app.until(|state| state.connection == Connection::Online);
}

#[test]
fn expired_session_cancels_queued_work_without_changing_the_reason() {
    let mut app = App::new();
    let (entered, release) = app.fake.block_send();
    app.worker.sender.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "in flight".into(),
        reply: None,
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
        reply: None,
    });
    app.expire_session();
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
    let error = crate::client::decode_response("/api/test", 503, b"unavailable").unwrap_err();
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
        reply: None,
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
fn live_withdrawals_update_both_loaded_messages() {
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
                row.server_id
                    .as_ref()
                    .is_some_and(|server| server.as_str() == *id)
                    && row.body.is_withdrawn()
            })
        })
    });
}

#[test]
fn cached_thumbnail_does_not_clear_offline() {
    use crate::client::desktop::assets::Pixels;
    let mut app = App::new();
    let asset = FakeBackend::asset(true);
    app.fake.set_pixels(
        &asset,
        Pixels {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        },
    );
    app.on_engine(|engine| {
        engine.health.failure((engine.timing.health_now)());
        engine.connection(Connection::Offline);
    });
    app.worker.sender.send(Intent::Asset(asset));
    loop {
        let delta = app
            .worker
            .deltas
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let received = matches!(delta, Delta::Asset { .. });
        app.state.apply(delta);
        if received {
            break;
        }
    }
    app.settle();
    assert_eq!(app.state.connection, Connection::Offline);
}

#[test]
fn request_sequence_wraps_without_persisting_preferences() {
    let app = App::new();
    app.on_engine(|engine| {
        let prefs = engine.store.prefs().unwrap();
        engine.seq = 0x7fff_fffe;
        assert_eq!(engine.next_seq().unwrap(), 0x7fff_ffff);
        assert_eq!(engine.next_seq().unwrap(), 0);
        assert_eq!(engine.next_seq().unwrap(), 1);
        assert_eq!(engine.store.prefs().unwrap(), prefs);
        assert!(
            serde_json::to_value(prefs)
                .unwrap()
                .get("request_seq")
                .is_none()
        );
    });
}

#[test]
fn reconnect_without_revision_change_does_not_refresh_chats() {
    let mut app = App::new();
    let before = app.fake.calls();
    app.on_engine(|engine| {
        engine
            .live(StreamEvent::Connection(Err(Failure::NotDelivered)))
            .unwrap();
        engine.live(StreamEvent::Connection(Ok(()))).unwrap();
        engine.live(StreamEvent::Connection(Ok(()))).unwrap();
    });
    app.settle();
    let after = app.fake.calls();
    assert_eq!(
        after[before.len()..]
            .iter()
            .filter(|call| matches!(
                call,
                FakeRpc::Messages(_) | FakeRpc::Receipts(_) | FakeRpc::Chats
            ))
            .count(),
        0
    );
}

#[test]
fn refused_chat_refresh_stays_suppressed_across_resync_until_opened() {
    let mut app = App::new();
    let chat = ChatId("alice".into());
    let failure = classify(
        &crate::client::decode_response("/getRecentMessagesV2", 400, b"bad request").unwrap_err(),
    );
    app.fake
        .fail_rpc(FakeRpc::Messages(chat.clone()), failure.clone());
    let (tx, rx) = mpsc::sync_channel(1);
    app.on_engine(move |engine| engine.resync(None, tx).unwrap());
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        Err(failure)
    );
    app.until(|s| {
        s.refresh_failures
            .contains_key(&Refresh::Recent(ChatId("alice".into())))
    });
    app.settle();
    let count = |app: &App| {
        app.fake
            .calls()
            .iter()
            .filter(|c| **c == FakeRpc::Messages(ChatId("alice".into())))
            .count()
    };
    let before = count(&app);
    // Exercise both the selected-chat path and the background-chat path.
    for selected in [true, false] {
        let (tx, rx) = mpsc::sync_channel(1);
        app.on_engine(move |engine| {
            if !selected {
                engine.window = None;
            }
            engine.resync(None, tx).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        app.settle();
        assert_eq!(count(&app), before);
    }
    let intent = app.state.select(chat);
    app.worker.sender.send(intent);
    app.settle();
    assert_eq!(count(&app), before + 1);
}

#[test]
fn live_chat_operation_allows_a_previously_refused_refresh() {
    let mut app = App::new();
    let chat = ChatId("alice".into());
    let failure = classify(
        &crate::client::decode_response("/getRecentMessagesV2", 400, b"bad request").unwrap_err(),
    );
    app.fake.fail_rpc(FakeRpc::Messages(chat.clone()), failure);
    let (tx, rx) = mpsc::sync_channel(1);
    app.on_engine(move |engine| engine.resync(None, tx).unwrap());
    assert!(rx.recv_timeout(Duration::from_secs(3)).unwrap().is_err());
    app.settle();
    let before = app
        .fake
        .calls()
        .iter()
        .filter(|c| **c == FakeRpc::Messages(chat.clone()))
        .count();
    app.on_engine(|engine| {
        engine
            .live(StreamEvent::Operation(LiveOp {
                revision: engine.store.sync_revision().unwrap() + 1,
                replay: false,
                op: Op::Recent(ChatId("alice".into())),
            }))
            .unwrap();
    });
    app.settle();
    assert_eq!(
        app.fake
            .calls()
            .iter()
            .filter(|c| **c == FakeRpc::Messages(chat.clone()))
            .count(),
        before + 1
    );
}

#[test]
fn sticker_failure_is_visible_and_retry_recovers() {
    let mut app = App::new();
    app.fake
        .fail_rpc(FakeRpc::StickerPackages, Failure::Unanswered);
    app.worker.sender.send(Intent::StickerPackages);
    app.until(|s| s.sticker_packages_failed);
    app.worker.sender.send(Intent::StickerPackages);
    app.settle();
    assert!(!app.state.sticker_packages_failed);
    assert_eq!(
        app.fake
            .calls()
            .iter()
            .filter(|c| **c == FakeRpc::StickerPackages)
            .count(),
        2
    );
}

#[test]
fn unchanged_revision_connection_after_exit_restores_head_continuity() {
    let app = App::new();
    app.on_engine(|engine| {
        let chat = ChatId("continuity".into());
        let first = FakeBackend::message("continuity", 10000, "first", false);
        engine
            .store
            .apply_page(&chat, PageKind::Recent, &[first], true)
            .unwrap();
        engine.live(StreamEvent::Exited).unwrap();
        engine.live(StreamEvent::Connection(Ok(()))).unwrap();
        let revision = engine.store.sync_revision().unwrap() + 1;
        engine
            .live(StreamEvent::Operation(LiveOp {
                revision,
                replay: false,
                op: Op::Message(FakeBackend::message("continuity", 10001, "next", false).into()),
            }))
            .unwrap();
        let window = engine
            .store
            .window(&chat, Anchor::Latest, 100, 100)
            .unwrap();
        assert_eq!(window.rows.len(), 2);
        assert!(
            window.gaps.is_empty(),
            "live message must extend the head segment"
        );
    });
}

#[test]
fn suppression_is_only_for_invalid_requests_and_resync_owned_refreshes() {
    let mut app = App::new();
    for (http, body, suppress) in [
        (403, "forbidden", false),
        (200, r#"{"code":10051,"data":{"code":5}}"#, false),
        (400, "bad request", true),
        (200, r#"{"code":10051,"data":{"code":0}}"#, true),
    ] {
        let failure = classify(
            &crate::client::decode_response("/getRecentMessagesV2", http, body.as_bytes())
                .unwrap_err(),
        );
        let ordinary_failure = failure.clone();
        app.on_engine(move |engine| {
            engine.refused_refreshes.clear();
            engine.rpc_failed(Refresh::Recent(ChatId("alice".into())), ordinary_failure);
            assert!(engine.refused_refreshes.is_empty());
        });
        app.fake
            .fail_rpc(FakeRpc::Messages(ChatId("alice".into())), failure.clone());
        let (tx, rx) = mpsc::sync_channel(1);
        app.on_engine(move |engine| engine.resync(None, tx).unwrap());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            Err(failure)
        );
        app.settle();
        let before = app.fake.calls().len();
        app.on_engine(move |engine| {
            let chat = ChatId("alice".into());
            assert_eq!(engine.refused_refreshes.contains(&chat), suppress);
            engine.refresh(Refresh::Recent(chat)).unwrap();
        });
        app.settle();
        assert!(
            app.fake.calls()[before..].contains(&FakeRpc::Messages(ChatId("alice".into()))),
            "ordinary refresh must go out even when suppressed for resync"
        );
    }
    for action in 0..5 {
        app.on_engine(move |engine| {
            let chat = ChatId("alice".into());
            engine.refused_refreshes.insert(chat.clone());
            let intent = match action {
                0 => Intent::Members(chat.clone()),
                1 => Intent::GroupInfo(chat.clone()),
                2 => Intent::Page {
                    chat: chat.clone(),
                    kind: PageKind::Around {
                        id: ServerId::new("1").unwrap(),
                    },
                },
                3 => Intent::Page {
                    chat: chat.clone(),
                    kind: PageKind::Older {
                        before: Cursor {
                            sort_ts: 0,
                            sort_id: "0".into(),
                        },
                    },
                },
                _ => Intent::Window(WindowSub {
                    sub: 99,
                    chat: chat.clone(),
                    anchor: Anchor::Latest,
                    before: 20,
                    after: 20,
                }),
            };
            engine.intent(intent).unwrap();
            assert!(!engine.refused_refreshes.contains(&chat));
        });
        app.settle();
    }
}
