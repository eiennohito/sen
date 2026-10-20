use super::*;

impl Desktop {
    pub(crate) fn apply_batch(&mut self, ui: &AppWindow, deltas: impl IntoIterator<Item = Delta>) {
        let composer = ui.global::<ComposerState>();
        let conversation = ui.global::<ConversationState>();
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
            self.switch_chat(ui, self.state.chat_order[0].clone());
        } else if dirty {
            self.render(ui);
        }
        if let Some((anchor, caret)) = editor {
            let changed = composer.get_draft().as_str() != self.state.composer;
            if changed {
                composer.set_draft(self.state.composer.clone().into());
            }
            self.shown_composer = self.state.composer.clone();
            if changed
                || composer.get_editor_anchor() != anchor
                || composer.get_editor_caret() != caret
            {
                ui.invoke_restore_editor(anchor, caret);
            }
        }
        if window && self.history.latest_requested && self.state.reaches_end {
            self.history.latest_requested = false;
            conversation.set_following(true);
            conversation.invoke_latest();
        } else if window && !self.history.opening_history && conversation.get_following() {
            conversation.invoke_latest();
        }
    }
    pub(crate) fn apply(&mut self, ui: &AppWindow, d: Delta) -> (bool, bool, Option<(i32, i32)>) {
        let g = ui.global::<ComposerState>();
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
                && g.get_draft().as_str() != self.shown_composer
            {
                return (false, false, None);
            }
        }
        self.state.apply(d);
        (dirty, window, editor)
    }

    fn apply_auth(&mut self, ui: &AppWindow, d: &Delta) {
        let g = ui.global::<AuthState>();
        match d {
            Delta::Connection(Connection::SignedOut { reason }) => {
                g.set_session_cause(text::sign_out_reason(reason).into());
            }
            Delta::Pair(event) => {
                use line::backend::PairEvent;
                match event {
                    PairEvent::Stage(stage) => {
                        g.set_auth_phase(
                            if matches!(
                                stage,
                                line::client::pairing::Stage::Authenticated
                                    | line::client::pairing::Stage::Synthetic
                            ) {
                                AuthPhase::SignedIn
                            } else {
                                AuthPhase::Pairing
                            },
                        );
                        g.set_auth_stage(text::pairing_stage(*stage).into());
                        if matches!(stage, line::client::pairing::Stage::RequestCredentials) {
                            g.set_auth_pin("".into());
                        }
                    }
                    PairEvent::Qr(p) => {
                        g.set_auth_stage(text::WAITING_FOR_PHONE.into());
                        g.set_auth_prerequisite_missing(false);
                        g.set_auth_qr(slint::Image::from_rgba8(
                            slint::SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height),
                        ));
                    }
                    PairEvent::Pin(pin) => g.set_auth_pin(pin.into()),
                    PairEvent::Scanned => g.set_auth_stage(text::WAITING_FOR_PHONE.into()),
                }
            }
            Delta::PairFailed { reason } => {
                let cancelled = matches!(reason, line::client::error::SignInFailure::Cancelled);
                g.set_auth_prerequisite_missing(reason.is_prerequisite());
                g.set_auth_phase(AuthPhase::Preparing);
                g.set_auth_qr(slint::Image::default());
                g.set_auth_pin("".into());
                if cancelled {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
                g.set_auth_error(text::sign_in_failure(reason).into());
            }
            Delta::Connection(Connection::Online) => {
                g.set_session_cause("".into());
                g.set_auth_error("".into());
                g.set_auth_qr(slint::Image::default());
                g.set_auth_pin("".into());
                g.set_auth_phase(AuthPhase::SignedIn);
                if self.state.panel == Panel::SignIn {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
            }
            _ => {}
        }
    }

    fn apply_viewer(&mut self, ui: &AppWindow, d: &Delta) {
        let viewer = ui.global::<ViewerState>();
        match d {
            Delta::MediaFailed { key, failure } => {
                self.presenter.images.failed(key.clone(), *failure);
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    viewer.set_viewer_loading(false);
                    viewer.set_viewer_error(text::image_text(*failure).into());
                }
            }
            Delta::File { name, bytes } => {
                let weak = ui.as_weak();
                let name = name.clone();
                let bytes = bytes.clone();
                std::thread::spawn(move || {
                    let result = crate::viewer::save_bytes(&bytes, &name);
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<ContentState>();
                        g.set_file_saving(false);
                        g.set_file_save_status(
                            result.err().unwrap_or_else(|| "Saved".into()).into(),
                        );
                    });
                });
            }
            Delta::Asset { key, pixels } => {
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
                    viewer.set_viewer_image(self.presenter.images.get(key).unwrap());
                    viewer.set_viewer_loading(false);
                    viewer.set_viewer_status(
                        format!(
                            "{} · {} × {}",
                            self.viewer_name, pixels.width, pixels.height
                        )
                        .into(),
                    );
                }
            }
            _ => {}
        }
    }

    fn apply_new_chat(&mut self, ui: &AppWindow, d: &Delta) {
        let g = ui.global::<NewChatState>();
        let Delta::CreatedChat(result) = d else {
            return;
        };
        g.set_new_chat_busy(false);
        match result {
            Ok(id) => {
                g.set_new_chat_open(false);
                self.open_conversation(ui, id.clone());
            }
            Err(error) => {
                g.set_new_chat_uncertain(matches!(error, line::engine::CreateFailure::Uncertain));
                g.set_new_chat_error(text::create_failure(error).into());
            }
        }
    }

    fn apply_prefs(&mut self, ui: &AppWindow, d: &Delta) {
        let g = ui.global::<SettingsState>();
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
        g.set_text_size(prefs.text_size);
        g.set_enter_sends(prefs.enter_sends);
        let p = &prefs.desktop;
        g.set_background_mode(p.background);
        g.set_autostart(p.autostart);
        g.set_notifications_enabled(p.notifications);
        g.set_notification_preview(p.preview.into());
        g.set_notification_sound(p.sound);
        g.set_notification_mention_exception(p.mention_exception);
        g.set_notification_group_events(p.group_events);
    }

    fn apply_rest(&mut self, ui: &AppWindow, d: &Delta) {
        let conversation = ui.global::<ConversationState>();
        let chats = ui.global::<ChatsState>();
        match d {
            Delta::ReactionFailed(key) => {
                conversation.set_reaction_error_id(key.0.to_string().into());
            }
            Delta::UnsendFailed(key) => {
                if self.state.messages.contains_key(key) {
                    conversation.set_unsend_feedback_id(key.0.to_string().into());
                }
            }
            #[cfg(feature = "ui-debug")]
            Delta::InternalError(kind) => dev::automation::internal_error(ui, kind),
            Delta::Notify(m) => {
                if self.state.selected.as_ref() == Some(&m.chat_id) && !conversation.get_following()
                {
                    conversation.set_new_count(conversation.get_new_count() + 1);
                }
                if let Some(notifications) = &self.notifications {
                    notifications.incoming(ui, &self.state, m);
                }
            }
            Delta::MoreChats(more) => chats.set_more_chats(*more),
            Delta::AssetEvicted { key } => self.assets.evicted(key),
            _ => {}
        }
    }
}
