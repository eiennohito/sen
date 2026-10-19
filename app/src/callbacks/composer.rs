use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_reply, |ui, d, id| {
        if let Ok(key) = id.parse::<i64>()
            && let Some(message) = d.state.messages.get(&Key(key))
            && let Some(server) = &message.server_id
        {
            d.state.composer_reply = Some(line::store::ReplyReference {
                message_id: server.0.clone(),
                service_code: Some(1),
            });
            d.capture_draft(&ui);
            d.render(&ui);
            let weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_focus_composer();
                }
            });
        }
    });
    global_callback!(ui, actions, on_cancel_reply, |ui, d| {
        d.state.composer_reply = None;
        d.capture_draft(&ui);
        d.render(&ui);
        ui.invoke_focus_composer();
    });
    global_callback!(ui, actions, on_react, |ui, d, id, kind| {
        if let Ok(key) = id.parse() {
            ui.global::<State>().set_reaction_error_id("".into());
            d.sender.send(Intent::React {
                key: Key(key),
                kind: kind as u32,
            });
        }
    });
    global_callback!(ui, actions, on_load_stickers, |ui, d| {
        d.request_sticker_assets(&ui);
        d.sender.send(Intent::StickerPackages);
    });
    global_callback!(ui, actions, on_select_sticker_package, |ui, d, index| {
        ui.global::<State>().set_sticker_package(index);
        d.request_sticker_assets(&ui);
        d.presenter.present_compose(&ui, &d.state);
    });
    global_callback!(ui, actions, on_send_sticker, |ui, d, index| {
        if let Some(chat) = d.state.selected.clone()
            && let Some(sticker) = d
                .state
                .sticker_packages
                .get(ui.global::<State>().get_sticker_package() as usize)
                .and_then(|p| p.stickers.get(index as usize))
        {
            d.sender.send(Intent::SendSticker {
                chat,
                sticker: Box::new(sticker.clone()),
                reply: d.state.composer_reply.clone(),
            });
        }
    });
    callback!(ui, actions, on_draft_edited, |ui, d| {
        d.capture_draft(&ui);
    });
    callback!(ui, actions, on_send, |ui, d| {
        d.state.composer = ui.get_draft().to_string();
        if let Some(intent) = d.state.submit() {
            d.sender.send(intent);
            d.render(&ui);
        }
    });
    callback!(ui, actions, on_retry_message, |_ui, d, id| {
        let key = Key(id.parse().unwrap());
        d.sender.send(Intent::Retry(key));
    });
    callback!(ui, actions, on_resolve_recovery, |ui, d, action| {
        let id = ui.get_recovery_id();
        let kind = ui.get_recovery_kind();
        ui.set_recovery_kind("".into());
        if action == "cancel" || kind == "unsend" {
            ui.invoke_focus_history();
            ui.set_recovery_focus_id(id.clone());
            ui.set_recovery_focus_action(kind);
        }
        if action == "unsend" {
            ui.set_unsend_feedback_id("".into());
            d.sender.send(Intent::Unsend(Key(id.parse().unwrap())));
        }
        if action == "replace" {
            d.sender.send(Intent::Copy(Key(id.parse().unwrap())));
            ui.invoke_focus_composer();
        }
        if action == "append" {
            let m = &d.state.messages[&Key(id.parse().unwrap())];
            d.state.composer = format!("{}\n{}", ui.get_draft(), m.body.as_text());
            ui.set_draft(d.state.composer.clone().into());
            d.capture_draft(&ui);
            ui.invoke_restore_editor(d.state.composer.len() as i32, d.state.composer.len() as i32);
            ui.invoke_focus_composer();
        }
    });
    callback!(ui, actions, on_recover_message, |ui, d, id| {
        if ui.get_draft().is_empty() {
            d.sender.send(Intent::Copy(Key(id.parse().unwrap())));
        } else {
            ui.set_recovery_id(id);
            ui.set_recovery_kind("edit".into());
        }
    });
    global_callback!(ui, actions, on_discard_message, |_ui, d, id| {
        d.sender.send(Intent::Discard(Key(id.parse().unwrap())));
    });
}
