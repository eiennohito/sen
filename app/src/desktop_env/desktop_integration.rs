//! Desktop identity and local path policy. This module never opens account data.
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};
use zbus::{blocking::Connection, zvariant::OwnedValue};

pub const APP_ID: &str = "org.lineclient.Desktop";
pub const DESKTOP_FILE: &str = "org.lineclient.Desktop.desktop";
pub const APPLICATION_URI: &str = "application://org.lineclient.Desktop.desktop";
pub const OBJECT: &str = "/org/lineclient/Desktop";
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
        crate::event_loop::wake();
    }
    fn open(&self, _uris: Vec<String>, _platform_data: HashMap<String, OwnedValue>) {
        let _ = self.0.try_send(Activation::Show);
        crate::event_loop::wake();
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
                .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_graphic()))
                .map(Activation::OpenChat)
                .unwrap_or(Activation::Show)
        } else {
            Activation::Show
        };
        let _ = self.0.try_send(event);
        crate::event_loop::wake();
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
        for id in ["", "contains space", "new\nline"] {
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
    fn lock_excludes_second_writer_and_releases() {
        let path = std::env::temp_dir().join(format!("line-lock-test-{}", std::process::id()));
        let first = lock_file(&path).unwrap().unwrap();
        assert!(lock_file(&path).unwrap().is_none());
        drop(first);
        assert!(lock_file(&path).unwrap().is_some());
        std::fs::remove_file(path).unwrap();
    }
}
