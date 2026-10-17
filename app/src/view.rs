use crate::*;

#[derive(Default)]
pub(crate) struct Reading {
    pub(crate) y: f32,
    pub(crate) latest: bool,
    pub(crate) editor_y: f32,
    pub(crate) messages: Vec<MessageRow>,
}
#[derive(Default)]
pub(crate) struct View {
    pub(crate) friends: Vec<FriendRow>,
    pub(crate) friends_loaded: bool,
    pub(crate) profile_id: String,
    pub(crate) friend_search_y: Option<f32>,
    pub(crate) rows: Vec<ChatRow>,
    pub(crate) ids: Vec<String>,
    pub(crate) current: String,
    pub(crate) messages: Vec<MessageRow>,
    pub(crate) history: HashMap<String, Reading>,
    pub(crate) generation: u64,
    pub(crate) query: String,
    pub(crate) commands: Option<SyncSender<account::Command>>,
    pub(crate) demo: bool,
    pub(crate) saved: storage::Saved,
    pub(crate) storage: Option<storage::Storage>,
    pub(crate) deferred: Vec<account::Update>,
    pub(crate) serial: u64,
    pub(crate) aliases: HashMap<(String, String), (String, i64)>,
    pub(crate) viewer_request: u64,
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
        self.rows[index].preview = single_line(&last.body).into();
        self.rows[index].activity = last.sort_key.clone();
        self.rows[index].time = list_time(last.sort_key.parse().unwrap_or(0)).into();
        self.filter(ui);
    }
    pub(crate) fn send_availability(&self, ui: &AppWindow) {
        let reason = if self.demo {
            "Offline preview · sending is unavailable"
        } else if self
            .storage
            .as_ref()
            .is_none_or(|storage| !storage.healthy())
            || !ui.get_storage_error().is_empty()
        {
            "Sending requires working draft storage"
        } else if !ui.get_connected() {
            "Offline · you can keep editing"
        } else if self.current.is_empty() {
            "Choose a conversation"
        } else if ui.get_draft().trim().is_empty() {
            "Write a message to enable Send"
        } else if ui.get_draft().len() > 10_000 {
            "Message exceeds 10,000 UTF-8 bytes · shorten it"
        } else if self.saved.outbox.len() >= 256 {
            "Outbox is full · review unresolved messages"
        } else {
            ""
        };
        ui.set_can_send(reason.is_empty());
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
    }
    pub(crate) fn persist(&mut self, ui: &AppWindow) {
        self.capture(ui);
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
        if id == self.current {
            return;
        }
        self.capture(ui);
        self.remember(ui);
        self.current = id;
        self.generation += 1;
        ui.set_has_chat(true);
        ui.set_selected_chat(index as i32);
        ui.set_chat_title(self.rows[index].title.clone());
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
            self.messages = demo::examples()
                .get(index)
                .map(|chat| chat.messages.clone())
                .unwrap_or_default();
        }
        self.render(ui);
        ui.invoke_show_history(y, latest);
        ui.set_editor_y(editor_y);
        ui.set_chat_subtitle("".into());
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
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok())
        {
            true
        } else {
            ui.set_list_error("Account is busy. Try again shortly.".into());
            false
        }
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
                row.delivery = "Sent".into();
            }
        }
        messages.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        group_messages(&mut messages);
        messages
    }
    pub(crate) fn render(&self, ui: &AppWindow) {
        let desired = self.with_outbox(&self.current, self.messages.clone());
        let model = ui.get_messages();
        // Stable row identity retains text selection and active context menus
        // across confirmations, receipts, and refreshes of unchanged messages.
        if let Some(model) = model.as_any().downcast_ref::<VecModel<MessageRow>>() {
            for (i, row) in desired.iter().enumerate() {
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
        if (ui.get_list_pressed()
            && matches!(
                event,
                account::Update::Chats { .. }
                    | account::Update::Incoming { .. }
                    | account::Update::ChatMeta { .. }
                    | account::Update::Messages { .. }
                    | account::Update::Sent { .. }
                    | account::Update::SendFailed { .. }
            ))
            || (ui.get_history_dragging()
                && matches!(
                    event,
                    account::Update::Messages { .. }
                        | account::Update::Incoming { .. }
                        | account::Update::Receipt { .. }
                        | account::Update::Image { .. }
                        | account::Update::ImageFailed { .. }
                        | account::Update::Sent { .. }
                        | account::Update::SendFailed { .. }
                ))
        {
            self.deferred.push(event);
            return;
        }
        match event {
            // Older requests are not issued by the UI until S10 supplies paging.
            account::Update::Older { .. } | account::Update::OlderFailed { .. } => {}
            account::Update::Sync(state) => {
                use line_wasm_poc::client::desktop::sync::SyncState;
                let (connected, retrying, message) = match state {
                    SyncState::Connected => (true, false, String::new()),
                    SyncState::Reconnecting { attempt } => {
                        (false, true, format!("Reconnecting… (attempt {attempt})"))
                    }
                    SyncState::Offline => (
                        false,
                        false,
                        "Offline. Your history and drafts are available.".into(),
                    ),
                    SyncState::AuthExpired => (
                        false,
                        false,
                        "Your sign-in has expired. Sign in again to reconnect.".into(),
                    ),
                };
                ui.set_connected(connected);
                ui.set_connection_failed(!connected);
                ui.set_retrying_connection(retrying);
                ui.set_connection_status(message.into());
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
                        }
                        if existing.sender_key == row.sender_key {
                            row.sender_avatar = existing.sender_avatar.clone();
                            row.has_sender_avatar = existing.has_sender_avatar;
                        }
                        *existing = row;
                    } else {
                        if !row.outgoing {
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
                self.refresh_preview(ui, &chat, &rows);
                if chat == self.current {
                    if !replay && !ui.get_following() {
                        ui.set_new_count(ui.get_new_count() + arrivals);
                    }
                    self.messages = rows;
                    self.render(ui);
                    self.persist(ui);
                } else {
                    self.history
                        .entry(chat)
                        .or_insert_with(|| Reading {
                            latest: true,
                            ..Default::default()
                        })
                        .messages = rows;
                }
            }
            account::Update::ChatMeta {
                chat,
                title,
                preview,
                timestamp,
                unread: _,
                muted: _,
            } => {
                if let Some(index) = self.ids.iter().position(|id| id == &chat) {
                    if !title.is_empty() {
                        self.rows[index].title = single_line(&title).into();
                    }
                    self.rows[index].preview = single_line(&preview).into();
                    self.rows[index].activity = order_key(timestamp).into();
                    self.rows[index].time = list_time(timestamp).into();
                    self.filter(ui);
                }
            }
            // A bare endpoint cannot establish the reader's exact range. S8
            // consumes ReadRanges before exposing Read or a group reader count.
            account::Update::Receipt { .. } => {}
            account::Update::ReadElsewhere { .. }
            | account::Update::ReadMarked { .. }
            | account::Update::ReadFailed { .. }
            | account::Update::ReadRanges(_) => {}
            account::Update::SendRejected {
                id,
                local_id,
                reason,
            } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|o| o.chat == id && o.local_id == local_id)
                {
                    item.status = format!("Not submitted · {reason}");
                }
                ui.set_sending(false);
                self.render(ui);
                self.persist(ui);
            }
            account::Update::SendUncertain { id, local_id } => {
                self.update(ui, account::Update::SendFailed { id, local_id });
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
                            id: friend.id.into(),
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
                ui.set_profile_available(
                    self.friends
                        .iter()
                        .any(|friend| friend.id == self.profile_id),
                );
                if ui.get_profile_open()
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
                ui.set_friends_error("Could not refresh friends and groups. Loaded entries are kept. Use Refresh to retry.".into());
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
            account::Update::OriginalFailed { request } => {
                if request != self.viewer_request || !ui.get_viewer_open() {
                    return;
                }
                ui.set_viewer_loading(false);
                ui.set_viewer_error("Could not load the original photo. Retry, or refresh this conversation if the photo is no longer available.".into());
                ui.set_viewer_status("Preview only".into());
            }
            account::Update::ConnectionFailed => {
                ui.set_connection_failed(true);
                ui.set_retrying_connection(false);
                ui.set_connected(false);
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
                    ui.set_auth_error("Sign-in could not complete or the QR code expired. Check your connection and that curl and qrencode are installed, then try again. Your previous local session has been kept.".into());
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
                self.friend_search_y = None;
                self.viewer_request = 0;
                ui.set_connected(false);
                self.storage.take();
                self.saved = Default::default();
                self.current.clear();
                self.rows.clear();
                self.ids.clear();
                self.messages.clear();
                self.history.clear();
                self.aliases.clear();
                self.deferred.clear();
                self.query.clear();
                self.generation += 1;
                ui.set_chats(ModelRc::default());
                ui.set_messages(ModelRc::default());
                ui.set_draft("".into());
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
                    if let Some(index) = self.ids.iter().position(|id| id == &chat.id) {
                        self.rows[index].title = single_line(&chat.title).into();
                        self.rows[index].preview = single_line(&chat.preview).into();
                        self.rows[index].activity = order_key(chat.timestamp).into();
                        self.rows[index].time = list_time(chat.timestamp).into();
                        self.rows[index].initial =
                            chat.title.chars().next().unwrap_or('·').to_string().into();
                        if self.rows[index].avatar_key != chat.avatar_key {
                            self.rows[index].avatar_key = chat.avatar_key.into();
                            self.rows[index].avatar = Default::default();
                        }
                    } else {
                        self.rows.push(ChatRow {
                            index: self.rows.len() as i32,
                            title: single_line(&chat.title).into(),
                            initial: chat.title.chars().next().unwrap_or('·').to_string().into(),
                            preview: single_line(&chat.preview).into(),
                            activity: order_key(chat.timestamp).into(),
                            time: list_time(chat.timestamp).into(),
                            avatar_key: chat.avatar_key.into(),
                            ..Default::default()
                        });
                        self.ids.push(chat.id);
                    }
                }
                self.filter(ui);
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
            account::Update::ChatsFailed => {
                ui.set_connection_failed(true);
                ui.set_connected(false);
                self.send_availability(ui);
                ui.set_loading_chats(false);
                ui.set_list_error("Conversations could not be loaded. Use Refresh or More conversations to retry.".into());
            }
            account::Update::Messages {
                id,
                generation,
                messages,
            } => {
                if id == self.current && generation != self.generation {
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
                ui.set_loading_history(false);
                ui.set_history_error("".into());
                self.render(ui);
                self.persist(ui);
            }
            account::Update::Image { key, pixels } => {
                let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &pixels.rgba,
                    pixels.width,
                    pixels.height,
                ));
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
                for row in &mut self.rows {
                    if row.avatar_key == key {
                        row.avatar = image.clone();
                    }
                }
                let chats = ui.get_chats();
                if let Some(rows) = chats.as_any().downcast_ref::<VecModel<ChatRow>>() {
                    for i in 0..rows.row_count() {
                        if let Some(mut row) = rows.row_data(i)
                            && row.avatar_key == key
                        {
                            row.avatar = image.clone();
                            rows.set_row_data(i, row);
                        }
                    }
                }
                fn apply(rows: &mut [MessageRow], key: &str, image: &slint::Image) {
                    for row in rows {
                        if row.sender_key == key {
                            row.sender_avatar = image.clone();
                            row.has_sender_avatar = true;
                        }
                        if row.image_key == key {
                            row.picture = image.clone();
                            row.has_picture = true;
                            row.image_failed = false;
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
                self.render(ui);
            }
            account::Update::ImageFailed { key } => {
                for row in self.messages.iter_mut().chain(
                    self.history
                        .values_mut()
                        .flat_map(|history| history.messages.iter_mut()),
                ) {
                    if row.image_key == key {
                        row.image_failed = true;
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
                    item.status = "Server confirmed · read status unavailable".into();
                    item.server_id = server_id;
                }
                ui.set_sending(false);
                self.render(ui);
                self.persist(ui);
            }
            account::Update::SendFailed { id, local_id } => {
                if let Some(item) = self
                    .saved
                    .outbox
                    .iter_mut()
                    .find(|o| o.chat == id && o.local_id == local_id)
                {
                    item.status =
                        "Confirmation unavailable · may have arrived. Check before sending again."
                            .into();
                }
                ui.set_sending(false);
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
                ui.set_connection_failed(true);
                ui.set_connected(false);
                ui.set_retrying_connection(false);
                if generation.is_some() {
                    ui.set_loading_history(false);
                    ui.set_history_error(message.into());
                } else {
                    ui.set_list_error(message.into());
                    ui.set_connected(false);
                    ui.set_loading_chats(false);
                }
                self.send_availability(ui);
            }
        }
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
                        item.status = "Confirmation unavailable after restart · may have arrived. Check before sending again.".into();
                    }
                }
                ui.set_text_size(if (12..=24).contains(&saved.font_size) {
                    saved.font_size
                } else {
                    15
                });
                self.saved = saved;
                self.storage = Some(storage);
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
        if self.demo {
            ui.set_retrying_connection(false);
            return;
        }
        if !ui.get_connection_failed() || ui.get_retrying_connection() || ui.get_auth_busy() {
            return;
        }
        ui.set_retrying_connection(self.request(ui, account::Command::Reconnect));
    }
    pub(crate) fn begin_sign_in(&mut self, ui: &AppWindow) {
        if !ui.get_auth_open() || ui.get_auth_busy() || !ui.get_connection_failed() {
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
    pub(crate) fn submit(&mut self, ui: &AppWindow) {
        self.send_availability(ui);
        if !ui.get_can_send() {
            return;
        }
        self.serial += 1;
        let local_id = format!(
            "local-{}-{}",
            chrono::Utc::now().timestamp_micros(),
            self.serial
        );
        let text = ui.get_draft().to_string();
        self.saved.outbox.push(storage::Outgoing {
            local_id: local_id.clone(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            chat: self.current.clone(),
            text: text.clone(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        ui.set_draft("".into());
        self.capture(ui);
        // Commit the unresolved operation before any network request, so a crash
        // cannot turn a possibly delivered message into an ordinary draft.
        if !self
            .storage
            .as_ref()
            .is_some_and(|s| s.flush(self.saved.clone()))
        {
            self.saved.outbox.pop();
            ui.set_draft(text.into());
            ui.set_storage_error("Could not save the pending message. Nothing was submitted; preserve the draft and retry after fixing local storage.".into());
            self.send_availability(ui);
            return;
        }
        if !self.request(
            ui,
            account::Command::Send {
                id: self.current.clone(),
                generation: self.generation,
                text: text.clone(),
                local_id,
            },
        ) && let Some(item) = self.saved.outbox.last_mut()
        {
            item.status =
                "Not submitted · connection unavailable. Append text to draft to retry.".into();
        }
        ui.set_sending(true);
        self.refresh_preview(ui, &self.current.clone(), &self.messages.clone());
        self.render(ui);
        ui.invoke_latest();
        self.persist(ui);
    }
}
