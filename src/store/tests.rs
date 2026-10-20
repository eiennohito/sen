use super::*;

use std::sync::atomic::{AtomicU64, Ordering};

fn chat() -> ChatId {
    ChatId("chat".into())
}
fn id(n: u64) -> ServerId {
    ServerId::new(n.to_string()).unwrap()
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
        rev: 0,
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
        None,
    )
    .unwrap()
}

#[test]
fn stale_page_preserves_confirmed_pending_row() {
    let mut s = store();
    let key = pending(&mut s, 1);
    s.link_sent(1, id(101), 101).unwrap();
    page(&mut s, &[1, 2], false);
    assert_eq!(all(&s).len(), 3);
    assert_eq!(s.newest(&chat()).unwrap().unwrap().key, key);
}
#[test]
fn newer_hint_requires_refresh_until_page_catches_up() {
    let mut s = store();
    s.apply_live(&chat(), &input(20), false).unwrap();
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
            let key = pending(&mut s, 0);
            let mut echo = input(101);
            echo.outgoing = true;
            echo.request_seq = Some(0);
            if !echo_first {
                s.link_sent(0, id(101), 101).unwrap();
            }
            if live_match {
                s.apply_live(&chat(), &echo, true).unwrap();
            } else {
                s.apply_page(&chat(), PageKind::Recent, &[echo], false)
                    .unwrap();
            }
            let duplicate = all(&s).into_iter().find(|r| r.key != key).map(|r| r.key);
            s.take_changes();
            s.link_sent(0, id(101), 999).unwrap();
            let changes = s.take_changes();
            if let Some(duplicate) = duplicate {
                assert!(changes.messages_removed.contains(&duplicate));
            }
            let rows = all(&s);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, key);
            assert_eq!(rows[0].sort.sort_ts, 101);
            assert_eq!(rows[0].body, Content::text("101"));
            assert_eq!(rows[0].local_id, Some(LocalId("local-0".into())));
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
    updated.rev = 5;
    updated.body = Content::text("edited");
    s.apply_live(&chat(), &updated, true).unwrap();
    let row = &all(&s)[0];
    assert_eq!(row.key, original.key);
    assert!(row.body.is_withdrawn());
    assert_eq!(row.delivery, Delivery::Sent);
}
#[test]
fn reaction_and_body_revisions_only_advance() {
    let mut s = store();
    let mut m = input(1);
    m.rev = 2;
    m.reactions = reaction("new");
    s.apply_live(&chat(), &m, false).unwrap();
    s.set_reactions(&id(1), reaction("old"), 1).unwrap();
    page(&mut s, &[1], true);
    assert_eq!(all(&s)[0].reactions, reaction("new"));
    assert_eq!(all(&s)[0].body, m.body);
    m.rev = 3;
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
    s.apply_live(&chat(), &m, false).unwrap();
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
            reason: crate::client::error::Refusal::Internal,
        },
    )
    .unwrap();
    assert!(matches!(
        s.all_pending().unwrap()[0].delivery,
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
    assert_eq!(s.all_pending().unwrap().len(), 1);
    s.take_changes();
    s.discard(k).unwrap();
    let changes = s.take_changes();
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
fn same_millisecond_order_is_path_independent_with_pending_last() {
    let mut s = store();
    let nums = [1, 9, 10, 100];
    for n in nums {
        let mut m = input(n);
        m.created_ms = 100;
        s.apply_live(&chat(), &m, false).unwrap();
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

#[test]
fn segments_gaps_overlap_and_cursor_adjacency() {
    let mut s = store();
    page(&mut s, &[10, 20], false);
    s.apply_live(&chat(), &input(40), false).unwrap();
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
    s.apply_live(&chat(), &input(50), true).unwrap();
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
fn empty_start_page_segments() {
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
            s.apply_page(&chat(), PageKind::Recent, &[], reaches_start)
                .unwrap();
            let changes = s.take_changes();
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
    s.take_changes();
    s.upsert_chats(&[a], true).unwrap();
    let changes = s.take_changes();
    assert!(changes.chats_removed.contains(&b.id));
    assert!(!s.chat(&b.id).unwrap().unwrap().left);
    assert_eq!(s.chat_list(10, 0).unwrap().len(), 1);
    s.upsert_chats(&[b], false).unwrap();
    assert_eq!(s.chat_list(1, 1).unwrap().len(), 1);
}
#[test]
fn unread_read_markers_and_partial_acknowledgements() {
    let mut s = store();
    page(&mut s, &[1, 2, 3], true);
    s.upsert_chats(
        &[ChatInput {
            id: chat(),
            server_unread: 9,
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    s.increment_unread(&chat()).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].unread_count, 10);
    s.mark_read(&chat(), id(2)).unwrap();
    assert_eq!(s.chat_item(&chat()).unwrap().unwrap().unread_count, 8);
    s.upsert_details(&[ChatInput {
        id: chat(),
        server_unread: 99,
        last_read_server_id: Some(id(999)),
        last_message_id: Some(id(999)),
        ..Default::default()
    }])
    .unwrap();
    let details = s.chat(&chat()).unwrap().unwrap();
    assert_eq!(details.server_unread, 10);
    assert_eq!(s.chat_item(&chat()).unwrap().unwrap().unread_count, 8);
    assert_eq!(details.last_read_server_id, Some(id(2)));
    assert!(s.pending_reads().unwrap().contains(&chat()));
    s.mark_read(&chat(), id(1)).unwrap();
    assert_eq!(
        s.chat_list(1, 0).unwrap()[0].chat.last_read_server_id,
        Some(id(2))
    );
    let mut server = ChatInput {
        id: chat(),
        server_unread: 10,
        last_message_id: Some(id(100)),
        ..Default::default()
    };
    s.upsert_chats(&[server.clone()], false).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].unread_count, 8);
    s.confirm_read(&chat(), &id(2)).unwrap();
    server.server_unread = 8;
    s.upsert_chats(&[server], false).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].unread_count, 8);
    s.mark_read(&chat(), id(3)).unwrap();
    assert_eq!(s.chat_list(1, 0).unwrap()[0].unread_count, 7);
}
#[test]
fn drafts_preferences_and_account() {
    let mut s = store();
    let d = Draft {
        reply: Some(ReplyReference {
            message_id: crate::store::ServerId::new("42").unwrap(),
            service_code: Some(1),
        }),
        text: "hello".into(),
        cursor: 3,
        anchor: 0,
        updated: 99,
    };
    s.save_draft(&chat(), &d).unwrap();
    assert_eq!(s.load_draft(&chat()).unwrap(), Some(d.clone()));
    let key = s
        .insert_pending(
            &chat(),
            LocalId("reply-pending".into()),
            1234,
            Content::text("reply"),
            100,
            d.reply.clone(),
        )
        .unwrap();
    assert_eq!(s.message(key).unwrap().reply.as_deref(), d.reply.as_ref());
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
fn disk_reopen_preserves_state_and_permissions() {
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
                reply: None,
                text: "draft".into(),
                cursor: 2,
                anchor: 0,
                updated: 10,
            },
        )
        .unwrap();
        s.upsert_chats(
            &[ChatInput {
                id: chat(),
                server_unread: 3,
                ..Default::default()
            }],
            false,
        )
        .unwrap();
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
fn pre_migration_schema_discards_local_store() {
    let version = 12;
    let dir = TempDir::new();
    {
        let mut s = Store::open(&dir.0).unwrap();
        pending(&mut s, 1);
        page(&mut s, &[1], true);
        s.save_draft(
            &chat(),
            &Draft {
                text: "draft".into(),
                reply: None,
                cursor: 5,
                anchor: 5,
                updated: 1,
            },
        )
        .unwrap();
        s.mark_read(&chat(), id(1)).unwrap();
        s.set_prefs(&Prefs {
            enter_sends: true,
            ..Default::default()
        })
        .unwrap();
        assert!(!s.pending_reads().unwrap().is_empty());
        s.conn.pragma_update(None, "user_version", version).unwrap();
    }
    let s = Store::open(&dir.0).unwrap();
    assert!(s.all_pending().unwrap().is_empty());
    assert!(s.chat(&chat()).unwrap().is_none());
    assert!(s.load_draft(&chat()).unwrap().is_none());
    assert_eq!(s.sync_revision().unwrap(), 0);
    assert_eq!(s.prefs().unwrap(), Prefs::default());
    assert!(s.pending_reads().unwrap().is_empty());
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
        None,
    )
    .unwrap();
    s.apply_live(&chat(), &input(30), false).unwrap();
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
            s.apply_live(&chat(), &input(3), false).unwrap();
            // A list hint ahead of local history must not be bridged by a send.
            s.conn
                .execute("UPDATE chat SET last_message_id='9',last_activity=9", [])
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
                is_self: id == "self",
                contact: Friend {
                    id: id.into(),
                    name: Some(name.into()),
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
        let key = other
            .insert_pending(
                &chat(),
                LocalId(format!("local-{index}")),
                seq,
                item.content(),
                100,
                None,
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
fn new_message_before_read_ack_counts_from_zero() {
    let mut s = store();
    page(&mut s, &[10, 20], true);
    s.upsert_chats(
        &[ChatInput {
            id: chat(),
            server_unread: 9,
            last_message_id: Some(id(20)),
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    s.mark_read(&chat(), id(20)).unwrap();
    assert_eq!(s.chat(&chat()).unwrap().unwrap().server_unread, 0);
    s.apply_operation(
        &LiveOperation {
            revision: 1,
            replay: false,
            op: Operation::Message(Box::new(input(30))),
        },
        true,
    )
    .unwrap();
    let c = s.chat_list(1, 0).unwrap().remove(0).chat;
    assert_eq!(c.server_unread, 1);
    assert_eq!(s.chat_item(&chat()).unwrap().unwrap().unread_count, 1);
    s.confirm_read(&chat(), &id(20)).unwrap();
    assert_eq!(s.chat_item(&chat()).unwrap().unwrap().unread_count, 1);
    let anchor = s.unread_anchor(&chat()).unwrap().unwrap();
    s.remove_chat(&chat()).unwrap();
    assert!(s.chat_item(&chat()).unwrap().is_none());
    assert_eq!(s.unread_anchor(&chat()).unwrap(), Some(anchor));
}

#[test]
fn server_ids_validate_construction_and_deserialization() {
    for value in ["", "01", "-1", "1a", " 1", "123456789012345678901"] {
        assert!(ServerId::new(value).is_err());
        assert!(serde_json::from_value::<ServerId>(serde_json::json!(value)).is_err());
    }
    for value in ["0", "1", "99999999999999999999"] {
        assert_eq!(ServerId::new(value).unwrap().as_str(), value);
    }
}

#[test]
fn pending_row_rejects_retry_and_discard_without_changing_sequence() {
    let mut s = store();
    let key = pending(&mut s, 7);
    assert!(s.retry(key, 8).is_err());
    assert_eq!(s.message(key).unwrap().request_seq, Some(7));
    assert!(s.discard(key).is_err());
    let row = s.message(key).unwrap();
    assert_eq!(row.request_seq, Some(7));
    assert_eq!(row.delivery, Delivery::Pending);
}

#[test]
fn room_identity_is_case_sensitive() {
    assert!(ChatId("R00000000000000000000000000000001".into()).is_room());
    for id in [
        "r00000000000000000000000000000001",
        "U00000000000000000000000000000001",
        "C00000000000000000000000000000001",
        "unknown",
    ] {
        assert!(!ChatId(id.into()).is_room());
    }
}

#[test]
fn integer_preview_preferences_load_without_losing_saved_fields() {
    let mut store = Store::open_in_memory().unwrap();
    for (value, preview) in [
        (0, NotificationPreview::NameAndText),
        (1, NotificationPreview::NameOnly),
        (2, NotificationPreview::NewMessage),
    ] {
        let json = format!(
            r#"{{"text_size":18,"enter_sends":true,"desktop":{{"background":true,"autostart":true,"notifications":true,"preview":{value},"sound":true,"mention_exception":true,"group_events":true}},"last_chat":"alice"}}"#
        );
        store
            .conn
            .execute("INSERT OR REPLACE INTO preferences VALUES(1,?1)", [&json])
            .unwrap();
        let prefs = store.prefs().unwrap();
        assert_eq!(prefs.desktop.preview, preview);
        store.set_prefs(&prefs).unwrap();
        let saved: String = store
            .conn
            .query_row("SELECT value FROM preferences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }
}

#[test]
fn stored_unknown_flex_loads_and_is_filled_by_sync() {
    for revision in [0, 10] {
        let mut s = store();
        page(&mut s, &[1], true);
        s.conn
            .execute(
                "UPDATE message SET body_json=?,body_rev=? WHERE server_id='1'",
                params![r#"{"Unknown":{"content_type":22}}"#, revision],
            )
            .unwrap();
        assert_eq!(all(&s)[0].body, Content::Unknown { content_type: 22 });
        let mut refreshed = input(1);
        refreshed.body = Content::Flex {
            alt_text: "Login notice".into(),
            lines: vec!["New login".into()],
            links: vec![],
        };
        s.apply_page(&chat(), PageKind::Recent, &[refreshed.clone()], true)
            .unwrap();
        assert_eq!(all(&s)[0].body, refreshed.body);
        assert_eq!(all(&s)[0].body_rev, Some(revision));
    }
}

#[test]
fn v13_migration_preserves_local_data() {
    let dir = TempDir::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let conn = Connection::open(dir.0.join("store.sqlite3")).unwrap();
    conn.execute_batch(include_str!("schema-v13.test.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 13).unwrap();
    let mut s = Store::initialize(conn).unwrap();
    page(&mut s, &[1, 2], true);
    pending(&mut s, 3);
    s.upsert_chats(
        &[ChatInput {
            id: ChatId("other".into()),
            title: "Other chat".into(),
            server_unread: 7,
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    for id in [chat(), ChatId("other".into())] {
        s.save_draft(
            &id,
            &Draft {
                text: format!("draft for {}", id.0),
                cursor: 2,
                anchor: 1,
                updated: 10,
                reply: None,
            },
        )
        .unwrap();
    }
    s.set_prefs(&Prefs {
        enter_sends: true,
        ..Default::default()
    })
    .unwrap();
    s.conn
        .execute("UPDATE chat SET unread_stale=1", [])
        .unwrap();
    // Compare all persisted rows, omitting only the column being removed.
    let snapshot = |conn: &Connection| {
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                let columns: Vec<String> = conn
                    .prepare(&format!(
                        "SELECT name FROM pragma_table_info('{table}')
                     WHERE name != 'unread_stale' ORDER BY cid"
                    ))
                    .unwrap()
                    .query_map([], |r| r.get(0))
                    .unwrap()
                    .collect::<std::result::Result<_, _>>()
                    .unwrap();
                let mut stmt = conn
                    .prepare(&format!(
                        "SELECT {0} FROM {table} ORDER BY {0}",
                        columns.join(",")
                    ))
                    .unwrap();
                let count = stmt.column_count();
                stmt.query_map([], |r| {
                    (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot(&s.conn);
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert_eq!(
        s.conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(snapshot(&s.conn), before);
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('chat') WHERE name='unread_stale'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn newer_schema_is_refused_without_changing_file() {
    let dir = TempDir::new();
    let mut s = Store::open(&dir.0).unwrap();
    page(&mut s, &[1, 2], true);
    pending(&mut s, 1);
    s.conn.pragma_update(None, "user_version", 15).unwrap();
    drop(s);
    let path = dir.0.join("store.sqlite3");
    let before = std::fs::read(&path).unwrap();
    let error = Store::open(&dir.0)
        .err()
        .expect("newer store must be refused");
    assert!(error.to_string().contains("15"), "{error}");
    assert!(error.to_string().contains("newer"), "{error}");
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn unknown_chat_counts_arrivals_then_uses_metadata_count() {
    let mut s = store();
    for revision in 1..=3 {
        s.apply_operation(
            &LiveOperation {
                revision,
                replay: false,
                op: Operation::Message(Box::new(input(revision as u64))),
            },
            true,
        )
        .unwrap();
        let item = s.chat_item(&chat()).unwrap().unwrap();
        assert_eq!(item.unread_count, revision as u32);
    }
    s.upsert_chats(
        &[ChatInput {
            id: chat(),
            title: "Metadata arrived".into(),
            server_unread: 9,
            ..Default::default()
        }],
        false,
    )
    .unwrap();
    let item = s.chat_item(&chat()).unwrap().unwrap();
    assert_eq!(item.chat.server_unread, 9);
    assert_eq!(item.unread_count, 9);
}
