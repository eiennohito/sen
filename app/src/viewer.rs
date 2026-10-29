//! Photo navigation and deliberate local exports. No network operations here.
use crate::AppWindow;
use crate::ViewerState;
use slint::ComponentHandle;
use std::{fs, path::PathBuf};

pub(crate) fn save_photo(ui: &AppWindow, source: Option<PathBuf>, name: String) {
    if ui.global::<ViewerState>().get_viewer_saving()
        || ui.global::<ViewerState>().get_viewer_loading()
        || !ui.global::<ViewerState>().get_viewer_error().is_empty()
    {
        return;
    }
    let Some(source) = source else {
        ui.global::<ViewerState>()
            .set_viewer_save_status("Original unavailable".into());
        return;
    };
    ui.global::<ViewerState>().set_viewer_saving(true);
    ui.global::<ViewerState>().set_viewer_save_status("".into());
    let key = ui.global::<ViewerState>().get_viewer_key();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = fs::read(&source)
            .map_err(|_| "Could not read this photo.".to_string())
            .and_then(|bytes| {
                let extension = source.extension().and_then(|s| s.to_str()).unwrap_or("img");
                let name = sen::client::desktop::assets::media_name(&name, extension);
                crate::export::save_bytes(&bytes, &name, "Save photo")
            });
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<ViewerState>().set_viewer_saving(false);
            if !ui.global::<ViewerState>().get_viewer_open()
                || ui.global::<ViewerState>().get_viewer_key() != key
            {
                return;
            }
            ui.global::<ViewerState>().set_viewer_save_status(
                match result {
                    Ok(Some(path)) => format!("Saved to {}", path.display()),
                    Ok(None) => "".into(),
                    Err(error) => error,
                }
                .into(),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_cannot_be_exported_as_an_original() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let (ui, _runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            ui.invoke_show_viewer("fixture".into());
            ui.global::<ViewerState>()
                .set_viewer_image(slint::Image::from_rgba8(slint::SharedPixelBuffer::<
                    slint::Rgba8Pixel,
                >::new(1, 1)));
            let source = crate::tests::image_file(&ui.global::<ViewerState>().get_viewer_image());
            ui.global::<ViewerState>().set_viewer_loading(true);
            save_photo(&ui, Some(source.clone()), "sen_photo".into());
            assert!(!ui.global::<ViewerState>().get_viewer_saving());
            ui.global::<ViewerState>().set_viewer_loading(false);
            ui.global::<ViewerState>()
                .set_viewer_error("Original unavailable".into());
            save_photo(&ui, Some(source), "sen_photo".into());
            assert!(!ui.global::<ViewerState>().get_viewer_saving());
            ui.global::<ViewerState>().set_viewer_error("".into());
            save_photo(&ui, None, "sen_photo".into());
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_save_status(),
                "Original unavailable"
            );
            ui.global::<ViewerState>().invoke_close_viewer();
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_image().size().width,
                0
            );
        })
        .join()
        .unwrap();
    }
}
