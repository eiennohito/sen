use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, on_profile_photo, |ui, d| {
        if ui.get_profile_avatar().size().width == 0 {
            return;
        }
        ui.global::<State>().invoke_remember_detail_focus(false);
        ui.global::<State>().set_viewer_from_detail(true);
        ui.global::<State>().set_viewer_has_previous(false);
        ui.global::<State>().set_viewer_has_next(false);
        ui.set_viewer_loading(false);
        ui.set_viewer_error("".into());
        d.viewer_asset = None;
        ui.set_viewer_key(format!("profile:{}", ui.get_profile_id()).into());
        ui.invoke_reset_viewer();
        ui.set_viewer_image(ui.get_profile_avatar());
        ui.set_viewer_status(ui.get_profile_name());
        d.state.panel = Panel::Viewer;
        d.render(&ui);
        ui.invoke_focus_viewer_close();
    });
    callback!(ui, actions, on_open_photo, |ui, d, key| {
        d.open_photo(&ui, key.as_str());
    });
    global_callback!(ui, actions, on_viewer_step, |ui, d, direction| {
        let photos = d
            .state
            .rows()
            .iter()
            .filter(|m| matches!(m.body, Content::Photo { .. }))
            .filter_map(|m| m.body.image().cloned())
            .map(|a| a.key())
            .collect::<Vec<_>>();
        if let Some(i) = photos
            .iter()
            .position(|k| k == ui.get_viewer_key().as_str())
        {
            let next = i as i32 + direction.signum();
            if next >= 0 && (next as usize) < photos.len() {
                let key = &photos[next as usize];
                ui.set_viewer_key(key.into());
                d.open_photo(&ui, key);
            }
        }
    });
    global_callback!(ui, actions, on_save_file, |ui, d, id| {
        let m = &d.state.messages[&Key(id.parse().unwrap())];
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
        viewer::save_photo(&ui, d.destination.clone());
    });
}
