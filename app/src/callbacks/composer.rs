use crate::ComposerState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_reply, |ui, d, id| {
        if let Some(key) = super::message_key(&id)
            && let Some(message) = d.state.messages.get(&key)
            && let Some(server) = &message.server_id
        {
            d.state.composer_reply = Some(line::store::ReplyReference {
                message_id: server.as_str().to_owned(),
                service_code: Some(1),
            });
            d.capture_draft(&ui);
            d.render(&ui);
            let weak = ui.as_weak();
            // Menu activation restores its opener after the callback; focus the draft afterwards.
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_focus_composer();
                }
            });
        }
    });
    global_callback!(ui, actions, G, on_cancel_reply, |ui, d| {
        d.state.composer_reply = None;
        d.capture_draft(&ui);
        d.render(&ui);
        ui.invoke_focus_composer();
    });

    global_callback!(ui, actions, G, on_load_stickers, |ui, d| {
        d.request_sticker_assets(&ui);
        d.sender.send(Intent::StickerPackages);
    });
    global_callback!(ui, actions, G, on_select_sticker_package, |ui, d, index| {
        let g = ui.global::<G>();
        g.set_sticker_package(index);
        d.request_sticker_assets(&ui);
        d.presenter.present_compose(&ui, &d.state);
    });
    global_callback!(ui, actions, G, on_send_sticker, |ui, d, index| {
        let g = ui.global::<G>();
        if let Some(chat) = d.state.selected.clone()
            && let Some(sticker) = d
                .state
                .sticker_packages
                .get(g.get_sticker_package() as usize)
                .and_then(|p| p.stickers.get(index as usize))
        {
            d.sender.send(Intent::SendSticker {
                chat,
                sticker: Box::new(sticker.clone()),
                reply: d.state.composer_reply.clone(),
            });
        }
    });
    global_callback!(ui, actions, G, on_draft_edited, |ui, d| {
        d.capture_draft(&ui);
        d.presenter.present_draft(&ui, &d.state);
    });
    global_callback!(ui, actions, G, on_send, |ui, d| {
        let g = ui.global::<G>();
        d.state.composer = g.get_draft().to_string();
        if let Some(intent) = d.state.submit() {
            d.sender.send(intent);
            d.render(&ui);
        }
    });
}
