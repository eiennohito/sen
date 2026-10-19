use super::*;

const SELECT: &str = "SELECT key,chat_id,server_id,local_id,request_seq,sort_ts,sort_id,
    sender_id,outgoing,body_json,reactions_json,reactions_rev,
    delivery,created_at_local,body_rev,mentioned,skip_badge,reply_json FROM message";

fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    decode_at(r, 0)
}
pub(super) fn decode_at(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        key: Key(r.get(offset)?),
        chat_id: ChatId(r.get(offset + 1)?),
        server_id: r.get(offset + 2)?,
        local_id: r.get::<_, Option<String>>(offset + 3)?.map(LocalId),
        request_seq: r.get(offset + 4)?,
        sort: Cursor {
            sort_ts: r.get(offset + 5)?,
            sort_id: r.get(offset + 6)?,
        },
        sender_id: r.get(offset + 7)?,
        outgoing: r.get(offset + 8)?,
        body: parse(r, offset + 9)?,
        reactions: parse(r, offset + 10)?,
        reactions_rev: r.get(offset + 11)?,
        delivery: parse(r, offset + 12)?,
        created_at_local: r.get(offset + 13)?,
        body_rev: r.get(offset + 14)?,
        mentioned: r.get(offset + 15)?,
        reply: parse(r, offset + 17)?,
        skip_badge: r.get(offset + 16)?,
    })
}
pub(super) fn select_rows(
    conn: &Connection,
    clause: &str,
    args: impl rusqlite::Params,
) -> Result<Vec<StoredMessage>> {
    let mut stmt = conn.prepare(&format!("{SELECT} {clause}"))?;
    Ok(stmt
        .query_map(args, decode)?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
pub(super) fn parse<T: serde::de::DeserializeOwned>(
    r: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = r.get(index)?;
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}
pub(super) fn by_key(conn: &Connection, key: Key) -> Result<StoredMessage> {
    Ok(conn.query_row(&format!("{SELECT} WHERE key=?"), [key.0], decode)?)
}

pub(super) fn by_server(conn: &Connection, id: &ServerId) -> Result<Option<StoredMessage>> {
    let key = conn
        .query_row(
            "SELECT key FROM message WHERE server_id=?",
            [id.as_str()],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    key.map(|k| by_key(conn, Key(k))).transpose()
}
fn pending_key(conn: &Connection, seq: u32) -> Result<Option<Key>> {
    Ok(conn
        .query_row(
            "SELECT key FROM message WHERE request_seq=? AND server_id IS NULL",
            [seq],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .map(Key))
}
pub(super) fn save(conn: &Connection, m: &StoredMessage) -> Result<()> {
    conn.execute(
        "UPDATE message SET server_id=?2,local_id=?3,request_seq=?4,sort_ts=?5,sort_id=?6,
        sender_id=?7,outgoing=?8,body_json=?9,reactions_json=?10,reactions_rev=?11,
        delivery=?12,created_at_local=?13,body_rev=?14,mentioned=?15,skip_badge=?16,reply_json=?17 WHERE key=?1",
        params![
            m.key.0,
            m.server_id.as_ref().map(|s| s.as_str()),
            m.local_id.as_ref().map(|s| s.0.as_str()),
            m.request_seq,
            m.sort.sort_ts,
            m.sort.sort_id,
            m.sender_id,
            m.outgoing,
            serde_json::to_string(&m.body)?,
            serde_json::to_string(&m.reactions)?,
            m.reactions_rev,
            serde_json::to_string(&m.delivery)?,
            m.created_at_local,
            m.body_rev, m.mentioned, m.skip_badge, serde_json::to_string(&m.reply)?,
        ],
    )?;
    Ok(())
}
fn insert(conn: &Connection, row: &mut StoredMessage) -> Result<()> {
    ensure_chat(conn, &row.chat_id)?;
    conn.execute(
        "INSERT INTO message(chat_id,server_id,local_id,request_seq,sort_ts,sort_id,sender_id,
        outgoing,body_json,reactions_json,reactions_rev,delivery,created_at_local,
        body_rev,mentioned,skip_badge,reply_json)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            row.chat_id.0,
            row.server_id.as_ref().map(|id| id.as_str()),
            row.local_id.as_ref().map(|id| id.0.as_str()),
            row.request_seq,
            row.sort.sort_ts,
            row.sort.sort_id,
            row.sender_id,
            row.outgoing,
            serde_json::to_string(&row.body)?,
            serde_json::to_string(&row.reactions)?,
            row.reactions_rev,
            serde_json::to_string(&row.delivery)?,
            row.created_at_local,
            row.body_rev,
            row.mentioned,
            row.skip_badge,
            serde_json::to_string(&row.reply)?
        ],
    )?;
    row.key = Key(conn.last_insert_rowid());
    Ok(())
}
fn new_row(chat: &ChatId, ts: i64) -> StoredMessage {
    StoredMessage {
        key: Key(0),
        chat_id: chat.clone(),
        server_id: None,
        local_id: None,
        request_seq: None,
        sort: Cursor {
            sort_ts: ts,
            sort_id: String::new(),
        },
        sender_id: String::new(),
        outgoing: true,
        body: Content::default(),
        reactions: vec![],
        reactions_rev: 0,
        body_rev: None,
        delivery: Delivery::Pending,
        created_at_local: ts,
        mentioned: false,
        reply: None,
        skip_badge: false,
    }
}
fn receipt(conn: &Connection, m: &mut StoredMessage) -> Result<()> {
    if m.outgoing
        && let Some(id) = &m.server_id
    {
        let marker: Option<String> = conn.query_row(
            "SELECT receipt_server_id FROM chat WHERE chat_id=?",
            [&m.chat_id.0],
            |r| r.get(0),
        )?;
        if let Some(marker) = marker
            && *id <= ServerId::new(marker)?
        {
            m.delivery = Delivery::Read;
        }
    }
    Ok(())
}
fn server_row(
    conn: &Connection,
    changes: &mut Changes,
    chat: &ChatId,
    id: &ServerId,
    ts: i64,
    pending: Option<Key>,
) -> Result<StoredMessage> {
    ensure_chat(conn, chat)?;
    let existing = by_server(conn, id)?;
    let row = if let Some(key) = pending {
        let mut target = by_key(conn, key)?;
        if target.chat_id != *chat {
            return Err(StoreError::Invalid(
                "request sequence belongs to another chat".into(),
            ));
        }
        if let Some(other) = existing {
            if other.chat_id != *chat {
                return Err(StoreError::Invalid(
                    "server id belongs to another chat".into(),
                ));
            }
            conn.execute("DELETE FROM message WHERE key=?", [other.key.0])?;
            changes.messages_upserted.remove(&other.key);
            changes.messages_removed.insert(other.key);
            let local = target.local_id;
            let seq = target.request_seq;
            let local_time = target.created_at_local;
            target = other;
            target.key = key;
            target.local_id = local;
            target.request_seq = seq;
            target.created_at_local = local_time;
        } else {
            target.sort.sort_ts = ts;
        }
        target
    } else if let Some(row) = existing {
        if row.chat_id != *chat {
            return Err(StoreError::Invalid(
                "server id belongs to another chat".into(),
            ));
        }
        row
    } else {
        new_row(chat, ts)
    };
    Ok(row)
}
fn merge_server_row(
    conn: &Connection,
    changes: &mut Changes,
    input: &MessageInput,
    pending: Option<Key>,
) -> Result<StoredMessage> {
    let id = &input.server_id;
    let ts = server_time(input);
    let mut row = server_row(conn, changes, &input.chat_id, id, ts, pending)?;
    let was_withdrawn = row.body.is_withdrawn();
    let first_content = row.body_rev.is_none();
    if row.body_rev.is_none_or(|rev| input.rev > rev) {
        row.body = input.body.clone();
        row.mentioned = input.mentioned;
        row.reply = input.reply.clone();
        row.skip_badge = input.skip_badge;
        row.sender_id = input.sender_id.clone();
        row.outgoing = input.outgoing;
        row.body_rev = Some(input.rev);
    }
    if row.reactions_rev < input.rev || (first_content && row.reactions_rev == 0) {
        row.reactions = input.reactions.clone();
        row.reactions_rev = input.rev;
    }
    row.request_seq = row.request_seq.or(input.request_seq);
    row.sort.sort_ts = ts;
    if was_withdrawn {
        row.body = Content::Withdrawn;
    }
    save_server_row(conn, changes, row, id)
}
fn save_server_row(
    conn: &Connection,
    changes: &mut Changes,
    mut row: StoredMessage,
    id: &ServerId,
) -> Result<StoredMessage> {
    row.server_id = Some(id.clone());
    row.sort.sort_id = server_sort(id);
    if row.delivery != Delivery::Read {
        row.delivery = Delivery::Sent;
    }
    if conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM withdrawal WHERE server_id=?)",
        [id.as_str()],
        |r| r.get::<_, bool>(0),
    )? {
        row.body = Content::Withdrawn;
    }
    receipt(conn, &mut row)?;
    if row.key == Key(0) {
        insert(conn, &mut row)?;
    } else {
        save(conn, &row)?;
    }
    conn.execute(
        "UPDATE chat SET removed=0 WHERE chat_id=?",
        [&row.chat_id.0],
    )?;
    changes.messages_upserted.insert(row.key);
    changes.chats_changed.insert(row.chat_id.clone());
    Ok(row)
}
fn server_time(input: &MessageInput) -> i64 {
    if input.created_ms > 0 {
        input.created_ms
    } else {
        input.delivered_ms.filter(|ts| *ts > 0).unwrap_or(0)
    }
}

/// Internally persisted inclusive continuity range. Cursors supply adjacency evidence.
#[derive(Clone, Debug)]
pub(super) struct Segment {
    pub lo: Cursor,
    pub hi: Cursor,
    pub start: bool,
}
pub(super) fn segments(conn: &Connection, chat: &ChatId) -> Result<Vec<Segment>> {
    let mut s=conn.prepare(
"SELECT lo_ts,lo_id,hi_ts,hi_id,reaches_start FROM segment WHERE chat_id=? ORDER BY lo_ts,lo_id"
)?;
    Ok(s.query_map([&chat.0], |r| {
        Ok(Segment {
            lo: Cursor {
                sort_ts: r.get(0)?,
                sort_id: r.get(1)?,
            },
            hi: Cursor {
                sort_ts: r.get(2)?,
                sort_id: r.get(3)?,
            },
            start: r.get(4)?,
        })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()?)
}
fn add_segment(conn: &Connection, chat: &ChatId, range: Segment) -> Result<()> {
    ensure_chat(conn, chat)?;
    let mut all = segments(conn, chat)?;
    all.push(range);
    all.sort_by(|a, b| a.lo.cmp(&b.lo));
    let mut merged: Vec<Segment> = Vec::new();
    for s in all {
        if let Some(last) = merged.last_mut()
            && s.lo <= last.hi
        {
            last.hi = last.hi.clone().max(s.hi);
            last.start |= s.start;
            continue;
        }
        merged.push(s);
    }
    conn.execute("DELETE FROM segment WHERE chat_id=?", [&chat.0])?;
    for s in merged {
        conn.execute(
            "INSERT INTO segment VALUES(?,?,?,?,?,?)",
            params![
                chat.0,
                s.lo.sort_ts,
                s.lo.sort_id,
                s.hi.sort_ts,
                s.hi.sort_id,
                s.start,
            ],
        )?;
    }
    Ok(())
}
impl Store {
    pub fn apply_page(
        &mut self,
        chat: &ChatId,
        kind: PageKind,
        messages: &[MessageInput],
        reaches_start: bool,
    ) -> Result<Changes> {
        // Opening a friend can fetch an empty Recent page before any chat exists.
        // Only a message (or explicit chat metadata) should create that chat.
        if matches!(kind, PageKind::Recent) && messages.is_empty() && self.chat(chat)?.is_none() {
            return Ok(Changes::default());
        }
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            let mut cursors = Vec::new();
            for input in messages {
                cursors.push(merge_server_row(tx, c, input, None)?.sort);
            }
            match &kind {
                PageKind::Older { before } => cursors.push(before.clone()),
                PageKind::Recent | PageKind::Around { .. } => {}
            }
            if cursors.is_empty() && reaches_start {
                if let Some(s) = segments(tx, chat)?.last() {
                    cursors.push(s.lo.clone());
                } else {
                    cursors.push(Cursor {
                        sort_ts: i64::MIN,
                        sort_id: String::new(),
                    });
                }
            }
            if let (Some(lo), Some(hi)) = (cursors.iter().min(), cursors.iter().max()) {
                add_segment(
                    tx,
                    chat,
                    Segment {
                        lo: lo.clone(),
                        hi: hi.clone(),
                        start: reaches_start,
                    },
                )?;
            }
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn apply_live(
        &mut self,
        chat: &ChatId,
        message: &MessageInput,
        continuous: bool,
    ) -> Result<Changes> {
        self.mutate(|tx, c| {
            let pending = if message.outgoing {
                message
                    .request_seq
                    .map(|seq| pending_key(tx, seq))
                    .transpose()?
                    .flatten()
            } else {
                None
            };
            let row = merge_server_row(tx, c, message, pending)?;
            let lo = if continuous {
                segments(tx, chat)?
                    .last()
                    .map(|s| s.hi.clone())
                    .unwrap_or_else(|| row.sort.clone())
            } else {
                row.sort.clone()
            };
            add_segment(
                tx,
                chat,
                Segment {
                    lo: lo.clone().min(row.sort.clone()),
                    hi: lo.max(row.sort),
                    start: false,
                },
            )?;
            Ok(())
        })
    }
    pub fn insert_pending(
        &mut self,
        chat: &ChatId,
        local_id: LocalId,
        request_seq: u32,
        body: Content,
        now_ms: i64,
        reply: Option<ReplyReference>,
    ) -> Result<(Key, Changes)> {
        let mut key = Key(0);
        let c = self.mutate(|tx, c| {
            let mut row = new_row(chat, now_ms);
            row.local_id = Some(local_id);
            row.request_seq = Some(request_seq);
            row.sort.sort_id = format!("~{}", row.local_id.as_ref().unwrap().0);
            row.body = body;
            row.reply = reply.map(Box::new);
            insert(tx, &mut row)?;
            key = row.key;
            c.messages_upserted.insert(key);
            c.chats_changed.insert(chat.clone());
            Ok(())
        })?;
        Ok((key, c))
    }
    pub fn link_sent(
        &mut self,
        request_seq: u32,
        server_id: ServerId,
        created_ms: i64,
    ) -> Result<Changes> {
        self.mutate(|tx, c| {
            if let Some(key) = pending_key(tx, request_seq)? {
                let before = by_key(tx, key)?;
                let head = if !super::queries::head_needs_refresh(tx, &before.chat_id)? {
                    segments(tx, &before.chat_id)?.last().cloned()
                } else {
                    None
                };
                let row = server_row(tx, c, &before.chat_id, &server_id, created_ms, Some(key))?;
                save_server_row(tx, c, row, &server_id)?;
                if let Some(head) = head {
                    let row = by_server(tx, &server_id)?.unwrap();
                    add_segment(
                        tx,
                        &row.chat_id,
                        Segment {
                            lo: head.hi.clone().min(row.sort.clone()),
                            hi: head.hi.max(row.sort),
                            start: false,
                        },
                    )?;
                }
            } else {
                let row = by_server(tx, &server_id)?
                    .ok_or_else(|| StoreError::Invalid("unknown send request".into()))?;
                if row.request_seq != Some(request_seq) {
                    return Err(StoreError::Invalid("send request mismatch".into()));
                }
            }
            Ok(())
        })
    }
    pub fn recover_sends(&mut self) -> Result<()> {
        for row in self.all_pending()? {
            if row.delivery == Delivery::Pending {
                let delivery = Delivery::Rejected {
                    reason: crate::client::error::Refusal::AppClosed,
                };
                self.mark_failed(row.key, delivery)?;
            }
        }
        Ok(())
    }
    pub fn mark_failed(&mut self, key: Key, delivery: Delivery) -> Result<Changes> {
        if !matches!(delivery, Delivery::Rejected { .. }) {
            return Err(StoreError::Invalid("expected failure delivery".into()));
        }
        self.local_delivery(key, delivery, None)
    }
    pub fn retry(&mut self, key: Key, new_request_seq: u32) -> Result<Changes> {
        self.local_delivery(key, Delivery::Pending, Some(new_request_seq))
    }

    fn local_delivery(
        &mut self,
        key: Key,
        delivery: Delivery,
        seq: Option<u32>,
    ) -> Result<Changes> {
        self.mutate(|tx, c| {
            let mut row = by_key(tx, key)?;
            if row.server_id.is_some() {
                return Err(StoreError::Invalid(
                    "confirmed row cannot fail or retry".into(),
                ));
            }
            if seq.is_some() && !matches!(row.delivery, Delivery::Rejected { .. }) {
                return Err(StoreError::Invalid("only failed rows can retry".into()));
            }
            row.delivery = delivery;
            if seq.is_some() {
                row.request_seq = seq;
            }
            save(tx, &row)?;
            c.messages_upserted.insert(key);
            c.chats_changed.insert(row.chat_id);
            Ok(())
        })
    }
    pub fn discard(&mut self, key: Key) -> Result<Changes> {
        self.mutate(|tx, c| {
            let row = by_key(tx, key)?;
            if row.server_id.is_some() || !matches!(row.delivery, Delivery::Rejected { .. }) {
                return Err(StoreError::Invalid(
                    "only failed rows can be discarded".into(),
                ));
            }
            tx.execute("DELETE FROM message WHERE key=?", [key.0])?;
            c.messages_removed.insert(key);
            c.chats_changed.insert(row.chat_id);
            Ok(())
        })
    }
    pub fn withdraw(&mut self, id: &ServerId) -> Result<Changes> {
        self.mutate(|tx, c| {
            tx.execute("INSERT OR IGNORE INTO withdrawal VALUES(?)", [id.as_str()])?;
            if let Some(mut row) = by_server(tx, id)?
                && !row.body.is_withdrawn()
            {
                row.body = Content::Withdrawn;
                save(tx, &row)?;
                c.messages_upserted.insert(row.key);
                c.chats_changed.insert(row.chat_id);
            }
            Ok(())
        })
    }
    pub fn set_reactions(
        &mut self,
        id: &ServerId,
        reactions: Vec<Reaction>,
        rev: i64,
    ) -> Result<Changes> {
        self.mutate(|tx, c| {
            if let Some(mut row) = by_server(tx, id)?
                && rev > row.reactions_rev
            {
                row.reactions = reactions;
                row.reactions_rev = rev;
                save(tx, &row)?;
                c.messages_upserted.insert(row.key);
                c.chats_changed.insert(row.chat_id);
            }
            Ok(())
        })
    }
    pub fn advance_read_receipt(&mut self, chat: &ChatId, up_to: ServerId) -> Result<Changes> {
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            let old = tx
                .query_row(
                    "SELECT receipt_server_id FROM chat WHERE chat_id=?",
                    [&chat.0],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .map(ServerId::new)
                .transpose()?;
            let marker = max(old, Some(up_to)).expect("marker supplied");
            tx.execute(
                "UPDATE chat SET receipt_server_id=?2 WHERE chat_id=?1",
                params![chat.0, marker,],
            )?;
            let mut stmt = tx.prepare(
                "UPDATE message SET delivery=?3
                WHERE chat_id=?1 AND server_id IS NOT NULL AND sort_id<=?2
                AND outgoing=1 AND delivery<>?3 RETURNING key",
            )?;
            let keys = stmt.query_map(
                params![
                    chat.0,
                    server_sort(&marker),
                    serde_json::to_string(&Delivery::Read)?
                ],
                |r| Ok(Key(r.get(0)?)),
            )?;
            for key in keys {
                c.messages_upserted.insert(key?);
            }
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
}
