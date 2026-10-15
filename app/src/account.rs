//! A single account worker owns all tokens, WASM objects, requests and decoding.
use line_wasm_poc::client::desktop::{
    Chat, Client, Message,
    assets::{Asset, Pixels},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Paths {
    pub state: PathBuf,
    pub wasm: PathBuf,
    pub cache: PathBuf,
}
pub enum Command {
    Reconnect,
    Reauthenticate,
    Original {
        chat: String,
        key: String,
        request: u64,
    },
    Chats {
        next: bool,
    },
    Messages {
        id: String,
        generation: u64,
        refresh: bool,
    },
    Send {
        id: String,
        generation: u64,
        text: String,
        local_id: String,
    },
}

pub struct ChatView {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub timestamp: i64,
    pub avatar_key: String,
}
pub struct ReactionView {
    pub label: String,
    pub image_key: String,
}
pub struct MessageView {
    pub id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_key: String,
    pub sticker: bool,
    pub photo: bool,
    pub body: String,
    pub timestamp: i64,
    pub outgoing: bool,
    pub image_key: String,
    pub reactions: Vec<ReactionView>,
}
pub enum Update {
    Original {
        request: u64,
        pixels: Arc<Pixels>,
    },
    OriginalFailed {
        request: u64,
    },
    ConnectionFailed,
    SignIn(line_wasm_poc::client::pairing::Event),
    SignInFailed {
        cancelled: bool,
    },
    SessionChanged,
    Connected(String),
    Chats {
        chats: Vec<ChatView>,
        more: bool,
    },
    Messages {
        id: String,
        generation: u64,
        messages: Vec<MessageView>,
    },
    Sent {
        id: String,
        local_id: String,
        server_id: String,
    },
    SendFailed {
        id: String,
        local_id: String,
    },
    ChatsFailed,
    ImageFailed {
        key: String,
    },
    Image {
        key: String,
        pixels: Arc<Pixels>,
    },
    Error {
        message: String,
        generation: Option<u64>,
    },
}

pub struct Worker {
    pub commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    pub pairing_cancel: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.pairing_cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(paths: Paths, publish: impl Fn(Update) + Send + 'static) -> Worker {
    let (tx, rx) = mpsc::sync_channel(8);
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    let pairing_cancel = Arc::new(AtomicBool::new(false));
    let pairing_stop = pairing_cancel.clone();
    std::thread::spawn(move || {
        let Some(mut client) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else {
            return;
        };
        publish(Update::Connected(client.account_name.clone()));
        let mut pending = Some(Command::Chats { next: false });
        let mut jobs: Vec<(String, Asset)> = Vec::new();
        let mut failed_images = std::collections::HashMap::<String, Instant>::new();
        while !cancel.load(Ordering::Relaxed) {
            let command = pending.take().or_else(|| rx.try_recv().ok());
            if let Some(command) = command {
                // Prefer user commands over outstanding images from an old view.
                match command {
                    Command::Original { chat, key, request } => {
                        let result = client.messages(&chat).and_then(|messages| {
                            let asset = messages.into_iter().filter_map(|m| m.image).find(|asset| asset.key() == key).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "photo is no longer loaded"))?;
                            client.original_image(&asset)
                        });
                        match result { Ok(pixels) => publish(Update::Original { request, pixels }), Err(_) => publish(Update::OriginalFailed { request }) }
                    }
                    command @ (Command::Reconnect | Command::Reauthenticate) => {
                        if matches!(command, Command::Reauthenticate) && !reauthenticate(&paths, &cancel, &pairing_stop, &publish) { continue; }
                        let Some(reconnected) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else { break; };
                        client = reconnected;
                        jobs.clear(); failed_images.clear();
                        publish(Update::Connected(client.account_name.clone()));
                        pending = Some(Command::Chats { next: false });
                    }

                    Command::Chats { next } => match client.chats(next) {
                        Ok(chats) => {
                            let views = chats
                                .into_iter()
                                .map(|chat| chat_view(chat, &mut jobs))
                                .collect();
                            publish(Update::Chats {
                                chats: views,
                                more: client.more_chats,
                            });
                        }
                        Err(_) => publish(Update::ChatsFailed),
                    },
                    Command::Messages {
                        id,
                        generation,
                        refresh,
                    } => match if refresh {
                        failed_images.clear();
                        client.refresh_messages(&id)
                    } else {
                        client.messages(&id)
                    } {
                        Ok(messages) => publish(Update::Messages {
                            id,
                            generation,
                            messages: messages
                                .into_iter()
                                .map(|m| message_view(m, &mut jobs))
                                .collect(),
                        }),
                        Err(_) => publish(Update::Error {
                            message: "Could not load recent messages. Already loaded messages are kept. Use Retry messages.".into(),
                            generation: Some(generation),
                        }),
                    },
                    Command::Send {
                        id,
                        generation,
                        text,
                        local_id,
                    } => match client.send_text(&id, &text) {
                        Ok(message) => {
                            publish(Update::Sent {
                                id: id.clone(),
                                local_id,
                                server_id: message.id,
                            });
                            pending = Some(Command::Messages {
                                id,
                                generation,
                                refresh: true,
                            });
                        }
                        Err(_) => publish(Update::SendFailed { id, local_id }),
                    },
                }
                let mut seen = HashSet::new();
                jobs.retain(|(key, _)| seen.insert(key.clone()));
                jobs.truncate(256);
                jobs.reverse();
            } else if let Some((key, asset)) = jobs.pop() {
                if failed_images
                    .get(&key)
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(300))
                {
                    publish(Update::ImageFailed { key });
                    continue;
                }
                match client.image(&asset) {
                    Ok(pixels) => {
                        let pixels = if asset.kind() == "avatar" {
                            Arc::new(line_wasm_poc::client::desktop::assets::circular_avatar(
                                &pixels,
                            ))
                        } else {
                            pixels
                        };
                        publish(Update::Image { key, pixels });
                    }
                    Err(_) => {
                        publish(Update::ImageFailed { key: key.clone() });
                        if failed_images.len() >= 256 {
                            failed_images.clear();
                        }
                        failed_images.insert(key, Instant::now());
                    }
                }
            } else {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(command) => pending = Some(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        }
    });
    Worker {
        commands: tx,
        stop,
        pairing_cancel,
    }
}

/// Failed startup retains a command receiver: Retry and Sign in remain usable.
fn connect(
    paths: &Paths,
    rx: &mpsc::Receiver<Command>,
    stop: &AtomicBool,
    pairing_cancel: &AtomicBool,
    publish: &impl Fn(Update),
) -> Option<Client> {
    let mut attempt = true;
    while !stop.load(Ordering::Relaxed) {
        if attempt {
            if let Ok(client) = Client::open(&paths.state, &paths.wasm, &paths.cache) {
                return Some(client);
            }
            publish(Update::ConnectionFailed);
            attempt = false;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Command::Reconnect) => attempt = true,
            Ok(Command::Reauthenticate) => {
                if reauthenticate(paths, stop, pairing_cancel, publish) {
                    attempt = true;
                }
            }
            Ok(Command::Original { request, .. }) => publish(Update::OriginalFailed { request }),
            Ok(Command::Send { id, local_id, .. }) => publish(Update::SendFailed { id, local_id }),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

fn reauthenticate(
    paths: &Paths,
    stop: &AtomicBool,
    cancel: &AtomicBool,
    publish: &impl Fn(Update),
) -> bool {
    let result =
        line_wasm_poc::client::pairing::sign_in(&paths.state, &paths.wasm, cancel, |event| {
            publish(Update::SignIn(event))
        });
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    if result.is_ok() {
        publish(Update::SessionChanged);
        true
    } else {
        publish(Update::SignInFailed {
            cancelled: cancel.load(Ordering::Relaxed),
        });
        false
    }
}

fn asset_key(asset: Option<Asset>, jobs: &mut Vec<(String, Asset)>) -> String {
    if let Some(asset) = asset {
        let key = asset.key();
        jobs.push((key.clone(), asset));
        key
    } else {
        String::new()
    }
}
fn chat_view(chat: Chat, jobs: &mut Vec<(String, Asset)>) -> ChatView {
    ChatView {
        id: chat.id,
        title: chat.title,
        preview: chat.preview,
        timestamp: chat.timestamp_ms,
        avatar_key: asset_key(chat.avatar, jobs),
    }
}
fn message_view(message: Message, jobs: &mut Vec<(String, Asset)>) -> MessageView {
    MessageView {
        id: message.id,
        sender_id: message.sender_id,
        sender_name: message.sender_name,
        sender_key: asset_key(message.sender_avatar, jobs),
        photo: message
            .image
            .as_ref()
            .is_some_and(|asset| asset.kind() == "photo"),
        sticker: message
            .image
            .as_ref()
            .is_some_and(|asset| asset.kind() == "sticker"),
        body: message.body,
        timestamp: message.timestamp_ms,
        outgoing: message.outgoing,
        image_key: asset_key(message.image, jobs),
        reactions: message
            .reactions
            .into_iter()
            .map(|reaction| ReactionView {
                label: format!("{} {}", reaction.label, reaction.count),
                image_key: asset_key(reaction.asset, jobs),
            })
            .collect(),
    }
}

/// Uses the same library as the worker. Emits counts only; never sends messages.
pub fn check(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::open(&paths.state, &paths.wasm, &paths.cache)?;
    let chats = client.chats(false)?;
    let mut counts = (0, 0, 0, 0);
    let mut encrypted = 0;
    let mut assets = Vec::new();
    for chat in chats.iter().take(3) {
        if let Some(asset) = &chat.avatar {
            assets.push(asset.clone());
        }
        let messages = client.messages(&chat.id)?;
        for message in messages {
            counts.0 += 1;
            if message.encrypted && !message.decryption_failed {
                encrypted += 1;
            }
            if !message.decryption_failed {
                counts.1 += 1;
            }
            if let Some(asset) = message.image {
                assets.push(asset);
            }
            for reaction in message.reactions {
                counts.3 += reaction.count;
                if let Some(asset) = reaction.asset {
                    assets.push(asset);
                }
            }
        }
    }
    let mut seen = HashSet::new();
    assets.retain(|asset| seen.insert(asset.key()));
    let mut unavailable = 0;
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for asset in assets.iter().take(12) {
        match client.image(asset) {
            Ok(_) => {
                counts.2 += 1;
                *kinds.entry(asset.kind()).or_default() += 1;
                client.image(asset)?;
            }
            Err(_) => unavailable += 1,
        }
    }
    let stats = client.cache_stats();
    println!(
        "Live account check: {} chats, {} sampled messages, {} readable payloads, {} images decoded, {} images unavailable, {} reactions; cache hits {}, misses {}, downloads {}. No messages sent; contents not logged.",
        chats.len(),
        counts.0,
        counts.1,
        counts.2,
        unavailable,
        counts.3,
        stats.hits,
        stats.misses,
        stats.downloads
    );
    println!("Verified encrypted payloads: {encrypted}; decoded asset categories: {kinds:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_startup_keeps_retry_and_explicit_sign_in_receiver_alive() {
        let root = std::env::temp_dir().join(format!("line-missing-auth-{}", std::process::id()));
        let (tx, rx) = mpsc::channel();
        // Missing session fails before any crypto/network operation.
        let worker = start(
            Paths {
                state: root.join("missing.json"),
                wasm: root.join("missing.wasm"),
                cache: root.join("cache"),
            },
            move |event| {
                let _ = tx.send(event);
            },
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Update::ConnectionFailed
        ));
        assert!(worker.commands.try_send(Command::Reconnect).is_ok());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Update::ConnectionFailed
        ));
        // Exercise command handoff with cancellation already set: no pairing.
        worker.pairing_cancel.store(true, Ordering::Relaxed);
        assert!(worker.commands.try_send(Command::Reauthenticate).is_ok());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Update::SignInFailed { cancelled: true }
        ));
        assert!(!root.exists());
    }
}
