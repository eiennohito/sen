//! Photo navigation and deliberate local exports. No network operations here.
use crate::{AppWindow, State, view::View};
use image::ImageEncoder;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};

impl View {
    pub(crate) fn open_viewer_photo(&mut self, ui: &AppWindow, key: &str) {
        self.open_photo(ui, key);
        let photos: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.photo && !m.image_key.is_empty())
            .collect();
        let index = photos.iter().position(|m| m.image_key == key);
        ui.global::<State>()
            .set_viewer_has_previous(index.is_some_and(|i| i > 0));
        ui.global::<State>()
            .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
    }
    pub(crate) fn step_photo(&mut self, ui: &AppWindow, direction: i32) {
        let photos: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.photo && !m.image_key.is_empty())
            .map(|m| m.image_key.to_string())
            .collect();
        let Some(index) = photos
            .iter()
            .position(|key| key == ui.get_viewer_key().as_str())
        else {
            return;
        };
        let next = index as i64 + i64::from(direction.signum());
        if let Some(key) = usize::try_from(next).ok().and_then(|i| photos.get(i)) {
            self.open_viewer_photo(ui, key);
        }
    }
}

pub(crate) fn save_photo(ui: &AppWindow) {
    if ui.global::<State>().get_viewer_saving()
        || ui.get_viewer_loading()
        || !ui.get_viewer_error().is_empty()
    {
        return;
    }
    let Some(pixels) = ui.get_viewer_image().to_rgba8() else {
        return;
    };
    ui.global::<State>().set_viewer_saving(true);
    ui.global::<State>().set_viewer_save_status("".into());
    let demo = ui.get_demo();
    let key = ui.get_viewer_key();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result =
            encode_png(&pixels).and_then(|bytes| save_bytes(&bytes, "LINE photo.png", demo));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<State>().set_viewer_saving(false);
            if !ui.get_viewer_open() || ui.get_viewer_key() != key {
                return;
            }
            ui.global::<State>().set_viewer_save_status(
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
pub(crate) fn save_bytes(
    bytes: &[u8],
    suggested_name: &str,
    demo: bool,
) -> Result<Option<PathBuf>, String> {
    let name = Path::new(suggested_name)
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|name| !name.is_empty() && !name.contains(['\r', '\n']))
        .unwrap_or("LINE attachment");
    if !demo {
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
    }
    let directory = if demo {
        std::env::temp_dir().join("line-desktop-demo-exports")
    } else {
        downloads_directory()?
    };
    write_unique(&directory, name, bytes).map(Some)
}
fn write_unique(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
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
    let connection = zbus::blocking::Connection::session()?;
    let unique = connection
        .unique_name()
        .ok_or("missing bus identity")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("line_save_{}", std::process::id());
    let expected = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    let request = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        expected.as_str(),
        "org.freedesktop.portal.Request",
    )?;
    let mut responses = request.receive_signal("Response")?;
    let portal = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    )?;
    let mut options: HashMap<&str, OwnedValue> = HashMap::new();
    options.insert("handle_token", Str::from(token).into());
    options.insert("current_name", Str::from(name).into());
    options.insert("modal", true.into());
    let actual: OwnedObjectPath = portal.call("SaveFile", &("", "Save photo", options))?;
    if actual.as_str() != expected {
        return Err("unexpected portal request path".into());
    }
    let response = responses.next().ok_or("portal closed without response")?;
    let (code, mut values): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    if code == 1 {
        return Ok(None);
    }
    if code != 0 {
        return Err("portal export failed".into());
    }
    let uris: Vec<String> = values
        .remove("uris")
        .ok_or("portal returned no destination")?
        .try_into()?;
    let path = url::Url::parse(uris.first().ok_or("empty destination")?)?
        .to_file_path()
        .map_err(|_| "destination is not a local file")?;
    Ok(Some(path))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_cannot_be_exported_as_an_original() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let ui = crate::create_app_window().unwrap();
            ui.set_viewer_image(slint::Image::from_rgba8(
                SharedPixelBuffer::<Rgba8Pixel>::new(1, 1),
            ));
            ui.set_viewer_loading(true);
            save_photo(&ui);
            assert!(!ui.global::<State>().get_viewer_saving());
            ui.set_viewer_loading(false);
            ui.set_viewer_error("Original unavailable".into());
            save_photo(&ui);
            assert!(!ui.global::<State>().get_viewer_saving());
        })
        .join()
        .unwrap();
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
