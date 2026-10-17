//! Desktop identity and local path policy. This module never opens account data.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};
use zbus::{blocking::Connection, zvariant::OwnedValue};

pub const APP_ID: &str = "org.lineclient.Desktop";
const OBJECT: &str = "/org/lineclient/Desktop";
#[derive(Debug, Clone)]
pub enum Activation {
    Show,
    OpenChat(String),
}
struct Application(mpsc::SyncSender<Activation>);
#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        let _ = self.0.try_send(Activation::Show);
    }
    fn open(&self, _uris: Vec<String>, _platform_data: HashMap<String, OwnedValue>) {
        let _ = self.0.try_send(Activation::Show);
    }
    fn activate_action(
        &self,
        action_name: &str,
        parameter: Vec<OwnedValue>,
        _platform_data: HashMap<String, OwnedValue>,
    ) {
        let event = if action_name == "open-chat" && parameter.len() == 1 {
            parameter
                .into_iter()
                .next()
                .and_then(|v| String::try_from(v).ok())
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 256
                        && id.bytes().all(|byte| byte.is_ascii_graphic())
                })
                .map(Activation::OpenChat)
                .unwrap_or(Activation::Show)
        } else {
            Activation::Show
        };
        let _ = self.0.try_send(event);
    }
}
pub struct Instance {
    _connection: Option<Connection>,
    _lock: Option<File>,
    events: std::cell::RefCell<mpsc::Receiver<Activation>>,
    pub warning: Option<String>,
}
pub enum Launch {
    Primary(Instance),
    Forwarded,
}
impl Instance {
    pub fn take_events(&self) -> mpsc::Receiver<Activation> {
        let (_, empty) = mpsc::channel();
        self.events.replace(empty)
    }
    /// Must precede window, storage and worker creation. The bus executor runs off the UI thread.
    pub fn acquire_with_activation(
        activate_existing: bool,
    ) -> Result<Launch, Box<dyn std::error::Error>> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("desktop-bus".into())
            .spawn(move || {
                let result = (|| {
                    let (tx, rx) = mpsc::sync_channel(16);
                    let connection = zbus::blocking::connection::Builder::session()?
                        .serve_at(OBJECT, Application(tx))?
                        .build()?;
                    match connection.request_name_with_flags(
                        APP_ID,
                        zbus::fdo::RequestNameFlags::DoNotQueue.into(),
                    ) {
                        Ok(
                            zbus::fdo::RequestNameReply::PrimaryOwner
                            | zbus::fdo::RequestNameReply::AlreadyOwner,
                        ) => Ok(Some((connection, rx))),
                        Err(zbus::Error::NameTaken) | Ok(_) => {
                            if activate_existing {
                                let _ = connection.call_method(
                                    Some(APP_ID),
                                    OBJECT,
                                    Some("org.freedesktop.Application"),
                                    "Activate",
                                    &(HashMap::<String, OwnedValue>::new(),),
                                );
                            }
                            Ok(None)
                        }
                        Err(error) => Err(error),
                    }
                })();
                let _ = ready_tx.send(result as Result<_, zbus::Error>);
            })?;
        match ready_rx.recv()? {
            Ok(Some((connection, events))) => Ok(Launch::Primary(Self {
                _connection: Some(connection),
                _lock: None,
                events: std::cell::RefCell::new(events),
                warning: None,
            })),
            Ok(None) => Ok(Launch::Forwarded),
            Err(_) => {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
                let (lock, warning) = match runtime {
                    Some(dir) => match lock_file(&dir.join("line-desktop.lock")) {
                        Ok(Some(file)) => (
                            Some(file),
                            "Desktop activation is unavailable. A local lock prevents another instance.",
                        ),
                        Ok(None) => return Ok(Launch::Forwarded),
                        Err(_) => (
                            None,
                            "Desktop activation and the local lock are unavailable. Another instance may be running.",
                        ),
                    },
                    None => (
                        None,
                        "Desktop activation and the runtime directory are unavailable. Another instance may be running.",
                    ),
                };
                let (_, events) = mpsc::channel();
                Ok(Launch::Primary(Self {
                    _connection: None,
                    _lock: lock,
                    events: std::cell::RefCell::new(events),
                    warning: Some(warning.into()),
                }))
            }
        }
    }
}
fn lock_file(path: &Path) -> std::io::Result<Option<File>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(0x20000)
        .open(path)?; // O_NOFOLLOW
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[derive(Clone, Debug)]
pub struct Locations {
    pub session: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub wasm: PathBuf,
    pub config: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Existing {
    session: PathBuf,
    wasm: PathBuf,
    state: PathBuf,
}
impl Locations {
    pub fn from_environment() -> Result<Self, String> {
        Self::resolve(|key| std::env::var_os(key).map(PathBuf::from))
    }
    fn resolve(get: impl Fn(&str) -> Option<PathBuf>) -> Result<Self, String> {
        let home = get("HOME")
            .filter(|p| p.is_absolute())
            .ok_or("HOME must be an absolute path")?;
        let base = |key, suffix| {
            get(key)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(suffix))
        };
        let data = base("XDG_DATA_HOME", ".local/share").join("line-desktop");
        Ok(Self {
            session: data.join("session.json"),
            wasm: data.join("ltsm.wasm"),
            state: base("XDG_STATE_HOME", ".local/state").join("line-desktop"),
            cache: base("XDG_CACHE_HOME", ".cache").join("line-desktop"),
            config: base("XDG_CONFIG_HOME", ".config").join("line-desktop/paths.json"),
        })
    }
    pub fn load_existing(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if !self.config.exists() {
            return Ok(false);
        }
        let old: Existing = serde_json::from_slice(&fs::read(&self.config)?)?;
        self.session = old.session;
        self.wasm = old.wasm;
        self.state = old.state;
        Ok(true)
    }
    pub fn legacy_available(root: &Path) -> bool {
        root.join("artifacts/auth/session.json").exists()
            || root.join(line_wasm_poc::DEFAULT_WASM).exists()
    }
    pub fn choose(
        &mut self,
        root: &Path,
        existing: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if existing {
            self.session = root.join("artifacts/auth/session.json");
            self.wasm = root.join(line_wasm_poc::DEFAULT_WASM);
            self.state = root.join("artifacts/auth/desktop-state");
        }
        let value = Existing {
            session: self.session.clone(),
            wasm: self.wasm.clone(),
            state: self.state.clone(),
        };
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.config.parent().ok_or("missing config parent")?)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.config)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&value)?)?;
        file.sync_all()?;
        Ok(())
    }
}
pub fn storage_directory(session: &Path) -> PathBuf {
    if let Ok(mut paths) = Locations::from_environment() {
        let _ = paths.load_existing();
        if session == paths.session {
            return paths.state;
        }
    }
    session
        .parent()
        .unwrap_or(Path::new("."))
        .join("desktop-state")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_queue_is_bounded_and_chat_ids_are_checked() {
        let (tx, rx) = mpsc::sync_channel(1);
        let app = Application(tx);
        app.activate(HashMap::new());
        app.activate(HashMap::new()); // Full queues never block a D-Bus reply.
        assert!(matches!(rx.try_recv().unwrap(), Activation::Show));
        assert!(rx.try_recv().is_err());
        for id in ["", "contains space", "new\nline", &"x".repeat(257)] {
            app.activate_action(
                "open-chat",
                vec![OwnedValue::from(zbus::zvariant::Str::from(id))],
                HashMap::new(),
            );
            assert!(matches!(rx.try_recv().unwrap(), Activation::Show));
        }
        app.activate_action(
            "open-chat",
            vec![OwnedValue::from(zbus::zvariant::Str::from(
                "synthetic-chat",
            ))],
            HashMap::new(),
        );
        assert!(
            matches!(rx.try_recv().unwrap(), Activation::OpenChat(id) if id == "synthetic-chat")
        );
    }
    #[test]
    fn xdg_paths_are_absolute_and_separate() {
        let paths = Locations::resolve(|key| match key {
            "HOME" => Some("/home/test".into()),
            "XDG_DATA_HOME" => Some("relative".into()),
            "XDG_STATE_HOME" => Some("/state".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            paths.session,
            Path::new("/home/test/.local/share/line-desktop/session.json")
        );
        assert_eq!(paths.state, Path::new("/state/line-desktop"));
        assert_eq!(
            paths.wasm,
            Path::new("/home/test/.local/share/line-desktop/ltsm.wasm")
        );
    }
    #[test]
    fn existing_files_require_explicit_choice_and_stay_in_place() {
        let root = std::env::temp_dir().join(format!("line-migration-test-{}", std::process::id()));
        fs::create_dir_all(root.join("artifacts/auth")).unwrap();
        let old = root.join("artifacts/auth/session.json");
        fs::write(&old, b"synthetic sentinel, not credentials").unwrap();
        let mut paths = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        assert!(Locations::legacy_available(&root));
        assert!(!paths.load_existing().unwrap());
        assert!(!paths.config.exists());
        paths.choose(&root, true).unwrap();
        let mut reloaded = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        assert!(reloaded.load_existing().unwrap());
        assert_eq!(reloaded.session, old);
        assert_eq!(reloaded.state, root.join("artifacts/auth/desktop-state"));
        assert_eq!(
            fs::read(&old).unwrap(),
            b"synthetic sentinel, not credentials"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn start_fresh_records_defaults_without_touching_existing_files() {
        let root = std::env::temp_dir().join(format!("line-fresh-test-{}", std::process::id()));
        fs::create_dir_all(root.join("artifacts/auth")).unwrap();
        let old = root.join("artifacts/auth/session.json");
        fs::write(&old, b"synthetic old file").unwrap();
        let mut paths = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        let session = paths.session.clone();
        paths.choose(&root, false).unwrap();
        assert_eq!(paths.session, session);
        assert!(!session.exists());
        assert_eq!(fs::read(old).unwrap(), b"synthetic old file");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&paths.config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn lock_excludes_second_writer_and_releases() {
        let path = std::env::temp_dir().join(format!("line-lock-test-{}", std::process::id()));
        let first = lock_file(&path).unwrap().unwrap();
        assert!(lock_file(&path).unwrap().is_none());
        drop(first);
        assert!(lock_file(&path).unwrap().is_some());
        fs::remove_file(path).unwrap();
    }
}
