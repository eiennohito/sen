//! Deliberate file exports through the portal or unique Downloads files.
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zbus::zvariant::Str;
/// Blocking: call from an export helper, never from the UI or account worker.
/// A cancelled chooser writes nothing. The fallback always creates a new file.
pub(crate) fn save_bytes(
    bytes: &[u8],
    suggested_name: &str,
    title: &str,
) -> Result<Option<PathBuf>, String> {
    let name = Path::new(suggested_name)
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|name| !name.is_empty() && !name.contains(['\r', '\n']))
        .unwrap_or("sen_file");
    save_dialog(bytes, name, title)
}
fn save_dialog(bytes: &[u8], name: &str, title: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(test)]
    if std::env::var("SEN_TEST_BUS").as_deref() != Ok("1") {
        return Err("tests require a private dbus-run-session".into());
    }
    #[cfg(feature = "demo")]
    if crate::dev::DEMO_SESSION.get().is_some() {
        return write_unique(&crate::dev::demo::directory().join("exports"), name, bytes).map(Some);
    }
    match portal_destination(name, title) {
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
        Err(error) => sen::reporting::log("export", "downloads-fallback", error.as_ref()),
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
                .unwrap_or("sen_file");
            match name.extension().and_then(|v| v.to_str()) {
                Some(extension) => format!("{stem}_{index}.{extension}"),
                None => format!("{stem}_{index}"),
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
    let config = sen::locations::Locations::config_home().map_err(|e| e.to_string())?;
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
fn portal_destination(
    name: &str,
    title: &str,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    crate::desktop_env::portal::choose(
        "SaveFile",
        title,
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
    #[test]
    fn photo_exports_preserve_bytes_and_use_private_unique_names() {
        let root = sen::test_support::TempDir::new("export");
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(3, 2)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let jpeg = jpeg.into_inner();
        let first = write_unique(&root.0, "sen_photo.jpg", &jpeg).unwrap();
        let second = write_unique(&root.0, "sen_photo.jpg", &jpeg).unwrap();
        assert_ne!(first, second);
        assert_eq!(second.file_name().unwrap(), "sen_photo_1.jpg");
        assert_eq!(fs::read(&first).unwrap(), jpeg);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(first).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(test)]
mod portal_tests {
    use super::*;
    use std::sync::mpsc;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};
    struct Chooser(mpsc::Sender<(String, String)>);
    #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
    impl Chooser {
        async fn save_file(
            &self,
            _parent: &str,
            title: &str,
            options: HashMap<String, OwnedValue>,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] connection: &zbus::Connection,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let token = <&str>::try_from(&options["handle_token"]).unwrap();
            let sender = header
                .sender()
                .unwrap()
                .as_str()
                .trim_start_matches(':')
                .replace('.', "_");
            let path = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/request/{sender}/{token}"
            ))
            .unwrap();
            self.0
                .send((
                    title.to_owned(),
                    <&str>::try_from(&options["current_name"])
                        .unwrap()
                        .to_owned(),
                ))
                .unwrap();
            connection
                .emit_signal(
                    None::<&str>,
                    &path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(1u32, HashMap::<String, OwnedValue>::new()),
                )
                .await
                .unwrap();
            Ok(path)
        }
    }
    #[test]
    #[ignore = "requires a private dbus-run-session"]
    fn file_and_photo_exports_use_their_own_chooser_titles() {
        assert_eq!(std::env::var("SEN_TEST_BUS").as_deref(), Ok("1"));
        std::thread::spawn(|| {
            let (titles, requests) = mpsc::channel();
            let server = zbus::blocking::connection::Builder::session()
                .unwrap()
                .serve_at("/org/freedesktop/portal/desktop", Chooser(titles))
                .unwrap()
                .name("org.freedesktop.portal.Desktop")
                .unwrap()
                .build()
                .unwrap();
            let _platform = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            assert_eq!(
                save_bytes(b"file bytes", "sen_file", "Save file").unwrap(),
                None
            );
            assert_eq!(
                requests
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap(),
                ("Save file".to_owned(), "sen_file".to_owned())
            );
            let root = sen::test_support::TempDir::new("export-photo");
            let source = root.0.join("photo.png");
            fs::write(&source, b"original bytes").unwrap();
            crate::viewer::save_photo(&ui, Some(source.clone()), "sen_photo".into());
            assert_eq!(
                requests
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap(),
                ("Save photo".to_owned(), "sen_photo.png".to_owned())
            );
            fs::remove_file(source).unwrap();
            let _ = server.close();
        })
        .join()
        .unwrap();
    }
}
