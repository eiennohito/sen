//! Managed XDG autostart entry. Only a deliberate preference change writes it.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
pub(crate) fn directory() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("Home folder is unavailable.")?;
    Ok(std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("autostart"))
}
pub(crate) fn set_enabled(
    directory: &Path,
    executable: &Path,
    enabled: bool,
) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let destination = directory.join("org.lineclient.Desktop.desktop");
    if destination.exists()
        && !fs::read_to_string(&destination)
            .is_ok_and(|text| text.contains("X-LINE-Managed=true\n"))
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
        "[Desktop Entry]\nType=Application\nName=LINE\nExec={command}\nIcon=org.lineclient.Desktop\nTerminal=false\nStartupNotify=true\nX-LINE-Managed=true\n"
    );
    let result = file
        .write_all(contents.as_bytes())
        .and_then(|_| file.sync_all())
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
    fn autostart_is_explicit_and_keeps_custom_entries() {
        let root = std::env::temp_dir().join(format!("line-autostart-test-{}", std::process::id()));
        let entry = root.join("org.lineclient.Desktop.desktop");
        assert!(!entry.exists());
        set_enabled(&root, Path::new("/tmp/LINE % preview"), true).unwrap();
        let contents = fs::read_to_string(&entry).unwrap();
        assert!(contents.contains("Exec=/usr/bin/env -- \"/tmp/LINE %% preview\""));
        set_enabled(&root, Path::new("/tmp/executable"), false).unwrap();
        assert!(!entry.exists());
        fs::write(&entry, "[Desktop Entry]\nName=Custom\n").unwrap();
        assert!(set_enabled(&root, Path::new("/tmp/executable"), false).is_err());
        assert!(entry.exists());
        fs::remove_dir_all(root).unwrap();
    }
}

pub(crate) struct Manager {
    desired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    wake: std::sync::mpsc::SyncSender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Manager {
    pub(crate) fn start(
        directory: PathBuf,
        executable: PathBuf,
        handler: impl Fn(u64, bool, Result<(), String>) + Send + 'static,
    ) -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
            mpsc,
        };
        let (desired, revision, stop) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (wake, receiver) = mpsc::sync_channel(1);
        let (enabled, generation, stopped) = (desired.clone(), revision.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            let mut applied = 0;
            while receiver.recv().is_ok() {
                loop {
                    let version = generation.load(Ordering::SeqCst);
                    if version != applied {
                        let value = enabled.load(Ordering::SeqCst);
                        let result = set_enabled(&directory, &executable, value);
                        applied = version;
                        if generation.load(Ordering::SeqCst) == version {
                            handler(version, value, result);
                        }
                    }
                    if generation.load(Ordering::SeqCst) != applied {
                        continue;
                    }
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    break;
                }
            }
        });
        Self {
            desired,
            revision,
            stop,
            wake,
            worker: Some(worker),
        }
    }
    pub(crate) fn request(&self, enabled: bool) {
        use std::sync::atomic::Ordering;
        self.desired.store(enabled, Ordering::SeqCst);
        self.revision.fetch_add(1, Ordering::SeqCst);
        let _ = self.wake.try_send(());
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.wake.try_send(());
    }
    pub(crate) fn shutdown(mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            std::thread::spawn(move || {
                let _ = worker.join();
            });
        }
    }
}
#[cfg(test)]
mod worker_tests {
    use super::*;
    #[test]
    fn rapid_autostart_changes_end_in_latest_state() {
        let root =
            std::env::temp_dir().join(format!("line-autostart-worker-{}", std::process::id()));
        let manager = Manager::start(root.clone(), "/tmp/line-test".into(), |_, _, _| {});
        for _ in 0..20 {
            manager.request(true);
            manager.request(false);
        }
        manager.shutdown();
        assert!(!root.join("org.lineclient.Desktop.desktop").exists());
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
    }
}
