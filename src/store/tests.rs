use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

fn chat() -> ChatId {
    ChatId("chat".into())
}
fn id(n: u64) -> ServerId {
    ServerId(n.to_string())
}
fn input(n: u64) -> MessageInput {
    MessageInput {
        server_id: id(n),
        chat_id: chat(),
        created_ms: n as i64,
        delivered_ms: None,
        sender_id: "sender".into(),
        outgoing: false,
        content_type: 0,
        request_seq: None,
        body: json!({"text":n}),
        reactions: json!([]),
        reactions_rev: 0,
    }
}
fn store() -> Store {
    Store::open_in_memory().unwrap()
}
fn page(s: &mut Store, nums: &[u64], start: bool) {
    s.apply_page(
        &chat(),
        PageKind::Recent,
        &nums.iter().map(|n| input(*n)).collect::<Vec<_>>(),
        start,
    )
    .unwrap();
}
fn all(s: &Store) -> Vec<StoredMessage> {
    s.window(&chat(), Anchor::Latest, usize::MAX, 0)
        .unwrap()
        .rows
}
fn pending(s: &mut Store, seq: u32) -> Key {
    s.insert_pending(
        &chat(),
        LocalId(format!("local-{seq}")),
        seq,
        json!("pending"),
        100,
    )
    .unwrap()
    .0
}

#[test]
fn m13_stale_page_keeps_confirmed_pending_key() {
    let mut s = store();
    let key = pending(&mut s, 1);
    s.link_sent(1, id(101), 101).unwrap();
    page(&mut s, &[1, 2], false);
    assert_eq!(all(&s).len(), 3);
    assert_eq!(s.newest(&chat()).unwrap().unwrap().key, key);
}
#[test]
fn m14_stale_page_keeps_live_and_newer_list_requests_refresh() {
    let mut s = store();
    s.apply_live(&chat(), &input(20), false, Some(2)).unwrap();
    page(&mut s, &[1, 2], false);
    assert_eq!(s.newest(&chat()).unwrap().unwrap().server_id, Some(id(20)));
    s.upsert_chats(
        &[ChatInput {
            id: chat(),
            last_message_id: Some(id(30)),
            last_message_ms: Some(30),
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    assert!(s.needs_refresh(&chat()).unwrap());
    let list = s.chat_list(10, 0).unwrap();
    assert_eq!(list[0].preview.as_ref().unwrap().server_id, Some(id(20)));
    page(&mut s, &[20, 30], false);
    assert!(!s.needs_refresh(&chat()).unwrap());
}
#[test]
fn echo_and_result_orders_preserve_key_and_merge_duplicate() {
    for echo_first in [false, true] {
        for live_match in [false, true] {
            let mut s = store();
            let key = pending(&mut s, 1);
            let mut echo = input(101);
            echo.outgoing = true;
            echo.request_seq = Some(1);
            if !echo_first {
                s.link_sent(1, id(101), 101).unwrap();
            }
            if live_match {
                s.apply_live(&chat(), &echo, true, None).unwrap();
            } else {
                s.apply_page(&chat(), PageKind::Recent, &[echo], false)
                    .unwrap();
            }
            let duplicate = all(&s).into_iter().find(|r| r.key != key).map(|r| r.key);
            let changes = s.link_sent(1, id(101), 101).unwrap();
            if let Some(duplicate) = duplicate {
                assert!(changes.messages_removed.contains(&duplicate));
            }
            let rows = all(&s);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, key);
            assert_eq!(rows[0].body, json!({"text":101}));
            assert_eq!(rows[0].local_id, Some(LocalId("local-1".into())));
        }
    }
}
#[test]
fn withdrawal_tombstone_and_sticky_body() {
    let mut s = store();
    s.withdraw(&id(1)).unwrap();
    page(&mut s, &[1], true);
    let original = all(&s)[0].clone();
    assert!(original.withdrawn);
    assert!(original.body.is_null());
    let mut updated = input(1);
    updated.reactions_rev = 5;
    updated.body = json!("edited");
    s.apply_live(&chat(), &updated, true, None).unwrap();
    let row = &all(&s)[0];
    assert_eq!(row.key, original.key);
    assert!(row.withdrawn);
    assert!(row.body.is_null());
    assert_eq!(row.delivery, Delivery::Sent);
}
#[test]
fn reaction_and_content_revisions_only_advance() {
    let mut s = store();
    let mut m = input(1);
    m.reactions_rev = 2;
    m.reactions = json!(["new"]);
    s.apply_live(&chat(), &m, false, None).unwrap();
    s.set_reactions(&id(1), json!(["old"]), 1).unwrap();
    page(&mut s, &[1], true);
    assert_eq!(all(&s)[0].reactions, json!(["new"]));
    assert_eq!(all(&s)[0].body, m.body);
    m.reactions_rev = 3;
    m.body = json!("edit");
    m.reactions = json!(["latest"]);
    s.apply_page(&chat(), PageKind::Recent, &[m], false)
        .unwrap();
    assert_eq!(all(&s)[0].body, json!("edit"));
    s.set_reactions(&id(1), json!(["op"]), 4).unwrap();
    assert_eq!(all(&s)[0].reactions, json!(["op"]));
}
#[test]
fn receipt_never_regresses_and_applies_to_later_pages() {
    let mut s = store();
    let mut m = input(2);
    m.outgoing = true;
    s.apply_live(&chat(), &m, false, None).unwrap();
    s.advance_read_receipt(&chat(), id(10)).unwrap();
    s.apply_page(&chat(), PageKind::Recent, &[m.clone()], false)
        .unwrap();
    assert_eq!(all(&s)[0].delivery, Delivery::Read);
    m.server_id = id(9);
    m.created_ms = 9;
    s.apply_page(&chat(), PageKind::Recent, &[m], false)
        .unwrap();
    assert!(all(&s).iter().all(|r| r.delivery == Delivery::Read));
    s.advance_read_receipt(&chat(), id(1)).unwrap();
    assert!(all(&s).iter().all(|r| r.delivery == Delivery::Read));
}
#[test]
fn outbox_failure_retry_discard_and_confirmation_guards() {
    let mut s = store();
    let k = pending(&mut s, 1);
    s.mark_failed(
        k,
        Delivery::Rejected {
            reason: "denied".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        s.pending(&chat()).unwrap()[0].delivery,
        Delivery::Rejected { .. }
    ));
    s.retry(k, 2).unwrap();
    assert_eq!(s.message(k).unwrap().delivery, Delivery::Pending);
    assert_eq!(s.message(k).unwrap().request_seq, Some(2));
    s.mark_failed(k, Delivery::Uncertain).unwrap();
    assert_eq!(s.pending(&chat()).unwrap().len(), 1);
    let changes = s.discard(k).unwrap();
    assert!(changes.messages_removed.contains(&k));
    assert!(all(&s).is_empty());
    let k2 = pending(&mut s, 3);
    assert_ne!(k, k2);
    s.link_sent(3, id(5), 5).unwrap();
    assert!(s.discard(k2).is_err());
    assert!(s.retry(k2, 4).is_err());
    assert!(s.mark_failed(k2, Delivery::Uncertain).is_err());
    assert!(s.mark_failed(k2, Delivery::Read).is_err());
}
#[test]
fn decimal_order_matches_existing_length_then_lexicographic() {
    let ids = [
        "0",
        "1",
        "9",
        "10",
        "99",
        "100",
        "9999999999999999999",
        "10000000000000000000",
        "99999999999999999999",
    ];
    for a in ids {
        for b in ids {
            assert_eq!(
                a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                server_sort(&ServerId(a.into()))
                    .unwrap()
                    .cmp(&server_sort(&ServerId(b.into())).unwrap())
            );
        }
    }
    for invalid in ["", "01", "-1", "a", "100000000000000000000"] {
        assert!(server_sort(&ServerId(invalid.into())).is_err());
    }
}
#[test]
fn same_millisecond_order_is_path_independent_with_pending_last() {
    for reverse in [false, true] {
        let mut s = store();
        let mut nums = vec![1, 9, 10, 100];
        if reverse {
            nums.reverse();
        }
        for n in nums {
            let mut m = input(n);
            m.created_ms = 100;
            s.apply_live(&chat(), &m, false, None).unwrap();
        }
        let k = pending(&mut s, 1);
        assert_eq!(all(&s).last().unwrap().key, k);
        assert_eq!(
            all(&s)
                .iter()
                .filter_map(|m| m.server_id.clone())
                .collect::<Vec<_>>(),
            vec![id(1), id(9), id(10), id(100)]
        );
        s.link_sent(1, id(20), 100).unwrap();
        assert_eq!(
            all(&s)
                .iter()
                .map(|m| m.server_id.clone().unwrap())
                .collect::<Vec<_>>(),
            vec![id(1), id(9), id(10), id(20), id(100)]
        );
        let mut m = input(1);
        m.created_ms = 0;
        m.delivered_ms = Some(100);
        s.apply_page(&chat(), PageKind::Recent, &[m], false)
            .unwrap();
        assert_eq!(all(&s)[0].sort.sort_ts, 100);
    }
}
#[test]
fn segments_gaps_overlap_and_cursor_adjacency() {
    let mut s = store();
    page(&mut s, &[10, 20], false);
    s.apply_live(&chat(), &input(40), false, None).unwrap();
    let before = all(&s)[0].sort.clone();
    s.apply_page(
        &chat(),
        PageKind::Older { before },
        &[input(1), input(5)],
        true,
    )
    .unwrap();
    let w = s.window(&chat(), Anchor::Latest, 100, 0).unwrap();
    assert!(w.reaches_start);
    assert!(w.reaches_end);
    assert_eq!(
        w.gaps,
        vec![Gap {
            before: Some(w.rows[3].key),
            after: w.rows[4].key
        }]
    );
    page(&mut s, &[20, 30, 40], false);
    let w = s.window(&chat(), Anchor::Latest, 100, 0).unwrap();
    assert!(w.gaps.is_empty());
    assert_eq!(messages::segments(&s.conn, &chat()).unwrap().len(), 1);
    s.apply_live(&chat(), &input(50), true, None).unwrap();
    assert!(
        s.window(&chat(), Anchor::Latest, 100, 0)
            .unwrap()
            .gaps
            .is_empty()
    );
    let key = all(&s)[2].key;
    let w = s.window(&chat(), Anchor::Key(key), 1, 1).unwrap();
    assert_eq!(w.rows.len(), 3);
    assert!(!w.reaches_start);
    assert!(!w.reaches_end);
    assert!(w.gaps.is_empty());
}
#[test]
fn empty_start_and_newer_page_segments() {
    let mut s = store();
    page(&mut s, &[5, 10], false);
    let before = all(&s)[0].sort.clone();
    s.apply_page(&chat(), PageKind::Older { before }, &[], true)
        .unwrap();
    assert!(
        s.window(&chat(), Anchor::Latest, 10, 0)
            .unwrap()
            .reaches_start
    );
    let after = all(&s).last().unwrap().sort.clone();
    s.apply_page(&chat(), PageKind::Newer { after }, &[input(20)], false)
        .unwrap();
    assert!(
        s.window(&chat(), Anchor::Latest, 10, 0)
            .unwrap()
            .gaps
            .is_empty()
    );
    let mut empty = store();
    page(&mut empty, &[], true);
    assert!(
        empty
            .window(&chat(), Anchor::Latest, 10, 0)
            .unwrap()
            .reaches_start
    );
}
#[test]
fn live_failure_rolls_back_row_segment_chat_and_revision() {
    let mut s = store();
    s.set_sync_revision(8).unwrap();
    s.conn.execute_batch("CREATE TRIGGER fail_revision BEFORE UPDATE OF sync_revision ON account BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(s.apply_live(&chat(), &input(1), false, Some(9)).is_err());
    assert_eq!(s.sync_revision().unwrap(), 8);
    assert!(all(&s).is_empty());
    assert!(messages::segments(&s.conn, &chat()).unwrap().is_empty());
    assert!(s.chat_list(10, 0).unwrap().is_empty());
}
#[test]
fn invalid_page_rolls_back_preceding_upserts() {
    let mut s = store();
    let mut invalid = input(2);
    invalid.chat_id = ChatId("wrong".into());
    assert!(
        s.apply_page(&chat(), PageKind::Recent, &[input(1), invalid], false)
            .is_err()
    );
    assert!(all(&s).is_empty());
}
#[test]
fn chat_list_sweeps_metadata_and_activity_do_not_regress() {
    let mut s = store();
    page(&mut s, &[50], false);
    let a = ChatInput {
        id: chat(),
        title: "Title".into(),
        last_message_id: Some(id(2)),
        last_message_ms: Some(2),
        ..Default::default()
    };
    let b = ChatInput {
        id: ChatId("other".into()),
        ..Default::default()
    };
    s.upsert_chats(&[a.clone(), b.clone()], false).unwrap();
    let list = s.chat_list(10, 0).unwrap();
    assert_eq!(list[0].activity_ms, 50);
    assert_eq!(list[0].preview.as_ref().unwrap().server_id, Some(id(50)));
    assert!(!s.needs_refresh(&chat()).unwrap());
    let changes = s.upsert_chats(&[a], true).unwrap();
    assert!(changes.chats_removed.contains(&b.id));
    assert_eq!(s.chat_list(10, 0).unwrap().len(), 1);
    s.upsert_chats(&[b], false).unwrap();
    assert_eq!(s.chat_list(1, 1).unwrap().len(), 1);
}
#[test]
fn unread_read_markers_drafts_preferences_and_account() {
    let mut s = store();
    page(&mut s, &[1, 2, 3], true);
    s.set_unread(&chat(), 9, true).unwrap();
    s.increment_unread(&chat()).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].chat.unread_count, 10);
    s.mark_read(&chat(), id(2)).unwrap();
    let c = s.chat_list(1, 0).unwrap()[0].chat.clone();
    assert_eq!(c.unread_count, 1);
    assert!(!c.unread_stale);
    s.mark_read(&chat(), id(1)).unwrap();
    assert_eq!(
        s.chat_list(1, 0).unwrap()[0].chat.last_read_server_id,
        Some(id(2))
    );
    let d = Draft {
        text: "hello".into(),
        cursor: 3,
        updated: 99,
    };
    s.save_draft(&chat(), &d).unwrap();
    assert_eq!(s.load_draft(&chat()).unwrap(), Some(d));
    s.clear_draft(&chat()).unwrap();
    assert!(s.load_draft(&chat()).unwrap().is_none());
    s.set_pref("scroll", json!(123)).unwrap();
    assert_eq!(s.get_pref("scroll").unwrap(), Some(json!(123)));
    assert_eq!(s.get_pref("missing").unwrap(), None);
    s.set_account("own", json!({"name":"Me"})).unwrap();
    s.set_sync_revision(10).unwrap();
    s.set_sync_revision(5).unwrap();
    assert_eq!(s.account().unwrap().own_mid, "own");
    assert_eq!(s.sync_revision().unwrap(), 10);
}
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "line-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn disk_reopen_preserves_state_permissions_and_pragmas() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let key;
    {
        let mut s = Store::open(&dir.0).unwrap();
        page(&mut s, &[1, 2], true);
        key = pending(&mut s, 1);
        s.mark_failed(key, Delivery::Uncertain).unwrap();
        s.withdraw(&id(9)).unwrap();
        s.set_account("own", json!({"name":"Me"})).unwrap();
        s.set_sync_revision(42).unwrap();
        s.save_draft(
            &chat(),
            &Draft {
                text: "draft".into(),
                cursor: 2,
                updated: 10,
            },
        )
        .unwrap();
        s.set_pref("anchor", json!(key.0)).unwrap();
        s.set_unread(&chat(), 3, true).unwrap();
        assert_eq!(
            s.conn
                .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            s.conn
                .pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            s.conn
                .pragma_query_value(None, "foreign_keys", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    assert_eq!(
        std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let mut s = Store::open(&dir.0).unwrap();
    assert_eq!(all(&s).len(), 3);
    assert_eq!(s.message(key).unwrap().delivery, Delivery::Uncertain);
    assert_eq!(s.sync_revision().unwrap(), 42);
    assert_eq!(s.account().unwrap().own_mid, "own");
    assert_eq!(s.load_draft(&chat()).unwrap().unwrap().text, "draft");
    assert_eq!(s.get_pref("anchor").unwrap(), Some(json!(key.0)));
    assert!(
        s.window(&chat(), Anchor::Latest, 10, 0)
            .unwrap()
            .reaches_start
    );
    page(&mut s, &[9], false);
    assert!(
        all(&s)
            .iter()
            .find(|r| r.server_id == Some(id(9)))
            .unwrap()
            .withdrawn
    );
}
#[test]
fn newer_schema_is_refused_without_migration() {
    let dir = TempDir::new();
    {
        let s = Store::open(&dir.0).unwrap();
        s.conn.pragma_update(None, "user_version", 2).unwrap();
    }
    assert!(matches!(
        Store::open(&dir.0),
        Err(StoreError::NewerSchema(2))
    ));
}

#[test]
fn cross_chat_sequence_cannot_steal_pending_row() {
    let mut s = store();
    let key = pending(&mut s, 1);
    let mut echo = input(2);
    echo.chat_id = ChatId("other".into());
    echo.outgoing = true;
    echo.request_seq = Some(1);
    assert!(s.apply_live(&echo.chat_id, &echo, true, Some(5)).is_err());
    assert_eq!(s.message(key).unwrap().delivery, Delivery::Pending);
    assert_eq!(s.sync_revision().unwrap(), 0);
}
#[test]
fn reaction_operation_does_not_suppress_body_revision() {
    let mut s = store();
    page(&mut s, &[1], true);
    s.set_reactions(&id(1), json!(["new"]), 10).unwrap();
    let mut m = input(1);
    m.body = json!("edited");
    m.reactions_rev = 5;
    m.reactions = json!(["stale"]);
    s.apply_page(&chat(), PageKind::Recent, &[m], false)
        .unwrap();
    let row = &all(&s)[0];
    assert_eq!(row.body, json!("edited"));
    assert_eq!(row.reactions, json!(["new"]));
    assert_eq!(row.reactions_rev, 10);
    assert_eq!(row.content_rev, Some(5));
}
#[test]
fn pending_between_confirmed_rows_does_not_hide_gap() {
    let mut s = store();
    page(&mut s, &[10], true);
    s.insert_pending(&chat(), LocalId("between".into()), 1, json!("pending"), 20)
        .unwrap();
    s.apply_live(&chat(), &input(30), false, None).unwrap();
    let w = s.window(&chat(), Anchor::Latest, 10, 0).unwrap();
    assert_eq!(
        w.gaps,
        vec![Gap {
            before: Some(w.rows[1].key),
            after: w.rows[2].key
        }]
    );
}
#[test]
fn signed_revision_first_content_and_duplicate_requests() {
    let mut s = store();
    let key = pending(&mut s, 1);
    assert!(
        s.insert_pending(&chat(), LocalId("different".into()), 1, json!("other"), 5)
            .is_err()
    );
    assert_eq!(all(&s).len(), 1);
    s.link_sent(1, id(1), 1).unwrap();
    let mut m = input(1);
    m.reactions_rev = i64::MIN;
    m.reactions = json!(["first"]);
    s.apply_page(&chat(), PageKind::Recent, &[m.clone()], true)
        .unwrap();
    let row = s.message(key).unwrap();
    assert_eq!(row.body, m.body);
    assert_eq!(row.reactions, m.reactions);
}

#[test]
fn reactions_before_echo_survive_equal_or_older_content() {
    let mut s = store();
    let key = pending(&mut s, 1);
    s.link_sent(1, id(1), 1).unwrap();
    s.set_reactions(&id(1), json!(["newer"]), 10).unwrap();
    let mut m = input(1);
    m.reactions_rev = 5;
    s.apply_page(&chat(), PageKind::Recent, &[m.clone()], true)
        .unwrap();
    let row = s.message(key).unwrap();
    assert_eq!(row.body, m.body);
    assert_eq!(row.reactions, json!(["newer"]));
    assert_eq!(row.reactions_rev, 10);
}
#[test]
fn partial_chat_activity_hints_require_refresh() {
    let mut s = store();
    page(&mut s, &[1], true);
    for metadata in [
        ChatInput {
            id: chat(),
            last_message_id: Some(id(5)),
            ..Default::default()
        },
        ChatInput {
            id: chat(),
            last_message_ms: Some(5),
            ..Default::default()
        },
    ] {
        s.upsert_chats(&[metadata], false).unwrap();
        assert!(s.needs_refresh(&chat()).unwrap());
    }
}
