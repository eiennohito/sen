use super::*;
use crate::{client::error::classify, controller::AppState, test_support::TempDir};
use std::fs;

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

fn draft(text: &str) -> Draft {
    Draft {
        text: text.into(),
        cursor: 0,
        anchor: 0,
        updated: 0,
        reply: None,
    }
}
fn set_draft(e: &mut Engine, text: &str) {
    let chat = ChatId("alice".into());
    let draft = draft(text);
    e.intent(Intent::Draft { chat, draft }).unwrap();
}
fn pending(store: &mut Store, body: Content, seq: u32, at: i64) -> Key {
    let chat = ChatId("alice".into());
    let id = LocalId(seq.to_string());
    store
        .insert_pending(&chat, id, seq, body, at, None)
        .unwrap()
}
fn account_location(locations: &crate::locations::Locations) -> StoreLocation {
    StoreLocation::Account {
        root: locations.data.clone(),
        cache: locations.cache.clone(),
        mid: None,
    }
}
type App = harness::App<FakeBackend>;
impl App {
    fn new() -> Self {
        Self::at(StoreLocation::Memory)
    }
    fn start(location: StoreLocation, fake: FakeBackend) -> Self {
        let opener = fake.clone();
        let timing = harness::test_timing();
        let options = WorkerOptions { timing };
        let worker = Worker::start(location, move || opener.open(), options);
        Self::with_fake(worker, fake)
    }
    fn at(location: StoreLocation) -> Self {
        let mut app = Self::start(location, FakeBackend::canned());
        app.online();
        app.select("alice");
        app.until(|state| !state.messages.is_empty());
        app.settle();
        app
    }
    fn account(locations: &crate::locations::Locations) -> Self {
        Self::at(account_location(locations))
    }
    fn send_text(&self, text: &str) {
        self.send(Intent::Send {
            chat: ChatId("alice".into()),
            text: text.into(),
            reply: None,
        });
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
    app.on_engine(|e| {
        let now = (e.timing.health_now)() + e.timing.offline_after;
        e.timing.health_now = Arc::new(move || now);
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
    app.on_engine(move |e| {
        e.timing.health_now = Arc::new(move || *engine_clock.lock().unwrap());
        e.timing.offline_after = Duration::from_secs(3600);
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
    app.on_engine(|e| {
        e.timing.retry_initial = Duration::from_millis(20);
        e.retry_later(Refresh::Recent(ChatId("alice".into())));
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
    app.send_text("in flight");
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
    app.send_text("queued");
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
    app.on_engine(|e| {
        e.timing.retry_initial = Duration::from_secs(3600);
        e.timing.retry_max = Duration::from_secs(3600);
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
    app.wait_engine(move |e| e.retry_after.contains_key(&Refresh::Recent(chat.clone())));
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
    app.on_engine(|e| {
        e.connection(Connection::Offline);
    });
    app.worker.sender.send(Intent::Asset(asset));
    app.until_delta(|d| matches!(d, Delta::Asset { .. }), harness::WAIT);
    app.settle();
    assert_eq!(app.state.connection, Connection::Offline);
}

#[test]
fn reconnect_without_revision_change_does_not_refresh_chats() {
    let mut app = App::new();
    let before = app.fake.calls();
    app.on_engine(|e| {
        e.live(StreamEvent::Connection(Err(Failure::NotDelivered)))
            .unwrap();
        e.live(StreamEvent::Connection(Ok(()))).unwrap();
        e.live(StreamEvent::Connection(Ok(()))).unwrap();
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
        app.on_engine(move |e| e.resync(None, tx).unwrap());
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
            app.on_engine(move |e| {
                if !selected {
                    e.window = None;
                }
                e.resync(None, tx).unwrap();
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
        app.on_engine(move |e| e.resync(None, tx).unwrap());
        assert!(resync_reply(&rx).is_err());
        app.settle();
        let before = app
            .fake
            .calls()
            .iter()
            .filter(|c| **c == FakeRpc::Messages(chat.clone()))
            .count();
        app.on_engine(|e| {
            e.live(StreamEvent::Operation(LiveOp {
                revision: e.store.sync_revision().unwrap() + 1,
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
        app.on_engine(move |e| e.resync(None, tx).unwrap());
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
    app.on_engine(|e| {
        let chat = ChatId("continuity".into());
        let first = FakeBackend::message("continuity", 10000, "first", false);
        e.store
            .apply_page(&chat, PageKind::Recent, &[first], true)
            .unwrap();
        e.live(StreamEvent::Exited).unwrap();
        e.live(StreamEvent::Connection(Ok(()))).unwrap();
        let revision = e.store.sync_revision().unwrap() + 1;
        e.live(StreamEvent::Operation(LiveOp {
            revision,
            replay: false,
            op: Op::Message(FakeBackend::message("continuity", 10001, "next", false).into()),
        }))
        .unwrap();
        let window = e.store.window(&chat, Anchor::Latest, 100, 100).unwrap();
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
    app.on_engine(move |e| {
        let mut revision = e.store.sync_revision().unwrap();
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
            e.live(StreamEvent::Raw(raw)).unwrap();
            assert!(e.inbox.lanes.lock().unwrap().refresh.is_empty());
        }
        e.rpc(
            |_| Ok(()),
            move |e, _, _| {
                assert_eq!(e.store.sync_revision()?, revision);
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
    app.on_engine(move |e| {
        e.timing.offline_after = Duration::ZERO;
        e.health.failure((e.timing.health_now)());
        e.connection(Connection::Offline);
        e.rpc(
            |backend| backend.session(),
            move |e, result, current| {
                assert_eq!(result, Ok(true));
                assert!(current);
                assert_eq!(
                    e.connection,
                    Connection::Offline,
                    "cache hit cleared offline health"
                );
                // A remote response must clear the same failure.
                e.rpc(
                    |backend| backend.profile(),
                    move |e, result, _| {
                        assert!(result.is_ok());
                        assert_eq!(e.connection, Connection::Online);
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
    const CHILD: &str = "SEN_TEST_CLEANUP_CHILD";
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
    let temp = TempDir::new("cleanup");
    let root = temp.0.clone();
    fs::write(root.join("clipboard-attachments"), b"not a directory").unwrap();
    let fake = FakeBackend::canned();
    fake.fail_rpc(FakeRpc::StartAssets, Failure::Refused(Refusal::Internal));
    let paths = crate::locations::Locations::account(root.clone(), temp.0.join("cache"));
    let mut app = App::start(account_location(&paths), fake);
    app.until(|s| s.connection == Connection::Online);
    let path = root.join("attachment");
    fs::write(&path, b"attachment body").unwrap();
    let mut item = crate::attachments::Attachment::prepare(&path).unwrap();
    item.preview = b"broken preview".to_vec();
    app.worker.sender.send(Intent::SendAttachments {
        chat: ChatId("alice".into()),
        items: vec![item],
    });
    app.settle();
    app.on_engine(move |e| {
        let window = e
            .store
            .window(&ChatId("alice".into()), Anchor::Latest, 80, 80)
            .unwrap();
        assert!(
            window
                .rows
                .iter()
                .any(|row| matches!(&row.body, Content::File { name, .. }
                    if name.as_deref() == Some("attachment")))
        );
    });
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
    app.on_engine(move |e| {
        // A missing media handle must not delay a local preview in the fetch queue.
        e.media = None;
        e.intent(Intent::SendAttachments {
            chat: ChatId("alice".into()),
            items: vec![item],
        })
        .unwrap();
        assert!(e.waiting_assets.is_empty());
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
    let removed_key = app.on_engine(|e| {
        let key = pending(
            &mut e.store,
            Content::text("discard me"),
            1234,
            i64::MAX / 2,
        );
        e.store
            .mark_failed(
                key,
                Delivery::Rejected {
                    reason: Refusal::AppClosed,
                },
            )
            .unwrap();
        e.publish().unwrap();
        e.store.discard(key).unwrap();
        set_draft(e, "draft");
        key
    });
    let deltas: Vec<_> = app.worker.deltas.try_iter().collect();
    assert!(
        deltas
            .iter()
            .any(|d| matches!(d, Delta::Window { remove, .. } if remove.contains(&removed_key)))
    );
    assert!(deltas.iter().any(|d| {
        match d {
            Delta::Chats { upsert, .. } => upsert
                .iter()
                .any(|r| r.draft.as_ref().is_some_and(|d| d.text == "draft")),
            _ => false,
        }
    }));
    assert!(
        deltas
            .iter()
            .any(|d| matches!(d, Delta::Window { upsert, .. }
        if upsert.iter().any(|row| row.key == removed_key)))
    );
    app.on_engine(|e| {
        set_draft(e, "draft again");
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
    let mut app = App::start(StoreLocation::Memory, fake);
    app.until(|s| matches!(s.connection, Connection::SignedOut { .. }));
    let failed_since = Instant::now();
    app.on_engine(move |e| e.health.failure(failed_since));
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
    app.on_engine(move |e| assert_eq!(e.health.failed_since, Some(failed_since)));
}

#[test]
fn last_account_startup_and_logout_when_backend_cannot_open() -> anyhow::Result<()> {
    for value in [
        "U123\n",
        "../corrupt",
        "\n",
        "broken/filename",
        "self",
        "auth",
    ] {
        let temp = TempDir::new("last-account");
        let paths = temp.locations();
        let root = &paths.data;
        let logout = matches!(value, "self" | "auth");
        temp.write(&root.join("auth/last-account"), value.as_bytes());
        let valid = value.trim() == "U123";
        if valid {
            let kept = Draft {
                updated: 1,
                ..draft("kept")
            };
            Store::open_account(root, "U123")?.save_draft(&ChatId("alice".into()), &kept)?;
        }
        let fake = FakeBackend::signed_out();
        if logout {
            for path in [
                paths.session.clone(),
                root.join("auth/keep"),
                paths.cache.join(value).join("keep"),
            ] {
                temp.write(&path, b"{}");
            }
            fake.fail_open(Some(crate::client::error::SignInFailure::ExtensionMissing));
        }
        let mut app = App::start(account_location(&paths), fake);
        app.signed_out();
        if logout {
            app.send(Intent::Logout { delete_data: true });
            app.until_delta(|d| matches!(d, Delta::Reset), harness::WAIT);
            app.signed_out();
            assert_eq!(
                app.state.connection,
                Connection::SignedOut {
                    reason: SignOutReason::NoSession
                }
            );
            assert!(!paths.session.exists());
            assert!(!root.join("auth/last-account").exists());
            assert_eq!(paths.cache.join(value).exists(), value == "auth");
            assert!(root.join("auth/keep").exists());
        } else {
            app.on_engine(move |e| {
                assert_eq!(e.store_mid.as_deref(), valid.then_some("U123"));
                if valid {
                    let kept = e
                        .store
                        .load_draft(&ChatId("alice".into()))
                        .unwrap()
                        .unwrap();
                    assert_eq!(kept.text, "kept");
                }
            });
        }
    }
    Ok(())
}

#[test]
fn identified_account_survives_copy_failure_without_publishing_a_store() {
    let temp = TempDir::new("identity-copy-failure");
    let root = temp.0.join("sen");
    let old = temp.old_account("self");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("store.sqlite3"), b"corrupt").unwrap();
    let mut app = App::account(&temp.locations());
    assert_eq!(app.state.connection, Connection::Online);
    assert!(!root.join("self/store.sqlite3").exists());
    app.settle();
}

#[test]
fn logout_keeps_history_and_isolates_the_next_account() {
    let temp = TempDir::new("logout-keep");
    let paths = temp.locations();
    let mut app = App::account(&paths);
    app.on_engine(|e| set_draft(e, "private draft"));
    app.fake.fail_rpc(FakeRpc::Logout, Failure::NotDelivered);
    app.on_engine(|e| {
        e.account = AccountSession::SigningIn {
            epoch: 0,
            previous: Box::new(AccountSession::Ready),
        };
        e.intent(Intent::Logout { delete_data: false }).unwrap();
        assert!(e.account.signing_in());
        e.account = AccountSession::Ready;
    });
    app.send(Intent::Logout { delete_data: false });
    app.signed_out();
    assert!(app.fake.calls().contains(&FakeRpc::Logout));
    app.fake.configure(|c| c.complete_sign_in = true);
    app.send(Intent::SignIn);
    app.online();
    app.select("alice");
    app.until(|s| s.composer == "private draft");
    app.on_engine(|e| {
        e.intent(Intent::Logout { delete_data: false }).unwrap();
        e.generation.fetch_add(1, Ordering::SeqCst);
    });
    app.signed_out();
    app.on_engine(|e| assert!(matches!(e.account, AccountSession::NoSession)));
    app.fake.configure(|c| {
        c.mid = Some("other".into());
        c.chats = vec![ChatInput {
            id: ChatId("second-only".into()),
            ..Default::default()
        }];
    });
    for chat in ["alice", "weekend"] {
        app.fake.set_page(ChatId(chat.into()), vec![]);
    }
    app.send(Intent::SignIn);
    app.until(|s| {
        s.connection == Connection::Online && s.chats.contains_key(&ChatId("second-only".into()))
    });
    app.select("second-only");
    app.settle();
    assert_eq!(app.state.chats.len(), 1);
    assert_ne!(app.state.composer, "private draft");
    assert!(app.state.messages.is_empty());
}

#[test]
fn logout_deletes_only_the_requested_account_after_joining_writers() -> anyhow::Result<()> {
    let temp = TempDir::new("logout-delete");
    let paths = temp.locations();
    let root = &paths.data;
    let mut app = App::account(&paths);
    let previews = root.join("images/local-previews-crash");
    let roots = [root, &paths.cache, &previews];
    for mid in ["self", "other"] {
        for base in roots {
            temp.write(&base.join(mid).join("keep"), b"private");
        }
        let path = root.join(format!("clipboard-attachments/{mid}.txt"));
        temp.write(&path, b"attachment");
        let mut item = crate::attachments::Attachment::prepare(&path)?;
        item.clipboard = true;
        let mut store = Store::open_account(root, mid)?;
        pending(&mut store, item.content(), 99, now_ms());
    }
    let output = paths.cache.join("self/late-image");
    let joined = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = joined.clone();
    let (key, generation) = app.on_engine(move |e| {
        let key = pending(&mut e.store, Content::text("pending"), 100, now_ms());
        e.in_flight.insert(("self".into(), key));
        e.media = Some(AssetRequests {
            stop: Arc::new(CancelToken::new()),
            request: Arc::new(|_, _, _| {}),
            retry: Arc::new(|_| {}),
            join: Arc::new(move || {
                fs::create_dir_all(output.parent().unwrap()).unwrap();
                fs::write(&output, b"image").unwrap();
                done.store(true, Ordering::SeqCst);
            }),
        });
        (key, e.generation.load(Ordering::SeqCst))
    });
    temp.write(&paths.session, b"{}");
    let (entered, release) = app.fake.block_send();
    app.send_text("blocked during logout");
    entered.recv_timeout(harness::WAIT)?;
    app.send(Intent::Logout { delete_data: true });
    app.on_engine(|_| {});
    let detached = !paths.session.exists()
        && !root.join("auth/last-account").exists()
        && root.join("auth/logout-pending").exists();
    release.send(()).unwrap();
    assert!(detached, "logout must be durable before the server call");
    app.signed_out();
    app.on_engine(move |e| {
        let result = Err(Failure::NotDelivered);
        e.sent(("self".into(), key), 100, result, generation)
            .unwrap()
    });
    assert!(joined.load(Ordering::SeqCst), "media worker was detached");
    for base in roots {
        assert!(!base.join("self").exists());
        assert!(base.join("other/keep").exists());
    }
    let clipboard = root.join("clipboard-attachments");
    assert!(!clipboard.join("self.txt").exists());
    assert!(clipboard.join("other.txt").exists());
    Store::open(&temp.old_account("self"))?;
    Store::copy_first_start(root, "self", &root.join("self"))?;
    assert!(
        !root.join("self").exists(),
        "deleted history was imported again"
    );
    Ok(())
}

#[test]
fn logout_reports_each_local_cleanup_failure_and_leaves_a_retry_marker() -> anyhow::Result<()> {
    for failed in [
        "sen/auth/session.json",
        "sen/auth/last-account",
        "keyring",
        "sen/self",
        "cache/self",
        "sen/auth/logout-pending",
    ] {
        let temp = TempDir::new("logout-failure");
        let paths = temp.locations();
        let root = &paths.data;
        let mut app = App::account(&paths);
        let path = temp.0.join(failed);
        if failed == "keyring" {
            temp.write(
                &paths.session,
                br#"{"format":"line-desktop-sealed-session","key_id":"fake-key"}"#,
            );
        } else if failed.ends_with("self") {
            fs::create_dir_all(path.parent().unwrap())?;
            if path.exists() {
                fs::rename(&path, temp.0.join("outside"))?;
            }
            std::os::unix::fs::symlink(root, path)?;
        } else {
            let _ = fs::remove_file(&path);
            fs::create_dir(path)?;
        }
        app.send(Intent::Logout { delete_data: true });
        app.until_delta(|d| matches!(d, Delta::LogoutCleanupFailed), harness::WAIT);
        app.signed_out();
        assert!(root.join("auth/logout-pending").exists(), "{failed}");
        app.send(Intent::SignIn);
        app.on_engine(|e| assert!(matches!(e.account, AccountSession::NoSession)));
        assert!(!app.fake.calls().contains(&FakeRpc::SignIn));
    }
    Ok(())
}

#[test]
fn startup_finishes_a_logout_interrupted_at_each_local_step() -> anyhow::Result<()> {
    for step in 0..5 {
        let temp = TempDir::new("logout-crash");
        let locations = temp.locations();
        let paths = [
            "sen/auth/session.json",
            "sen/auth/last-account",
            "sen/self",
            "cache/self",
        ];
        for (i, path) in paths.iter().enumerate() {
            let path = temp.0.join(path);
            temp.write(&if i < 2 { path } else { path.join("keep") }, b"self");
        }
        let marker = locations.data.join("auth/logout-pending");
        temp.write(
            &marker,
            br#"{"mid":"self","key_id":null,"delete_data":true}"#,
        );
        for path in paths.into_iter().take(step) {
            let path = temp.0.join(path);
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        let mut app = App::start(account_location(&locations), FakeBackend::canned());
        app.signed_out();
        assert!(
            app.fake.calls().is_empty(),
            "recovery must not open the backend"
        );
        assert!(!marker.exists());
        for path in paths {
            assert!(!temp.0.join(path).exists(), "{step}: {path}");
        }
    }
    Ok(())
}
