use crate::ContentState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_retry_image, |ui, d, key| {
        d.sender.send(Intent::RetryAsset(key.to_string()));
        ui.invoke_focus_history();
    });
    global_callback!(ui, actions, G, on_save_file, |ui, d, id| {
        let g = ui.global::<G>();
        let Some(key) = super::message_key(&id) else {
            return;
        };
        let Some(m) = d.state.messages.get(&key) else {
            return;
        };
        if let Content::File {
            asset: Some(asset),
            name,
            ..
        } = &m.body
        {
            let asset = asset.clone();
            let name = name.clone().unwrap_or_else(|| "閃 attachment".into());
            g.set_file_save_id(id.clone());
            g.set_file_save_status("".into());
            g.set_file_saving(true);
            d.sender.send(Intent::File { asset, name });
        }
    });
}
