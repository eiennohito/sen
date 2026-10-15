//! Encrypted, account-scoped drafts and unresolved submissions. No network access.
use line_wasm_poc::client::desktop::LocalStateStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub selected: String,
    pub selected_title: String,
    pub drafts: HashMap<String, String>,
    pub outbox: Vec<Outgoing>,
    pub font_size: i32,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Outgoing {
    pub local_id: String,
    #[serde(default)]
    pub timestamp_ms: i64,
    pub chat: String,
    pub text: String,
    pub status: String,
    pub server_id: String,
}
type Write = (Saved, Option<mpsc::Sender<bool>>);
pub struct Storage {
    tx: Option<mpsc::Sender<Write>>,
    thread: Option<thread::JoinHandle<()>>,
    healthy: Arc<AtomicBool>,
}
impl Storage {
    pub fn open(
        paths: &super::account::Paths,
        on_error: impl Fn() + Send + 'static,
    ) -> Result<(Self, Saved), String> {
        let (tx, rx) = mpsc::channel::<Write>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<Saved, String>>(1);
        let cache = paths
            .state
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("desktop-state");
        let session = paths.state.clone();
        let healthy = Arc::new(AtomicBool::new(true));
        let status = healthy.clone();
        let thread = thread::spawn(move || {
            let result = (|| {
                let store = LocalStateStore::open(&cache, &session)?;
                let saved = store
                    .load()?
                    .map(|bytes| serde_json::from_slice(&bytes))
                    .transpose()?
                    .unwrap_or_default();
                Ok::<_, Box<dyn std::error::Error>>((store, saved))
            })();
            let (store, saved) = match result {
                Ok(result) => result,
                Err(_) => {
                    let _ = ready_tx.send(Err("Local drafts could not be opened. Preserve any new text, correct the saved session or local storage, and restart. Sending is unavailable.".into()));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(saved));
            while let Ok((mut saved, mut ack)) = rx.recv() {
                // Coalesce ordinary keystrokes, but never delay an explicit flush.
                if ack.is_none() {
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(150);
                    while let Ok((newer, next_ack)) = rx
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    {
                        saved = newer;
                        ack = next_ack;
                        if ack.is_some() {
                            break;
                        }
                    }
                }
                let ok = serde_json::to_vec(&saved)
                    .ok()
                    .is_some_and(|bytes| store.save(&bytes).is_ok());
                status.store(ok, Ordering::Relaxed);
                if !ok {
                    on_error();
                }
                if let Some(ack) = ack {
                    let _ = ack.send(ok);
                }
            }
        });
        let saved = ready_rx
            .recv()
            .map_err(|_| "Local storage worker stopped".to_string())??;
        Ok((
            Self {
                tx: Some(tx),
                thread: Some(thread),
                healthy,
            },
            saved,
        ))
    }
    pub fn healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
    pub fn save(&self, saved: Saved) {
        if let Some(tx) = &self.tx {
            let _ = tx.send((saved, None));
        }
    }
    pub fn flush(&self, saved: Saved) -> bool {
        let (tx, rx) = mpsc::channel();
        self.tx
            .as_ref()
            .is_some_and(|sender| sender.send((saved, Some(tx))).is_ok())
            && rx.recv().unwrap_or(false)
    }
}
impl Drop for Storage {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    #[test]
    fn restart_preserves_drafts_and_uncertain_sends_in_private_account_storage() {
        let root = std::env::temp_dir().join(format!("line-desktop-state-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let session = root.join("session.json");
        fs::write(&session, br#"{"login_key":"c3ludGhldGljIHNlc3Npb24="}"#).unwrap();
        let paths = super::super::account::Paths {
            state: session.clone(),
            wasm: root.join("unused.wasm"),
            cache: root.join("cache"),
        };
        let (storage, mut saved) = Storage::open(&paths, || {}).unwrap();
        saved.selected = "chat-a".into();
        saved.font_size = 24;
        saved
            .drafts
            .insert("chat-a".into(), "秘密の下書き 🐈\nline two".into());
        saved.outbox.push(Outgoing {
            local_id: "local-1".into(),
            timestamp_ms: 1,
            chat: "chat-b".into(),
            text: "possibly delivered".into(),
            status: "Pending…".into(),
            server_id: String::new(),
        });
        assert!(storage.flush(saved));
        drop(storage);
        let (storage, reopened) = Storage::open(&paths, || {}).unwrap();
        assert_eq!(reopened.selected, "chat-a");
        assert_eq!(reopened.font_size, 24);
        assert_eq!(reopened.drafts["chat-a"], "秘密の下書き 🐈\nline two");
        assert_eq!(reopened.outbox[0].chat, "chat-b");
        assert_eq!(reopened.outbox[0].status, "Pending…");
        drop(storage);
        let state_root = root.join("desktop-state/local-state");
        let account = fs::read_dir(&state_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let path = account.join("desktop.state");
        assert_eq!(
            fs::metadata(&account).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let bytes = fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows(b"possibly delivered".len())
                .any(|w| w == b"possibly delivered")
        );
        fs::write(&session, br#"{"login_key":"ZGlmZmVyZW50IGFjY291bnQ="}"#).unwrap();
        let (storage, other) = Storage::open(&paths, || {}).unwrap();
        assert!(other.drafts.is_empty());
        assert!(other.outbox.is_empty());
        drop(storage);
        fs::write(&session, br#"{"login_key":"c3ludGhldGljIHNlc3Npb24="}"#).unwrap();
        let mut corrupt = bytes;
        corrupt[25] ^= 1;
        fs::write(path, corrupt).unwrap();
        assert!(
            Storage::open(&paths, || {}).is_err(),
            "corrupt state was silently reset"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
