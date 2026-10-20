use crate::ConversationState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_react, |ui, d, id, kind| {
        let g = ui.global::<G>();
        if let Some(key) = super::message_key(&id) {
            g.set_reaction_error_id("".into());
            d.sender.send(Intent::React {
                key,
                kind: kind as u32,
            });
        }
    });
    global_callback!(ui, actions, G, on_retry_message, |_ui, d, id| {
        let Some(key) = super::message_key(&id) else {
            return;
        };
        d.sender.send(Intent::Retry(key));
    });
    global_callback!(ui, actions, G, on_resolve_recovery, |ui, d, action| {
        let g = ui.global::<G>();
        let id = g.get_recovery_id();
        let Some(key) = super::message_key(&id) else {
            return;
        };
        let kind = g.get_recovery_kind();
        g.set_recovery_kind(RecoveryKind::None);
        if action == RecoveryAction::Cancel || kind == RecoveryKind::Unsend {
            ui.invoke_focus_history();
        }
        if action == RecoveryAction::Unsend {
            g.set_unsend_feedback_id("".into());
            d.sender.send(Intent::Unsend(key));
        }
        if action == RecoveryAction::Replace {
            d.sender.send(Intent::Copy(key));
            ui.invoke_focus_composer();
        }
        if action == RecoveryAction::Append {
            let Some(m) = d.state.messages.get(&key) else {
                return;
            };
            d.state.composer = format!(
                "{}\n{}",
                ui.global::<ComposerState>().get_draft(),
                m.body.as_text()
            );
            ui.global::<ComposerState>()
                .set_draft(d.state.composer.clone().into());
            d.capture_draft(&ui);
            ui.invoke_restore_editor(d.state.composer.len() as i32, d.state.composer.len() as i32);
            ui.invoke_focus_composer();
        }
    });
    global_callback!(ui, actions, G, on_recover_message, |ui, d, id| {
        let g = ui.global::<G>();
        let Some(key) = super::message_key(&id) else {
            return;
        };
        if ui.global::<ComposerState>().get_draft().is_empty() {
            d.sender.send(Intent::Copy(key));
        } else {
            g.set_recovery_id(id);
            g.set_recovery_kind(RecoveryKind::Edit);
        }
    });
    global_callback!(ui, actions, G, on_discard_message, |_ui, d, id| {
        let Some(key) = super::message_key(&id) else {
            return;
        };
        d.sender.send(Intent::Discard(key));
    });
    global_callback!(ui, actions, G, on_read_state_changed, |ui, d| {
        d.observe_reading(&ui);
    });

    global_callback!(ui, actions, G, on_jump_message, |ui, d, id| {
        let Ok(server_id) = ServerId::new(id.to_string()) else {
            return;
        };
        let target = d.state.reply_target(&server_id).map(|m| m.key);
        if let Some(key) = target {
            d.jump_to_message(&ui, key);
        } else if let Some(chat) = d.state.selected.clone() {
            let id = server_id;
            d.history.jump_target = Some(id.clone());
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Around { id },
            });
        }
    });
    global_callback!(ui, actions, G, on_return_to_message, |ui, d| {
        let g = ui.global::<G>();
        d.history.jump_target = None;
        if let Some((key, offset)) = d.history.return_anchor.take() {
            // The origin is kept below the top cover, which may have changed.
            d.history.jump_to(key, offset - g.get_reading().visible_top);
            g.set_following(false);
            g.set_return_visible(false);
            g.set_highlight_message("".into());
            let intent = d.state.subscribe_window(Anchor::Key(key), 80, 80);
            d.sender.send(intent);
        }
    });
    global_callback!(ui, actions, G, on_latest_request, |ui, d| {
        let g = ui.global::<G>();
        d.history.jump_target = None;
        g.set_return_visible(false);
        g.set_highlight_message("".into());
        d.history.return_anchor = None;
        d.history.opening_history = false;
        if !matches!(d.state.window_anchor, Anchor::Latest) {
            d.history.latest_requested = true;
            d.history.restore_anchor = None;
            d.history.paging_anchor = None;
            d.history.requested_gaps.clear();
            let intent = d.state.subscribe_window(Anchor::Latest, 80, 0);
            d.sender.send(intent);
            g.set_following(true);
        }
    });
    global_callback!(ui, actions, G, on_row_geometry, |ui, d, k, id, y, h| {
        match k {
            HistoryRowKind::Unread => d.history.divider_top = Some(y),
            HistoryRowKind::Gap | HistoryRowKind::Message => {
                let id = if k == HistoryRowKind::Gap {
                    id.strip_prefix("gap-").unwrap_or("")
                } else {
                    &id
                };
                let Some(key) = super::message_key(id) else {
                    return;
                };
                let geometry = if k == HistoryRowKind::Gap {
                    &mut d.history.gap_geometry
                } else {
                    &mut d.history.geometry
                };
                geometry.insert(key, (y, h));
            }
        }
        // Row geometry can change without changing the viewport (opening and restoring anchors).
        d.observe_reading(&ui);
    });
    global_callback!(ui, actions, G, on_history_scrolled, |ui, d| {
        let g = ui.global::<G>();
        let geometry = g.get_reading();
        for intent in reading::page_older(&mut d.history, &mut d.state, geometry) {
            d.sender.send(intent);
        }
    });
}
