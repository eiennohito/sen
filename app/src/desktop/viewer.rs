use crate::*;
impl crate::Desktop {
    pub(crate) fn open_profile_photo(&mut self, ui: &AppWindow) {
        if ui.global::<State>().get_profile_avatar().size().width == 0 {
            return;
        }
        ui.global::<State>().invoke_remember_detail_focus(false);
        ui.global::<State>().set_viewer_has_previous(false);
        ui.global::<State>().set_viewer_has_next(false);
        let id = ui.global::<State>().get_profile_id();
        let asset = crate::presenter::detail::profile_avatar(&self.state, id.as_str())
            .and_then(|a| a.original());
        ui.global::<State>()
            .set_viewer_key(format!("profile:{id}").into());
        ui.invoke_reset_viewer();
        ui.global::<State>()
            .set_viewer_image(ui.global::<State>().get_profile_avatar());
        ui.global::<State>()
            .set_viewer_name(ui.global::<State>().get_profile_name());
        ui.global::<State>().set_viewer_loading(asset.is_some());
        ui.global::<State>().set_viewer_error("".into());
        ui.global::<State>().set_viewer_status(
            if asset.is_some() {
                "Loading original…"
            } else {
                ""
            }
            .into(),
        );
        self.viewer_source = None;
        self.viewer_asset = asset.as_ref().map(|a| a.key());
        if let Some(asset) = asset {
            self.sender.send(Intent::Original(asset));
        }
        self.state.panel = Panel::Viewer;
        self.render(ui);
        ui.invoke_focus_viewer_close();
    }
    pub(crate) fn photos(&self) -> Vec<String> {
        self.state
            .rows()
            .iter()
            .filter(|m| matches!(m.body, Content::Photo { .. }))
            .filter_map(|m| m.body.image().cloned())
            .map(|asset| asset.key())
            .collect()
    }
    pub(crate) fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let asset = self
            .state
            .rows()
            .iter()
            .filter_map(|m| m.body.image().cloned())
            .find(|a| a.key() == key)
            .and_then(|a| a.original());
        if let Some(asset) = asset {
            let photos = self.photos();
            let index = photos.iter().position(|k| k == key);
            ui.global::<State>()
                .set_viewer_has_previous(index.is_some_and(|i| i > 0));
            ui.global::<State>()
                .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
            ui.global::<State>()
                .set_viewer_name(asset.display_name().into());
            self.viewer_source = None;
            self.viewer_asset = Some(asset.key());
            if let Some(preview) = self.presenter.images.get(key) {
                ui.global::<State>().set_viewer_image(preview.clone());
            }
            ui.global::<State>()
                .set_viewer_status("Preview · loading original…".into());
            ui.global::<State>().set_viewer_loading(true);
            ui.global::<State>().set_viewer_error("".into());
            if !ui.global::<State>().get_viewer_open() {
                ui.global::<State>().invoke_show_viewer(key.into());
            }
            self.sender.send(Intent::Original(asset));
        }
    }
}
