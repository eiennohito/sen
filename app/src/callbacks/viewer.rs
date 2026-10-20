use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_photo_closed, |_ui, d| {
        d.viewer_asset = None;
        d.viewer_source = None;
    });
    global_callback!(ui, actions, on_profile_photo, |ui, d| {
        d.open_profile_photo(&ui);
    });
    global_callback!(ui, actions, on_open_photo, |ui, d, key| {
        d.open_photo(&ui, key.as_str());
    });
    global_callback!(ui, actions, on_viewer_step, |ui, d, direction| {
        let photos = d.photos();
        if let Some(i) = photos
            .iter()
            .position(|k| k == ui.global::<State>().get_viewer_key().as_str())
        {
            let next = i as i32 + direction.signum();
            if next >= 0 && (next as usize) < photos.len() {
                let key = &photos[next as usize];
                ui.global::<State>().set_viewer_key(key.into());
                d.open_photo(&ui, key);
            }
        }
    });
    global_callback!(ui, actions, on_save_file, |ui, d, id| {
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
            let name = name.clone().unwrap_or_else(|| "attachment".into());
            ui.global::<State>().set_file_save_id(id.clone());
            ui.global::<State>().set_file_save_status("".into());
            ui.global::<State>().set_file_saving(true);
            d.sender.send(Intent::File { asset, name });
        }
    });
    global_callback!(ui, actions, on_save_photo, |ui, d| {
        viewer::save_photo(&ui, d.destination, d.viewer_source.clone());
    });
}
