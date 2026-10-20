//! Managed XDG autostart entry, refreshed at startup when enabled and on preference changes.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
pub(crate) fn directory() -> Result<PathBuf, String> {
    line::locations::Locations::config_home()
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
    let temporary = directory.join(format!(".line-desktop-{}.tmp", std::process::id()));
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
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_managed_entry_is_rebranded_in_place() {
        let root =
            std::env::temp_dir().join(format!("line-autostart-rebrand-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let entry = root.join(crate::desktop_env::desktop_integration::DESKTOP_FILE);
        fs::write(&entry, "[Desktop Entry]\nName=LINE\nX-LINE-Managed=true\n").unwrap();
        for _ in 0..2 {
            set_enabled(&root, Path::new("/synthetic/client"), true).unwrap();
            let contents = fs::read_to_string(&entry).unwrap();
            assert!(contents.contains("Name=閃\n"));
            assert!(contents.contains("X-Sen-Managed=true\n"));
            assert!(!contents.contains("X-LINE-Managed"));
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        }
        set_enabled(&root, Path::new("/synthetic/client"), false).unwrap();
        assert!(!entry.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn autostart_is_explicit_and_keeps_custom_entries() {
        let root = std::env::temp_dir().join(format!("line-autostart-test-{}", std::process::id()));
        let entry = root.join(crate::desktop_env::desktop_integration::DESKTOP_FILE);
        assert!(!entry.exists());
        set_enabled(&root, Path::new("/tmp/閃 % preview"), true).unwrap();
        let contents = fs::read_to_string(&entry).unwrap();
        assert!(contents.contains("Exec=/usr/bin/env -- \"/tmp/閃 %% preview\""));
        set_enabled(&root, Path::new("/tmp/executable"), false).unwrap();
        assert!(!entry.exists());
        fs::write(&entry, "[Desktop Entry]\nName=Custom\n").unwrap();
        assert!(set_enabled(&root, Path::new("/tmp/executable"), false).is_err());
        assert!(entry.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
