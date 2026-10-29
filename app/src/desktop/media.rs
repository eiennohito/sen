//! Photos and videos handed to the desktop's default applications. The engine
//! writes a named local copy; opening it runs on a helper thread.
use crate::*;

pub(crate) enum MediaTarget {
    /// The viewer key the request was made for.
    Viewer(String),
    /// The message row id of a video.
    Message(String),
}
pub(crate) struct MediaOpen {
    key: String,
    target: MediaTarget,
}

impl crate::Desktop {
    pub(crate) fn open_photo_externally(&mut self, ui: &AppWindow) {
        let viewer = ui.global::<ViewerState>();
        if viewer.get_viewer_opening()
            || viewer.get_viewer_loading()
            || !viewer.get_viewer_error().is_empty()
        {
            return;
        }
        let Some(asset) = self.viewer_original(ui) else {
            viewer.set_viewer_save_status("Original unavailable".into());
            return;
        };
        let name = self.viewer_file_name.clone();
        viewer.set_viewer_opening(true);
        viewer.set_viewer_save_status("".into());
        self.media_open = Some(MediaOpen {
            key: asset.key(),
            target: MediaTarget::Viewer(viewer.get_viewer_key().into()),
        });
        self.sender.send(Intent::Media { asset, name });
    }

    pub(crate) fn play_video(&mut self, ui: &AppWindow, id: &str) {
        let g = ui.global::<ContentState>();
        if g.get_media_opening() {
            return;
        }
        let found = id
            .parse()
            .ok()
            .and_then(|key| self.state.messages.get(&Key(key)))
            .and_then(|m| Some((m.body.video()?, m.sort.sort_ts)));
        g.set_media_open_id(id.into());
        let Some((asset, sent)) = found else {
            g.set_media_open_status("Video unavailable".into());
            return;
        };
        let name = asset.file_name().map_or_else(
            || presenter::format::generated_name("video", sent),
            str::to_owned,
        );
        g.set_media_opening(true);
        g.set_media_open_status("Downloading video…".into());
        self.media_open = Some(MediaOpen {
            key: asset.key(),
            target: MediaTarget::Message(id.into()),
        });
        self.sender.send(Intent::Media { asset, name });
    }

    pub(crate) fn media_ready(&mut self, ui: &AppWindow, key: &str, path: &std::path::Path) {
        let Some(open) = self.media_open.take_if(|open| open.key == key) else {
            return;
        };
        if let MediaTarget::Message(id) = &open.target {
            let g = ui.global::<ContentState>();
            if g.get_media_open_id() == id.as_str() {
                g.set_media_open_status("Opening…".into());
            }
        }
        let weak = ui.as_weak();
        let path = path.to_owned();
        std::thread::spawn(move || {
            let result = desktop_env::open_file(&path);
            let _ = weak.upgrade_in_event_loop(move |ui| finish(&ui, open.target, result.err()));
        });
    }

    /// True when the failure belonged to a pending open.
    pub(crate) fn media_failed(
        &mut self,
        ui: &AppWindow,
        key: &str,
        failure: sen::client::desktop::assets::Failure,
    ) -> bool {
        let Some(open) = self.media_open.take_if(|open| open.key == key) else {
            return false;
        };
        let error = match open.target {
            MediaTarget::Viewer(_) => text::image_text(failure),
            MediaTarget::Message(_) => text::video_text(failure),
        };
        finish(ui, open.target, Some(error.into()));
        true
    }
}

fn finish(ui: &AppWindow, target: MediaTarget, error: Option<String>) {
    let error = error.unwrap_or_default();
    match target {
        MediaTarget::Viewer(key) => {
            let viewer = ui.global::<ViewerState>();
            viewer.set_viewer_opening(false);
            if viewer.get_viewer_open() && viewer.get_viewer_key() == key.as_str() {
                viewer.set_viewer_save_status(error.into());
            }
        }
        MediaTarget::Message(id) => {
            let g = ui.global::<ContentState>();
            g.set_media_opening(false);
            if g.get_media_open_id() == id.as_str() {
                g.set_media_open_status(error.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playing_a_video_downloads_a_named_copy_and_hands_it_to_the_opener() {
        std::thread::spawn(|| {
            use sen::backend::FakeBackend;
            i_slint_backend_testing::init_no_event_loop();
            let mut row = crate::tests::message("");
            // Stored before videos had an asset: playback derives it from the preview.
            row.body = Content::Video {
                duration: Some(3000),
                asset: None,
                preview: sen::client::desktop::assets::Asset::avatar("/video-poster"),
                album_count: None,
                caption: None,
            };
            let id = row.key.0.to_string();
            let (ui, runtime, worker) =
                crate::tests::test_window(FakeBackend::canned(), |d, worker, _| {
                    while d.state.connection != Connection::Online {
                        d.state.apply(
                            worker.deltas.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
                        );
                    }
                    d.state.messages.insert(row.key, row);
                });
            let g = ui.global::<ContentState>();
            g.invoke_play_video(id.clone().into());
            assert!(g.get_media_opening());
            assert_eq!(g.get_media_open_status(), "Downloading video…");
            g.invoke_play_video(id.clone().into()); // Ignored while one is pending.
            let path = loop {
                let delta = worker.deltas.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
                let path = match &delta {
                    Delta::Media { path, .. } => Some(path.clone()),
                    _ => None,
                };
                runtime.update(|d| {
                    d.apply(&ui, delta);
                });
                if let Some(path) = path {
                    break path;
                }
            };
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("sen_video_") && name.ends_with(".mp4") && !name.contains(' '));
            assert!(path.is_file());
            // The opener runs on a helper thread; its result returns through the event loop.
            assert_eq!(g.get_media_open_status(), "Opening…");
            runtime.update(|d| assert!(d.media_open.is_none()));
        })
        .join()
        .unwrap();
    }
}
