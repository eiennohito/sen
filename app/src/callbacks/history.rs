use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_read_state_changed, |ui, d| {
        d.observe_reading(&ui);
    });
    callback!(ui, actions, on_more, |_ui, d| {
        d.sender.send(Intent::MoreChats);
    });
    callback!(ui, actions, on_retry_image, |ui, d, key| {
        d.sender.send(Intent::RetryAsset(key.to_string()));
        ui.invoke_focus_history();
    });
    global_callback!(ui, actions, on_jump_message, |ui, d, id| {
        let Ok(server_id) = ServerId::new(id.to_string()) else {
            return;
        };
        let target = d
            .state
            .reply_targets
            .get(&server_id)
            .map(|m| m.key)
            .or_else(|| {
                d.state
                    .messages
                    .values()
                    .find(|m| {
                        m.server_id
                            .as_ref()
                            .is_some_and(|s| s.as_str() == id.as_str())
                    })
                    .map(|m| m.key)
            });
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
        if let Some((key, offset, _)) = d.history.return_anchor.take() {
            // The origin is kept below the top cover, which may have changed.
            d.history.restore_anchor =
                Some((key, offset - ui.global::<State>().get_history_visible_top()));
            let known = d.history.geometry.contains_key(&key);
            d.history.restore_ready = known;
            if !known {
                d.history.geometry.clear();
            }
            ui.set_following(false);
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
            ui.set_following(true);
        }
    });
    global_callback!(ui, actions, on_row_geometry, |ui, d, id, top, height| {
        if id == "unread" {
            d.history.divider_top = Some(top);
        } else if let Some(key) = id.strip_prefix("gap-") {
            d.history
                .gap_geometry
                .insert(Key(key.parse().unwrap()), (top, height));
        } else {
            d.history
                .geometry
                .insert(Key(id.parse().unwrap()), (top, height));
        }
        d.observe_reading(&ui);
    });
    global_callback!(ui, actions, on_history_scrolled, |ui, d| {
        d.observe_reading(&ui);
        for intent in
            reading::page_older(&mut d.history, &mut d.state, Desktop::reading_geometry(&ui))
        {
            d.sender.send(intent);
        }
    });
}
