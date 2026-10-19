use super::*;

impl Desktop {
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
        let d = if let Delta::Original { key, pixels, bytes } = d {
            if self.viewer_asset.as_ref() == Some(&key) {
                self.viewer_source = Some(bytes);
            }
            Delta::Asset { key, pixels }
        } else {
            d
        };
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
            && !self.state.window_loading
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
            Delta::Asset { .. }
                | Delta::AssetEvicted { .. }
                | Delta::MediaFailed { .. }
                | Delta::Draft { .. }
        );
        match &d {
            Delta::Connection(_) | Delta::Pair(_) | Delta::PairFailed { .. } => {
                self.apply_auth(ui, &d)
            }
            Delta::MediaFailed { .. } | Delta::File { .. } | Delta::Asset { .. } => {
                self.apply_viewer(ui, &d)
            }
            Delta::CreatedChat(_) => self.apply_new_chat(ui, &d),
            Delta::Prefs(_) => self.apply_prefs(ui, &d),
            _ => self.apply_rest(ui, &d),
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

    fn apply_auth(&mut self, ui: &AppWindow, d: &Delta) {
        match d {
            Delta::Connection(Connection::SignedOut { reason }) => {
                ui.global::<State>()
                    .set_session_cause(text::sign_out_reason(reason).into());
            }
            Delta::Pair(event) => {
                use line::backend::PairEvent;
                match event {
                    PairEvent::Stage(stage) => {
                        ui.set_auth_stage(text::pairing_stage(*stage).into());
                        if matches!(stage, line::client::pairing::Stage::RequestCredentials) {
                            ui.set_auth_pin("".into());
                        }
                    }
                    PairEvent::Qr(p) => {
                        ui.set_auth_stage(text::WAITING_FOR_PHONE.into());
                        ui.global::<State>().set_auth_prerequisite_missing(false);
                        ui.set_auth_qr(slint::Image::from_rgba8(
                            slint::SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height),
                        ));
                    }
                    PairEvent::Pin(pin) => ui.set_auth_pin(pin.into()),
                    PairEvent::Scanned => ui.set_auth_stage(text::WAITING_FOR_PHONE.into()),
                }
            }
            Delta::PairFailed { reason } => {
                let cancelled = matches!(reason, line::client::error::SignInFailure::Cancelled);
                ui.global::<State>()
                    .set_auth_prerequisite_missing(reason.is_prerequisite());
                ui.set_auth_cancelling(false);
                ui.set_auth_qr(slint::Image::default());
                ui.set_auth_pin("".into());
                if cancelled {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
                ui.set_auth_error(text::sign_in_failure(reason).into());
            }
            Delta::Connection(Connection::Online) => {
                ui.global::<State>().set_session_cause("".into());
                ui.set_auth_error("".into());
                ui.set_auth_qr(slint::Image::default());
                ui.set_auth_pin("".into());
                ui.set_auth_cancelling(false);
                if self.state.panel == Panel::SignIn {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
            }
            _ => {}
        }
    }

    fn apply_viewer(&mut self, ui: &AppWindow, d: &Delta) {
        match d {
            Delta::MediaFailed { key, failure } => {
                self.presenter.failures.insert(key.clone(), *failure);
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_loading(false);
                    ui.set_viewer_error(text::image_text(*failure).into());
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
            _ => {}
        }
    }

    fn apply_new_chat(&mut self, ui: &AppWindow, d: &Delta) {
        let Delta::CreatedChat(result) = d else {
            return;
        };
        ui.global::<State>().set_new_chat_busy(false);
        match result {
            Ok(id) => {
                ui.global::<State>().set_new_chat_open(false);
                self.open_chat(ui, id.clone());
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
                    line::engine::CreateFailure::Uncertain
                ));
                global.set_new_chat_error(text::create_failure(error).into());
            }
        }
    }

    fn apply_prefs(&mut self, ui: &AppWindow, d: &Delta) {
        let Delta::Prefs(prefs) = d else {
            return;
        };
        if self.state.prefs.desktop.notifications
            && !prefs.desktop.notifications
            && let Some(n) = &self.notifications
        {
            n.service.withdraw_notifications();
        }
        self.background.preferences(ui, &prefs.desktop);
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

    fn apply_rest(&mut self, ui: &AppWindow, d: &Delta) {
        match d {
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
            Delta::InternalError(kind) => {
                #[cfg(feature = "ui-debug")]
                {
                    let message = format!("Internal error: {kind} (see log)");
                    ui.set_internal_error_seen(true);
                    ui.set_internal_error(message.into());
                    let weak = ui.as_weak();
                    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_internal_error("".into());
                        }
                    });
                }
                #[cfg(not(feature = "ui-debug"))]
                let _ = kind;
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
            Delta::AssetEvicted { key } => self.assets.evicted(key),
            _ => {}
        }
    }
}
