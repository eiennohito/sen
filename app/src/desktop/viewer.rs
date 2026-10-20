use crate::*;
impl crate::Desktop {
    pub(crate) fn open_profile_photo(&mut self, ui: &AppWindow) {
        if ui.global::<DetailState>().get_profile_avatar().size().width == 0 {
            return;
        }
        focus::remember_detail(ui, false);
        ui.global::<ViewerState>().set_viewer_has_previous(false);
        ui.global::<ViewerState>().set_viewer_has_next(false);
        let id = ui.global::<DetailState>().get_profile_id();
        let asset = crate::presenter::detail::profile_avatar(&self.state, id.as_str())
            .and_then(|a| a.original());
        ui.global::<ViewerState>()
            .set_viewer_key(format!("profile:{id}").into());
        ui.invoke_reset_viewer();
        ui.global::<ViewerState>()
            .set_viewer_image(ui.global::<DetailState>().get_profile_avatar());
        self.viewer_name = ui.global::<DetailState>().get_profile_name().to_string();
        ui.global::<ViewerState>()
            .set_viewer_loading(asset.is_some());
        ui.global::<ViewerState>().set_viewer_error("".into());
        ui.global::<ViewerState>().set_viewer_status(
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
    }
    pub(crate) fn photos(&self) -> Vec<String> {
        Self::photo_keys(&self.state.rows())
    }
    fn photo_keys(rows: &[&StoredMessage]) -> Vec<String> {
        rows.iter()
            .filter(|m| matches!(m.body, Content::Photo { .. }))
            .filter_map(|m| m.body.image().cloned())
            .map(|asset| asset.key())
            .collect()
    }
    pub(crate) fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let rows = self.state.rows();
        let asset = rows.iter().find_map(|m| {
            let asset = m.body.image().filter(|a| a.key() == key)?;
            if matches!(m.body, Content::Sticker { .. }) {
                Some(asset.clone())
            } else {
                asset.original()
            }
        });
        if let Some(asset) = asset {
            let photos = Self::photo_keys(&rows);
            let index = photos.iter().position(|k| k == key);
            ui.global::<ViewerState>()
                .set_viewer_has_previous(index.is_some_and(|i| i > 0));
            ui.global::<ViewerState>()
                .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
            self.viewer_name = asset.display_name().to_owned();
            self.viewer_source = None;
            self.viewer_asset = Some(asset.key());
            if let Some(preview) = self.presenter.images.get(key) {
                ui.global::<ViewerState>().set_viewer_image(preview.clone());
            }
            ui.global::<ViewerState>()
                .set_viewer_status("Preview · loading original…".into());
            ui.global::<ViewerState>().set_viewer_loading(true);
            ui.global::<ViewerState>().set_viewer_error("".into());
            if !ui.global::<ViewerState>().get_viewer_open() {
                ui.invoke_show_viewer(key.into());
            }
            self.sender.send(Intent::Original(asset));
        }
    }
}
