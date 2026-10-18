use super::messages::{by_key, segments, select_rows};
use super::*;
use serde_json::Value;

fn sql_limit(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

const CHAT_SELECT: &str = "SELECT chat_id,kind,title,avatar,member_count,muted,left,last_key,
    last_activity,server_unread,unread_stale,last_read_server_id,list_rank FROM chat";
fn decode_chat(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRecord> {
    Ok(ChatInput {
        id: ChatId(r.get(0)?),
        kind: r.get(1)?,
        title: r.get(2)?,
        avatar: r.get(3)?,
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
fn chats(conn: &Connection) -> Result<Vec<ChatRecord>> {
    let mut stmt = conn.prepare(CHAT_SELECT)?;
    Ok(stmt
        .query_map([], decode_chat)?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
fn chat_record(conn: &Connection, id: &ChatId) -> Result<Option<ChatRecord>> {
    Ok(conn
        .query_row(
            &format!("{CHAT_SELECT} WHERE chat_id=?"),
            [&id.0],
            decode_chat,
        )
        .optional()?)
}

impl Store {
    pub fn newest(&self, chat: &ChatId) -> Result<Option<StoredMessage>> {
        Ok(select_rows(
            &self.conn,
            "WHERE chat_id=? ORDER BY sort_ts DESC,sort_id DESC LIMIT 1",
            [&chat.0],
        )?
        .pop())
    }
    pub fn pending(&self, chat: &ChatId) -> Result<Vec<StoredMessage>> {
        select_rows(
            &self.conn,
            "WHERE chat_id=? AND server_id IS NULL ORDER BY sort_ts,sort_id",
            [&chat.0],
        )
    }
    pub fn needs_refresh(&self, chat: &ChatId) -> Result<bool> {
        let Some(record) = chat_record(&self.conn, chat)? else {
            return Ok(false);
        };
        let confirmed = select_rows(
            &self.conn,
            "WHERE chat_id=? AND server_id IS NOT NULL ORDER BY sort_ts DESC,sort_id DESC LIMIT 1",
            [&chat.0],
        )?
        .pop();
        let newer_hint = match (&record.last_message_id, record.last_message_ms, &confirmed) {
            (Some(id), Some(ts), Some(row)) => {
                Cursor {
                    sort_ts: ts,
                    sort_id: server_sort(id)?,
                } > row.sort
            }
            (Some(id), None, Some(row)) => server_sort(id)? > row.sort.sort_id,
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
        Ok(!segments(&self.conn, chat)?
            .iter()
            .any(|s| s.lo <= newest.sort && newest.sort <= s.hi))
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
            "WHERE chat_id=? ORDER BY sort_ts,sort_id LIMIT 1",
            [&chat.0],
        )?
        .pop();
        let mut gaps = Vec::new();
        let reaches_start = selected.first().map_or_else(
            || seg.iter().any(|s| s.start),
            |first| {
                seg.iter()
                    .any(|s| s.start && s.lo <= first.sort && first.sort <= s.hi)
                    && first_stored.as_ref().is_some_and(|r| r.key == first.key)
            },
        );
        if let Some(first) = selected.first()
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
        for (index, row) in selected.iter().enumerate() {
            if row.server_id.is_none() {
                continue;
            }
            if let Some(previous) = previous_confirmed
                && !seg
                    .iter()
                    .any(|s| s.lo <= previous.sort && row.sort <= s.hi)
            {
                gaps.push(Gap {
                    before: Some(selected[index - 1].key),
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
    pub fn chat_list(&self, limit: usize, offset: usize) -> Result<Vec<ChatListItem>> {
        let mut stmt=self.conn.prepare(
            "SELECT chat_id FROM chat WHERE left=0
             ORDER BY max(coalesce(last_activity,0),coalesce((SELECT sort_ts FROM message
                 WHERE message.chat_id=chat.chat_id ORDER BY sort_ts DESC,sort_id DESC LIMIT 1),0)) DESC,
                 list_rank,chat_id LIMIT ? OFFSET ?")?;
        let ids = stmt
            .query_map(params![sql_limit(limit), sql_limit(offset),], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut items = Vec::new();
        for id in ids {
            let chat = chat_record(&self.conn, &ChatId(id))?.expect("selected chat exists");
            let preview = self.newest(&chat.id)?;
            let activity_ms = preview
                .as_ref()
                .map(|r| r.sort.sort_ts)
                .into_iter()
                .chain(chat.last_message_ms)
                .max()
                .unwrap_or(0);
            items.push(ChatListItem {
                chat,
                preview,
                activity_ms,
            });
        }
        Ok(items)
    }

    pub fn upsert_chats(&mut self, list: &[ChatInput], complete_sweep: bool) -> Result<Changes> {
        self.mutate(|tx, c| {
            for input in list {
                if let Some(id) = &input.last_message_id {
                    server_sort(id)?;
                }
                let old = chat_record(tx, &input.id)?;
                let read = numeric_max(
                    old.as_ref().and_then(|r| r.last_read_server_id.clone()),
                    input.last_read_server_id.clone(),
                )?;
                let incoming_order = (
                    input.last_message_ms,
                    input
                        .last_message_id
                        .as_ref()
                        .map(server_sort)
                        .transpose()?,
                );
                let old_order = old
                    .as_ref()
                    .map(|r| {
                        Ok::<_, StoreError>((
                            r.last_message_ms,
                            r.last_message_id.as_ref().map(server_sort).transpose()?,
                        ))
                    })
                    .transpose()?;
                let (last_id, last_time) = if old_order.is_some_and(|order| order > incoming_order)
                {
                    let old = old.as_ref().expect("old metadata exists");
                    (old.last_message_id.clone(), old.last_message_ms)
                } else {
                    (input.last_message_id.clone(), input.last_message_ms)
                };
                tx.execute(
                    "INSERT INTO chat(chat_id, kind, title, avatar, member_count, muted, left, last_key,
                    last_activity, server_unread, unread_stale, last_read_server_id, list_rank)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(chat_id) DO UPDATE
                    SET kind=excluded.kind, title=excluded.title, avatar=excluded.avatar,
                    member_count=excluded.member_count, muted=excluded.muted, left=excluded.left,
                    last_key=excluded.last_key, last_activity=excluded.last_activity,
                    server_unread=excluded.server_unread, unread_stale=excluded.unread_stale,
                    last_read_server_id=excluded.last_read_server_id, list_rank=excluded.list_rank",
                    params![
                        input.id.0,
                        input.kind,
                        input.title,
                        input.avatar,
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
                c.chats_changed.insert(input.id.clone());
                if input.left {
                    c.chats_removed.insert(input.id.clone());
                }
            }
            if complete_sweep {
                let seen: std::collections::BTreeSet<_> = list.iter().map(|i| &i.id).collect();
                for chat in chats(tx)? {
                    if !chat.left && !seen.contains(&chat.id) {
                        tx.execute("UPDATE chat SET left=1 WHERE chat_id=?", [&chat.id.0])?;
                        c.chats_changed.insert(chat.id.clone());
                        c.chats_removed.insert(chat.id);
                    }
                }
            }
            Ok(())
        })
    }
    pub fn set_unread(&mut self, chat: &ChatId, count: u32, stale: bool) -> Result<Changes> {
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            tx.execute(
                "UPDATE chat SET server_unread=?,unread_stale=? WHERE chat_id=?",
                params![count, stale, chat.0,],
            )?;
            c.chats_changed.insert(chat.clone());
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
        server_sort(&up_to)?;
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            let old = tx
                .query_row(
                    "SELECT last_read_server_id FROM chat WHERE chat_id=?",
                    [&chat.0],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .map(ServerId);
            let marker = numeric_max(old, Some(up_to))?.expect("marker supplied");
            let marker_sort = server_sort(&marker)?;
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM message WHERE chat_id=? AND outgoing=0
                 AND server_id IS NOT NULL AND substr('00000000000000000000'||server_id,-20)>?",
                params![chat.0, marker_sort], |r| r.get(0),
            )?;
            tx.execute(
                "UPDATE chat SET last_read_server_id=?,server_unread=?,unread_stale=0 WHERE chat_id=?",
                params![
                    marker.0,
                    count,
                    chat.0,
                ],
            )?;
            tx.execute(
                "INSERT INTO read_marker VALUES(?,?,'read') ON CONFLICT(chat_id, state) DO UPDATE
                SET server_id=excluded.server_id",
                params![chat.0, marker.0,],
            )?;
            c.chats_changed.insert(chat.clone());
            Ok(())
        })
    }
    pub fn save_draft(&mut self, chat: &ChatId, draft: &Draft) -> Result<Changes> {
        self.mutate(|tx, c| {
            ensure_chat(tx, chat)?;
            tx.execute(
                "INSERT INTO draft VALUES(?,?,?,?) ON CONFLICT(chat_id) DO UPDATE SET text=excluded.text,
                cursor=excluded.cursor, updated=excluded.updated",
                params![
                    chat.0,
                    draft.text,
                    draft.cursor as i64,
                    draft.updated,
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
                "SELECT text,cursor,updated FROM draft WHERE chat_id=?",
                [&chat.0],
                |r| {
                    Ok(Draft {
                        text: r.get(0)?,
                        cursor: r.get::<_, i64>(1)? as usize,
                        updated: r.get(2)?,
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
    pub fn get_pref(&self, key: &str) -> Result<Option<Value>> {
        let value = self
            .conn
            .query_row("SELECT value FROM pref WHERE key=?", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(value.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn set_pref(&mut self, key: &str, value: Value) -> Result<Changes> {
        self.mutate(|tx, _| {
            tx.execute(
                "INSERT INTO pref VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value.to_string(),],
            )?;
            Ok(())
        })
    }
    pub fn account(&self) -> Result<Account> {
        let (own_mid, profile, sync_revision, global_revision, individual_revision, opened_at) =
            self.conn.query_row(
                "SELECT own_mid,profile_json,sync_revision,global_revision,
                 individual_revision,opened_at FROM account",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, String>(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )?;
        Ok(Account {
            own_mid,
            profile_json: serde_json::from_str(&profile)?,
            sync_revision,
            global_revision,
            individual_revision,
            opened_at,
        })
    }
    pub fn set_account(&mut self, own_mid: &str, profile_json: Value) -> Result<Changes> {
        self.mutate(|tx, _| {
            tx.execute(
                "UPDATE account SET own_mid=?,profile_json=?",
                params![own_mid, profile_json.to_string(),],
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
