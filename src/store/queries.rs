use super::messages::{by_key, by_server, segments, select_rows};
use super::*;
use serde_json::Value;

fn sql_limit(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

const CHAT_SELECT: &str =
    "SELECT chat_id,is_group,title,avatar,member_count,muted,left,last_message_id,
    last_activity,server_unread,unread_stale,last_read_server_id,list_rank FROM chat";
fn decode_chat(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatInput> {
    Ok(ChatInput {
        id: ChatId(r.get(0)?),
        is_group: r.get(1)?,
        title: r.get(2)?,
        avatar: r
            .get::<_, Option<String>>(3)?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        member_count: r.get(4)?,
        muted: r.get(5)?,
        left: r.get(6)?,
        last_message_id: r.get::<_, Option<String>>(7)?.map(ServerId),
        last_message_ms: r.get(8)?,
        unread_count: r.get(9)?,
        unread_stale: r.get(10)?,
        last_read_server_id: r.get::<_, Option<String>>(11)?.map(ServerId),
        list_rank: r.get(12)?,
    })
}
fn chats(conn: &Connection) -> Result<Vec<ChatInput>> {
    let mut stmt = conn.prepare(CHAT_SELECT)?;
    Ok(stmt
        .query_map([], decode_chat)?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
fn chat_record(conn: &Connection, id: &ChatId) -> Result<Option<ChatInput>> {
    let mut chat = conn
        .query_row(
            &format!("{CHAT_SELECT} WHERE chat_id=?"),
            [&id.0],
            decode_chat,
        )
        .optional()?;
    if let Some(chat) = &mut chat {
        chat.unread_count = unread(conn, chat)?;
    }
    Ok(chat)
}

fn unread(conn: &Connection, input: &ChatInput) -> Result<u32> {
    let Some(read) = &input.last_read_server_id else {
        return Ok(input.unread_count);
    };
    let local = server_sort(read);
    let ack: Option<String> = conn.query_row(
        "SELECT read_acked_server_id FROM chat WHERE chat_id=?",
        [&input.id.0],
        |r| r.get(0),
    )?;
    let server = ack
        .map(ServerId)
        .as_ref()
        .map(server_sort)
        .unwrap_or_default();
    if local <= server {
        return Ok(input.unread_count);
    }
    let newest: Option<String> = conn.query_row("SELECT server_id FROM message WHERE chat_id=? AND server_id IS NOT NULL ORDER BY sort_id DESC LIMIT 1", [&input.id.0], |r| r.get(0)).optional()?;
    if input.last_message_id.as_ref().is_some_and(|hint| {
        let last = numeric_max(Some(hint.clone()), newest.map(ServerId)).unwrap();
        local >= server_sort(&last)
    }) {
        return Ok(0);
    }
    let covered: u32 = conn.query_row(
        "SELECT count(*) FROM (SELECT sort_id FROM message WHERE chat_id=?1 AND outgoing=0
        AND server_id IS NOT NULL ORDER BY sort_id DESC LIMIT ?2) WHERE sort_id>?3 AND sort_id<=?4",
        params![input.id.0, input.unread_count, server, local],
        |r| r.get(0),
    )?;
    Ok(input.unread_count.saturating_sub(covered))
}

impl Store {
    pub fn chat(&self, id: &ChatId) -> Result<Option<ChatInput>> {
        chat_record(&self.conn, id)
    }
    pub fn contains_server(&self, id: &ServerId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM message WHERE server_id=?)",
            [&id.0],
            |r| r.get(0),
        )?)
    }
    pub fn newest_confirmed(&self, chat: &ChatId) -> Result<Option<StoredMessage>> {
        Ok(select_rows(
            &self.conn,
            "WHERE chat_id=? AND server_id IS NOT NULL ORDER BY sort_ts DESC,sort_id DESC LIMIT 1",
            [&chat.0],
        )?
        .pop())
    }
    pub fn newest(&self, chat: &ChatId) -> Result<Option<StoredMessage>> {
        Ok(select_rows(
            &self.conn,
            "WHERE chat_id=? ORDER BY sort_ts DESC,sort_id DESC LIMIT 1",
            [&chat.0],
        )?
        .pop())
    }
    pub fn all_pending(&self) -> Result<Vec<StoredMessage>> {
        select_rows(
            &self.conn,
            "WHERE server_id IS NULL ORDER BY sort_ts,sort_id",
            [],
        )
    }
    pub fn needs_refresh(&self, chat: &ChatId) -> Result<bool> {
        head_needs_refresh(&self.conn, chat)
    }

    pub fn unread_anchor(&self, chat: &ChatId) -> Result<Option<StoredMessage>> {
        let Some(c) = self.chat(chat)? else {
            return Ok(None);
        };
        let rows = if c.unread_count == 0 {
            vec![]
        } else if let Some(id) = c.last_read_server_id {
            select_rows(
                &self.conn,
                "WHERE chat_id=?1 AND outgoing=0 AND server_id IS NOT NULL AND sort_id>?2 ORDER BY sort_ts,sort_id LIMIT 1",
                params![chat.0, server_sort(&id)],
            )?
        } else {
            select_rows(
                &self.conn,
                "WHERE chat_id=?1 AND outgoing=0 AND server_id IS NOT NULL ORDER BY sort_ts DESC,sort_id DESC LIMIT 1 OFFSET ?2",
                params![chat.0, c.unread_count - 1],
            )?
        };
        Ok(rows.into_iter().next())
    }

    pub fn message_by_server(&self, id: &ServerId) -> Result<Option<StoredMessage>> {
        by_server(&self.conn, id)
    }
    pub fn window(
        &self,
        chat: &ChatId,
        anchor: Anchor,
        before: usize,
        after: usize,
    ) -> Result<Window> {
        let seg = segments(&self.conn, chat)?;
        let center = match anchor {
            Anchor::Latest => self.newest(chat)?,
            Anchor::Unread => self.unread_anchor(chat)?.or(self.newest(chat)?),
            Anchor::Key(key) => {
                let row = by_key(&self.conn, key)?;
                if &row.chat_id != chat {
                    return Err(StoreError::Invalid("window anchor not in chat".into()));
                }
                Some(row)
            }
        };
        let mut selected = Vec::new();
        if let Some(center) = center {
            selected = select_rows(
                &self.conn,
                "WHERE chat_id=?1 AND (sort_ts,sort_id)<(?2,?3) ORDER BY sort_ts DESC,sort_id DESC LIMIT ?4",
                params![
                    chat.0,
                    center.sort.sort_ts,
                    center.sort.sort_id,
                    sql_limit(before),
                ],
            )?;
            selected.reverse();
            let later = select_rows(
                &self.conn,
                "WHERE chat_id=?1 AND (sort_ts,sort_id)>(?2,?3) ORDER BY sort_ts,sort_id LIMIT ?4",
                params![
                    chat.0,
                    center.sort.sort_ts,
                    center.sort.sort_id,
                    sql_limit(after),
                ],
            )?;
            selected.push(center);
            selected.extend(later);
        }
        let first_stored = select_rows(
            &self.conn,
            "WHERE chat_id=? AND server_id IS NOT NULL ORDER BY sort_ts,sort_id LIMIT 1",
            [&chat.0],
        )?
        .pop();
        let mut gaps = Vec::new();
        let first_confirmed = selected.iter().find(|m| m.server_id.is_some());
        let reaches_start = first_confirmed.map_or_else(
            || first_stored.is_none() && seg.iter().any(|s| s.start),
            |first| {
                seg.iter()
                    .any(|s| s.start && s.lo <= first.sort && first.sort <= s.hi)
                    && first_stored.as_ref().is_some_and(|r| r.key == first.key)
            },
        );
        if let Some(first) = first_confirmed
            && !reaches_start
            && !seg
                .iter()
                .any(|s| s.start && s.lo <= first.sort && first.sort <= s.hi)
        {
            gaps.push(Gap {
                before: None,
                after: first.key,
            });
        }
        let mut previous_confirmed: Option<&StoredMessage> = None;
        for row in &selected {
            if row.server_id.is_none() {
                continue;
            }
            if let Some(previous) = previous_confirmed
                && !seg
                    .iter()
                    .any(|s| s.lo <= previous.sort && row.sort <= s.hi)
            {
                gaps.push(Gap {
                    before: Some(previous.key),
                    after: row.key,
                });
            }
            previous_confirmed = Some(row);
        }
        let newest = self.newest(chat)?;
        let reaches_end = selected
            .last()
            .is_none_or(|last| newest.as_ref().is_some_and(|r| r.key == last.key))
            && !self.needs_refresh(chat)?;
        Ok(Window {
            rows: selected,
            gaps,
            reaches_start,
            reaches_end,
        })
    }
    pub fn stored_page(&self, chat: &ChatId, kind: &PageKind) -> Result<bool> {
        let PageKind::Older { before } = kind else {
            return Ok(false);
        };
        Ok(segments(&self.conn, chat)?
            .iter()
            .any(|s| (s.lo < *before || s.start) && *before <= s.hi))
    }
    pub fn chat_list(&self, limit: usize, offset: usize) -> Result<Vec<ChatListItem>> {
        self.chat_rows(limit, offset, None, "")
    }
    pub fn search_chats(&self, query: &str) -> Result<Vec<ChatListItem>> {
        self.chat_rows(usize::MAX, 0, None, query)
    }
    pub fn chat_item(&self, id: &ChatId) -> Result<Option<ChatListItem>> {
        Ok(self.chat_rows(1, 0, Some(id), "")?.pop())
    }
    fn chat_rows(
        &self,
        limit: usize,
        offset: usize,
        id: Option<&ChatId>,
        query: &str,
    ) -> Result<Vec<ChatListItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.chat_id,c.is_group,c.title,c.avatar,c.member_count,c.muted,c.left,c.last_message_id,
             c.last_activity,c.server_unread,c.unread_stale,c.last_read_server_id,c.list_rank,
             m.key,m.chat_id,m.server_id,m.local_id,m.request_seq,m.sort_ts,m.sort_id,
             m.sender_id,m.outgoing,m.body_json,m.reactions_json,m.reactions_rev,
             m.delivery,m.created_at_local,m.body_reactions_rev,m.mentioned,m.skip_badge,m.reply_json,
             d.text,d.cursor,d.anchor,d.updated,
             (SELECT count(*) FROM message f WHERE f.chat_id=c.chat_id AND f.server_id IS NULL
              AND json_type(f.delivery,'$.Rejected') IS NOT NULL),
             max(coalesce(c.last_activity,0),coalesce(m.sort_ts,0)) AS activity,d.reply_json
             FROM chat c
             LEFT JOIN message m ON m.key=(SELECT key FROM message WHERE chat_id=c.chat_id
                 ORDER BY sort_ts DESC,sort_id DESC LIMIT 1)
             LEFT JOIN draft d ON d.chat_id=c.chat_id
             WHERE c.removed=0 AND (?3 IS NULL OR c.chat_id=?3)
             AND (?4='' OR c.title LIKE ?5 OR json_extract(m.body_json,'$.Text.text') LIKE ?5)
             ORDER BY activity DESC,c.list_rank,c.chat_id LIMIT ?1 OFFSET ?2",
        )?;
        let mut rows = stmt
            .query_map(
                params![
                    sql_limit(limit),
                    sql_limit(offset),
                    id.map(|i| &i.0),
                    query,
                    format!("%{query}%")
                ],
                |r| {
                    Ok(ChatListItem {
                        chat: decode_chat(r)?,
                        preview: if r.get::<_, Option<i64>>(13)?.is_some() {
                            Some(super::messages::decode_at(r, 13)?)
                        } else {
                            None
                        },
                        draft: r
                            .get::<_, Option<String>>(31)?
                            .map(|text| -> rusqlite::Result<Draft> {
                                Ok(Draft {
                                    reply: super::messages::parse(r, 37)?,
                                    text,
                                    cursor: r.get(32)?,
                                    anchor: r.get(33)?,
                                    updated: r.get(34)?,
                                })
                            })
                            .transpose()?,
                        failed_count: r.get(35)?,
                        activity_ms: r.get(36)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for row in &mut rows {
            row.chat.unread_count = unread(&self.conn, &row.chat)?;
        }
        Ok(rows)
    }

    pub fn upsert_chats(&mut self, list: &[ChatInput], complete_sweep: bool) -> Result<Changes> {
        self.mutate(|tx, c| {
            for input in list {
                let old = chat_record(tx, &input.id)?;
                let read = numeric_max(
                    old.as_ref().and_then(|r| r.last_read_server_id.clone()),
                    input.last_read_server_id.clone(),
                );

                let incoming_order = (input.last_message_ms, input.last_message_id.as_ref().map(server_sort));
                let old_order = old.as_ref().map(|r| (r.last_message_ms, r.last_message_id.as_ref().map(server_sort)));
                let (last_id, last_time) = if old_order.is_some_and(|order| order > incoming_order)
                {
                    let old = old.as_ref().expect("old metadata exists");
                    (old.last_message_id.clone(), old.last_message_ms)
                } else {
                    (input.last_message_id.clone(), input.last_message_ms)
                };
                tx.execute(
                    "INSERT INTO chat(chat_id, is_group, title, avatar, member_count, muted, left, last_message_id,
                    last_activity, server_unread, unread_stale, last_read_server_id, list_rank)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(chat_id) DO UPDATE
                    SET removed=0, is_group=excluded.is_group, title=excluded.title, avatar=excluded.avatar,
                    member_count=excluded.member_count, muted=excluded.muted, left=excluded.left,
                    last_message_id=excluded.last_message_id, last_activity=excluded.last_activity,
                    server_unread=excluded.server_unread, unread_stale=excluded.unread_stale,
                    last_read_server_id=excluded.last_read_server_id, list_rank=excluded.list_rank",
                    params![
                        input.id.0,
                        input.is_group,
                        input.title,
                        input.avatar.as_ref().map(serde_json::to_string).transpose()?,
                        input.member_count,
                        input.muted,
                        input.left,
                        last_id.map(|id| id.0),
                        last_time,
                        input.unread_count,
                        input.unread_stale,
                        read.map(|id| id.0),
                        input.list_rank,
                    ],
                )?;
                if let Some(marker) = &input.last_read_server_id {
                    let old: Option<String> = tx.query_row("SELECT read_acked_server_id FROM chat WHERE chat_id=?", [&input.id.0], |r| r.get(0))?;
                    let ack = numeric_max(old.map(ServerId), Some(marker.clone())).unwrap();
                    tx.execute("UPDATE chat SET read_acked_server_id=? WHERE chat_id=?", params![ack.0,input.id.0])?;
                }
                c.chats_changed.insert(input.id.clone());
            }
            if complete_sweep {
                let seen: std::collections::BTreeSet<_> = list.iter().map(|i| &i.id).collect();
                for chat in chats(tx)? {
                    if !seen.contains(&chat.id) {
                        tx.execute("UPDATE chat SET removed=1 WHERE chat_id=?", [&chat.id.0])?;
                        c.chats_changed.insert(chat.id.clone());
                        c.chats_removed.insert(chat.id);
                    }
                }
            }
            Ok(())
        })
    }
    pub fn increment_unread(&mut self, chat: &ChatId) -> Result<Changes> {
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            tx.execute(
                "UPDATE chat SET server_unread=min(server_unread+1,4294967295) WHERE chat_id=?",
                [&chat.0],
            )?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn mark_read(&mut self, chat: &ChatId, up_to: ServerId) -> Result<Changes> {
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            let old = tx
                .query_row(
                    "SELECT last_read_server_id FROM chat WHERE chat_id=?",
                    [&chat.0],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .map(ServerId);
            let marker = numeric_max(old.clone(), Some(up_to)).expect("marker supplied");
            tx.execute(
                "UPDATE chat SET last_read_server_id=?,unread_stale=0 WHERE chat_id=?",
                params![marker.0, chat.0],
            )?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn save_draft(&mut self, chat: &ChatId, draft: &Draft) -> Result<Changes> {
        if draft.text.is_empty() && self.chat(chat)?.is_none() {
            return Ok(Changes::default());
        }
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            tx.execute(
                "INSERT INTO draft(chat_id,text,cursor,updated,anchor,reply_json) VALUES(?,?,?,?,?,?) ON CONFLICT(chat_id) DO UPDATE SET text=excluded.text,
                cursor=excluded.cursor, updated=excluded.updated, anchor=excluded.anchor,reply_json=excluded.reply_json",
                params![
                    chat.0,
                    draft.text,
                    draft.cursor as i64,
                    draft.updated,
                    draft.anchor as i64,
                    serde_json::to_string(&draft.reply)?,
                ],
            )?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn load_draft(&self, chat: &ChatId) -> Result<Option<Draft>> {
        Ok(self
            .conn
            .query_row(
                "SELECT text,cursor,updated,anchor,reply_json FROM draft WHERE chat_id=?",
                [&chat.0],
                |r| {
                    Ok(Draft {
                        reply: super::messages::parse(r, 4)?,
                        text: r.get(0)?,
                        cursor: r.get::<_, i64>(1)? as usize,
                        updated: r.get(2)?,
                        anchor: r.get::<_, i64>(3)? as usize,
                    })
                },
            )
            .optional()?)
    }
    pub fn clear_draft(&mut self, chat: &ChatId) -> Result<Changes> {
        self.mutate(|tx, c| {
            tx.execute("DELETE FROM draft WHERE chat_id=?", [&chat.0])?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn prefs(&self) -> Result<Prefs> {
        let text: String = self
            .conn
            .query_row("SELECT value FROM preferences", [], |r| r.get(0))?;
        Ok(serde_json::from_str(&text)?)
    }
    pub fn set_prefs(&mut self, prefs: &Prefs) -> Result<Changes> {
        self.mutate(|tx, _| {
            tx.execute(
                "UPDATE preferences SET value=?",
                [serde_json::to_string(prefs)?],
            )?;
            Ok(())
        })
    }
    pub fn account(&self) -> Result<Account> {
        self.conn
            .query_row(
                "SELECT own_mid,profile_json,sync_revision FROM account",
                [],
                |r| {
                    Ok(Account {
                        own_mid: r.get(0)?,
                        profile: super::messages::parse(r, 1)?,
                        sync_revision: r.get(2)?,
                    })
                },
            )
            .map_err(Into::into)
    }
    pub fn set_account(&mut self, own_mid: &str, profile: Profile) -> Result<Changes> {
        self.mutate(|tx, _| {
            tx.execute(
                "UPDATE account SET own_mid=?,profile_json=?",
                params![own_mid, serde_json::to_string(&profile)?,],
            )?;
            Ok(())
        })
    }
    pub fn sync_revision(&self) -> Result<i64> {
        Ok(self.account()?.sync_revision)
    }
    pub fn set_sync_revision(&mut self, revision: i64) -> Result<Changes> {
        self.mutate(|tx, _| {
            tx.execute(
                "UPDATE account SET sync_revision=max(sync_revision,?)",
                [revision],
            )?;
            Ok(())
        })
    }
    /// Fetch a row by its stable identity without requiring the current sort position.
    pub fn message(&self, key: Key) -> Result<StoredMessage> {
        by_key(&self.conn, key)
    }
}

const CONTACT_COLUMNS: &str = "mid,name,status,is_group,avatar,member_count,original_name,overridden_name,relation,account_type,favorite_time,status_metadata";
fn decode_contact(r: &rusqlite::Row<'_>) -> rusqlite::Result<Friend> {
    Ok(Friend {
        id: r.get(0)?,
        name: r.get(1)?,
        status: r.get(2)?,
        group: r.get(3)?,
        avatar: r
            .get::<_, Option<String>>(4)?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        member_count: r.get(5)?,
        profile: ContactProfile {
            status_metadata: super::messages::parse(r, 11)?,
            original_name: r.get(6)?,
            overridden_name: r.get(7)?,
            relation: r.get(8)?,
            account_type: r.get(9)?,
            favorite_time: r.get(10)?,
            invited: false,
        },
    })
}
fn upsert_contact(tx: &Connection, row: &Friend) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO contact({CONTACT_COLUMNS}) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(mid) DO UPDATE SET name=excluded.name,status=excluded.status,is_group=excluded.is_group,
            avatar=excluded.avatar,member_count=excluded.member_count,original_name=excluded.original_name,
            overridden_name=excluded.overridden_name,relation=excluded.relation,account_type=excluded.account_type,
            favorite_time=excluded.favorite_time,status_metadata=excluded.status_metadata"
        ),
        params![
            row.id,
            row.name,
            row.status,
            row.group,
            row.avatar.as_ref().map(serde_json::to_string).transpose()?,
            row.member_count,
            row.profile.original_name,
            row.profile.overridden_name,
            row.profile.relation,
            row.profile.account_type,
            row.profile.favorite_time,
            serde_json::to_string(&row.profile.status_metadata)?
        ],
    )?;
    Ok(())
}
impl Store {
    pub fn save_contacts(&mut self, rows: &[Friend]) -> Result<Changes> {
        self.mutate(|tx, _| {
            for row in rows {
                upsert_contact(tx, row)?;
            }
            Ok(())
        })
    }
    pub fn contacts(&self) -> Result<Vec<Friend>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONTACT_COLUMNS} FROM contact ORDER BY mid"
        ))?;
        Ok(stmt
            .query_map([], decode_contact)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn save_members(&mut self, chat: &ChatId, rows: &[Member]) -> Result<Changes> {
        self.mutate(|tx, _| {
            ensure_chat(tx, chat)?;
            tx.execute("DELETE FROM member WHERE chat_id=?", [&chat.0])?;
            for row in rows {
                let exists = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM contact WHERE mid=?)",
                    [&row.contact.id],
                    |r| r.get::<_, bool>(0),
                )?;
                if !exists {
                    if row.resolved {
                        upsert_contact(tx, &row.contact)?;
                    } else {
                        continue;
                    }
                }
                tx.execute(
                    "INSERT INTO member VALUES(?,?,?,?)",
                    params![
                        chat.0,
                        row.contact.id,
                        row.is_self,
                        row.contact.profile.invited
                    ],
                )?;
            }
            Ok(())
        })
    }
    pub fn members(&self, chat: &ChatId) -> Result<Vec<Member>> {
        let mut stmt = self
            .conn
            .prepare("SELECT mid,is_self,invited FROM member WHERE chat_id=? ORDER BY rowid")?;
        let rows = stmt
            .query_map([&chat.0], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, is_self, invited)| {
                let mut contact = self.conn.query_row(
                    &format!("SELECT {CONTACT_COLUMNS} FROM contact WHERE mid=?"),
                    [id],
                    decode_contact,
                )?;
                contact.profile.invited = invited;
                Ok(Member {
                    contact,
                    is_self,
                    resolved: true,
                })
            })
            .collect()
    }
}

impl Store {
    pub fn upsert_details(&mut self, list: &[ChatInput]) -> Result<Changes> {
        let mut list = list.to_vec();
        for c in &mut list {
            if let Some(old) = chat_record(&self.conn, &c.id)? {
                c.unread_count = self.conn.query_row(
                    "SELECT server_unread FROM chat WHERE chat_id=?",
                    [&c.id.0],
                    |r| r.get(0),
                )?;
                c.unread_stale = old.unread_stale;
                c.last_read_server_id = self
                    .conn
                    .query_row(
                        "SELECT read_acked_server_id FROM chat WHERE chat_id=?",
                        [&c.id.0],
                        |r| r.get::<_, Option<String>>(0),
                    )?
                    .map(ServerId);
                c.last_message_id = old.last_message_id;
                c.last_message_ms = old.last_message_ms;
            }
        }
        self.upsert_chats(&list, false)
    }
}

impl Store {
    pub fn confirm_read(&mut self, chat: &ChatId, id: &ServerId) -> Result<Changes> {
        self.mutate(|tx, _| {
            ensure_chat(tx, chat)?;
            let old: Option<String> = tx.query_row(
                "SELECT read_acked_server_id FROM chat WHERE chat_id=?",
                [&chat.0],
                |r| r.get(0),
            )?;
            let marker = numeric_max(old.map(ServerId), Some(id.clone())).unwrap();
            let mut raw = tx.query_row(
                &format!("{CHAT_SELECT} WHERE chat_id=?"),
                [&chat.0],
                decode_chat,
            )?;
            // Retain the count beyond the acknowledged marker, including partial acknowledgements.
            raw.last_read_server_id = Some(marker.clone());
            raw.unread_count = unread(tx, &raw)?;
            tx.execute(
                "UPDATE chat SET read_acked_server_id=?,server_unread=? WHERE chat_id=?",
                params![marker.0, raw.unread_count, chat.0],
            )?;
            Ok(())
        })
    }
    pub fn pending_reads(&self) -> Result<Vec<ChatId>> {
        let mut stmt = self.conn.prepare("SELECT chat_id,last_read_server_id,read_acked_server_id FROM chat WHERE last_read_server_id IS NOT NULL")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|(_, local, ack)| {
                ack.as_ref().is_none_or(|ack| {
                    server_sort(&ServerId(local.clone())) > server_sort(&ServerId(ack.clone()))
                })
            })
            .map(|(chat, _, _)| ChatId(chat))
            .collect())
    }
}

pub(super) fn head_needs_refresh(conn: &Connection, chat: &ChatId) -> Result<bool> {
    let Some(record) = chat_record(conn, chat)? else {
        return Ok(false);
    };
    let confirmed = select_rows(
        conn,
        "WHERE chat_id=? AND server_id IS NOT NULL ORDER BY sort_ts DESC,sort_id DESC LIMIT 1",
        [&chat.0],
    )?
    .pop();
    let newer_hint = match (&record.last_message_id, record.last_message_ms, &confirmed) {
        (Some(id), Some(ts), Some(row)) => {
            Cursor {
                sort_ts: ts,
                sort_id: server_sort(id),
            } > row.sort
        }
        (Some(id), None, Some(row)) => server_sort(id) > row.sort.sort_id,
        (None, Some(ts), Some(row)) => ts > row.sort.sort_ts,
        (Some(_), _, None) | (_, Some(_), None) => true,
        _ => false,
    };
    if newer_hint {
        return Ok(true);
    }
    let Some(newest) = confirmed else {
        return Ok(false);
    };
    Ok(!segments(conn, chat)?
        .iter()
        .any(|s| s.lo <= newest.sort && newest.sort <= s.hi))
}

impl Store {
    pub fn full_chat(&self, chat: &ChatId) -> Result<Value> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT wire_json FROM chat_detail WHERE chat_id=?",
                [&chat.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or(Value::Null))
    }
    pub fn save_full_chat(&mut self, chat: &ChatId, raw: Value) -> Result<Changes> {
        self.mutate(|tx,_| { tx.execute("INSERT INTO chat_detail VALUES(?,?) ON CONFLICT(chat_id) DO UPDATE SET wire_json=excluded.wire_json",params![chat.0,raw.to_string()])?; Ok(()) })
    }
}

impl Store {
    pub fn set_chat_membership(&mut self, chat: &ChatId, left: bool) -> Result<Changes> {
        self.mutate(|tx, c| {
            tx.execute(
                "UPDATE chat SET left=? WHERE chat_id=?",
                params![left, chat.0],
            )?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
}

impl Store {
    pub fn remove_chat(&mut self, chat: &ChatId) -> Result<Changes> {
        self.mutate(|tx, c| {
            tx.execute("UPDATE chat SET removed=1 WHERE chat_id=?", [&chat.0])?;
            c.chats_changed.insert(chat.clone());
            c.chats_removed.insert(chat.clone());
            Ok(())
        })
    }
}
