use crate::*;
impl crate::Desktop {
    pub(crate) fn show_original(&self, ui: &AppWindow, path: &std::path::Path) {
        let viewer = ui.global::<ViewerState>();
        viewer.set_viewer_loading(false);
        viewer.set_viewer_error("".into());
        viewer.set_viewer_retryable(false);
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
                viewer.set_viewer_retryable(self.viewer_original(ui).is_some());
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
        ui.global::<ViewerState>().set_viewer_retryable(false);
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
            self.sender.send(Intent::Original {
                asset,
                retry: false,
            });
        }
        self.state.panel = Panel::Viewer;
        self.render(ui);
    }
    pub(crate) fn viewer_original(
        &self,
        ui: &AppWindow,
    ) -> Option<sen::client::desktop::assets::Asset> {
        let key = self.viewer_asset.as_ref()?;
        if ui
            .global::<ViewerState>()
            .get_viewer_key()
            .starts_with("profile:")
        {
            crate::presenter::detail::profile_avatar(
                &self.state,
                ui.global::<DetailState>().get_profile_id().as_str(),
            )
            .and_then(|a| a.original())
            .filter(|a| a.key() == *key)
        } else {
            self.state
                .rows()
                .iter()
                .filter_map(|m| m.body.image()?.original())
                .find(|a| a.key() == *key)
        }
    }
    pub(crate) fn retry_original(&mut self, ui: &AppWindow) {
        let viewer = ui.global::<ViewerState>();
        if !viewer.get_viewer_open() || !viewer.get_viewer_retryable() {
            return;
        }
        let asset = self.viewer_original(ui);
        viewer.set_viewer_retryable(false);
        if let Some(asset) = asset {
            viewer.set_viewer_error("".into());
            viewer.set_viewer_loading(true);
            viewer.set_viewer_status("Loading original…".into());
            self.sender.send(Intent::Original { asset, retry: true });
        }
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
            asset.original()
        });
        if let Some(asset) = asset {
            ui.global::<ViewerState>().set_viewer_retryable(false);
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
            self.refresh_photo_navigation(ui);
            self.sender.send(Intent::Original {
                asset,
                retry: false,
            });
        }
    }

    pub(crate) fn refresh_photo_navigation(&self, ui: &AppWindow) {
        let viewer = ui.global::<ViewerState>();
        if viewer.get_viewer_open() {
            let photos = self.photos();
            let index = photos
                .iter()
                .position(|key| key == viewer.get_viewer_key().as_str());
            viewer.set_viewer_has_previous(index.is_some_and(|i| i > 0));
            viewer.set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewer_navigation_tracks_photos_arriving_after_open() {
        std::thread::spawn(|| {
            i_slint_backend_testing::init_no_event_loop();
            let photo = |id| {
                let mut row = crate::tests::message("");
                row.key = Key(id);
                row.sort.sort_ts = id;
                row.body = Content::Photo {
                    asset: sen::client::desktop::assets::Asset::avatar(&format!("/photo-{id}")),
                    size: None,
                    album_count: None,
                    caption: None,
                };
                row
            };
            let first = photo(1);
            let key = first.body.image().unwrap().key();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |d, _, _| {
                    d.state.selected = Some(first.chat_id.clone());
                    d.state.messages.insert(first.key, first);
                });
            ui.global::<ViewerState>().invoke_open_photo(key.into());
            let viewer = ui.global::<ViewerState>();
            assert!(viewer.get_viewer_open());
            assert!(!viewer.get_viewer_has_next());
            let second = photo(2);
            let second_key = second.body.image().unwrap().key();
            runtime.update(|d| {
                d.apply_batch(
                    &ui,
                    [Delta::Window {
                        sub: d.state.window_sub,
                        upsert: vec![second],
                        remove: vec![],
                        gaps: vec![],
                        reaches_start: true,
                        reaches_end: true,
                        replies: vec![],
                        unread_anchor: None,
                    }],
                );
            });
            assert!(
                viewer.get_viewer_has_next(),
                "arriving photo must become reachable"
            );
            viewer.invoke_viewer_step(1);
            assert_eq!(viewer.get_viewer_key(), second_key);
            assert!(viewer.get_viewer_has_previous());
            assert!(!viewer.get_viewer_has_next());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn viewer_retry_returns_to_the_same_original_and_preserves_outside_focus() {
        std::thread::spawn(|| {
            use sen::backend::{FakeBackend, FakeRpc};
            use sen::client::desktop::assets::Failure;
            i_slint_backend_testing::init_no_event_loop();
            let fake = FakeBackend::canned();
            let asset = sen::client::desktop::assets::Asset::avatar("/retry-flow").unwrap();
            let original = asset.original().unwrap();
            let key = original.key();
            fake.set_asset_failure(&original, Failure::Network);
            let (ui, runtime, worker) = crate::tests::test_window(fake.clone(), |d, worker, _| {
                while d.state.connection != Connection::Online {
                    d.state.apply(
                        worker.deltas.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
                    );
                }
                let mut row = crate::tests::message("");
                row.body = Content::Photo {
                    asset: Some(asset.clone()), size: None, album_count: None, caption: None,
                };
                d.state.messages.insert(row.key, row);
            });
            ui.show().unwrap();
            ui.global::<ViewerState>().invoke_open_photo(asset.key().into());
            let receive = || loop {
                let delta = worker.deltas.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
                let done = matches!(&delta, Delta::Original { key: k, .. } | Delta::MediaFailed { key: k, .. } if k == &key);
                runtime.update(|d| { d.apply(&ui, delta); });
                if done { break; }
            };
            receive();
            slint::platform::update_timers_and_animations();
            let viewer = ui.global::<ViewerState>();
            assert!(!viewer.get_viewer_loading());
            assert!(viewer.get_viewer_retryable());
            let retry_visible = || {
                i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, "Retry")
                    .next().is_some()
            };
            assert!(retry_visible());
            assert_eq!(crate::tests::focus_label(&ui), "Retry");
            crate::tests::chord(&ui, slint::platform::Key::Shift, slint::platform::Key::Tab.into());
            assert_ne!(crate::tests::focus_label(&ui), "Retry");
            crate::tests::press(&ui, slint::platform::Key::Tab);
            assert_eq!(crate::tests::focus_label(&ui), "Retry");
            crate::tests::press(&ui, slint::platform::Key::Return);
            assert!(viewer.get_viewer_loading());
            assert!(viewer.get_viewer_error().is_empty());
            viewer.invoke_retry_original(); // A second activation while loading is ignored.
            receive();
            assert!(viewer.get_viewer_error().is_empty());
            assert!(!viewer.get_viewer_loading());
            assert!(viewer.get_viewer_image().size().width > 0);
            runtime.update(|d| assert!(d.viewer_source.is_some()));
            assert_eq!(
                fake.calls().iter().filter(|call| **call == FakeRpc::Asset(key.clone())).count(),
                2,
            );
            for failure in [Failure::Http(404), Failure::Http(410), Failure::Decode, Failure::Http(500)] {
                runtime.update(|d| { d.apply(&ui, Delta::MediaFailed { key: key.clone(), failure }); });
                slint::platform::update_timers_and_animations();
                assert_eq!(retry_visible(), !matches!(failure, Failure::Http(404 | 410)));
            }
            viewer.set_viewer_retryable(false);
            ui.global::<ShellState>().set_panel(PanelState::None);
            slint::platform::update_timers_and_animations();
            ui.invoke_focus_search();
            assert_eq!(crate::tests::focus_label(&ui), "Find a conversation");
            let before = crate::tests::focus_item(&ui);
            runtime.update(|d| { d.apply(&ui, Delta::MediaFailed { key, failure: Failure::Network }); });
            slint::platform::update_timers_and_animations();
            assert_eq!(crate::tests::focus_item(&ui), before);
            runtime.update(|d| {
                d.state.messages.clear();
                d.apply(&ui, Delta::MediaFailed {
                    key: d.viewer_asset.clone().unwrap(), failure: Failure::Network,
                });
            });
            assert!(!viewer.get_viewer_retryable());
        }).join().unwrap();
    }
    #[test]
    fn viewer_original_failures_end_loading_and_profiles_wait_for_the_uncropped_image() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
                ui.global::<DetailState>()
                    .set_profile_avatar(slint::Image::from_rgba8(slint::SharedPixelBuffer::new(
                        192, 192,
                    )));
                ui.global::<DetailState>().set_profile_id("contact".into());
                desktop.state.contacts.insert(
                    "contact".into(),
                    Friend {
                        id: "contact".into(),
                        avatar: sen::client::desktop::assets::Asset::avatar("/contact"),
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
                        failure: sen::client::desktop::assets::Failure::Network,
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
                assert!(ui.global::<ViewerState>().get_viewer_retryable());
                assert_eq!(
                    ui.global::<ViewerState>().get_viewer_status(),
                    "Original unavailable"
                );
            });
        })
        .join()
        .unwrap();
    }
}
