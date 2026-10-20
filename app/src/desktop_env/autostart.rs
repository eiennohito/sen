//! Managed XDG autostart entry, refreshed at startup when enabled and on preference changes.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
pub(crate) fn directory() -> Result<PathBuf, String> {
    sen::locations::Locations::config_home()
        .map(|path| path.join("autostart"))
        .map_err(|_| "Home folder is unavailable.".to_owned())
}
pub(crate) fn set_enabled(
    directory: &Path,
    executable: &Path,
    enabled: bool,
) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let destination = directory.join(crate::desktop_env::desktop_integration::DESKTOP_FILE);
    if destination.exists()
        && !fs::read_to_string(&destination).is_ok_and(|text| {
            text.contains("X-Sen-Managed=true\n") || text.contains("X-LINE-Managed=true\n")
        })
    {
        return Err(
            "An existing custom autostart entry was kept. Edit it in your desktop settings.".into(),
        );
    }
    if !enabled {
        if destination.exists() {
            fs::remove_file(destination).map_err(|_| "Could not disable autostart.")?;
        }
        return Ok(());
    }
    if !executable.is_absolute() {
        return Err("Autostart needs an absolute executable path.".into());
    }
    let text = executable
        .to_str()
        .ok_or("The executable path cannot be encoded.")?;
    if text.contains(['\r', '\n']) {
        return Err("The executable path is invalid.".into());
    }
    let mut command = String::from("\"");
    for character in text.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            command.push('\\');
        }
        command.push(character);
        if character == '%' {
            command.push('%');
        }
    }
    command.push('"');
    // Desktop Entry string escaping precedes Exec token escaping.
    command = command.replace('\\', "\\\\");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| "Could not create the autostart folder.")?;
    let temporary = directory.join(format!(".sen-desktop-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| "Could not prepare autostart.")?;
    command = format!("/usr/bin/env -- {command}");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=閃\nExec={command}\nIcon={}\nTerminal=false\nStartupNotify=true\nX-Sen-Managed=true\n",
        crate::desktop_env::desktop_integration::APP_ID
    );
    let result = file
        .write_all(contents.as_bytes())
        .and_then(|_| fs::rename(&temporary, &destination));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("Could not save autostart.".into());
    }
    // remove after the first release
    let old = directory.join("org.lineclient.Desktop.desktop");
    if fs::read_to_string(&old).is_ok_and(|text| {
        text.contains("X-Sen-Managed=true\n") || text.contains("X-LINE-Managed=true\n")
    }) {
        fs::remove_file(old).map_err(|_| "Could not remove the old autostart entry.")?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_managed_entry_is_rebranded_in_place() {
        let directory = sen::test_support::TempDir::new("autostart");
        let root = &directory.0;
        let entry = root.join(crate::desktop_env::desktop_integration::DESKTOP_FILE);
        let old = root.join("org.lineclient.Desktop.desktop");
        for marker in ["X-LINE-Managed", "X-Sen-Managed"] {
            fs::write(&old, format!("[Desktop Entry]\nName=LINE\n{marker}=true\n")).unwrap();
            set_enabled(root, Path::new("/synthetic/client"), true).unwrap();
            let contents = fs::read_to_string(&entry).unwrap();
            assert!(!old.exists());
            assert!(contents.contains("Name=閃\n"));
            assert!(contents.contains("X-Sen-Managed=true\n"));
            assert!(!contents.contains("X-LINE-Managed"));
            assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        }
        set_enabled(root, Path::new("/synthetic/client"), false).unwrap();
        assert!(!entry.exists());
    }
    #[test]
    fn custom_old_name_entry_is_kept() {
        let directory = sen::test_support::TempDir::new("autostart-custom-old");
        let old = directory.0.join("org.lineclient.Desktop.desktop");
        let custom = "[Desktop Entry]\nName=Custom\n";
        fs::write(&old, custom).unwrap();
        set_enabled(&directory.0, Path::new("/synthetic/client"), true).unwrap();
        assert_eq!(fs::read_to_string(old).unwrap(), custom);
        assert!(
            directory
                .0
                .join(crate::desktop_env::desktop_integration::DESKTOP_FILE)
                .exists()
        );
    }
    #[test]
    fn autostart_is_explicit_and_keeps_custom_entries() {
        let directory = sen::test_support::TempDir::new("autostart");
        let root = &directory.0;
        let entry = root.join(crate::desktop_env::desktop_integration::DESKTOP_FILE);
        assert!(!entry.exists());
        set_enabled(root, Path::new("/tmp/閃 % preview"), true).unwrap();
        let contents = fs::read_to_string(&entry).unwrap();
        assert!(contents.contains("Exec=/usr/bin/env -- \"/tmp/閃 %% preview\""));
        set_enabled(root, Path::new("/tmp/executable"), false).unwrap();
        assert!(!entry.exists());
        fs::write(&entry, "[Desktop Entry]\nName=Custom\n").unwrap();
        assert!(set_enabled(root, Path::new("/tmp/executable"), false).is_err());
        assert!(entry.exists());
    }
}
