use crate::*;

#[derive(Default)]
pub(crate) struct Reading {
    pub(crate) y: f32,
    pub(crate) latest: bool,
    pub(crate) editor_y: f32,
    pub(crate) editor_caret: i32,
    pub(crate) editor_anchor: i32,
    pub(crate) messages: Vec<MessageRow>,
}
#[derive(Default)]
pub(crate) struct View {
    pub(crate) unread: unread::Unread,
    pub(crate) asset_priority:
        Option<std::sync::Arc<line_wasm_poc::client::desktop::assets::Priority>>,
    pub(crate) preview_requested: Vec<String>,
    pub(crate) read_activity: Option<bool>,
    pub(crate) read_gate: Option<std::sync::Arc<account::ReadGate>>,
    pub(crate) attention: Option<attention::Attention>,
    pub(crate) friends: Vec<FriendRow>,
    pub(crate) friends_loaded: bool,
    pub(crate) profile_id: String,
    pub(crate) members: Vec<MemberRow>,
    pub(crate) member_request: u64,
    pub(crate) members_loaded: bool,
    pub(crate) members_pending: bool,
    pub(crate) friend_search_y: Option<f32>,
    pub(crate) rows: Vec<ChatRow>,
    pub(crate) ids: Vec<String>,
    pub(crate) current: String,
    pub(crate) messages: Vec<MessageRow>,
    pub(crate) paging: RefCell<HashMap<String, history_window::HistoryWindow>>,
    pub(crate) recent: HashMap<String, Vec<MessageRow>>,
    pub(crate) demo_previews: bool,
    pub(crate) demo_preview_loading: bool,
    pub(crate) demo_paging: bool,
    pub(crate) demo_page_failure: bool,
    pub(crate) history: HashMap<String, Reading>,
    pub(crate) generation: u64,
    pub(crate) query: String,
    pub(crate) list_retry: std::cell::Cell<account::ListRequest>,
    pub(crate) commands: Option<account::CommandSender>,
    pub(crate) demo: bool,
    pub(crate) demo_send_failure: std::sync::Arc<std::sync::atomic::AtomicU8>,
    pub(crate) saved: storage::Saved,
    pub(crate) storage: Option<storage::Storage>,
    pub(crate) deferred: Vec<account::Update>,
    pub(crate) deferred_counted: std::collections::HashSet<String>,
    pub(crate) serial: u64,
    pub(crate) retained_request_seqs: Option<Vec<u32>>,
    pub(crate) aliases: HashMap<(String, String), (String, i64)>,
    pub(crate) viewer_request: u64,
    pub(crate) notification_connection: u8,
    pub(crate) notification_through: HashMap<String, String>,
    pub(crate) background: background::Controller,
    pub(crate) notification_reads: HashMap<String, String>,
    pub(crate) file_request: u64,
    pub(crate) paths: Option<account::Paths>,
    pub(crate) pairing_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl View {
    pub(crate) fn refresh_preview(&mut self, ui: &AppWindow, chat: &str, messages: &[MessageRow]) {
        let rows = self.with_outbox(chat, messages.to_vec());
        let Some(last) = rows.last() else {
            return;
        };
        let Some(index) = self.ids.iter().position(|id| id == chat) else {
            return;
        };
        if last.sort_key < self.rows[index].activity || last.sort_key == order_key(i64::MAX) {
            return;
        }
        preview::set(&mut self.rows[index], last, ui.get_text_size());
        self.rows[index].activity = last.sort_key.clone();
        self.rows[index].time = list_time(last.sort_key.parse().unwrap_or(0)).into();
        self.filter(ui);
    }
    pub(crate) fn send_availability(&self, ui: &AppWindow) {
        let reason = if (!self.demo
            && self
                .storage
                .as_ref()
                .is_none_or(|storage| !storage.healthy()))
            || !ui.get_storage_error().is_empty()
        {
            "Sending requires working draft storage"
        } else if !ui.get_connected() {
            "Offline · you can keep editing"
        } else if ui.get_draft().len() > 10_000 {
            "Message exceeds 10,000 UTF-8 bytes · shorten it"
        } else if self.saved.outbox.len() >= 256 {
            "Outbox is full · review unresolved messages"
        } else {
            ""
        };
        ui.set_can_send(
            reason.is_empty() && !self.current.is_empty() && !ui.get_draft().trim().is_empty(),
        );
        ui.set_send_reason(reason.into());
    }
    pub(crate) fn capture(&mut self, ui: &AppWindow) {
        if !self.current.is_empty() {
            self.saved
                .drafts
                .insert(self.current.clone(), ui.get_draft().to_string());
        }
        if !self.current.is_empty() {
            self.saved.selected = self.current.clone();
            self.saved.selected_title = ui.get_chat_title().to_string();
        }
        self.saved.font_size = ui.get_text_size();
        self.saved.enter_sends = ui.get_enter_sends();
        self.capture_notification_settings(ui);
        self.background.capture(ui, &mut self.saved);
    }
    pub(crate) fn persist(&mut self, ui: &AppWindow) {
        self.capture(ui);
        self.retain_send_requests();
        self.filter(ui);
        if let Some(storage) = &self.storage {
            storage.save(self.saved.clone());
        }
        self.send_availability(ui);
    }
    pub(crate) fn remember(&mut self, ui: &AppWindow) {
        if self.current.is_empty() {
            return;
        }
        self.history.insert(
            self.current.clone(),
            Reading {
                y: ui.get_history_y(),
                latest: ui.get_following(),
                editor_y: ui.get_editor_y(),
                editor_caret: ui.get_editor_caret(),
                editor_anchor: ui.get_editor_anchor(),
                messages: self.messages.clone(),
            },
        );
        // Keep the current and seven recent conversation views bounded.
        if self.history.len() > 8 {
            let key = self
                .history
                .keys()
                .find(|key| **key != self.current)
                .cloned();
            if let Some(key) = key {
                self.history.remove(&key);
            }
        }
    }
    pub(crate) fn select(&mut self, ui: &AppWindow, index: usize) {
        let Some(id) = self.ids.get(index).cloned() else {
            return;
        };
        if ui.get_profile_open()
            && (ui.get_members_open() && self.current != id
                || !ui.get_members_open() && self.profile_id != id)
        {
            ui.set_profile_open(false);
        }
        if id == self.current {
            return;
        }
        self.capture(ui);
        self.remember(ui);
        self.current = id;
        if let Some(priority) = &self.asset_priority {
            priority.set(self.current.clone(), std::iter::empty());
        }
        if let Some(gate) = &self.read_gate {
            gate.revoke();
        }
        for entry in self.unread.chats.values_mut() {
            entry.queued = false;
        }
        self.read_activity = None;
        self.unread.enter(&self.current);
        if self.history.contains_key(&self.current) {
            self.unread.chats.get_mut(&self.current).unwrap().jump = false;
        }
        ui.global::<State>().set_unread_divider("".into());
        self.generation += 1;
        ui.set_has_chat(true);
        ui.set_selected_chat(index as i32);
        ui.set_chat_title(self.rows[index].title.clone());
        ui.set_chat_group(self.rows[index].group);
        ui.set_member_count(if self.rows[index].member_count > 0 {
            self.rows[index].member_count
        } else {
            -1
        });
        ui.set_draft(
            self.saved
                .drafts
                .get(&self.current)
                .cloned()
                .unwrap_or_default()
                .into(),
        );
        ui.set_history_error("".into());
        ui.set_new_count(0);
        let reading = self.history.get(&self.current);
        self.messages = reading.map(|r| r.messages.clone()).unwrap_or_default();
        let (y, latest, editor_y) = reading
            .map(|r| (r.y, r.latest, r.editor_y))
            .unwrap_or((0., true, 0.));
        if self.demo && self.messages.is_empty() {
            self.messages = (if self.demo_previews {
                demo::preview_examples()
            } else {
                demo::examples()
            })
            .get(index)
            .map(|chat| chat.messages.clone())
            .unwrap_or_default();
            if self.demo_previews
                && let Some(last) = self.messages.last_mut()
            {
                last.sort_key = self.rows[index].activity.clone();
                last.delivered_time = last.sort_key.clone();
                last.time = display_time(last.sort_key.parse().unwrap_or(0)).into();
            }
        }
        if self.demo_preview_loading {
            for row in &mut self.messages {
                demo::clear_picture(row);
            }
        }
        let (anchor, caret) = reading
            .map(|r| (r.editor_anchor, r.editor_caret))
            .unwrap_or((0, 0));
        self.bound_history(ui, &self.current.clone());
        self.paging_status(ui);
        self.unread_page(ui);
        self.render(ui);
        ui.invoke_show_history(y, latest);
        ui.invoke_restore_editor(anchor, caret);
        ui.set_editor_y(editor_y);
        ui.set_chat_subtitle(chat_subtitle(&self.rows[index]).into());
        if !self.demo {
            self.load(ui, false);
        }
        self.persist(ui);
    }
    pub(crate) fn load(&self, ui: &AppWindow, refresh: bool) {
        if self.current.is_empty() || self.demo {
            return;
        }
        ui.set_history_error("".into());
        ui.set_loading_history(self.request(
            ui,
            account::Command::Messages {
                id: self.current.clone(),
                generation: self.generation,
                refresh,
            },
        ));
    }
    pub(crate) fn request(&self, ui: &AppWindow, command: account::Command) -> bool {
        let list_request = match &command {
            account::Command::Chats { next: false } => Some(account::ListRequest::Refresh),
            account::Command::Chats { next: true } => Some(account::ListRequest::More),
            account::Command::ChatDetails => Some(account::ListRequest::Details),
            _ => None,
        };
        if let Some(request) = list_request {
            self.list_retry.set(request);
        }
        let feedback = match &command {
            account::Command::Chats { .. } | account::Command::ChatDetails => 1,
            account::Command::Messages { .. } => 2,
            account::Command::Friends => 3,
            _ => 0,
        };
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok())
        {
            true
        } else {
            let message = "Account is busy. Try again shortly.".into();
            match feedback {
                1 => ui.set_list_error(message),
                2 => ui.set_history_error(message),
                3 => ui.set_friends_error(message),
                _ => {} // Send and media callers place their own item-local failure.
            }
            false
        }
    }
    pub(crate) fn retry_list(&self, ui: &AppWindow) {
        ui.set_list_error("".into());
        ui.set_loading_chats(self.request(ui, self.list_retry.get().command()));
    }
    pub(crate) fn connection_failed(&self, ui: &AppWindow, reason: account::ConnectionFailure) {
        if reason == account::ConnectionFailure::Session {
            ui.set_session_unavailable(true);
        }
        ui.set_connection_failed(true);
        ui.set_connection_status(if ui.get_auth_expired() {
            "Session expired".into()
        } else if ui.get_session_unavailable() {
            "Signed out".into()
        } else {
            "Offline".into()
        });
        ui.set_retrying_connection(false);
        ui.set_connected(false);
    }
    pub(crate) fn with_outbox(&self, chat: &str, mut messages: Vec<MessageRow>) -> Vec<MessageRow> {
        for item in self.saved.outbox.iter().filter(|o| o.chat == chat) {
            if !item.server_id.is_empty() && messages.iter().any(|m| m.id == item.server_id) {
                continue;
            }
            messages.push(MessageRow {
                id: item.local_id.clone().into(),
                body: item.text.clone().into(),
                outgoing: true,
                sort_key: order_key(outgoing_timestamp(item)).into(),
                time: display_time(outgoing_timestamp(item)).into(),
                delivery: if !item.server_id.is_empty() {
                    "Sent"
                } else if item.status == "Pending…" {
                    "Sending…"
                } else if item.status.starts_with("Not submitted") {
                    "Not sent"
                } else {
                    "Unconfirmed"
                }
                .into(),
                delivery_detail: item.status.clone().into(),
                recoverable: item.server_id.is_empty() && item.status != "Pending…",
                ..Default::default()
            });
        }
        for row in &mut messages {
            if let Some((local, timestamp)) =
                self.aliases.get(&(chat.to_owned(), row.id.to_string()))
            {
                row.id = local.clone().into();
                row.sort_key = order_key(*timestamp).into();
                row.time = display_time(*timestamp).into();
                if row.delivery != "Read" {
                    row.delivery = "Sent".into();
                }
            }
        }
        messages.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        group_messages(&mut messages);
        let sender = self
            .ids
            .iter()
            .position(|id| id == chat)
            .and_then(|i| self.rows.get(i))
            .map(|row| row.title.as_str())
            .unwrap_or("contact");
        for row in &mut messages {
            crate::links::decorate_message(row, sender);
        }
        messages
    }
    pub(crate) fn render(&self, ui: &AppWindow) {
        let mut all = self.with_outbox(&self.current, self.messages.clone());
        let mut replies: HashMap<String, String> = all
            .iter()
            .map(|row| {
                (
                    row.id.to_string(),
                    format!(
                        "{}{}{}",
                        row.sender_name,
                        if row.sender_name.is_empty() { "" } else { ": " },
                        single_line(&row.body)
                    )
                    .chars()
                    .take(80)
                    .collect(),
                )
            })
            .collect();
        for ((chat, server), (local, _)) in &self.aliases {
            if chat == &self.current
                && let Some(preview) = replies.get(local).cloned()
            {
                replies.insert(server.clone(), preview);
            }
        }
        for row in &mut all {
            if !row.reply_id.is_empty() {
                row.reply_preview = replies
                    .get(row.reply_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| "Message unavailable".into())
                    .into();
            }
        }
        let ids: Vec<String> = all.iter().map(|row| row.id.to_string()).collect();
        let range = self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .range(&ids, ui.get_following());
        let evicted = self
            .paging
            .borrow()
            .get(&self.current)
            .is_some_and(|window| window.newer_evicted);
        ui.global::<State>()
            .set_history_can_follow(range.end == all.len() && !evicted);
        let mut desired = all[range].to_vec();
        ui.global::<State>()
            .set_history_retained(self.messages.len() as i32);
        ui.global::<State>().set_history_pages(
            self.paging
                .borrow()
                .get(&self.current)
                .map(|window| window.page_count())
                .unwrap_or(0) as i32,
        );
        let model = ui.get_messages();
        // Stable row identity retains text selection and active context menus
        // across confirmations, receipts, and refreshes of unchanged messages.
        if let Some(model) = model.as_any().downcast_ref::<VecModel<MessageRow>>() {
            for (i, row) in desired.iter_mut().enumerate() {
                // Link models are derived presentation data. Reuse the old model
                // for unchanged text so metadata updates preserve row identity.
                if let Some(old) = model
                    .row_data(i)
                    .filter(|old| old.id == row.id && old.body == row.body)
                {
                    row.links = old.links;
                    if row.rich_runs.row_count() == old.rich_runs.row_count()
                        && (0..row.rich_runs.row_count()).all(|i| {
                            let a = row.rich_runs.row_data(i).unwrap_or_default();
                            let b = old.rich_runs.row_data(i).unwrap_or_default();
                            a.text == b.text
                                && a.image_key == b.image_key
                                && a.emoji == b.emoji
                                && a.newline == b.newline
                        })
                    {
                        if let Some(existing) =
                            old.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>()
                        {
                            for i in 0..row.rich_runs.row_count() {
                                if let Some(incoming) = row.rich_runs.row_data(i)
                                    && incoming.has_picture
                                    && let Some(mut previous) = existing.row_data(i)
                                {
                                    previous.picture = incoming.picture;
                                    previous.has_picture = true;
                                    existing.set_row_data(i, previous);
                                }
                            }
                        }
                        row.rich_runs = old.rich_runs;
                    }
                }
                if model.row_data(i).is_some_and(|old| old.id == row.id) {
                    if model.row_data(i).as_ref() != Some(row) {
                        model.set_row_data(i, row.clone());
                    }
                } else if let Some(existing) = (i + 1..model.row_count())
                    .find(|&j| model.row_data(j).is_some_and(|old| old.id == row.id))
                {
                    // Remove obsolete leading rows while retaining the matching
                    // component and its native text selection/context menu.
                    for _ in i..existing {
                        model.remove(i);
                    }
                    model.set_row_data(i, row.clone());
                } else if i < model.row_count() {
                    model.insert(i, row.clone());
                } else {
                    model.push(row.clone());
                }
            }
            while model.row_count() > desired.len() {
                model.remove(model.row_count() - 1);
            }
            return;
        }
        ui.set_messages(ModelRc::new(VecModel::from(desired)));
    }
    pub(crate) fn update(&mut self, ui: &AppWindow, event: account::Update) {
        // Attention and unread accounting follow receipt, independently of
        // deferred visual mutations during a selection or scrollbar drag.
        self.notification_update(ui, &event);
        if let account::Update::Incoming {
            chat,
            messages,
            replay: false,
        } = &event
        {
            self.unread.incoming(
                chat,
                messages
                    .iter()
                    .filter(|m| !m.outgoing && !m.skip_badge)
                    .map(|m| m.id.clone()),
            );
            self.unread_ui(ui);
        }
        self.update_render(ui, event);
    }
    pub(crate) fn update_render(&mut self, ui: &AppWindow, event: account::Update) {
        if (ui.get_list_pressed()
            && matches!(
                event,
                account::Update::Chats { .. }
                    | account::Update::Withdrawn { .. }
                    | account::Update::Incoming { .. }
                    | account::Update::ChatMeta { .. }
                    | account::Update::Messages { .. }
                    | account::Update::Older { .. }
                    | account::Update::OlderFailed { .. }
                    | account::Update::Sent { .. }
                    | account::Update::SendFailed { .. }
                    | account::Update::SendRejected { .. }
                    | account::Update::SendUncertain { .. }
                    | account::Update::OwnEcho { .. }
            ))
            || ((ui.get_history_dragging() || ui.global::<State>().get_history_interacting())
                && matches!(
                    event,
                    account::Update::Older { .. }
                        | account::Update::OlderFailed { .. }
                        | account::Update::Messages { .. }
                        | account::Update::Withdrawn { .. }
                        | account::Update::Incoming { .. }
                        | account::Update::Receipt { .. }
                        | account::Update::Image { .. }
                        | account::Update::ImageFailed { .. }
                        | account::Update::Sent { .. }
                        | account::Update::SendFailed { .. }
                        | account::Update::SendRejected { .. }
                        | account::Update::SendUncertain { .. }
                        | account::Update::OwnEcho { .. }
                ))
        {
            self.defer_update(ui, event);
            return;
        }
        match event {
            account::Update::Older {
                chat,
                before_message,
                messages,
                reached_beginning,
            } => {
                if self
                    .paging
                    .borrow()
                    .get(&chat)
                    .and_then(|window| window.request.as_ref())
                    .is_none_or(|request| request.cursor.message != before_message)
                {
                    return;
                }
                self.older_arrived(
                    ui,
                    &chat,
                    messages.into_iter().map(message_row).collect(),
                    reached_beginning,
                );
            }
            account::Update::OlderFailed {
                chat,
                before_message,
            } => {
                if let Some(window) = self.paging.borrow_mut().get_mut(&chat) {
                    window.failed(&before_message);
                }
                if chat == self.current {
                    self.paging_status(ui);
                }
            }
            account::Update::Withdrawn { chat, message_id } => {
                let withdraw = |rows: &mut Vec<MessageRow>| {
                    if let Some(row) = rows.iter_mut().find(|row| row.id == message_id) {
                        row.kind = MessageKind::Withdrawn;
                        row.body = "Message withdrawn".into();
                        row.image_key = "".into();
                        row.has_picture = false;
                        row.rich_runs = ModelRc::default();
                        row.reply_id = "".into();
                        row.detail = "".into();
                    }
                };
                if chat == self.current {
                    withdraw(&mut self.messages);
                    self.render(ui);
                }
                if let Some(history) = self.history.get_mut(&chat) {
                    withdraw(&mut history.messages);
                }
            }
            account::Update::Sync(state) => {
                use line_wasm_poc::client::desktop::sync::SyncState;
                match state {
                    SyncState::Connected => {
                        ui.set_connected(true);
                        ui.set_connection_failed(false);
                        ui.set_auth_expired(false);
                        ui.set_session_unavailable(false);
                        ui.set_retrying_connection(false);
                        ui.set_connection_status("".into());
                    }
                    SyncState::Reconnecting { .. } => {
                        ui.set_connected(false);
                        ui.set_retrying_connection(true);
                        ui.set_connection_failed(
                            ui.get_auth_expired() || ui.get_session_unavailable(),
                        );
                        ui.set_connection_status("Connecting…".into());
                    }
                    SyncState::Offline => {
                        self.connection_failed(ui, account::ConnectionFailure::Network)
                    }
                    SyncState::AuthExpired => {
                        ui.set_auth_expired(true);
                        self.connection_failed(ui, account::ConnectionFailure::Session);
                    }
                }
                self.send_availability(ui);
            }
            account::Update::Incoming {
                chat,
                messages,
                replay,
            } => {
                let mut rows = if chat == self.current {
                    self.messages.clone()
                } else {
                    self.history
                        .get(&chat)
                        .map(|h| h.messages.clone())
                        .unwrap_or_default()
                };
                let mut arrivals = 0;
                for message in messages {
                    let mut row = message_row(message);
                    if let Some(existing) = rows.iter_mut().find(|old| old.id == row.id) {
                        if existing.image_key == row.image_key {
                            row.picture = existing.picture.clone();
                            row.has_picture = existing.has_picture;
                            row.image_failed = existing.image_failed;
                            row.image_failure = existing.image_failure.clone();
                            row.image_error = existing.image_error.clone();
                            row.image_retryable = existing.image_retryable;
                            row.file_failed = existing.file_failed;
                            row.file_failure = existing.file_failure.clone();
                            row.file_error = existing.file_error.clone();
                            row.file_retryable = existing.file_retryable;
                        }
                        if existing.sender_key == row.sender_key {
                            row.sender_avatar = existing.sender_avatar.clone();
                            row.has_sender_avatar = existing.has_sender_avatar;
                        }
                        *existing = row;
                    } else {
                        if !row.outgoing && !self.deferred_counted.remove(row.id.as_str()) {
                            arrivals += 1;
                        }
                        rows.push(row);
                    }
                }
                rows.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then_with(|| a.id.cmp(&b.id)));
                for item in self
                    .saved
                    .outbox
                    .iter()
                    .filter(|o| o.chat == chat && !o.server_id.is_empty())
                {
                    if rows.iter().any(|m| m.id == item.server_id) {
                        self.aliases.insert(
                            (chat.clone(), item.server_id.clone()),
                            (item.local_id.clone(), outgoing_timestamp(item)),
                        );
                    }
                }
                self.saved.outbox.retain(|o| {
                    o.chat != chat
                        || o.server_id.is_empty()
                        || !rows.iter().any(|m| m.id == o.server_id)
                });
                self.unread.page(&chat, &rows);
                self.unread_ui(ui);
                self.refresh_preview(ui, &chat, &rows);
                if chat == self.current {
                    if !replay && !ui.get_following() {
                        ui.set_new_count(ui.get_new_count() + arrivals);
                    }
                    self.messages = rows;
                    self.keep_recent(&chat, true);
                    self.bound_history(ui, &chat);
                    self.render(ui);
                    self.persist(ui);
                } else {
                    self.history
                        .entry(chat.clone())
                        .or_insert_with(|| Reading {
                            latest: true,
                            ..Default::default()
                        })
                        .messages = rows;
                    self.keep_recent(&chat, false);
                    self.bound_history(ui, &chat);
                }
            }
            account::Update::ChatMeta {
                chat,
                title,
                message,
                unread,
                muted,
            } => {
                self.unread.server(&chat, unread, None);
                if let Some(index) = self.ids.iter().position(|id| id == &chat) {
                    self.rows[index].muted = muted.unwrap_or(self.rows[index].muted);
                    if !title.is_empty() {
                        self.rows[index].title = single_line(&title).into();
                    }
                    if let Some(muted) = muted {
                        self.rows[index].muted = muted;
                    }
                    let message = message_row(*message);
                    preview::set(&mut self.rows[index], &message, ui.get_text_size());
                    self.rows[index].activity = message.sort_key.clone();
                    self.rows[index].time = list_time(message.sort_key.parse().unwrap_or(0)).into();
                    self.filter(ui);
                }
            }
            // A bare endpoint cannot establish the reader's exact range. S8
            // consumes ReadRanges before exposing Read or a group reader count.
            account::Update::Receipt { .. } => {}
            account::Update::ReadElsewhere {
                chat,
                up_to_message,
            } => {
                self.unread.elsewhere(&chat, &up_to_message);
                if let Some(attention) = &self.attention {
                    attention.withdraw(&chat);
                }
                self.unread_ui(ui);
                self.persist(ui);
            }
            account::Update::ReadMarked {
                chat,
                up_to_message,
            } => {
                if let Some(e) = self.unread.chats.get_mut(&chat)
                    && (up_to_message.len(), up_to_message.as_str())
                        >= (e.last_read.len(), e.last_read.as_str())
                {
                    e.pending = false;
                }
            }
            account::Update::ReadFailed { .. } => {}
            account::Update::ReadRanges(ranges) => {
                self.apply_read_ranges(ui, &ranges);
            }
            account::Update::OwnEcho {
                chat,
                request_seq,
                server_id,
            } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|item| item.chat == chat && item.request_seq == Some(request_seq))
                {
                    item.server_id = server_id;
                    item.status = "Sent".into();
                    self.render(ui);
                    self.persist(ui);
                }
            }
            account::Update::SendRejected {
                id,
                local_id,
                reason,
            } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|o| o.chat == id && o.local_id == local_id && o.server_id.is_empty())
                {
                    item.status = format!("Not submitted · {reason}");
                }
                self.render(ui);
                self.persist(ui);
            }
            account::Update::SendUncertain { id, local_id } => {
                self.update(ui, account::Update::SendFailed { id, local_id });
            }
            account::Update::Members {
                chat,
                request,
                members,
                contacts_failed,
            } => {
                if chat == self.current && request == self.member_request && ui.get_members_open() {
                    self.members_pending = false;
                    ui.set_members_loading(false);
                    ui.set_members_failed(members.is_none() || contacts_failed);
                    if let Some(members) = members {
                        self.members_loaded = true;
                        self.members = members
                            .into_iter()
                            .map(|member| {
                                let contact = member.contact;
                                let previous = self.members.iter().find(|m| {
                                    m.id == contact.id && m.avatar_key == contact.avatar_key
                                });
                                MemberRow {
                                    initial: member_initial(&contact.name),
                                    name: contact.name.into(),
                                    id: contact.id.into(),
                                    status: contact.status.into(),
                                    avatar_key: contact.avatar_key.into(),
                                    avatar: previous.map(|m| m.avatar.clone()).unwrap_or_default(),
                                    is_self: member.is_self,
                                }
                            })
                            .collect();
                        ui.set_member_count(self.members.len() as i32);
                        if let Some(row) = self
                            .rows
                            .iter_mut()
                            .find(|r| r.index == ui.get_selected_chat())
                        {
                            row.member_count = self.members.len() as i32;
                            ui.set_chat_subtitle(chat_subtitle(row).into());
                        }
                        self.filter_members(ui);
                    }
                }
            }
            account::Update::Friends(friends) => {
                let old = &self.friends;
                self.friends = friends
                    .into_iter()
                    .map(|friend| {
                        let avatar = old
                            .iter()
                            .find(|row| row.id == friend.id && row.avatar_key == friend.avatar_key)
                            .map(|row| row.avatar.clone())
                            .unwrap_or_default();
                        FriendRow {
                            header: false,
                            id: friend.id.into(),
                            initial: name_initial(&friend.name),
                            name: friend.name.into(),
                            status: friend.status.into(),
                            group: friend.group,
                            avatar_key: friend.avatar_key.into(),
                            avatar,
                        }
                    })
                    .collect();
                self.friends_loaded = true;
                ui.set_loading_friends(false);
                ui.set_friends_error("".into());
                if ui.get_members_open() && ui.get_member_profile() {
                    ui.set_profile_available(
                        self.members
                            .iter()
                            .any(|m| m.id == self.profile_id && !m.is_self)
                            && self.member_chat_available(&self.profile_id),
                    );
                } else if !ui.get_members_open() {
                    ui.set_profile_available(
                        self.friends
                            .iter()
                            .any(|friend| friend.id == self.profile_id),
                    );
                }
                if ui.get_profile_open()
                    && !ui.get_members_open()
                    && let Some(friend) = self
                        .friends
                        .iter()
                        .find(|friend| friend.id == self.profile_id)
                {
                    ui.set_profile_name(friend.name.clone());
                    ui.set_profile_status(friend.status.clone());
                    ui.set_profile_avatar(friend.avatar.clone());
                }
                self.filter_friends(ui);
            }
            account::Update::FriendsFailed => {
                ui.set_loading_friends(false);
                ui.set_friends_error("Couldn’t get friends and groups. Retry.".into());
            }
            account::Update::FileBytes {
                request,
                name,
                bytes,
            } => {
                if request == self.file_request {
                    let id = ui.global::<State>().get_file_save_id();
                    for row in self
                        .messages
                        .iter_mut()
                        .chain(
                            self.history
                                .values_mut()
                                .flat_map(|h| h.messages.iter_mut()),
                        )
                        .chain(self.recent.values_mut().flat_map(|rows| rows.iter_mut()))
                    {
                        if row.id == id {
                            row.file_failed = false;
                            row.file_failure = "".into();
                            row.file_error = "".into();
                            row.file_retryable = false;
                        }
                    }
                    self.render(ui);
                    crate::rich_content::save_download(ui, name, bytes, self.demo);
                }
            }
            account::Update::FileFailed { request, failure } => {
                if request == self.file_request {
                    let state = ui.global::<State>();
                    state.set_file_saving(false);
                    state.set_file_save_status("".into());
                    let id = state.get_file_save_id();
                    for row in self
                        .messages
                        .iter_mut()
                        .chain(
                            self.history
                                .values_mut()
                                .flat_map(|h| h.messages.iter_mut()),
                        )
                        .chain(self.recent.values_mut().flat_map(|rows| rows.iter_mut()))
                    {
                        if row.id == id {
                            row.file_failed = true;
                            row.file_failure = format!("{failure:?}").into();
                            row.file_error = failure.file_text().into();
                            row.file_retryable = failure.retriable();
                        }
                    }
                    self.render(ui);
                }
            }
            account::Update::Original { request, pixels } => {
                if request != self.viewer_request || !ui.get_viewer_open() {
                    return;
                }
                ui.set_viewer_image(slint::Image::from_rgba8(
                    slint::SharedPixelBuffer::clone_from_slice(
                        &pixels.rgba,
                        pixels.width,
                        pixels.height,
                    ),
                ));
                ui.set_viewer_status(
                    format!("Original · {} × {}", pixels.width, pixels.height).into(),
                );
                ui.set_viewer_loading(false);
                ui.set_viewer_error("".into());
            }
            account::Update::OriginalFailed { request, failure } => {
                if request != self.viewer_request || !ui.get_viewer_open() {
                    return;
                }
                ui.set_viewer_loading(false);
                ui.set_viewer_error(failure.image_text().into());
                ui.global::<State>()
                    .set_viewer_retryable(failure.retriable());
                ui.set_viewer_status("Preview only".into());
            }
            account::Update::ConnectionFailed(reason) => {
                self.connection_failed(ui, reason);
                ui.set_loading_chats(false);
                ui.set_loading_history(false);
                self.send_availability(ui);
            }
            account::Update::SignIn(event) => {
                if ui.get_auth_cancelling() {
                    return;
                }
                use line_wasm_poc::client::pairing::Event;
                match event {
                    Event::Stage(stage) => {
                        let text = if stage.contains("QR ready") {
                            "Scan the QR code with LINE on your phone."
                        } else if stage.contains("PIN") {
                            "Approve the verification code on your phone."
                        } else if stage.contains("Creating") {
                            "Preparing sign-in…"
                        } else {
                            "Verifying and securing your sign-in…"
                        };
                        ui.set_auth_stage(text.into());
                    }
                    Event::Qr {
                        width,
                        height,
                        rgba,
                    } => ui.set_auth_qr(slint::Image::from_rgba8(
                        slint::SharedPixelBuffer::clone_from_slice(&rgba, width, height),
                    )),
                    Event::Scanned => ui.set_auth_qr(Default::default()),
                    Event::Pin(pin) => ui.set_auth_pin(pin.into()),
                }
            }
            account::Update::SignInFailed { cancelled } => {
                ui.set_auth_busy(false);
                ui.set_auth_cancelling(false);
                ui.set_auth_qr(Default::default());
                ui.set_auth_pin("".into());
                if cancelled {
                    ui.invoke_close_sign_in();
                } else {
                    ui.set_auth_error("Sign-in could not complete or the QR code expired. Check your connection, then try again. Your previous local session has been kept.".into());
                }
            }
            account::Update::SessionChanged => {
                // Never carry another pairing's data into the new account.
                ui.set_viewer_open(false);
                ui.set_viewer_image(Default::default());
                ui.set_settings_open(false);
                ui.set_profile_open(false);
                ui.set_friends(ModelRc::default());
                ui.set_friend_query("".into());
                ui.set_friends_y(0.);
                ui.set_loading_friends(false);
                ui.set_friends_error("".into());
                ui.set_profile_id("".into());
                ui.set_profile_name("".into());
                ui.set_profile_status("".into());
                ui.set_profile_avatar(Default::default());
                self.friends.clear();
                self.friends_loaded = false;
                self.profile_id.clear();
                self.members.clear();
                self.member_request = self.member_request.wrapping_add(1);
                self.friend_search_y = None;
                self.viewer_request = 0;
                ui.set_connected(false);
                self.storage.take();
                self.saved = Default::default();
                self.retained_request_seqs = None;
                ui.set_recovery_kind("".into());
                ui.set_recovery_id("".into());
                ui.set_recovery_focus_id("".into());
                ui.set_enter_sends(false);
                self.unread = Default::default();
                self.read_activity = None;
                if let Some(attention) = &self.attention {
                    attention.withdraw_all();
                }
                self.unread_ui(ui);
                self.current.clear();
                self.rows.clear();
                self.ids.clear();
                self.messages.clear();
                self.history.clear();
                self.aliases.clear();
                self.deferred.clear();
                self.deferred_counted.clear();
                self.paging.borrow_mut().clear();
                self.recent.clear();
                self.demo_paging = false;
                self.query.clear();
                self.generation += 1;
                ui.set_chats(ModelRc::default());
                ui.set_messages(ModelRc::default());
                ui.set_draft("".into());
                ui.invoke_restore_editor(0, 0);
                ui.set_editor_y(0.);
                ui.set_has_chat(false);
                ui.set_query("".into());
                ui.set_selected_chat(-1);
                ui.set_chat_title("Your conversations".into());
                ui.set_chat_subtitle("".into());
                ui.set_account_name("".into());
                ui.set_history_error("".into());
                ui.set_list_error("".into());
                ui.set_text_size(15);
                ui.set_storage_retryable(false);
                ui.set_storage_error("".into());
                ui.set_new_count(0);
                ui.set_auth_qr(Default::default());
                ui.set_auth_pin("".into());
                ui.set_auth_busy(false);
                ui.set_auth_cancelling(false);
                ui.invoke_account_ready();
                if let Some(paths) = self.paths.clone() {
                    self.open_storage(ui, &paths);
                }
            }
            account::Update::Connected(name) => {
                ui.set_connection_failed(false);
                ui.set_auth_expired(false);
                ui.set_session_unavailable(false);
                ui.set_connection_status("".into());
                ui.set_retrying_connection(false);
                ui.set_account_name(name.into());
                ui.set_connected(true);
                ui.set_loading_chats(true);
                self.send_availability(ui);
                if ui.get_section() == 1 {
                    self.load_friends(ui);
                }
            }
            account::Update::Chats { chats, more } => {
                // Canonical indices retain identity while the visible list is
                // sorted newest-first. Updates wait for an active click to finish.
                for chat in chats {
                    let previous = self
                        .ids
                        .iter()
                        .position(|id| id == &chat.id)
                        .map(|i| self.rows[i].preview_runs.clone())
                        .unwrap_or_default();
                    let mut preview_row = ChatRow {
                        preview_runs: previous,
                        preview: single_line(&chat.preview).into(),
                        outgoing_preview: chat.outgoing_preview,
                        ..Default::default()
                    };
                    if let Some(message) = chat.preview_message {
                        preview::set(&mut preview_row, &message_row(message), ui.get_text_size());
                    } else {
                        preview_row.preview_runs = ModelRc::default();
                    }
                    self.unread.server_through(
                        &chat.id,
                        chat.unread,
                        chat.last_read_message.as_deref(),
                        chat.last_message_id.as_deref(),
                    );
                    if let Some(index) = self.ids.iter().position(|id| id == &chat.id) {
                        self.rows[index].member_count = chat
                            .member_count
                            .and_then(|n| i32::try_from(n).ok())
                            .unwrap_or(0);
                        self.rows[index].group = chat.group;
                        self.rows[index].outgoing_preview = chat.outgoing_preview;
                        self.rows[index].muted = chat.muted.unwrap_or(false);
                        self.rows[index].title = single_line(&chat.title).into();
                        self.rows[index].preview = preview_row.preview;
                        self.rows[index].preview_runs = preview_row.preview_runs;
                        self.rows[index].activity = order_key(chat.timestamp).into();
                        self.rows[index].time = list_time(chat.timestamp).into();
                        self.rows[index].initial = name_initial(&chat.title);
                        if self.rows[index].avatar_key != chat.avatar_key {
                            self.rows[index].avatar_key = chat.avatar_key.into();
                            self.rows[index].avatar = Default::default();
                        }
                    } else {
                        self.rows.push(ChatRow {
                            index: self.rows.len() as i32,
                            group: chat.group,
                            member_count: chat
                                .member_count
                                .and_then(|n| i32::try_from(n).ok())
                                .unwrap_or(0),
                            outgoing_preview: chat.outgoing_preview,
                            muted: chat.muted.unwrap_or(false),
                            title: single_line(&chat.title).into(),
                            initial: name_initial(&chat.title),
                            preview: preview_row.preview,
                            preview_runs: preview_row.preview_runs,
                            activity: order_key(chat.timestamp).into(),
                            time: list_time(chat.timestamp).into(),
                            avatar_key: chat.avatar_key.into(),
                            ..Default::default()
                        });
                        self.ids.push(chat.id);
                    }
                }
                if let Some(row) = self.rows.iter().find(|r| r.index == ui.get_selected_chat()) {
                    ui.set_chat_group(row.group);
                    ui.set_member_count(if ui.get_members_open() && self.members_loaded {
                        self.members.len() as i32
                    } else if row.member_count > 0 {
                        row.member_count
                    } else {
                        -1
                    });
                    if ui.get_members_open() && !self.demo {
                        self.load_members(ui);
                    }
                }
                self.filter(ui);
                self.unread_ui(ui);
                ui.set_more_chats(more);
                ui.set_loading_chats(false);
                ui.set_list_error("".into());
                if self.current.is_empty()
                    && (!self.rows.is_empty() || !self.saved.selected.is_empty())
                {
                    let index = if let Some(index) =
                        self.ids.iter().position(|id| id == &self.saved.selected)
                    {
                        index
                    } else if !self.saved.selected.is_empty() {
                        // The restored chat may be outside the first list page.
                        // Its encrypted saved identity is sufficient to open it.
                        let index = self.rows.len();
                        let title = if self.saved.selected_title.is_empty() {
                            "Restored conversation"
                        } else {
                            &self.saved.selected_title
                        };
                        self.rows.push(ChatRow {
                            index: index as i32,
                            title: title.into(),
                            ..Default::default()
                        });
                        self.ids.push(self.saved.selected.clone());
                        self.filter(ui);
                        index
                    } else {
                        ui.get_chats()
                            .row_data(0)
                            .map(|row| row.index as usize)
                            .unwrap_or(0)
                    };
                    self.select(ui, index);
                } else if let Some(index) = self.ids.iter().position(|id| id == &self.current) {
                    ui.set_selected_chat(index as i32);
                    ui.set_chat_title(self.rows[index].title.clone());
                }
            }
            account::Update::ChatsFailed(request) => {
                self.list_retry.set(request);
                ui.set_loading_chats(false);
                ui.set_list_error("Couldn’t load conversations. Retry.".into());
            }
            account::Update::Messages {
                id,
                generation,
                messages,
            } => {
                if id == self.current && generation != self.generation {
                    return;
                }
                if self
                    .paging
                    .borrow()
                    .get(&id)
                    .and_then(|w| w.request.as_ref())
                    .is_some_and(|request| request.cursor.message == "recent-range")
                {
                    self.older_arrived(
                        ui,
                        &id,
                        messages.into_iter().map(message_row).collect(),
                        false,
                    );
                    ui.set_loading_history(false);
                    return;
                }
                let mut messages = messages;
                messages.sort_by(|a, b| {
                    a.timestamp
                        .cmp(&b.timestamp)
                        .then_with(|| a.id.len().cmp(&b.id.len()))
                        .then_with(|| a.id.cmp(&b.id))
                });
                let mut rows: Vec<MessageRow> = messages.into_iter().map(message_row).collect();
                let old = if id == self.current {
                    Some(&self.messages)
                } else {
                    self.history.get(&id).map(|r| &r.messages)
                };
                if let Some(old) = old {
                    for row in &mut rows {
                        if let Some(previous) = old.iter().find(|m| m.id == row.id) {
                            if row.sender_key == previous.sender_key {
                                row.sender_avatar = previous.sender_avatar.clone();
                                row.has_sender_avatar = previous.has_sender_avatar;
                            }
                            if row.image_key == previous.image_key {
                                row.picture = previous.picture.clone();
                                row.has_picture = previous.has_picture;
                                row.image_failed = previous.image_failed;
                                row.image_failure = previous.image_failure.clone();
                                row.image_error = previous.image_error.clone();
                                row.image_retryable = previous.image_retryable;
                                row.file_failed = previous.file_failed;
                                row.file_failure = previous.file_failure.clone();
                                row.file_error = previous.file_error.clone();
                                row.file_retryable = previous.file_retryable;
                            }
                            if row.reactions == previous.reactions {
                                row.reactions = previous.reactions.clone();
                            }
                        }
                    }
                }
                for item in self
                    .saved
                    .outbox
                    .iter()
                    .filter(|o| o.chat == id && !o.server_id.is_empty())
                {
                    if rows.iter().any(|m| m.id == item.server_id) {
                        self.aliases.insert(
                            (id.clone(), item.server_id.clone()),
                            (item.local_id.clone(), outgoing_timestamp(item)),
                        );
                    }
                }
                self.aliases.retain(|(chat, server), _| {
                    chat != &id || rows.iter().any(|m| m.id == server.as_str())
                });
                self.saved.outbox.retain(|o| {
                    o.chat != id
                        || o.server_id.is_empty()
                        || !rows.iter().any(|m| m.id == o.server_id)
                });
                self.refresh_preview(ui, &id, &rows);
                if id != self.current || generation != self.generation {
                    if let Some(history) = self.history.get_mut(&id) {
                        history.messages = rows;
                    }
                    return;
                }
                if !ui.get_following() && !self.messages.is_empty() {
                    let new = rows
                        .iter()
                        .filter(|m| !m.outgoing && !self.messages.iter().any(|old| old.id == m.id))
                        .count();
                    ui.set_new_count(ui.get_new_count() + new as i32);
                }
                self.messages = rows;
                self.keep_recent(&id, true);
                self.bound_history(ui, &id);
                self.record_recent_page(&id);
                ui.set_loading_history(false);
                ui.set_history_error("".into());
                self.unread_page(ui);
                self.render(ui);
                self.persist(ui);
            }
            account::Update::Image { key, pixels } => {
                if self.demo_previews && self.demo_preview_loading {
                    return;
                }
                let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &pixels.rgba,
                    pixels.width,
                    pixels.height,
                ));
                for member in &mut self.members {
                    if member.avatar_key == key {
                        member.avatar = image.clone();
                        if member.id == self.profile_id && ui.get_member_profile() {
                            ui.set_profile_avatar(image.clone());
                        }
                    }
                }
                if ui.get_members_open() {
                    self.filter_members(ui);
                }
                for friend in &mut self.friends {
                    if friend.avatar_key == key {
                        friend.avatar = image.clone();
                        if friend.id == self.profile_id {
                            ui.set_profile_avatar(image.clone());
                        }
                    }
                }
                let friends = ui.get_friends();
                if let Some(rows) = friends.as_any().downcast_ref::<VecModel<FriendRow>>() {
                    for i in 0..rows.row_count() {
                        if let Some(mut row) = rows.row_data(i)
                            && row.avatar_key == key
                        {
                            row.avatar = image.clone();
                            rows.set_row_data(i, row);
                        }
                    }
                }
                self.preview_image(ui, &key, Some(&image));
                for row in &mut self.rows {
                    if row.avatar_key == key {
                        row.avatar = image.clone();
                    }
                }
                let chats = ui.get_chats();
                if let Some(rows) = chats.as_any().downcast_ref::<VecModel<ChatRow>>() {
                    for i in 0..rows.row_count() {
                        if let Some(mut row) = rows.row_data(i) {
                            if row.avatar_key == key {
                                row.avatar = image.clone();
                            }
                            if row.avatar_key == key {
                                rows.set_row_data(i, row);
                            }
                        }
                    }
                }
                fn apply(rows: &mut [MessageRow], key: &str, image: &slint::Image) {
                    for row in rows {
                        if row.sender_key == key {
                            row.sender_avatar = image.clone();
                            row.has_sender_avatar = true;
                        }
                        if let Some(runs) =
                            row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>()
                        {
                            for index in 0..runs.row_count() {
                                let mut run = runs.row_data(index).unwrap();
                                if run.image_key == key {
                                    run.picture = image.clone();
                                    run.has_picture = true;
                                    runs.set_row_data(index, run);
                                }
                            }
                        }
                        if row.image_key == key {
                            row.picture = image.clone();
                            row.has_picture = true;
                            row.image_failed = false;
                            row.image_failure = "".into();
                            row.image_error = "".into();
                            row.image_retryable = false;
                        }
                        if let Some(reactions) = row
                            .reactions
                            .as_any()
                            .downcast_ref::<VecModel<ReactionRow>>()
                        {
                            for i in 0..reactions.row_count() {
                                if let Some(mut r) = reactions.row_data(i)
                                    && r.image_key == key
                                {
                                    r.picture = image.clone();
                                    r.has_picture = true;
                                    reactions.set_row_data(i, r);
                                }
                            }
                        }
                    }
                }
                apply(&mut self.messages, &key, &image);
                for history in self.history.values_mut() {
                    apply(&mut history.messages, &key, &image);
                }
                for rows in self.recent.values_mut() {
                    apply(rows, &key, &image);
                }
                self.bound_all_histories(ui);
                self.render(ui);
            }
            account::Update::ImageFailed { key, failure } => {
                self.preview_image(ui, &key, None);
                for row in self
                    .messages
                    .iter_mut()
                    .chain(
                        self.history
                            .values_mut()
                            .flat_map(|history| history.messages.iter_mut()),
                    )
                    .chain(self.recent.values_mut().flat_map(|rows| rows.iter_mut()))
                {
                    if let Some(runs) = row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>() {
                        for i in 0..runs.row_count() {
                            let mut run = runs.row_data(i).unwrap();
                            if run.image_key == key {
                                run.picture = Default::default();
                                run.has_picture = false;
                                runs.set_row_data(i, run);
                            }
                        }
                    }
                    if row.image_key == key {
                        row.has_picture = false;
                        row.image_failed = true;
                        row.image_failure = format!("{failure:?}").into();
                        row.image_error = failure.image_text().into();
                        row.image_retryable = failure.retriable();
                    }
                }
                self.render(ui);
            }
            account::Update::Sent {
                id,
                local_id,
                server_id,
            } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|o| o.local_id == local_id && o.chat == id)
                {
                    item.status = "Sent".into();
                    item.server_id = server_id;
                }
                self.render(ui);
                self.persist(ui);
            }
            account::Update::SendFailed { id, local_id } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|o| o.chat == id && o.local_id == local_id && o.server_id.is_empty())
                {
                    item.status =
                        "The message may have arrived. Check before sending again.".into();
                }
                self.render(ui);
                self.persist(ui);
            }
            account::Update::Error {
                message,
                generation,
            } => {
                if generation.is_some_and(|g| g != self.generation) {
                    return;
                }
                if generation.is_some() {
                    ui.set_loading_history(false);
                    ui.set_history_error(message.into());
                } else {
                    ui.set_list_error(message.into());
                    ui.set_loading_chats(false);
                }
                self.send_availability(ui);
            }
        }
    }
    pub(crate) fn retry_image(&mut self, ui: &AppWindow, key: &str) {
        if !self
            .messages
            .iter()
            .any(|row| row.image_key == key && row.image_retryable)
        {
            return;
        }
        if self.demo {
            let pixels = line_wasm_poc::client::desktop::assets::Pixels {
                width: 2,
                height: 2,
                rgba: vec![100; 16],
            };
            self.update(
                ui,
                account::Update::Image {
                    key: key.into(),
                    pixels: std::sync::Arc::new(pixels),
                },
            );
        } else if self.request(ui, account::Command::RetryImage { key: key.into() }) {
            for row in &mut self.messages {
                if row.image_key == key {
                    row.image_failed = false;
                    row.image_retryable = false;
                }
            }
            self.render(ui);
        }
        ui.invoke_focus_history();
    }

    pub(crate) fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let Some(row) = self
            .messages
            .iter()
            .find(|row| row.image_key == key && row.photo)
        else {
            return;
        };
        self.serial += 1;
        self.viewer_request = self.serial;
        ui.set_viewer_image(row.picture.clone());
        ui.set_viewer_status("Preview · loading original…".into());
        ui.set_viewer_error("".into());
        ui.global::<State>().set_viewer_retryable(true);
        ui.set_viewer_loading(true);
        ui.invoke_show_viewer(key.into());
        if self.demo {
            ui.set_viewer_image(demo::photo(key.contains("portrait")));
            ui.set_viewer_status(
                if key.contains("portrait") {
                    "Synthetic original · 800 × 1200"
                } else {
                    "Synthetic original · 1200 × 800"
                }
                .into(),
            );
            ui.set_viewer_loading(false);
        } else if !self.request(
            ui,
            account::Command::Original {
                chat: self.current.clone(),
                key: key.into(),
                request: self.viewer_request,
            },
        ) {
            ui.set_viewer_loading(false);
            ui.set_viewer_error("The connection is busy. Try loading the original again.".into());
        }
    }
    pub(crate) fn open_storage(&mut self, ui: &AppWindow, paths: &account::Paths) {
        let weak = ui.as_weak();
        match storage::Storage::open(paths, move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                ui.set_storage_error("Drafts could not be saved. Preserve your text before quitting, fix local storage, then Retry saving.".into());
                ui.set_can_send(false);
                ui.set_send_reason("Sending requires working draft storage".into());
            });
        }) {
            Ok((storage, mut saved)) => {
                for item in &mut saved.outbox {
                    if item.status == "Pending…" {
                        item.status = "The message may have arrived before closing. Check before sending again.".into();
                    }
                }
                ui.set_text_size(if (12..=24).contains(&saved.font_size) {
                    saved.font_size
                } else {
                    15
                });
                ui.set_enter_sends(saved.enter_sends);
                self.unread.restore(&saved.unread);
                self.saved = saved;
                self.apply_notification_settings(ui);
                self.background.apply(ui, &self.saved);
                self.storage = Some(storage);
                self.unread_ui(ui);
                ui.set_storage_retryable(true);
                ui.set_storage_error("".into());
            }
            Err(error) => {
                ui.set_storage_retryable(false);
                ui.set_storage_error(error.into());
            }
        }
    }
    pub(crate) fn reconnect(&mut self, ui: &AppWindow) {
        if !ui.get_connection_failed() || ui.get_retrying_connection() || ui.get_auth_busy() {
            return;
        }
        if self.demo {
            ui.set_retrying_connection(true);
            let weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
                if let Some(ui) = weak.upgrade()
                    && ui.get_connection_failed()
                {
                    ui.set_retrying_connection(false);
                }
            });
            return;
        }
        ui.set_retrying_connection(self.request(ui, account::Command::Reconnect));
    }
    pub(crate) fn begin_sign_in(&mut self, ui: &AppWindow) {
        if !ui.get_auth_open()
            || ui.get_auth_busy()
            || !(ui.get_auth_expired() || ui.get_session_unavailable())
        {
            return;
        }
        if self.demo {
            ui.set_auth_error(
                "Sign-in is unavailable in the offline preview. No pairing request was made."
                    .into(),
            );
            return;
        }
        self.capture(ui);
        self.retain_send_requests();
        if let Some(storage) = &self.storage
            && !storage.flush(self.saved.clone())
        {
            ui.set_auth_error(
                "Your drafts could not be saved. Preserve them before signing in again.".into(),
            );
            return;
        }
        if let Some(cancel) = &self.pairing_cancel {
            cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        ui.set_auth_error("".into());
        ui.set_auth_qr(Default::default());
        ui.set_auth_pin("".into());
        ui.set_auth_stage("Preparing sign-in…".into());
        ui.set_auth_cancelling(false);
        ui.set_auth_busy(self.request(ui, account::Command::Reauthenticate));
        if !ui.get_auth_busy() {
            ui.set_auth_error(
                "Could not start sign-in. Retry the connection and try again.".into(),
            );
        }
    }
    pub(crate) fn cancel_sign_in(&mut self, ui: &AppWindow) {
        if ui.get_auth_busy() {
            if let Some(cancel) = &self.pairing_cancel {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            ui.set_auth_cancelling(true);
            ui.set_auth_qr(Default::default());
            ui.set_auth_pin("".into());
            ui.set_auth_stage(
                "Cancelling sign-in; waiting for the current connection request to finish…".into(),
            );
        } else {
            ui.invoke_close_sign_in();
        }
    }
    pub(crate) fn submit(&mut self, ui: &AppWindow) -> bool {
        self.send_availability(ui);
        if !ui.get_can_send() {
            return false;
        }
        let Some(request_seq) = self.allocate_request_seq() else {
            ui.set_storage_error("Couldn’t prepare this message. Try again.".into());
            self.send_availability(ui);
            return false;
        };
        self.serial += 1;
        let local_id = format!(
            "local-{}-{}",
            chrono::Utc::now().timestamp_micros(),
            self.serial
        );
        let text = ui.get_draft().to_string();
        self.saved.outbox.push(storage::Outgoing {
            local_id: local_id.clone(),
            request_seq: Some(request_seq),
            timestamp_ms: now_millis(),
            chat: self.current.clone(),
            text: text.clone(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        ui.set_draft("".into());
        self.capture(ui);
        // Commit the unresolved operation before any network request, so a crash
        // cannot turn a possibly delivered message into an ordinary draft.
        if !self.demo
            && !self
                .storage
                .as_ref()
                .is_some_and(|s| s.flush(self.saved.clone()))
        {
            self.saved.outbox.pop();
            ui.set_draft(text.into());
            ui.set_storage_error("Could not save the pending message. Nothing was submitted; preserve the draft and retry after fixing local storage.".into());
            self.send_availability(ui);
            return false;
        }
        let command = account::Command::Send {
            id: self.current.clone(),
            generation: self.generation,
            text: text.clone(),
            local_id,
            request_seq,
        };
        let accepted = self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok());
        if !accepted && let Some(item) = self.saved.outbox.last_mut() {
            item.status = "Not submitted · Try again when connected.".into();
        }
        self.refresh_preview(ui, &self.current.clone(), &self.messages.clone());
        self.render(ui);
        ui.invoke_latest();
        self.persist(ui);
        accepted
    }
}
