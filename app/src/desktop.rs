use crate::*;

pub(crate) struct Desktop {
    pub(crate) history: reading::Reading,
    pub(crate) shown_composer: String,
    pub(crate) reveal: Option<ChatId>,
    pub(crate) viewer_asset: Option<String>,
    pub(crate) destination: viewer::Destination,
    pub(crate) notifications: Option<notifications::Notifications>,
    pub(crate) background: background::Background,
    pub(crate) assets: assets::Assets,
    pub(crate) state: AppState,
    pub(crate) presenter: presenter::Presenter,
    pub(crate) sender: Sender,
    pub(crate) query: String,
    pub(crate) category: i32,
}
impl Desktop {
    pub(crate) fn reading_geometry(ui: &AppWindow) -> reading::Geometry {
        let global = ui.global::<State>();
        reading::Geometry {
            y: ui.get_history_y(),
            visible_top: global.get_history_visible_top(),
            height: global.get_history_viewport_height(),
            pill_height: global.get_history_pill_height(),
            active: global.get_window_active(),
            modal: global.get_modal_open(),
            dragging: ui.get_history_dragging(),
            interacting: global.get_history_interacting(),
            narrow: global.get_narrow(),
            conversation: ui.get_show_conversation(),
            older_start_known: global.get_older_start_known(),
        }
    }
    pub(crate) fn observe_reading(&mut self, ui: &AppWindow) {
        let effects = reading::observe(&mut self.history, &self.state, Self::reading_geometry(ui));
        for intent in effects.intents {
            self.sender.send(intent);
        }
        if let Some((y, latest)) = effects.show {
            ui.invoke_show_history(y, latest);
        }
        for intent in reading::exposure(&mut self.history, &self.state, Self::reading_geometry(ui))
        {
            self.sender.send(intent);
        }
        if let Some(y) = reading::restore(&mut self.history) {
            ui.invoke_history_jump(y);
        }
    }
    pub(crate) fn drain_notifications(&mut self, ui: &AppWindow) {
        if let Some(n) = &self.notifications {
            n.drain(ui, &mut self.state, &self.sender);
        }
    }
    pub(crate) fn request_sticker_assets(&mut self, ui: &AppWindow) {
        self.assets.stickers(
            &self.state,
            ui.global::<State>().get_sticker_package() as usize,
            &self.sender,
        );
    }
    pub(crate) fn render(&mut self, ui: &AppWindow) {
        #[cfg(feature = "ui-debug")]
        latency::RENDERS.set(latency::RENDERS.get() + 1);
        // Text typed or set without an edit event stays; only engine changes replace it.
        if self.state.composer == self.shown_composer {
            self.state.composer = ui.get_draft().to_string();
        }
        self.shown_composer = self.state.composer.clone();
        self.assets.visible(
            &self.state,
            ui.global::<State>().get_sticker_package() as usize,
            &self.sender,
        );

        self.history.oldest = self.state.selected.clone().zip(
            self.state
                .rows()
                .iter()
                .find(|m| m.server_id.is_some())
                .map(|m| m.sort.clone()),
        );
        self.background.apply(ui);
        if let Some(notifications) = &self.notifications {
            notifications.drain(ui, &mut self.state, &self.sender);
            notifications.service.set_unread_count(
                self.state
                    .chats
                    .values()
                    .filter(|c| c.chat.unread_count > 0)
                    .count() as u64,
            );
        }

        self.presenter
            .present(ui, &self.state, &self.query, self.category);
        // A chat opened from a profile is revealed once its row exists.
        if let Some(index) = self
            .reveal
            .as_ref()
            .and_then(|id| self.state.chat_order.iter().position(|c| c == id))
        {
            let chats = ui.get_chats();
            if let Some(position) =
                (0..chats.row_count()).find(|&i| chats.row_data(i).unwrap().index == index as i32)
            {
                ui.set_result_focus(position as i32);
                ui.invoke_reveal_chat(position as i32);
                self.reveal = None;
            }
        }
    }
    pub(crate) fn select(&mut self, ui: &AppWindow, index: usize) {
        if let Some(id) = self.state.chat_order.get(index).cloned() {
            if let Some(n) = &self.notifications {
                n.service.withdraw(&id.0);
            }
            self.capture_draft(ui);
            self.history.latest_requested = false;
            self.history.opening_history = true;
            self.history.divider_top = None;
            self.history.return_anchor = None;
            ui.global::<State>().set_return_visible(false);
            ui.global::<State>().set_highlight_message("".into());
            self.history.restore_anchor = None;
            self.history.geometry.clear();
            self.history.gap_geometry.clear();
            self.history.requested_gaps.clear();
            self.history.paging_anchor = None;
            let intent = self.state.select(id);
            self.sender.send(intent);
            ui.set_show_conversation(true);
            ui.invoke_show_history(0., false);
            self.render(ui);
        }
    }
    pub(crate) fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let asset = self
            .state
            .rows()
            .iter()
            .filter_map(|m| m.body.image().cloned())
            .find(|a| a.key() == key)
            .and_then(|a| a.original());
        if let Some(asset) = asset {
            let photos = self
                .state
                .rows()
                .iter()
                .filter(|m| matches!(m.body, Content::Photo { .. }))
                .filter_map(|m| m.body.image().cloned())
                .map(|a| a.key())
                .collect::<Vec<_>>();
            let index = photos.iter().position(|k| k == key);
            ui.global::<State>()
                .set_viewer_has_previous(index.is_some_and(|i| i > 0));
            ui.global::<State>()
                .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
            ui.global::<State>()
                .set_viewer_name(asset.display_name().into());
            self.viewer_asset = Some(asset.key());
            if let Some(preview) = self.presenter.images.get(key) {
                ui.set_viewer_image(preview.clone());
            }
            ui.set_viewer_status("Preview · loading original…".into());
            ui.set_viewer_loading(true);
            ui.set_viewer_error("".into());
            if !ui.get_viewer_open() {
                ui.invoke_show_viewer(key.into());
            }
            self.sender.send(Intent::Original(asset));
        }
    }
    pub(crate) fn capture_draft(&mut self, ui: &AppWindow) {
        self.state.composer = ui.get_draft().to_string();
        if let Some(chat) = &self.state.selected {
            self.sender.send(Intent::Draft {
                chat: chat.clone(),
                draft: Draft {
                    text: self.state.composer.clone(),
                    cursor: ui.get_editor_caret() as usize,
                    anchor: ui.get_editor_anchor() as usize,
                    updated: line_wasm_poc::engine::now_ms(),
                },
            });
        }
    }
    pub(crate) fn apply_batch(&mut self, ui: &AppWindow, deltas: impl IntoIterator<Item = Delta>) {
        let mut dirty = false;
        let mut window = false;
        let mut editor = None;
        for delta in deltas {
            let (changed, arrived, selection) = self.apply(ui, delta);
            dirty |= changed;
            window |= arrived;
            editor = selection.or(editor);
        }
        if self.state.selected.is_none() && !self.state.chat_order.is_empty() {
            self.select(ui, 0);
        } else if dirty {
            self.render(ui);
        }
        if let Some((anchor, caret)) = editor {
            if ui.get_draft().as_str() != self.state.composer {
                ui.set_draft(self.state.composer.clone().into());
            }
            self.shown_composer = self.state.composer.clone();
            ui.invoke_restore_editor(anchor, caret);
        }
        if window && self.history.latest_requested && self.state.reaches_end {
            self.history.latest_requested = false;
            ui.set_following(true);
            ui.invoke_latest();
        } else if window && !self.history.opening_history && ui.get_following() {
            ui.invoke_latest();
        }
    }
    pub(crate) fn apply(&mut self, ui: &AppWindow, d: Delta) -> (bool, bool, Option<(i32, i32)>) {
        // The engine may publish an unchanged window and a whole chat list on a
        // draft save. Keep that acknowledgement out of the conversation render.
        if let Delta::Window {
            sub,
            upsert,
            remove,
            gaps,
            reaches_start,
            reaches_end,
            replies,
            unread_anchor,
        } = &d
            && *sub == self.state.window_sub
            && remove.is_empty()
            && upsert.len() == self.state.messages.len()
            && upsert
                .iter()
                .all(|m| self.state.messages.get(&m.key) == Some(m))
            && gaps == &self.state.gaps
            && *reaches_start == self.state.reaches_start
            && *reaches_end == self.state.reaches_end
            && (self.state.unread_divider.is_some() || *unread_anchor == self.state.unread_divider)
            && replies.len() == self.state.reply_targets.len()
            && replies.iter().all(|m| {
                m.server_id
                    .as_ref()
                    .and_then(|id| self.state.reply_targets.get(id))
                    == Some(m)
            })
        {
            return (false, false, None);
        }
        if let Delta::Chats {
            sub,
            upsert,
            remove,
            order,
        } = &d
            && *sub == self.state.chat_sub
            && remove.is_empty()
            && *order == self.state.chat_order
            && upsert.iter().all(|item| {
                self.state.chats.get(&item.chat.id).is_some_and(|old| {
                    old.chat == item.chat
                        && old.preview == item.preview
                        && old.activity_ms == item.activity_ms
                        && old.failed_count == item.failed_count
                })
            })
        {
            for item in upsert {
                if self.state.chats[&item.chat.id].draft != item.draft {
                    self.presenter.patch_draft(&item.chat.id, &item.draft);
                }
            }
            self.state.apply(d);
            return (false, false, None);
        }
        let dirty = !matches!(
            &d,
            Delta::Asset { .. } | Delta::MediaFailed { .. } | Delta::Draft { .. }
        );
        match &d {
            Delta::CreatedChat(result) => {
                ui.global::<State>().set_new_chat_busy(false);
                match result {
                    Ok(id) => {
                        ui.global::<State>().set_new_chat_open(false);
                        self.capture_draft(ui);
                        let intent = self.state.select(id.clone());
                        self.sender.send(intent);
                        self.reveal = Some(id.clone());
                        self.state.panel = Panel::Chats;
                        ui.set_section(0);
                        ui.set_show_conversation(true);
                        ui.invoke_focus_composer();
                    }
                    Err(error) => {
                        let global = ui.global::<State>();
                        global.set_new_chat_uncertain(matches!(
                            error,
                            line_wasm_poc::engine::CreateFailure::Uncertain
                        ));
                        global.set_new_chat_error(text::create_failure(error).into());
                    }
                }
            }
            Delta::ReactionFailed(key) => {
                ui.global::<State>()
                    .set_reaction_error_id(key.0.to_string().into());
                let weak = ui.as_weak();
                let id = key.0.to_string();
                slint::Timer::single_shot(std::time::Duration::from_secs(4), move || {
                    if let Some(ui) = weak.upgrade()
                        && ui.global::<State>().get_reaction_error_id() == id
                    {
                        ui.global::<State>().set_reaction_error_id("".into());
                    }
                });
            }
            Delta::UnsendFailed(key) => {
                if self.state.messages.contains_key(key) {
                    ui.set_unsend_feedback_id(key.0.to_string().into());
                }
            }
            Delta::Pair(event) => {
                use line_wasm_poc::backend::PairEvent;
                match event {
                    PairEvent::Stage(stage) => {
                        ui.set_auth_stage(text::pairing_stage(*stage).into())
                    }
                    PairEvent::Qr(p) => ui.set_auth_qr(slint::Image::from_rgba8(
                        slint::SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height),
                    )),
                    PairEvent::Pin(pin) => ui.set_auth_pin(pin.into()),
                    PairEvent::Scanned => ui.set_auth_stage("Phone scanned the code".into()),
                }
            }
            Delta::PairFailed { cancelled } => {
                if *cancelled {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
                ui.set_auth_error(
                    "Sign-in did not finish. Your cached conversations are still available.".into(),
                );
            }
            Delta::Connection(Connection::Online) if ui.get_auth_busy() => {
                self.state.panel = Panel::Chats;
                self.state.detail.clear();
            }
            Delta::Notify(m) => {
                if self.state.selected.as_ref() == Some(&m.chat_id) && !ui.get_following() {
                    ui.set_new_count(ui.get_new_count() + 1);
                }
                if let Some(notifications) = &self.notifications {
                    notifications.incoming(ui, &self.state, m);
                }
            }
            Delta::MoreChats(more) => ui.set_more_chats(*more),
            Delta::MediaFailed { key, failure } => {
                self.presenter.failures.insert(key.clone(), *failure);
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_loading(false);
                    ui.set_viewer_error(text::viewer_failure(*failure).into());
                }
            }
            Delta::File { name, bytes } => {
                let weak = ui.as_weak();
                let name = name.clone();
                let bytes = bytes.clone();
                let destination = self.destination.clone();
                std::thread::spawn(move || {
                    let result = viewer::save_bytes(&bytes, &name, destination);
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.global::<State>().set_file_saving(false);
                        ui.global::<State>().set_file_save_status(
                            result.err().unwrap_or_else(|| "Saved".into()).into(),
                        );
                    });
                });
            }
            Delta::Asset { key, pixels } => {
                self.presenter.failures.remove(key);
                self.presenter.images.insert(
                    key.clone(),
                    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                        &pixels.rgba,
                        pixels.width,
                        pixels.height,
                    )),
                );
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_image(self.presenter.images[key].clone());
                    ui.set_viewer_loading(false);
                    ui.set_viewer_status(
                        format!(
                            "{} · {} × {}",
                            ui.global::<State>().get_viewer_name(),
                            pixels.width,
                            pixels.height
                        )
                        .into(),
                    );
                }
            }
            Delta::Prefs(prefs) => {
                ui.set_text_size(prefs.text_size);
                ui.set_enter_sends(prefs.enter_sends);
                let s = ui.global::<State>();
                let p = &prefs.desktop;
                s.set_background_mode(p.background);
                s.set_autostart(p.autostart);
                s.set_notifications_enabled(p.notifications);
                s.set_notification_preview(p.preview);
                s.set_notification_sound(p.sound);
                s.set_notification_mention_exception(p.mention_exception);
                s.set_notification_group_events(p.group_events);
            }
            _ => {}
        }
        let window = matches!(d,Delta::Window {sub,..} if sub==self.state.window_sub);
        if window {
            self.history.restore_ready = true;
        }
        if matches!(d, Delta::Connection(Connection::Online)) {
            self.history.requested_gaps.clear();
        }
        let editor = match &d {
            Delta::Draft { chat, draft } if self.state.selected.as_ref() == Some(chat) => Some(
                draft
                    .as_ref()
                    .map(|d| (d.anchor as i32, d.cursor as i32))
                    .unwrap_or((0, 0)),
            ),
            _ => None,
        };
        if let Delta::Draft { chat, draft } = &d {
            if let Some(item) = self.state.chats.get_mut(chat) {
                item.draft = draft.clone();
            }
            self.presenter.patch_draft(chat, draft);
            // A delayed selection draft must not replace text entered since
            // the selection was rendered.
            if self.state.selected.as_ref() == Some(chat)
                && ui.get_draft().as_str() != self.shown_composer
            {
                return (false, false, None);
            }
        }
        self.state.apply(d);
        (dirty, window, editor)
    }
}
