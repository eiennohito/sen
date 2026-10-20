//! Photo navigation and deliberate local exports. No network operations here.
use crate::AppWindow;
use crate::ViewerState;
use image::ImageEncoder;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zbus::zvariant::Str;

pub(crate) fn save_photo(ui: &AppWindow, source: Option<std::sync::Arc<Vec<u8>>>) {
    if ui.global::<ViewerState>().get_viewer_saving()
        || ui.global::<ViewerState>().get_viewer_loading()
        || !ui.global::<ViewerState>().get_viewer_error().is_empty()
    {
        return;
    }
    let Some(pixels) = ui.global::<ViewerState>().get_viewer_image().to_rgba8() else {
        return;
    };
    ui.global::<ViewerState>().set_viewer_saving(true);
    ui.global::<ViewerState>().set_viewer_save_status("".into());
    let key = ui.global::<ViewerState>().get_viewer_key();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = export_photo(&pixels, source.as_deref().map(|b| b.as_slice()));
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
fn export_photo(
    pixels: &SharedPixelBuffer<Rgba8Pixel>,
    source: Option<&[u8]>,
) -> Result<Option<PathBuf>, String> {
    if let Some(bytes) = source {
        let extension = image::guess_format(bytes)
            .ok()
            .and_then(|f| f.extensions_str().first().copied())
            .unwrap_or("img");
        save_bytes(bytes, &format!("LINE photo.{extension}"))
    } else {
        encode_png(pixels).and_then(|bytes| save_bytes(&bytes, "LINE photo.png"))
    }
}
fn encode_png(pixels: &SharedPixelBuffer<Rgba8Pixel>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            pixels.as_bytes(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "Could not encode this photo.".to_string())?;
    Ok(bytes)
}

/// Blocking: call from an export helper, never from the UI or account worker.
/// A cancelled chooser writes nothing. The fallback always creates a new file.
pub(crate) fn save_bytes(bytes: &[u8], suggested_name: &str) -> Result<Option<PathBuf>, String> {
    let name = Path::new(suggested_name)
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|name| !name.is_empty() && !name.contains(['\r', '\n']))
        .unwrap_or("LINE attachment");
    save_dialog(bytes, name)
}
pub(crate) fn save_dialog(bytes: &[u8], name: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(feature = "demo")]
    if crate::dev::DEMO_SESSION.get().is_some() {
        return write_unique(&crate::dev::demo::directory().join("exports"), name, bytes).map(Some);
    }
    match portal_destination(name) {
        Ok(Some(path)) => {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| "Could not open the selected file.".to_string())?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Could not save the selected file.".to_string())?;
            return Ok(Some(path));
        }
        Ok(None) => return Ok(None),
        Err(_) => {}
    }
    write_unique(&downloads_directory()?, name, bytes).map(Some)
}
pub(crate) fn write_unique(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| "Could not open Downloads.".to_string())?;
    for index in 0..1000 {
        let path = directory.join(if index == 0 {
            name.to_owned()
        } else {
            let name = Path::new(name);
            let stem = name
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("LINE attachment");
            match name.extension().and_then(|v| v.to_str()) {
                Some(extension) => format!("{stem} ({index}).{extension}"),
                None => format!("{stem} ({index})"),
            }
        });
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Could not save the file.".to_string())?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Could not save the file in Downloads.".into()),
        }
    }
    Err("Too many files with this name. Choose another filename.".into())
}
fn downloads_directory() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or("Home folder is unavailable.")?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(contents) = fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in contents.lines() {
            if let Some(value) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                let value = value.trim().trim_matches('"');
                let path = PathBuf::from(value.replace("$HOME", &home.to_string_lossy()));
                if path.is_absolute() {
                    return Ok(path);
                }
            }
        }
    }
    Ok(home.join("Downloads"))
}
fn portal_destination(name: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    crate::desktop_env::portal::choose(
        "SaveFile",
        "Save photo",
        HashMap::from([("current_name", Str::from(name).into())]),
    )?
    .map(Some)
    .or_else(|code| {
        if code == 1 {
            Ok(None)
        } else {
            Err("file chooser failed")
        }
    })?
    .map(|uris| {
        url::Url::parse(uris.first().ok_or("empty destination")?)?
            .to_file_path()
            .map_err(|_| "destination is not a local file".into())
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "demo")]
    #[test]
    fn preview_cannot_be_exported_as_an_original() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let ui = crate::create_app_window().unwrap();
            ui.global::<ViewerState>()
                .set_viewer_image(slint::Image::from_rgba8(
                    SharedPixelBuffer::<Rgba8Pixel>::new(1, 1),
                ));
            ui.global::<ViewerState>().set_viewer_loading(true);
            save_photo(&ui, None);
            assert!(!ui.global::<ViewerState>().get_viewer_saving());
            ui.global::<ViewerState>().set_viewer_loading(false);
            ui.global::<ViewerState>()
                .set_viewer_error("Original unavailable".into());
            save_photo(&ui, None);
            assert!(!ui.global::<ViewerState>().get_viewer_saving());
        })
        .join()
        .unwrap();
    }
    #[cfg(feature = "demo")]
    #[test]
    fn writing_original_photo_preserves_bytes() {
        let root = std::env::temp_dir().join(format!("line-source-export-{}", std::process::id()));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::RgbImage::new(3, 2))
            .unwrap();
        let path = write_unique(&root, "LINE photo.jpg", &jpeg).unwrap();
        assert_eq!(fs::read(path).unwrap(), jpeg);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn png_export_and_unique_names() {
        let root = std::env::temp_dir().join(format!("line-export-test-{}", std::process::id()));
        let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&[255, 0, 0, 255], 1, 1);
        let png = encode_png(&pixels).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 1);
        let first = write_unique(&root, "photo.png", &png).unwrap();
        let second = write_unique(&root, "photo.png", &png).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), png);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(first).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
