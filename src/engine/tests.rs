use super::*;
use crate::{client::error::classify, controller::AppState};

fn resync_reply(
    rx: &async_channel::Receiver<crate::backend::Result<()>>,
) -> crate::backend::Result<()> {
    futures_lite::future::block_on(futures_lite::future::or(
        async { rx.recv().await.unwrap() },
        async {
            async_io::Timer::after(harness::WAIT).await;
            panic!("resync reply timed out");
        },
    ))
}

type App = harness::App<FakeBackend>;
impl App {
    fn new() -> Self {
        Self::at(StoreLocation::Memory)
    }
    fn at(location: StoreLocation) -> Self {
        let fake = FakeBackend::canned();
        let opener = fake.clone();
        let worker = Worker::start(
            location,
            move || Ok(opener.clone()),
            WorkerOptions {
                timing: harness::test_timing(),
            },
        );
        let mut app = Self::with_fake(worker, fake);
        app.online();
        app.select("alice");
        app.until(|state| !state.messages.is_empty());
        app.settle();
        app
    }
    fn expire_session(&self) {
        let error = crate::client::error::classify(
            &crate::client::decode_response("/test", 200, br#"{"code":10051,"data":{"code":1}}"#)
                .unwrap_err(),
        );
        self.worker.sender.0.live(StreamEvent::Retry { error });
    }
}

#[test]
fn three_transient_stream_failures_have_no_visible_effect() {
    let mut app = App::new();
    let messages = app.state.messages.clone();
    for _ in 0..3 {
        app.worker.sender.0.live(StreamEvent::Retry {
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
        error: Failure::Unanswered,
    });
    app.on_engine(|engine| {
        let now = (engine.timing.health_now)() + engine.timing.offline_after;
        engine.timing.health_now = Arc::new(move || now);
    });
    app.until(|state| state.connection == Connection::Offline);
    let response = crate::client::decode_response("/api/test", 503, b"unavailable").unwrap_err();
    app.worker.sender.0.live(StreamEvent::Retry {
        error: classify(&response),
    });
    app.until(|state| state.connection == Connection::Online);
}

#[test]
fn health_deadline_is_recomputed_after_a_message() {
    let mut app = App::new();
    let clock = Arc::new(Mutex::new(Instant::now()));
    let engine_clock = clock.clone();
    app.on_engine(move |engine| {
        engine.timing.health_now = Arc::new(move || *engine_clock.lock().unwrap());
        engine.timing.offline_after = Duration::from_secs(3600);
    });
    app.worker.sender.0.live(StreamEvent::Retry {
        error: Failure::Unanswered,
    });
    app.on_engine(|_| {});
    *clock.lock().unwrap() += Duration::from_secs(3600);
    app.on_engine(|_| {});
    app.until(|state| state.connection == Connection::Offline);
    app.worker.sender.0.live(StreamEvent::Connection(Ok(())));
    app.until(|state| state.connection == Connection::Online);
}

#[test]
fn retry_deadline_set_by_completion_wakes_idle_engine() {
    let mut app = App::new();
    app.fake.set_page(
        ChatId("alice".into()),
        vec![FakeBackend::message("alice", 9000, "after retry", false)],
    );
    app.on_engine(|engine| {
        engine.timing.retry_initial = Duration::from_millis(20);
        engine.retry_later(Refresh::Recent(ChatId("alice".into())));
    });
    app.until(|state| {
        state
            .messages
            .values()
            .any(|row| row.body.as_text() == "after retry")
    });
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
    entered.recv_timeout(harness::WAIT).unwrap();
    app.fake.fail_rpc(
        FakeRpc::StickerPackages,
        Failure::SignedOut(SignOutReason::NotAuthorizedDevice),
    );
    app.worker.sender.send(Intent::StickerPackages);
    let action = ProfileAction::Mute {
        chat: ChatId("alice".into()),
        disabled: true,
    };
    app.state.action = Some(action.clone());
    app.worker
        .sender
        .send(Intent::ProfileAction(action.clone()));
    app.worker.sender.send(Intent::Send {
        chat: ChatId("alice".into()),
        text: "queued".into(),
        reply: None,
    });
    app.on_engine(|_| {});
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
    app.until(|state| state.action.is_none());
    assert_eq!(app.state.action_error, Some(action.clone()));
    assert!(!app.fake.calls().contains(&FakeRpc::StickerPackages));
    assert!(!app.fake.calls().contains(&FakeRpc::Action(action)));
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
    app.wait_engine(move |engine| {
        engine
            .retry_after
            .contains_key(&Refresh::Recent(chat.clone()))
    });
    let chat = ChatId("alice".into());
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
    app.fake.set_image(
        &asset,
        Pixels {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        },
    );
    app.on_engine(|engine| {
        engine.connection(Connection::Offline);
    });
    app.worker.sender.send(Intent::Asset(asset));
    loop {
        let delta = app.worker.deltas.recv_timeout(harness::WAIT).unwrap();
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
    for (status, body) in [
        (400, &b"bad request"[..]),
        (200, &br#"{"code":10051,"data":{"code":0}}"#[..]),
    ] {
        let mut app = App::new();
        let chat = ChatId("alice".into());
        let failure = classify(
            &crate::client::decode_response("/getRecentMessagesV2", status, body).unwrap_err(),
        );
        app.fake
            .fail_rpc(FakeRpc::Messages(chat.clone()), failure.clone());
        let (tx, rx) = async_channel::bounded(1);
        app.on_engine(move |engine| engine.resync(None, tx).unwrap());
        assert_eq!(resync_reply(&rx), Err(failure));
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
            let (tx, rx) = async_channel::bounded(1);
            app.on_engine(move |engine| {
                if !selected {
                    engine.window = None;
                }
                engine.resync(None, tx).unwrap();
            });
            resync_reply(&rx).unwrap();
            app.settle();
            assert_eq!(count(&app), before);
        }
        let intent = app.state.select(chat);
        app.worker.sender.send(intent);
        app.settle();
        assert_eq!(count(&app), before + 1);
    }
}

#[test]
fn live_chat_operation_allows_a_previously_refused_refresh() {
    for (status, body) in [
        (400, &b"bad request"[..]),
        (200, &br#"{"code":10051,"data":{"code":0}}"#[..]),
    ] {
        let mut app = App::new();
        let chat = ChatId("alice".into());
        let failure = classify(
            &crate::client::decode_response("/getRecentMessagesV2", status, body).unwrap_err(),
        );
        app.fake.fail_rpc(FakeRpc::Messages(chat.clone()), failure);
        let (tx, rx) = async_channel::bounded(1);
        app.on_engine(move |engine| engine.resync(None, tx).unwrap());
        assert!(resync_reply(&rx).is_err());
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
        let (tx, rx) = async_channel::bounded(1);
        app.on_engine(move |engine| engine.resync(None, tx).unwrap());
        resync_reply(&rx).unwrap();
        app.settle();
        assert_eq!(
            app.fake
                .calls()
                .iter()
                .filter(|c| **c == FakeRpc::Messages(chat.clone()))
                .count(),
            before + 2
        );
    }
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
fn empty_live_op_target_still_checkpoints_and_unknown_types_survive() {
    use crate::client::operation::{OperationType, RawOperation};
    let mut app = App::new();
    let (done, wait) = mpsc::channel();
    app.on_engine(move |engine| {
        let mut revision = engine.store.sync_revision().unwrap();
        for (wire, kind) in [
            (121, OperationType::UpdateChat),
            (-4000, OperationType::Unknown),
        ] {
            revision += 1;
            let raw: RawOperation = serde_json::json!({
                "type": wire, "param1": "", "revision": revision
            })
            .into();
            assert_eq!(raw.kind, kind);
            engine.live(StreamEvent::Raw(raw)).unwrap();
            assert!(engine.inbox.lanes.lock().unwrap().refresh.is_empty());
        }
        engine.rpc(
            |_| Ok(()),
            move |engine, _, _| {
                assert_eq!(engine.store.sync_revision()?, revision);
                done.send(()).unwrap();
                Ok(())
            },
        );
    });
    wait.recv_timeout(harness::WAIT).unwrap();
    app.settle();
}

#[test]
fn startup_probe_preserves_failure_classes_and_can_recover_offline() {
    use crate::client::error::{ServerRefusal, SignInFailure};
    let failures = [
        Failure::NotDelivered,
        Failure::Unanswered,
        classify(&anyhow::Error::new(SignInFailure::Network)),
        classify(&anyhow::Error::new(SignInFailure::Keyring)),
        classify(
            &crate::client::decode_response("/test", 401, br#"{"code":10051,"data":{"code":1}}"#)
                .unwrap_err(),
        ),
        Failure::Refused(Refusal::Server(ServerRefusal::Http(403))),
        Failure::Bug("probe failed".into()),
    ];
    for failure in failures {
        let fake = FakeBackend::canned();
        let opener = fake.clone();
        let failure_for_open = failure.clone();
        let repaired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let open_repaired = repaired.clone();
        let worker = Worker::start(
            StoreLocation::Memory,
            move || {
                if open_repaired.load(Ordering::SeqCst) {
                    Ok(opener.clone())
                } else {
                    Err(anyhow::Error::new(failure_for_open.clone()))
                }
            },
            WorkerOptions {
                timing: Timing {
                    retry_initial: Duration::ZERO,
                    ..Default::default()
                },
            },
        );
        let mut app = App::with_fake(worker, fake);
        match &failure {
            Failure::NotDelivered | Failure::Unanswered => {
                app.until(|s| s.connection == Connection::Offline);
                repaired.store(true, Ordering::SeqCst);
                app.until(|s| s.connection == Connection::Online);
            }
            Failure::SignedOut(reason) => {
                let reason = reason.clone();
                let mut unlock = false;
                while app.state.connection
                    != (Connection::SignedOut {
                        reason: reason.clone(),
                    })
                {
                    let delta = app.worker.deltas.recv_timeout(harness::WAIT).unwrap();
                    unlock |= matches!(
                        delta,
                        Delta::PairFailed {
                            reason: SignInFailure::Keyring
                        }
                    );
                    app.state.apply(delta);
                }
                if reason == SignOutReason::Unavailable(SignInFailure::Keyring) {
                    // The sign-in surface receives the unlock-keyring reason.
                    assert!(unlock);
                }
            }
            Failure::Refused(_) | Failure::Bug(_) => {
                let reason = crate::client::error::sign_in_failure(
                    &anyhow::Error::new(failure.clone()),
                    false,
                );
                app.until(|s| {
                    s.connection
                        == Connection::SignedOut {
                            reason: SignOutReason::Unavailable(reason.clone()),
                        }
                });
                assert!(!app.state.refresh_failures.contains_key(&Refresh::Session));
            }
        }
    }
}

#[test]
fn cached_rpc_cannot_erase_a_network_failure() {
    let mut app = App::new();
    let (done, wait) = mpsc::channel();
    app.on_engine(move |engine| {
        engine.timing.offline_after = Duration::ZERO;
        engine.health.failure((engine.timing.health_now)());
        engine.connection(Connection::Offline);
        engine.rpc(
            |backend| backend.session(),
            move |engine, result, current| {
                assert_eq!(result, Ok(true));
                assert!(current);
                assert_eq!(
                    engine.connection,
                    Connection::Offline,
                    "cache hit cleared offline health"
                );
                // A remote response must clear the same failure.
                engine.rpc(
                    |backend| backend.profile(),
                    move |engine, result, _| {
                        assert!(result.is_ok());
                        assert_eq!(engine.connection, Connection::Online);
                        done.send(()).unwrap();
                        Ok(())
                    },
                );
                Ok(())
            },
        );
    });
    wait.recv_timeout(harness::WAIT).unwrap();
    app.settle();
}

#[test]
fn cleanup_failures_are_reported_without_dropping_attachments() {
    const CHILD: &str = "LINE_TEST_CLEANUP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine::tests::cleanup_failures_are_reported_without_dropping_attachments",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        for operation in ["clipboard-prune", "start-assets", "attachment-preview"] {
            assert_eq!(
                stderr.matches(&format!("operation={operation} ")).count(),
                1,
                "{stderr}"
            );
        }
        return;
    }
    let temp = crate::test_support::TempDir::new("cleanup");
    let root = temp.0.clone();
    std::fs::create_dir_all(root.join("accounts")).unwrap();
    std::fs::write(root.join("clipboard-attachments"), b"not a directory").unwrap();
    let fake = FakeBackend::canned();
    fake.fail_rpc(FakeRpc::StartAssets, Failure::Refused(Refusal::Internal));
    let opener = fake.clone();
    let worker = Worker::start(
        StoreLocation::Account {
            root: root.join("accounts"),
            mid: None,
        },
        move || Ok(opener.clone()),
        Default::default(),
    );
    let mut app = App::with_fake(worker, fake);
    app.until(|s| s.connection == Connection::Online);
    let path = root.join("attachment");
    std::fs::write(&path, b"attachment body").unwrap();
    let mut item = crate::attachments::Attachment::prepare(&path).unwrap();
    item.preview = b"broken preview".to_vec();
    app.worker.sender.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![item],
    });
    app.settle();
    let (done, wait) = mpsc::channel();
    app.on_engine(move |engine| {
        let window = engine
            .store
            .window(&ChatId("alice".into()), Anchor::Latest, 80, 80)
            .unwrap();
        assert!(
            window
                .rows
                .iter()
                .any(|row| matches!(&row.body, Content::File { name, .. } if name.as_deref() == Some("attachment")))
        );
        done.send(()).unwrap();
    });
    wait.recv_timeout(harness::WAIT).unwrap();
    drop(app);
}

#[test]
fn corrupt_local_photo_completes_the_demo_asset_request() {
    use crate::client::desktop::assets::{Asset, Failure as MediaFailure};
    let app = App::new();
    let asset = Asset::local(crate::attachments::Attachment {
        path: PathBuf::from("unused-photo"),
        name: "photo.png".into(),
        size: 1,
        photo: None,
        preview: b"corrupt image".to_vec(),
        clipboard: false,
    });
    let key = asset.key();
    app.worker.sender.send(Intent::Asset(asset));
    loop {
        if let Delta::MediaFailed {
            key: failed,
            failure,
        } = app.worker.deltas.recv_timeout(harness::WAIT).unwrap()
            && failed == key
        {
            assert_eq!(failure, MediaFailure::Decode);
            break;
        }
    }
}

#[test]
fn local_preview_is_ready_before_any_fetcher_request_or_upload() {
    let app = App::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("immediate-photo.png");
    image::RgbImage::from_pixel(8, 6, image::Rgb([20, 30, 40]))
        .save(&path)
        .unwrap();
    let item = crate::attachments::Attachment::prepare(&path).unwrap();
    let key = crate::client::desktop::assets::Asset::local(item.clone()).key();
    app.on_engine(move |engine| {
        // A missing media handle must not delay a local preview in the fetch queue.
        engine.media = None;
        engine
            .intent(Intent::SendAttachments {
                chat: ChatId("alice".into()),
                items: vec![item],
            })
            .unwrap();
        assert!(engine.waiting_assets.is_empty());
    });
    let deltas: Vec<_> = app.worker.deltas.try_iter().collect();
    let path = deltas
        .iter()
        .find_map(|delta| match delta {
            Delta::Asset { key: ready, path } if ready == &key => Some(path),
            _ => None,
        })
        .expect("preview must be delivered synchronously");
    assert_eq!(image::open(path).unwrap().width(), 384);
}

#[test]
fn stale_window_row_does_not_remove_current_reply_target() {
    let mut app = App::new();
    let mut stale = app.state.messages.values().next().unwrap().clone();
    let id = stale.server_id.clone().unwrap();
    let stale_key = stale.key;
    let mut current = stale.clone();
    current.key = Key(stale_key.0 + 1000);
    app.state.messages.insert(current.key, current.clone());
    let mut removed = app.state.messages.clone();
    removed.remove(&stale_key);
    let removed_state = AppState {
        messages: removed,
        ..Default::default()
    };
    assert_eq!(removed_state.reply_target(&id), Some(&current));
    stale.server_id = None;
    app.state.messages.insert(stale_key, stale);
    assert_eq!(app.state.reply_target(&id), Some(&current));
}

#[test]
fn saving_draft_keeps_pending_window_updates() {
    let app = App::new();
    app.on_engine(|engine| {
        let key = engine
            .store
            .insert_pending(
                &ChatId("alice".into()),
                LocalId("discard-before-draft".into()),
                1234,
                Content::text("discard me"),
                i64::MAX / 2,
                None,
            )
            .unwrap();
        engine
            .store
            .mark_failed(
                key,
                Delivery::Rejected {
                    reason: Refusal::AppClosed,
                },
            )
            .unwrap();
        engine.publish().unwrap();
        engine.store.discard(key).unwrap();
        engine
            .intent(Intent::Draft {
                chat: ChatId("alice".into()),
                draft: Draft {
                    text: "draft".into(),
                    cursor: 0,
                    updated: 0,
                    anchor: 0,
                    reply: None,
                },
            })
            .unwrap();
    });
    let deltas: Vec<_> = app.worker.deltas.try_iter().collect();
    let removed_key = deltas
        .iter()
        .find_map(|d| match d {
            Delta::Window { upsert, .. } => upsert
                .iter()
                .find(|row| row.body.as_text() == "discard me")
                .map(|row| row.key),
            _ => None,
        })
        .unwrap();
    assert!(
        deltas
            .iter()
            .any(|d| matches!(d, Delta::Window { remove, .. } if remove.contains(&removed_key)))
    );
    assert!(deltas.iter().any(|d| matches!(d, Delta::Chats { upsert, .. } if upsert.iter().any(|r| r.draft.as_ref().is_some_and(|d| d.text == "draft")))));
    app.on_engine(|engine| {
        engine
            .intent(Intent::Draft {
                chat: ChatId("alice".into()),
                draft: Draft {
                    text: "draft again".into(),
                    cursor: 0,
                    updated: 0,
                    anchor: 0,
                    reply: None,
                },
            })
            .unwrap();
    });
    assert!(
        !app.worker
            .deltas
            .try_iter()
            .any(|d| matches!(d, Delta::Window { .. }))
    );
}

#[test]
fn failed_pairing_preserves_reason_and_transport_health() {
    use crate::client::error::SignInFailure;
    let fake = FakeBackend::signed_out();
    fake.fail_sign_in(Some(SignInFailure::Server));
    let opener = fake.clone();
    let worker = Worker::start(
        StoreLocation::Memory,
        move || Ok(opener.clone()),
        Default::default(),
    );
    let mut app = App::with_fake(worker, fake);
    app.until(|s| matches!(s.connection, Connection::SignedOut { .. }));
    let failed_since = Instant::now();
    app.on_engine(move |engine| engine.health.failure(failed_since));
    app.send(Intent::SignIn);
    app.until_delta(
        |d| {
            matches!(
                d,
                Delta::PairFailed {
                    reason: SignInFailure::Server
                }
            )
        },
        harness::WAIT,
    );
    assert_eq!(
        app.state.connection,
        Connection::SignedOut {
            reason: SignOutReason::NoSession
        }
    );
    app.on_engine(move |engine| assert_eq!(engine.health.failed_since, Some(failed_since)));
}
