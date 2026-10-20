use crate::ViewerState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_open_photo, |ui, d, key| {
        d.open_photo(&ui, key.as_str());
    });
    global_callback!(ui, actions, G, on_viewer_step, |ui, d, direction| {
        let g = ui.global::<G>();
        let photos = d.photos();
        if let Some(i) = photos.iter().position(|k| k == g.get_viewer_key().as_str()) {
            let next = i as i32 + direction;
            if next >= 0
                && let Some(key) = photos.get(next as usize)
            {
                g.set_viewer_key(key.clone().into());
                ui.invoke_reset_viewer();
                d.open_photo(&ui, key);
            }
        }
    });
    global_callback!(ui, actions, G, on_save_photo, |ui, d| {
        viewer::save_photo(&ui, d.viewer_source.clone());
    });
}
