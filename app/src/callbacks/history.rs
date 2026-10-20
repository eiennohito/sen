use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_read_state_changed, |ui, d| {
        d.observe_reading(&ui);
    });
    global_callback!(ui, actions, on_more, |_ui, d| {
        d.sender.send(Intent::MoreChats);
    });
    global_callback!(ui, actions, on_retry_image, |ui, d, key| {
        d.sender.send(Intent::RetryAsset(key.to_string()));
        ui.invoke_focus_history();
    });
    global_callback!(ui, actions, on_jump_message, |ui, d, id| {
        let Ok(server_id) = ServerId::new(id.to_string()) else {
            return;
        };
        let target = d.state.reply_target(server_id.as_str()).map(|m| m.key);
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
    global_callback!(ui, actions, on_return_to_message, |ui, d| {
        d.history.jump_target = None;
        if let Some((key, offset)) = d.history.return_anchor.take() {
            // The origin is kept below the top cover, which may have changed.
            d.history
                .jump_to(key, offset - ui.global::<State>().get_reading().visible_top);
            ui.global::<State>().set_following(false);
            ui.global::<State>().set_return_visible(false);
            ui.global::<State>().set_highlight_message("".into());
            let intent = d.state.subscribe_window(Anchor::Key(key), 80, 80);
            d.sender.send(intent);
        }
    });
    global_callback!(ui, actions, on_latest_request, |ui, d| {
        d.history.jump_target = None;
        ui.global::<State>().set_return_visible(false);
        ui.global::<State>().set_highlight_message("".into());
        d.history.return_anchor = None;
        d.history.opening_history = false;
        if !matches!(d.state.window_anchor, Anchor::Latest) {
            d.history.latest_requested = true;
            d.history.restore_anchor = None;
            d.history.paging_anchor = None;
            d.history.requested_gaps.clear();
            let intent = d.state.subscribe_window(Anchor::Latest, 80, 0);
            d.sender.send(intent);
            ui.global::<State>().set_following(true);
        }
    });
    global_callback!(
        ui,
        actions,
        on_row_geometry,
        |ui, d, kind, id, top, height| {
            match kind {
                HistoryRowKind::Unread => d.history.divider_top = Some(top),
                HistoryRowKind::Gap | HistoryRowKind::Message => {
                    let id = if kind == HistoryRowKind::Gap {
                        id.strip_prefix("gap-").unwrap_or("")
                    } else {
                        &id
                    };
                    let Some(key) = super::message_key(id) else {
                        return;
                    };
                    let geometry = if kind == HistoryRowKind::Gap {
                        &mut d.history.gap_geometry
                    } else {
                        &mut d.history.geometry
                    };
                    geometry.insert(key, (top, height));
                }
            }
            // Row geometry can change without changing the viewport (opening and restoring anchors).
            d.observe_reading(&ui);
        }
    );
    global_callback!(ui, actions, on_history_scrolled, |ui, d| {
        let geometry = ui.global::<State>().get_reading();
        for intent in reading::page_older(&mut d.history, &mut d.state, geometry) {
            d.sender.send(intent);
        }
    });
}
