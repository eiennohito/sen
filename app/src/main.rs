use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::mpsc::SyncSender};
mod account;
mod demo;
mod snapshot;
mod storage;
slint::include_modules!();

#[derive(Default)]
struct Reading {
    y: f32,
    latest: bool,
    editor_y: f32,
    messages: Vec<MessageRow>,
}
#[derive(Default)]
struct View {
    friends: Vec<FriendRow>,
    friends_loaded: bool,
    profile_id: String,
    friend_search_y: Option<f32>,
    rows: Vec<ChatRow>,
    ids: Vec<String>,
    current: String,
    messages: Vec<MessageRow>,
    history: HashMap<String, Reading>,
    generation: u64,
    query: String,
    commands: Option<SyncSender<account::Command>>,
    demo: bool,
    saved: storage::Saved,
    storage: Option<storage::Storage>,
    deferred: Vec<account::Update>,
    serial: u64,
    aliases: HashMap<(String, String), (String, i64)>,
    viewer_request: u64,
    paths: Option<account::Paths>,
    pairing_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl View {
    fn browse(&mut self, ui: &AppWindow, section: i32) {
        ui.set_section(section);
        ui.set_show_conversation(false);
        if section == 1 && !self.friends_loaded && !ui.get_loading_friends() {
            self.load_friends(ui);
        }
    }
    fn load_friends(&self, ui: &AppWindow) {
        if self.demo {
            return;
        }
        ui.set_friends_error("".into());
        let queued = self.request(ui, account::Command::Friends);
        ui.set_loading_friends(queued);
        if !queued {
            ui.set_friends_error("Friends could not be loaded. Try Refresh shortly.".into());
        }
    }
    fn filter_friends(&mut self, ui: &AppWindow) {
        let query = ui.get_friend_query().to_lowercase();
        let y = ui.get_friends_y();
        if !query.is_empty() && self.friend_search_y.is_none() {
            self.friend_search_y = Some(y);
        }
        let old = ui.get_friends();
        let focused = old
            .row_data(ui.get_friend_result() as usize)
            .map(|row| row.id);
        let height = ui.get_friend_row_height();
        let first = (-y / height).floor().max(0.) as usize;
        let anchor = old.row_data(first).map(|row| row.id);
        let mut rows: Vec<_> = self
            .friends
            .iter()
            .filter(|row| {
                (if row.group {
                    ui.get_groups_expanded()
                } else {
                    ui.get_friends_expanded()
                }) && row.name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            a.group
                .cmp(&b.group)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        let restored = anchor
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .map(|index| -(index as f32 * height) + (y + first as f32 * height))
            .unwrap_or(0.);
        ui.set_friend_result(
            focused
                .and_then(|id| rows.iter().position(|row| row.id == id))
                .map(|i| i as i32)
                .unwrap_or(-1),
        );
        ui.set_friends(ModelRc::new(VecModel::from(rows)));
        ui.set_friends_y(if query.is_empty() {
            self.friend_search_y.take().unwrap_or(restored)
        } else {
            restored
        });
    }
    fn open_profile(&mut self, ui: &AppWindow, id: &str) {
        let Some(friend) = self.friends.iter().find(|friend| friend.id == id) else {
            return;
        };
        self.profile_id = id.into();
        ui.set_profile_id(id.into());
        ui.set_profile_name(friend.name.clone());
        ui.set_profile_status(friend.status.clone());
        ui.set_profile_group(friend.group);
        ui.set_profile_avatar(friend.avatar.clone());
        ui.set_profile_available(true);
        ui.invoke_show_profile();
    }
    fn start_friend_chat(&mut self, ui: &AppWindow) {
        let Some(friend) = self
            .friends
            .iter()
            .find(|friend| friend.id == self.profile_id)
            .cloned()
        else {
            return;
        };
        let index = if let Some(index) = self.ids.iter().position(|id| id == friend.id.as_str()) {
            index
        } else {
            let index = self.ids.len();
            self.ids.push(friend.id.to_string());
            self.rows.push(ChatRow {
                index: index as i32,
                title: friend.name.clone(),
                initial: friend.name.chars().next().unwrap_or('·').to_string().into(),
                avatar: friend.avatar,
                avatar_key: friend.avatar_key,
                ..Default::default()
            });
            self.filter(ui);
            index
        };
        ui.set_profile_open(false);
        self.select(ui, index);
        ui.set_show_conversation(true);
        ui.invoke_focus_composer();
    }
    fn filter(&self, ui: &AppWindow) {
        let y = ui.get_list_y();
        let old = ui.get_chats();
        let height = ui.get_chat_row_height();
        let first = (-y / height).floor().max(0.) as usize;
        let anchor = if y < -0.5 {
            old.row_data(first).map(|row| row.index)
        } else {
            None
        };
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| row.title.to_lowercase().contains(&self.query))
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.activity
                .cmp(&a.activity)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.index.cmp(&b.index))
        });
        let restored = anchor
            .and_then(|index| rows.iter().position(|row| row.index == index))
            .map(|index| -(index as f32 * height) + (y + first as f32 * height))
            .unwrap_or(0.);
        ui.set_chats(ModelRc::new(VecModel::from(rows)));
        ui.set_list_y(restored);
    }
    fn refresh_preview(&mut self, ui: &AppWindow, chat: &str, messages: &[MessageRow]) {
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
    fn send_availability(&self, ui: &AppWindow) {
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
    fn capture(&mut self, ui: &AppWindow) {
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
    fn persist(&mut self, ui: &AppWindow) {
        self.capture(ui);
        if let Some(storage) = &self.storage {
            storage.save(self.saved.clone());
        }
        self.send_availability(ui);
    }
    fn remember(&mut self, ui: &AppWindow) {
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
    fn select(&mut self, ui: &AppWindow, index: usize) {
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
        if self.demo {
            ui.set_chat_subtitle(
                demo::examples()
                    .get(index)
                    .map(|chat| chat.subtitle)
                    .unwrap_or("Synthetic conversation")
                    .into(),
            );
        } else {
            ui.set_chat_subtitle("Recent history · read receipts unavailable".into());
            self.load(ui, false);
        }
        self.persist(ui);
    }
    fn load(&self, ui: &AppWindow, refresh: bool) {
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
    fn request(&self, ui: &AppWindow, command: account::Command) -> bool {
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok())
        {
            true
        } else {
            ui.set_status("Account connection is busy or unavailable. Try again shortly.".into());
            false
        }
    }
    fn with_outbox(&self, chat: &str, mut messages: Vec<MessageRow>) -> Vec<MessageRow> {
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
    fn render(&self, ui: &AppWindow) {
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
    fn update(&mut self, ui: &AppWindow, event: account::Update) {
        if (ui.get_list_pressed()
            && matches!(
                event,
                account::Update::Chats { .. }
                    | account::Update::Messages { .. }
                    | account::Update::Sent { .. }
                    | account::Update::SendFailed { .. }
            ))
            || (ui.get_history_dragging()
                && matches!(
                    event,
                    account::Update::Messages { .. }
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
                if !self.demo {
                    ui.set_status("Connection failed · Retry or Sign in again".into());
                }
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
                ui.set_status("Loading conversations…".into());
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
                ui.set_status(
                    format!("{} conversations · Refresh to update", self.ids.len()).into(),
                );
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
                let mut rows: Vec<MessageRow> = messages
                    .into_iter()
                    .map(|m| {
                        let reactions = m
                            .reactions
                            .into_iter()
                            .map(|r| ReactionRow {
                                label: r.label.into(),
                                image_key: r.image_key.into(),
                                ..Default::default()
                            })
                            .collect::<Vec<_>>();
                        let time = display_time(m.timestamp);
                        MessageRow {
                            id: m.id.into(),
                            sender_id: m.sender_id.into(),
                            sender_name: m.sender_name.into(),
                            sender_key: m.sender_key.into(),
                            sticker: m.sticker,
                            photo: m.photo,
                            sort_key: order_key(m.timestamp).into(),
                            body: m.body.into(),
                            time: time.into(),
                            outgoing: m.outgoing,
                            image_key: m.image_key.into(),
                            reactions: ModelRc::new(VecModel::from(reactions)),
                            ..Default::default()
                        }
                    })
                    .collect();
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
                if !self.demo {
                    ui.set_connection_failed(true);
                    ui.set_connected(false);
                }
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
                    ui.set_status(message.into());
                    ui.set_connected(false);
                    ui.set_loading_chats(false);
                }
                self.send_availability(ui);
            }
        }
    }
    fn open_photo(&mut self, ui: &AppWindow, key: &str) {
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
    fn open_storage(&mut self, ui: &AppWindow, paths: &account::Paths) {
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
    fn reconnect(&mut self, ui: &AppWindow) {
        if self.demo {
            ui.set_retrying_connection(false);
            return;
        }
        if !ui.get_connection_failed() || ui.get_retrying_connection() || ui.get_auth_busy() {
            return;
        }
        ui.set_retrying_connection(self.request(ui, account::Command::Reconnect));
    }
    fn begin_sign_in(&mut self, ui: &AppWindow) {
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
    fn cancel_sign_in(&mut self, ui: &AppWindow) {
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
    fn submit(&mut self, ui: &AppWindow) {
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
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Group only adjacent messages with known time and identity. Presentation is
/// derived after sorting/reconciliation, so refreshes and local sends agree.
fn group_messages(messages: &mut [MessageRow]) {
    let mut previous: Option<(String, bool, i64, String)> = None;
    for row in messages {
        row.sender_initial = row
            .sender_name
            .chars()
            .next()
            .unwrap_or('?')
            .to_string()
            .into();
        let (date, clock) = row.time.rsplit_once(' ').unwrap_or(("", row.time.as_str()));
        let date = date.to_owned();
        row.clock = clock.into();
        let timestamp = row.sort_key.parse::<i64>().unwrap_or(0);
        let sender = if row.outgoing {
            "self".to_owned()
        } else if !row.sender_id.is_empty() {
            row.sender_id.to_string()
        } else if row.sender_name.is_empty() {
            "direct-contact".to_owned()
        } else {
            row.id.to_string()
        };
        row.day_label = if previous.as_ref().is_none_or(|p| p.3 != date) {
            date.clone().into()
        } else {
            "".into()
        };
        row.group_start = previous.as_ref().is_none_or(|p| {
            p.0 != sender
                || p.1 != row.outgoing
                || p.3 != date
                || p.2 <= 0
                || timestamp <= 0
                || timestamp == i64::MAX
                || timestamp.saturating_sub(p.2) >= 5 * 60 * 1000
        });
        previous = Some((sender, row.outgoing, timestamp, date));
    }
}
fn order_key(timestamp: i64) -> String {
    format!("{:020}", timestamp.max(0))
}
fn display_time(timestamp: i64) -> String {
    if timestamp <= 0 || timestamp == i64::MAX {
        return String::new();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y/%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
fn list_time(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format(
                    if time.with_timezone(&chrono::Local).date_naive()
                        == chrono::Local::now().date_naive()
                    {
                        "%H:%M"
                    } else {
                        "%m/%d"
                    },
                )
                .to_string()
        })
        .unwrap_or_default()
}
fn outgoing_timestamp(item: &storage::Outgoing) -> i64 {
    if item.timestamp_ms > 0 {
        item.timestamp_ms
    } else {
        item.local_id
            .strip_prefix("local-")
            .and_then(|id| id.split('-').next())
            .and_then(|timestamp| timestamp.parse::<i64>().ok())
            .map(|micros| micros / 1000)
            .unwrap_or(i64::MAX)
    }
}
struct Options {
    paths: account::Paths,
    demo: bool,
    demo_photos: bool,
    demo_groups: bool,
    demo_friends: bool,
    snapshot: Option<String>,
    snapshot_size: (u32, u32),
    snapshot_text_size: i32,
    check: bool,
}
fn options() -> Result<Options, Box<dyn std::error::Error>> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("artifacts/cache"))
        .join("line-desktop");
    let mut out = Options {
        paths: account::Paths {
            state: "artifacts/auth/session.json".into(),
            wasm: line_wasm_poc::DEFAULT_WASM.into(),
            cache,
        },
        demo: false,
        demo_photos: false,
        demo_groups: false,
        demo_friends: false,
        snapshot: None,
        snapshot_size: (980, 720),
        snapshot_text_size: 15,
        check: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"mcp_debug\":{},\"renderer\":\"software\"}}",
                    cfg!(feature = "ui-debug")
                );
                std::process::exit(0);
            }
            "--demo" => out.demo = true,
            "--demo-friends" => {
                out.demo = true;
                out.demo_friends = true;
            }
            "--demo-photos" => {
                out.demo = true;
                out.demo_photos = true;
            }
            "--demo-groups" => {
                out.demo = true;
                out.demo_groups = true;
            }
            "--check-live" => out.check = true,
            "--state" => out.paths.state = args.next().ok_or("missing state path")?.into(),
            "--wasm" => out.paths.wasm = args.next().ok_or("missing WASM path")?.into(),
            "--cache-dir" => out.paths.cache = args.next().ok_or("missing cache path")?.into(),
            "--snapshot-size" => {
                let size = args.next().ok_or("missing WIDTHxHEIGHT")?;
                let (width, height) = size.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                out.snapshot_size = (width.parse()?, height.parse()?);
                if !(360..=3840).contains(&out.snapshot_size.0)
                    || !(520..=2160).contains(&out.snapshot_size.1)
                {
                    return Err("snapshot size outside supported bounds".into());
                }
            }
            "--snapshot-text-size" => {
                out.snapshot_text_size = args.next().ok_or("missing text size")?.parse()?;
                if !(12..=24).contains(&out.snapshot_text_size) {
                    return Err("text size must be 12–24".into());
                }
            }
            "--snapshot" => {
                out.snapshot = Some(args.next().ok_or("missing snapshot path")?);
                out.demo = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: line-desktop [--demo | --demo-photos | --demo-groups | --demo-friends] [--state FILE] [--wasm FILE] [--cache-dir DIR]\n       line-desktop --snapshot FILE.ppm [--snapshot-size WIDTHxHEIGHT] [--snapshot-text-size 12..24]\n       line-desktop --check-live [--state FILE] [--wasm FILE] [--cache-dir DIR]\nDefault: use saved account. --demo and --snapshot are offline. --check-live reports counts only and never sends messages."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if out.check && out.demo {
        return Err("--check-live cannot be combined with offline modes".into());
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    if options.check {
        return account::check(&options.paths);
    }
    let snapshot = if options.snapshot.is_some() {
        Some(snapshot::install()?)
    } else {
        None
    };
    if snapshot.is_none() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }
    slint::set_xdg_app_id("org.lineclient.Desktop")?;
    let ui = AppWindow::new()?;
    if snapshot.is_none() {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{Ime, WindowEvent},
        };
        let weak = ui.as_weak();
        // Observe IME state without intercepting native composition or editing.
        // TextEdit does not expose its internal preedit state to Slint callers.
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
                    WindowEvent::Ime(Ime::Preedit(text, _)) => ui.set_ime_active(!text.is_empty()),
                    WindowEvent::Ime(Ime::Commit(_) | Ime::Disabled)
                    | WindowEvent::Focused(false) => ui.set_ime_active(false),
                    _ => {}
                }
            }
            EventResult::Propagate
        });
    }
    let mut view = View {
        demo: options.demo,
        ..Default::default()
    };
    if !options.demo {
        view.open_storage(&ui, &options.paths);
        view.paths = Some(options.paths.clone());
    }
    let state = Rc::new(RefCell::new(view));
    let worker = if options.demo {
        ui.set_demo(true);
        ui.set_account_name("Offline preview".into());
        ui.set_status("Messages are examples".into());
        for (index, chat) in demo::examples().into_iter().enumerate() {
            let mut view = state.borrow_mut();
            view.ids.push(index.to_string());
            view.rows.push(ChatRow {
                index: index as i32,
                title: chat.title.into(),
                preview: single_line(chat.preview).into(),
                activity: order_key(3 - index as i64).into(),
                initial: chat.initial.into(),
                avatar: demo::avatar(index),
                ..Default::default()
            });
        }
        {
            let mut view = state.borrow_mut();
            view.friends = view
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| FriendRow {
                    id: i.to_string().into(),
                    name: row.title.clone(),
                    status: "こんにちは 🌿\nSynthetic profile".into(),
                    group: i == 1,
                    avatar: row.avatar.clone(),
                    ..Default::default()
                })
                .collect();
            view.friends.push(FriendRow {
                id: "new-friend".into(),
                name: "新しい友だち 🌸".into(),
                status: "まだ会話のない友だち".into(),
                ..Default::default()
            });
            view.friends_loaded = true;
            view.filter_friends(&ui);
        }
        state.borrow().filter(&ui);
        state.borrow_mut().select(&ui, 0);
        None
    } else {
        ui.set_status("Opening saved account…".into());
        ui.set_chat_title("Your conversations".into());
        let weak = ui.as_weak();
        let events = state.clone();
        UI_STATE.with(|slot| *slot.borrow_mut() = Some(events));
        let worker = account::start(options.paths, move |event| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                UI_STATE.with(|slot| {
                    if let Some(state) = slot.borrow().as_ref() {
                        state.borrow_mut().update(&ui, event);
                    }
                })
            });
        });
        state.borrow_mut().commands = Some(worker.commands.clone());
        state.borrow_mut().pairing_cancel = Some(worker.pairing_cancel.clone());
        Some(worker)
    };
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_photo_closed(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().viewer_request = 0;
            ui.set_viewer_image(Default::default());
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_photo(move |key| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_photo(&ui, &key);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_reconnect(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().reconnect(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_start_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().begin_sign_in(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_cancel_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().cancel_sign_in(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_browse(move |section| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().browse(&ui, section);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_refresh_friends(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().load_friends(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_filter_friends(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().filter_friends(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_profile(move |id| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_profile(&ui, &id);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_start_friend_chat(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().start_friend_chat(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_select_chat(move |index| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().select(&ui, index as usize);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_search_changed(move |query| {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.query = query.to_lowercase();
            view.filter(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_draft_edited(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().persist(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().persist(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.generation += 1;
            ui.set_list_error("".into());
            ui.set_loading_chats(view.request(&ui, account::Command::Chats { next: false }));
            view.load(&ui, true);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_retry_history(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().load(&ui, true);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_retry_storage(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.capture(&ui);
            if view
                .storage
                .as_ref()
                .is_some_and(|storage| storage.flush(view.saved.clone()))
            {
                ui.set_storage_error("".into());
            }
            view.send_availability(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_more(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_loading_chats(
                view.borrow()
                    .request(&ui, account::Command::Chats { next: true }),
            );
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_send(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().submit(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_recover_message(move |id| {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            if let Some(item) = view
                .saved
                .outbox
                .iter()
                .find(|o| o.local_id == id.as_str() && o.chat == view.current)
            {
                let existing = ui.get_draft();
                ui.set_draft(
                    if existing.is_empty() {
                        item.text.clone()
                    } else {
                        format!("{existing}\n{}", item.text)
                    }
                    .into(),
                );
                view.persist(&ui);
            }
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_demo_event(move |kind| {
        if let Some(ui) = weak.upgrade() {
            demo::scenario(&view, &ui, kind);
        }
    });
    // Model changes wait until the thumb is released, without an idle timer.
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_release_deferred(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            let events = std::mem::take(&mut view.deferred);
            for event in events {
                view.update(&ui, event);
            }
        }
    });
    if options.demo_photos {
        demo::scenario(&state, &ui, 5);
    }
    if options.demo_groups {
        demo::scenario(&state, &ui, 7);
    }
    if options.demo_friends {
        state.borrow_mut().browse(&ui, 1);
    }
    state.borrow().send_availability(&ui);
    if let (Some(window), Some(path)) = (snapshot, options.snapshot) {
        ui.show()?;
        ui.set_text_size(options.snapshot_text_size);
        ui.set_show_conversation(!options.demo_friends);
        snapshot::save(&window, &path, options.snapshot_size)?;
        ui.hide()?;
    } else {
        ui.run()?;
    }
    let mut view = state.borrow_mut();
    view.capture(&ui);
    if let Some(storage) = &view.storage {
        storage.flush(view.saved.clone());
    }
    drop(worker);
    UI_STATE.with(|slot| slot.borrow_mut().take());
    Ok(())
}
thread_local! {static UI_STATE:RefCell<Option<Rc<RefCell<View>>>>=const{RefCell::new(None)};}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str, text: &str) -> account::MessageView {
        account::MessageView {
            id: id.into(),
            sender_id: String::new(),
            sender_name: String::new(),
            sender_key: String::new(),
            sticker: false,
            photo: false,
            body: text.into(),
            timestamp: 1,
            outgoing: false,
            image_key: String::new(),
            reactions: Vec::new(),
        }
    }
    #[test]
    fn grouping_respects_identity_five_minutes_and_calendar_days() {
        let row = |id: &str, sender: &str, timestamp: i64, date: &str| MessageRow {
            id: id.into(),
            sender_id: sender.into(),
            sender_name: "Same display name".into(),
            sort_key: order_key(timestamp).into(),
            time: format!("{date} 12:00").into(),
            ..Default::default()
        };
        let mut rows = vec![
            row("1", "a", 1, "2026/10/15"),
            row("2", "a", 300_000, "2026/10/15"), // 4:59.999: grouped
            row("3", "a", 600_000, "2026/10/15"), // exactly five minutes: break
            row("4", "b", 600_001, "2026/10/15"), // same name, different identity
            row("5", "b", 600_002, "2026/10/16"), // midnight: break
        ];
        group_messages(&mut rows);
        assert_eq!(
            rows.iter().map(|m| m.group_start).collect::<Vec<_>>(),
            [true, false, true, true, true]
        );
        assert_eq!(
            rows.iter()
                .map(|m| m.day_label.as_str())
                .collect::<Vec<_>>(),
            ["2026/10/15", "", "", "", "2026/10/16"]
        );
        assert!(rows.iter().all(|m| m.clock == "12:00"));
        rows[3].outgoing = true;
        rows[4].outgoing = true;
        rows[4].time = rows[3].time.clone();
        group_messages(&mut rows);
        assert!(rows[3].group_start);
        assert!(!rows[4].group_start);
        // Unknown group senders must never merge into a fictitious person.
        rows[3].outgoing = false;
        rows[4].outgoing = false;
        rows[3].sender_id = "".into();
        rows[4].sender_id = "".into();
        group_messages(&mut rows);
        assert!(rows[3].group_start && rows[4].group_start);
    }
    #[test]
    fn synthetic_navigation_refresh_and_outbox_races() {
        let _window = snapshot::install().unwrap();
        let ui = AppWindow::new().unwrap();
        let mut view = View {
            demo: true,
            ..Default::default()
        };
        for (index, chat) in demo::examples().into_iter().enumerate() {
            view.ids.push(index.to_string());
            view.rows.push(ChatRow {
                index: index as i32,
                title: chat.title.into(),
                ..Default::default()
            });
        }
        view.filter(&ui);
        view.select(&ui, 0);
        ui.set_draft("日本語 🐈\nsecond line\n".into());
        ui.set_following(false);
        ui.set_history_y(-73.);
        view.select(&ui, 1);
        assert_eq!(view.history["0"].y, -73.);
        ui.set_draft("another draft".into());
        view.select(&ui, 0);
        assert_eq!(ui.get_draft(), "日本語 🐈\nsecond line\n");
        let original_model = ui.get_messages();
        view.select(&ui, 0);
        assert_eq!(ui.get_messages(), original_model);
        view.query = "weekend".into();
        view.filter(&ui);
        assert_eq!(ui.get_chats().row_count(), 1);
        assert_eq!(view.current, "0");
        assert_eq!(ui.get_draft(), "日本語 🐈\nsecond line\n");
        let generation = view.generation;
        view.update(
            &ui,
            account::Update::Messages {
                id: "1".into(),
                generation: generation - 1,
                messages: vec![message("late", "previous chat")],
            },
        );
        assert_eq!(view.current, "0");
        assert_eq!(view.history["1"].messages[0].id, "late");
        let subtitle = ui.get_chat_subtitle();
        view.update(
            &ui,
            account::Update::Error {
                message: "old failure".into(),
                generation: Some(generation - 1),
            },
        );
        assert_eq!(ui.get_chat_subtitle(), subtitle);
        assert!(ui.get_history_error().is_empty());
        view.update(
            &ui,
            account::Update::Messages {
                id: "0".into(),
                generation,
                messages: vec![message("a", "選択できるメッセージ 🐈\nline two")],
            },
        );
        let model = ui.get_messages();
        ui.set_new_count(0);
        view.update(
            &ui,
            account::Update::Messages {
                id: "0".into(),
                generation,
                messages: vec![
                    message("a", "選択できるメッセージ 🐈\nline two"),
                    message("b", "new arrival"),
                ],
            },
        );
        assert_eq!(ui.get_messages(), model, "append rebuilt the message model");
        assert_eq!(ui.get_new_count(), 1);
        assert!(!ui.get_following());
        view.update(&ui, account::Update::ChatsFailed);
        assert_eq!(view.ids.len(), 3);
        assert_eq!(view.current, "0");
        assert_eq!(ui.get_messages(), model);
        view.saved.outbox.push(storage::Outgoing {
            local_id: "pending".into(),
            timestamp_ms: 1,
            chat: "1".into(),
            text: "submitted text".into(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        ui.set_draft("newer typing".into());
        view.update(
            &ui,
            account::Update::SendFailed {
                id: "1".into(),
                local_id: "pending".into(),
            },
        );
        assert!(view.saved.outbox[0].status.contains("may have arrived"));
        assert_eq!(ui.get_draft(), "newer typing");
        assert_eq!(ui.get_messages().row_count(), 2);
        view.select(&ui, 1);
        assert_eq!(ui.get_messages().row_count(), 2);
        let pending_model = ui.get_messages();
        let pending_time = pending_model.row_data(1).unwrap().time;
        view.update(
            &ui,
            account::Update::Sent {
                id: "1".into(),
                local_id: "pending".into(),
                server_id: "server".into(),
            },
        );
        assert!(view.saved.outbox[0].status.starts_with("Server confirmed"));
        assert_eq!(ui.get_messages().row_data(1).unwrap().delivery, "Sent");
        assert_eq!(ui.get_messages().row_data(1).unwrap().time, pending_time);
        view.update(
            &ui,
            account::Update::Messages {
                id: "1".into(),
                generation: view.generation,
                messages: vec![
                    message("late", "previous chat"),
                    message("server", "submitted text"),
                ],
            },
        );
        assert_eq!(
            ui.get_messages().row_count(),
            2,
            "confirmation duplicated a local item"
        );
        assert_eq!(
            ui.get_messages(),
            pending_model,
            "server reconciliation replaced the pending row model"
        );
        assert_eq!(ui.get_messages().row_data(1).unwrap().id, "pending");
        assert_eq!(ui.get_messages().row_data(1).unwrap().time, pending_time);
        assert_eq!(ui.get_messages().row_data(1).unwrap().delivery, "Sent");
        assert!(view.saved.outbox.is_empty());
        assert_eq!(ui.get_draft(), "another draft");
        // An asset update changes only the affected row and reaction, keeping
        // both models alive while text selection/context menus are active.
        let reactions = Rc::new(VecModel::from(vec![ReactionRow {
            image_key: "reaction".into(),
            ..Default::default()
        }]));
        view.messages = vec![MessageRow {
            id: "photo".into(),
            image_key: "photo".into(),
            sender_key: "sender-avatar".into(),
            reactions: ModelRc::from(reactions.clone()),
            ..Default::default()
        }];
        view.render(&ui);
        let model = ui.get_messages();
        for key in ["photo", "reaction", "sender-avatar"] {
            view.update(
                &ui,
                account::Update::Image {
                    key: key.into(),
                    pixels: line_wasm_poc::client::desktop::assets::Pixels {
                        width: 1,
                        height: 1,
                        rgba: vec![255; 4],
                    }
                    .into(),
                },
            );
        }
        assert_eq!(ui.get_messages(), model);
        assert!(model.row_data(0).unwrap().has_picture);
        assert!(model.row_data(0).unwrap().has_sender_avatar);
        assert_eq!(view.messages[0].reactions, ModelRc::from(reactions.clone()));
        assert!(reactions.row_data(0).unwrap().has_picture);
        // Exercise the actual submission controller with an in-memory command
        // queue and a synthetic saved session. No account worker is started.
        let root = std::env::temp_dir().join(format!("line-ui-submit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let session = root.join("session.json");
        std::fs::write(&session, br#"{"login_key":"c3ludGhldGljIG9ubHk="}"#).unwrap();
        let paths = account::Paths {
            state: session,
            wasm: root.join("unused"),
            cache: root.join("cache"),
        };
        let (storage, _) = storage::Storage::open(&paths, || {}).unwrap();
        view.storage = Some(storage);
        view.demo = false;
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        view.commands = Some(tx);
        ui.set_connected(true);
        ui.set_draft(" \n\t".into());
        view.submit(&ui);
        assert!(!ui.get_can_send());
        assert!(rx.try_recv().is_err());
        ui.set_draft("synthetic submission 🐈".into());
        view.submit(&ui);
        assert!(ui.get_draft().is_empty());
        let account::Command::Send {
            id, text, local_id, ..
        } = rx.try_recv().unwrap()
        else {
            panic!("wrong command");
        };
        assert_eq!(text, "synthetic submission 🐈");
        assert_eq!(view.saved.outbox.last().unwrap().status, "Pending…");
        ui.set_draft("a newer committed draft".into());
        view.update(&ui, account::Update::SendFailed { id, local_id });
        assert_eq!(ui.get_draft(), "a newer committed draft");
        assert!(
            rx.try_recv().is_err(),
            "ambiguous send was automatically retried"
        );
        view.storage.take();
        std::fs::remove_dir_all(root).unwrap();
        ui.set_draft("preserve this draft during sign-in".into());
        view.update(&ui, account::Update::ConnectionFailed);
        assert!(ui.get_connection_failed());
        assert!(!ui.get_connected());
        assert!(!ui.get_auth_open(), "failure started pairing automatically");
        view.begin_sign_in(&ui);
        assert!(rx.try_recv().is_err());
        ui.invoke_open_sign_in();
        assert!(
            rx.try_recv().is_err(),
            "opening the explanation started pairing"
        );
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        view.pairing_cancel = Some(cancel.clone());
        view.begin_sign_in(&ui);
        assert!(ui.get_auth_busy());
        assert!(matches!(
            rx.try_recv().unwrap(),
            account::Command::Reauthenticate
        ));
        assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
        view.cancel_sign_in(&ui);
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(ui.get_auth_cancelling());
        view.update(&ui, account::Update::SignInFailed { cancelled: true });
        assert!(!ui.get_auth_open());
        assert!(!ui.get_auth_busy());
        assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
        view.reconnect(&ui);
        assert!(ui.get_retrying_connection());
        assert!(matches!(
            rx.try_recv().unwrap(),
            account::Command::Reconnect
        ));
        view.update(&ui, account::Update::Connected("Synthetic account".into()));
        assert!(!ui.get_connection_failed());
        assert!(!ui.get_retrying_connection());
        assert_eq!(ui.get_draft(), "preserve this draft during sign-in");
        ui.invoke_open_sign_in();
        ui.set_auth_busy(true);
        view.update(
            &ui,
            account::Update::SignIn(line_wasm_poc::client::pairing::Event::Qr {
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }),
        );
        view.update(
            &ui,
            account::Update::SignIn(line_wasm_poc::client::pairing::Event::Pin("123456".into())),
        );
        assert_eq!(ui.get_auth_qr().size().width, 2);
        assert_eq!(ui.get_auth_pin(), "123456");
        view.update(&ui, account::Update::SignInFailed { cancelled: false });
        assert!(!ui.get_auth_busy());
        assert_eq!(ui.get_auth_qr().size().width, 0);
        assert!(ui.get_auth_pin().is_empty());
        assert!(!ui.get_auth_error().is_empty());
        view.update(&ui, account::Update::SessionChanged);
        assert!(view.saved.drafts.is_empty());
        assert!(view.saved.outbox.is_empty());
        assert!(view.current.is_empty());
        assert_eq!(ui.get_messages().row_count(), 0);
        assert!(ui.get_draft().is_empty());
        assert_eq!(ui.get_chat_title(), "Your conversations");
        // API order is not presentation order. Keep selection/draft by identity
        // while both refresh and paged arrivals sort by latest activity.
        view.demo = true;
        let chat = |id: &str, timestamp| account::ChatView {
            id: id.into(),
            title: id.into(),
            preview: "first line\nsecond line\tlast".into(),
            timestamp,
            avatar_key: String::new(),
        };
        view.update(
            &ui,
            account::Update::Chats {
                chats: vec![chat("older", 100), chat("newest", 300), chat("middle", 200)],
                more: true,
            },
        );
        assert_eq!(ui.get_chats().row_data(0).unwrap().title, "newest");
        assert_eq!(view.current, "newest");
        assert_eq!(
            ui.get_chats().row_data(0).unwrap().preview,
            "first line second line last"
        );
        ui.set_draft("keep selected chat draft".into());
        ui.set_list_pressed(true);
        view.update(
            &ui,
            account::Update::Chats {
                chats: vec![chat("older", 400)],
                more: false,
            },
        );
        assert_eq!(ui.get_chats().row_data(0).unwrap().title, "newest");
        ui.set_list_pressed(false);
        for event in std::mem::take(&mut view.deferred) {
            view.update(&ui, event);
        }
        assert_eq!(ui.get_chats().row_data(0).unwrap().title, "older");
        assert_eq!(view.current, "newest");
        assert_eq!(ui.get_draft(), "keep selected chat draft");
        assert_eq!(view.ids.len(), 3);
        let mut later = message("11", "later");
        later.timestamp = 300;
        let mut earlier = message("9", "earlier");
        earlier.timestamp = 100;
        let mut same_time = message("10", "same time");
        same_time.timestamp = 300;
        view.update(
            &ui,
            account::Update::Messages {
                id: view.current.clone(),
                generation: view.generation,
                messages: vec![later, earlier, same_time],
            },
        );
        assert_eq!(
            view.messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["9", "10", "11"]
        );
        view.saved.outbox.push(storage::Outgoing {
            local_id: "outbox-between".into(),
            timestamp_ms: 200,
            chat: view.current.clone(),
            text: "pending between messages".into(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        let ordered = view.with_outbox(&view.current, view.messages.clone());
        assert_eq!(
            ordered.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["9", "outbox-between", "10", "11"]
        );
        // An old original-photo completion cannot replace a newer viewer or
        // reopen a dismissed surface. Preview/failure remains local.
        view.messages[0].photo = true;
        view.messages[0].image_key = "test-photo".into();
        view.open_photo(&ui, "test-photo");
        assert!(ui.get_viewer_open());
        let before = ui.get_viewer_image();
        view.update(
            &ui,
            account::Update::OriginalFailed {
                request: view.viewer_request + 1,
            },
        );
        assert_eq!(ui.get_viewer_image(), before);
        assert!(ui.get_viewer_error().is_empty());
        view.update(
            &ui,
            account::Update::OriginalFailed {
                request: view.viewer_request,
            },
        );
        assert!(!ui.get_viewer_error().is_empty());
        ui.set_viewer_open(false);
        view.update(
            &ui,
            account::Update::Original {
                request: view.viewer_request,
                pixels: line_wasm_poc::client::desktop::assets::Pixels {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                }
                .into(),
            },
        );
        assert!(!ui.get_viewer_open());
        assert_eq!(ui.get_viewer_image(), before);
        // Exercise the actual composer shortcut with a local callback only.
        // No account worker or transport is involved.
        ui.set_auth_open(false);
        ui.set_settings_open(false);
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.set_can_send(true);
        ui.set_draft("日本語 🐈".into());
        let submissions = Rc::new(std::cell::Cell::new(0));
        let count = submissions.clone();
        ui.on_send(move || count.set(count.get() + 1));
        ui.show().unwrap();
        ui.invoke_focus_composer();
        let key = |text: &str, pressed: bool| {
            ui.window().dispatch_event(if pressed {
                slint::platform::WindowEvent::KeyPressed { text: text.into() }
            } else {
                slint::platform::WindowEvent::KeyReleased { text: text.into() }
            });
        };
        key("\u{11}", true);
        key("\n", true);
        key("\n", false);
        key("\u{11}", false);
        assert_eq!(submissions.get(), 1);
        assert_eq!(ui.get_draft(), "日本語 🐈");
        ui.set_can_send(false);
        key("\u{11}", true);
        key("\n", true);
        key("\n", false);
        key("\u{11}", false);
        assert_eq!(submissions.get(), 1, "disabled shortcut submitted");
        assert_eq!(ui.get_draft(), "日本語 🐈");
        ui.set_can_send(true);
        ui.set_ime_active(true);
        key("\u{11}", true);
        key("\n", true);
        key("\n", false);
        key("\u{11}", false);
        assert_eq!(submissions.get(), 1, "IME composition submitted");
        ui.set_ime_active(false);
        let before_enter = ui.get_draft().to_string();
        key("\n", true);
        key("\n", false);
        assert_eq!(submissions.get(), 1);
        assert_eq!(ui.get_draft().len(), before_enter.len() + 1);
        // Friend browsing and profile activation never submit or replace a draft.
        view.update(
            &ui,
            account::Update::Friends(vec![
                account::FriendView {
                    id: view.current.clone(),
                    name: "葵 🌸".into(),
                    status: "こんにちは\n日本語".into(),
                    group: false,
                    avatar_key: "".into(),
                },
                account::FriendView {
                    id: "new-friend".into(),
                    name: "New friend".into(),
                    status: "".into(),
                    group: false,
                    avatar_key: "".into(),
                },
                account::FriendView {
                    id: "new-group".into(),
                    name: "Group".into(),
                    status: "".into(),
                    group: true,
                    avatar_key: "".into(),
                },
            ]),
        );
        let current = view.current.clone();
        let draft = ui.get_draft();
        let count = view.ids.len();
        view.browse(&ui, 1);
        assert_eq!(view.current, current);
        assert_eq!(ui.get_draft(), draft);
        assert_eq!(ui.get_friends().row_count(), 3);
        ui.set_friend_query("葵".into());
        view.filter_friends(&ui);
        assert_eq!(ui.get_friends().row_count(), 1);
        view.open_profile(&ui, &current);
        assert!(ui.get_profile_open());
        assert_eq!(ui.get_profile_status(), "こんにちは\n日本語");
        ui.invoke_close_profile();
        assert_eq!(view.current, current);
        assert_eq!(ui.get_draft(), draft);
        ui.set_friend_query("".into());
        ui.set_groups_expanded(false);
        view.filter_friends(&ui);
        assert_eq!(ui.get_friends().row_count(), 2);
        view.update(&ui, account::Update::FriendsFailed);
        assert_eq!(ui.get_friends().row_count(), 2);
        assert!(!ui.get_friends_error().is_empty());
        view.open_profile(&ui, &current);
        view.start_friend_chat(&ui);
        assert_eq!(view.ids.len(), count, "existing friend chat duplicated");
        view.open_profile(&ui, "new-friend");
        view.start_friend_chat(&ui);
        assert_eq!(view.current, "new-friend");
        assert_eq!(view.ids.len(), count + 1);
        assert!(ui.get_draft().is_empty());
        assert_eq!(
            view.saved.drafts.get(&current).map(String::as_str),
            Some(draft.as_str())
        );
        view.open_profile(&ui, "new-group");
        view.update(&ui, account::Update::Friends(vec![]));
        assert!(!ui.get_profile_available());
        view.start_friend_chat(&ui);
        assert_eq!(
            view.current, "new-friend",
            "removed group remained actionable"
        );
        view.update(&ui, account::Update::SessionChanged);
        assert!(view.friends.is_empty());
        assert!(!ui.get_profile_open());
        assert_eq!(ui.get_friend_query(), "");
        ui.hide().unwrap();
    }
}
