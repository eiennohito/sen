use crate::*;
impl crate::Desktop {
    pub(crate) fn show_original(&self, ui: &AppWindow, path: &std::path::Path) {
        let viewer = ui.global::<ViewerState>();
        viewer.set_viewer_loading(false);
        match slint::Image::load_from_path(path) {
            Ok(image) => {
                let size = image.size();
                viewer.set_viewer_image(image);
                viewer.set_viewer_status(
                    format!("{} · {} × {}", self.viewer_name, size.width, size.height).into(),
                );
            }
            Err(_) => {
                viewer.set_viewer_error("Could not load original".into());
                viewer.set_viewer_status("Original unavailable".into());
            }
        }
    }

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
            .set_viewer_image(slint::Image::default());
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
        self.state
            .rows()
            .iter()
            .filter(|m| matches!(m.body, Content::Photo { .. }))
            .filter_map(|m| m.body.image().cloned())
            .map(|asset| asset.key())
            .collect()
    }
    pub(crate) fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let asset = self.state.rows().iter().find_map(|m| {
            let asset = m.body.image().filter(|a| a.key() == key)?;
            asset.original()
        });
        if let Some(asset) = asset {
            let photos = self.photos();
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

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;
    #[test]
    fn viewer_original_failures_end_loading_and_profiles_wait_for_the_uncropped_image() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let ui = crate::tests::test_window();
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let mut desktop = crate::tests::desktop(worker.sender.clone());
            ui.global::<DetailState>()
                .set_profile_avatar(slint::Image::from_rgba8(slint::SharedPixelBuffer::new(
                    192, 192,
                )));
            ui.global::<DetailState>().set_profile_id("contact".into());
            desktop.state.contacts.insert(
                "contact".into(),
                Friend {
                    id: "contact".into(),
                    avatar: line::client::desktop::assets::Asset::avatar("/contact"),
                    ..Default::default()
                },
            );
            desktop.open_profile_photo(&ui);
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_image().size().width,
                0
            );
            assert!(ui.global::<ViewerState>().get_viewer_loading());
            let key = desktop.viewer_asset.clone().unwrap();
            desktop.apply(
                &ui,
                Delta::MediaFailed {
                    key: key.clone(),
                    failure: line::client::desktop::assets::Failure::Network,
                },
            );
            assert!(!ui.global::<ViewerState>().get_viewer_loading());
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_status(),
                "Original unavailable"
            );
            ui.global::<ViewerState>().set_viewer_loading(true);
            ui.global::<ViewerState>()
                .set_viewer_status("Loading original…".into());

            desktop.apply(
                &ui,
                Delta::Original {
                    key,
                    path: "missing-original.jpg".into(),
                    source: "missing-source.jpg".into(),
                },
            );
            assert!(!ui.global::<ViewerState>().get_viewer_loading());
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_error(),
                "Could not load original"
            );
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_status(),
                "Original unavailable"
            );
        })
        .join()
        .unwrap();
    }
}
