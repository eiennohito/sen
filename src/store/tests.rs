use super::*;

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
        reply: None,
        mentioned: false,
        skip_badge: false,
        request_seq: None,
        body: Content::text(n.to_string()),
        reactions: vec![],
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
        Content::text("pending"),
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
            let changes = s.link_sent(1, id(101), 999).unwrap();
            if let Some(duplicate) = duplicate {
                assert!(changes.messages_removed.contains(&duplicate));
            }
            let rows = all(&s);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, key);
            assert_eq!(rows[0].sort.sort_ts, 101);
            assert_eq!(rows[0].body, Content::text("101"));
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
    assert!(original.body.is_withdrawn());
    let mut updated = input(1);
    updated.reactions_rev = 5;
    updated.body = Content::text("edited");
    s.apply_live(&chat(), &updated, true, None).unwrap();
    let row = &all(&s)[0];
    assert_eq!(row.key, original.key);
    assert!(row.body.is_withdrawn());
    assert_eq!(row.delivery, Delivery::Sent);
}
#[test]
fn reaction_and_content_revisions_only_advance() {
    let mut s = store();
    let mut m = input(1);
    m.reactions_rev = 2;
    m.reactions = reaction("new");
    s.apply_live(&chat(), &m, false, None).unwrap();
    s.set_reactions(&id(1), reaction("old"), 1).unwrap();
    page(&mut s, &[1], true);
    assert_eq!(all(&s)[0].reactions, reaction("new"));
    assert_eq!(all(&s)[0].body, m.body);
    m.reactions_rev = 3;
    m.body = Content::text("edit");
    m.reactions = reaction("latest");
    s.apply_page(&chat(), PageKind::Recent, &[m], false)
        .unwrap();
    assert_eq!(all(&s)[0].body, Content::text("edit"));
    s.set_reactions(&id(1), reaction("op"), 4).unwrap();
    assert_eq!(all(&s)[0].reactions, reaction("op"));
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
            reason: crate::client::error::Refusal::NotAccepted,
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
    s.mark_failed(
        k,
        Delivery::Rejected {
            reason: crate::client::error::Refusal::Unanswered,
        },
    )
    .unwrap();
    assert_eq!(s.pending(&chat()).unwrap().len(), 1);
    let changes = s.discard(k).unwrap();
    assert!(changes.messages_removed.contains(&k));
    assert!(all(&s).is_empty());
    let k2 = pending(&mut s, 3);
    assert_ne!(k, k2);
    s.link_sent(3, id(5), 5).unwrap();
    assert!(s.discard(k2).is_err());
    assert!(s.retry(k2, 4).is_err());
    assert!(
        s.mark_failed(
            k2,
            Delivery::Rejected {
                reason: crate::client::error::Refusal::Unanswered
            }
        )
        .is_err()
    );
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
    ensure_chat(&empty.conn, &chat()).unwrap();
    page(&mut empty, &[], true);
    assert!(
        empty
            .window(&chat(), Anchor::Latest, 10, 0)
            .unwrap()
            .reaches_start
    );
}
#[test]
fn empty_recent_does_not_create_chat_until_fetched_or_sent_message() {
    for reaches_start in [false, true] {
        for fetched in [false, true] {
            let mut s = store();
            let changes = s
                .apply_page(&chat(), PageKind::Recent, &[], reaches_start)
                .unwrap();
            assert!(changes.chats_changed.is_empty());
            assert!(s.take_changes().chats_changed.is_empty());
            assert!(s.chat(&chat()).unwrap().is_none());
            assert!(s.chat_list(10, 0).unwrap().is_empty());
            assert!(messages::segments(&s.conn, &chat()).unwrap().is_empty());

            if fetched {
                page(&mut s, &[1], true);
            } else {
                pending(&mut s, 1);
            }
            assert_eq!(s.chat_list(10, 0).unwrap().len(), 1);
            let key = all(&s)[0].key;
            page(&mut s, &[], true);
            assert_eq!(s.chat_list(10, 0).unwrap().len(), 1);
            assert_eq!(all(&s)[0].key, key);
        }
    }
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
    assert!(!s.chat(&b.id).unwrap().unwrap().left);
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
    assert_eq!(c.unread_count, 8);
    assert!(!c.unread_stale);
    s.mark_read(&chat(), id(1)).unwrap();
    assert_eq!(
        s.chat_list(1, 0).unwrap()[0].chat.last_read_server_id,
        Some(id(2))
    );
    let mut server = ChatInput {
        id: chat(),
        unread_count: 10,
        last_message_id: Some(id(100)),
        ..Default::default()
    };
    s.upsert_chats(&[server.clone()], false).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].chat.unread_count, 8);
    s.confirm_read(&chat(), &id(2)).unwrap();
    server.unread_count = 8;
    s.upsert_chats(&[server], false).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].chat.unread_count, 8);
    s.mark_read(&chat(), id(3)).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].chat.unread_count, 7);
    let d = Draft {
        text: "hello".into(),
        cursor: 3,
        anchor: 0,
        updated: 99,
    };
    s.save_draft(&chat(), &d).unwrap();
    assert_eq!(s.load_draft(&chat()).unwrap(), Some(d));
    s.clear_draft(&chat()).unwrap();
    assert!(s.load_draft(&chat()).unwrap().is_none());
    s.set_account(
        "own",
        Profile {
            display_name: "Me".into(),
            ..Default::default()
        },
    )
    .unwrap();
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
        s.mark_failed(
            key,
            Delivery::Rejected {
                reason: crate::client::error::Refusal::Unanswered,
            },
        )
        .unwrap();
        s.withdraw(&id(9)).unwrap();
        s.set_account(
            "own",
            Profile {
                display_name: "Me".into(),
                ..Default::default()
            },
        )
        .unwrap();
        s.set_sync_revision(42).unwrap();
        s.save_draft(
            &chat(),
            &Draft {
                text: "draft".into(),
                cursor: 2,
                anchor: 0,
                updated: 10,
            },
        )
        .unwrap();
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
    assert_eq!(
        s.message(key).unwrap().delivery,
        Delivery::Rejected {
            reason: crate::client::error::Refusal::Unanswered
        }
    );
    assert_eq!(s.sync_revision().unwrap(), 42);
    assert_eq!(s.account().unwrap().own_mid, "own");
    assert_eq!(s.load_draft(&chat()).unwrap().unwrap().text, "draft");
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
            .body
            .is_withdrawn()
    );
}
#[test]
fn newer_schema_is_refused_without_migration() {
    let dir = TempDir::new();
    {
        let s = Store::open(&dir.0).unwrap();
        s.conn.pragma_update(None, "user_version", 7).unwrap();
    }
    assert!(matches!(
        Store::open(&dir.0),
        Err(StoreError::NewerSchema(7))
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
    s.set_reactions(&id(1), reaction("new"), 10).unwrap();
    let mut m = input(1);
    m.body = Content::text("edited");
    m.reactions_rev = 5;
    m.reactions = reaction("stale");
    s.apply_page(&chat(), PageKind::Recent, &[m], false)
        .unwrap();
    let row = &all(&s)[0];
    assert_eq!(row.body, Content::text("edited"));
    assert_eq!(row.reactions, reaction("new"));
    assert_eq!(row.reactions_rev, 10);
    assert_eq!(row.content_rev, Some(5));
}
#[test]
fn pending_between_confirmed_rows_does_not_hide_gap() {
    let mut s = store();
    page(&mut s, &[10], true);
    s.insert_pending(
        &chat(),
        LocalId("between".into()),
        1,
        Content::text("pending"),
        20,
    )
    .unwrap();
    s.apply_live(&chat(), &input(30), false, None).unwrap();
    let w = s.window(&chat(), Anchor::Latest, 10, 0).unwrap();
    assert_eq!(
        w.gaps,
        vec![Gap {
            before: Some(w.rows[0].key),
            after: w.rows[2].key
        }]
    );
}
#[test]
fn signed_revision_first_content_and_duplicate_requests() {
    let mut s = store();
    let key = pending(&mut s, 1);
    assert!(
        s.insert_pending(
            &chat(),
            LocalId("different".into()),
            1,
            Content::text("other"),
            5
        )
        .is_err()
    );
    assert_eq!(all(&s).len(), 1);
    s.link_sent(1, id(1), 1).unwrap();
    let mut m = input(1);
    m.reactions_rev = i64::MIN;
    m.reactions = reaction("first");
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
    s.set_reactions(&id(1), reaction("newer"), 10).unwrap();
    let mut m = input(1);
    m.reactions_rev = 5;
    s.apply_page(&chat(), PageKind::Recent, &[m.clone()], true)
        .unwrap();
    let row = s.message(key).unwrap();
    assert_eq!(row.body, m.body);
    assert_eq!(row.reactions, reaction("newer"));
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

#[test]
fn linked_send_extends_only_a_continuous_head() {
    for continuous in [true, false] {
        let mut s = store();
        page(&mut s, &[1, 2], true);
        if !continuous {
            s.apply_live(&chat(), &input(3), false, None).unwrap();
            // A list hint ahead of local history must not be bridged by a send.
            s.conn
                .execute("UPDATE chat SET last_key='9',last_activity=9", [])
                .unwrap();
        }
        pending(&mut s, 1);
        s.link_sent(1, id(101), 101).unwrap();
        let window = s.window(&chat(), Anchor::Latest, 100, 0).unwrap();
        if continuous {
            assert!(window.gaps.is_empty());
            assert!(window.reaches_end);
            assert!(!s.needs_refresh(&chat()).unwrap());
        } else {
            assert!(window.gaps.iter().any(|gap| gap.before.is_some()));
            assert!(s.needs_refresh(&chat()).unwrap());
        }
    }
}

#[test]
fn member_cache_preserves_backend_order_and_contact_names() {
    let mut s = store();
    let rows: Vec<(String, Member)> = [
        ("self", "You"),
        ("alice", "葵"),
        ("member-2", "蓮"),
        ("member-10", "Weekend walker 10"),
    ]
    .into_iter()
    .map(|(id, name)| {
        (
            id.into(),
            Member {
                resolved: true,
                is_self: id == "self",
                contact: Friend {
                    id: id.into(),
                    name: name.into(),
                    ..Default::default()
                },
            },
        )
    })
    .collect::<Vec<_>>();
    s.save_members(
        &chat(),
        &rows.iter().map(|(_, row)| row.clone()).collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        s.members(&chat()).unwrap(),
        rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>()
    );
}

fn reaction(actor: &str) -> Vec<Reaction> {
    vec![Reaction {
        actor: actor.into(),
        kind: ReactionKind::Predefined(2),
    }]
}

#[test]
fn clipboard_cleanup_retains_unsent_sources_across_accounts() {
    let root = std::env::temp_dir().join(format!("line-clipboard-prune-{}", std::process::id()));
    let accounts = root.join("accounts");
    let clipboard = root.join("clipboard-attachments");
    std::fs::create_dir_all(&clipboard).unwrap();
    let current = Store::open(&accounts.join("current")).unwrap();
    let mut other = Store::open(&accounts.join("other")).unwrap();
    for (index, delivery) in [
        Delivery::Pending,
        Delivery::Rejected {
            reason: crate::backend::Refusal::FileMissing,
        },
        Delivery::Sent,
    ]
    .into_iter()
    .enumerate()
    {
        let path = clipboard.join(format!("{index}.txt"));
        std::fs::write(&path, b"retry source").unwrap();
        let mut item = crate::attachments::Attachment::prepare(&path).unwrap();
        item.clipboard = true;
        let seq = index as u32 + 1;
        let (key, _) = other
            .insert_pending(
                &chat(),
                LocalId(format!("local-{index}")),
                seq,
                item.content(),
                100,
            )
            .unwrap();
        match delivery {
            Delivery::Pending => (),
            Delivery::Sent => {
                other.link_sent(seq, id(100), 100).unwrap();
            }
            failure => {
                other.mark_failed(key, failure).unwrap();
            }
        }
    }
    let orphan = clipboard.join("orphan.png");
    std::fs::write(&orphan, b"abandoned selection").unwrap();
    drop(other);
    current.prune_clipboard(&clipboard, &accounts).unwrap();
    for index in 0..2 {
        assert!(clipboard.join(format!("{index}.txt")).exists());
    }
    assert!(!clipboard.join("2.txt").exists());
    assert!(!orphan.exists());
    drop(current);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_converts_uncertain_rows_to_not_sent() {
    let dir = TempDir::new();
    let key = {
        let mut store = Store::open(&dir.0).unwrap();
        let key = pending(&mut store, 1);
        store
            .conn
            .execute(r#"UPDATE message SET delivery='"Uncertain"'"#, [])
            .unwrap();
        store.conn.pragma_update(None, "user_version", 5).unwrap();
        key
    };
    let store = Store::open(&dir.0).unwrap();
    assert_eq!(
        store.message(key).unwrap().delivery,
        Delivery::Rejected {
            reason: crate::client::error::Refusal::Unanswered
        }
    );
}
