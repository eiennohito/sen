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

pub struct Paths {
    pub state: PathBuf,
    pub wasm: PathBuf,
    pub cache: PathBuf,
}
pub enum Command {
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
    },
}

pub struct ChatView {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub avatar_key: String,
}
pub struct ReactionView {
    pub label: String,
    pub image_key: String,
}
pub struct MessageView {
    pub body: String,
    pub timestamp: i64,
    pub outgoing: bool,
    pub image_key: String,
    pub reactions: Vec<ReactionView>,
}
pub enum Update {
    Connected(String),
    Chats {
        chats: Vec<ChatView>,
        append: bool,
        more: bool,
    },
    Messages {
        id: String,
        generation: u64,
        messages: Vec<MessageView>,
    },
    Sent {
        id: String,
        generation: u64,
        text: String,
    },
    Image {
        key: String,
        pixels: Arc<Pixels>,
    },
    Error {
        message: String,
        sending: bool,
        generation: Option<u64>,
    },
}

pub struct Worker {
    pub commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(paths: Paths, publish: impl Fn(Update) + Send + 'static) -> Worker {
    let (tx, rx) = mpsc::sync_channel(8);
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    std::thread::spawn(move || {
        let mut client = match Client::open(&paths.state, &paths.wasm, &paths.cache) {
            Ok(client) => client,
            Err(error) => {
                publish(Update::Error {
                    message: format!("Could not open saved account: {error:#}"),
                    sending: false,
                    generation: None,
                });
                return;
            }
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
                    Command::Chats { next } => match client.chats(next) {
                        Ok(chats) => {
                            let views = chats
                                .into_iter()
                                .map(|chat| chat_view(chat, &mut jobs))
                                .collect();
                            publish(Update::Chats {
                                chats: views,
                                append: next,
                                more: client.more_chats,
                            });
                        }
                        Err(error) => publish(Update::Error {
                            message: format!("Could not load conversations: {error:#}"),
                            sending: false,
                            generation: None,
                        }),
                    },
                    Command::Messages {
                        id,
                        generation,
                        refresh,
                    } => match if refresh {
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
                        Err(error) => publish(Update::Error {
                            message: format!("Could not load messages: {error:#}"),
                            sending: false,
                            generation: Some(generation),
                        }),
                    },
                    Command::Send {
                        id,
                        generation,
                        text,
                    } => match client.send_text(&id, &text) {
                        Ok(_) => {
                            publish(Update::Sent {
                                id: id.clone(),
                                generation,
                                text,
                            });
                            pending = Some(Command::Messages {
                                id,
                                generation,
                                refresh: true,
                            });
                        }
                        Err(error) => publish(Update::Error {
                            message: format!(
                                "Send was not confirmed. The message may have arrived; check the conversation before trying again. {error:#}"
                            ),
                            sending: true,
                            generation: Some(generation),
                        }),
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
                    continue;
                }
                match client.image(&asset) {
                    Ok(pixels) => publish(Update::Image { key, pixels }),
                    Err(_) => {
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
    Worker { commands: tx, stop }
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
        avatar_key: asset_key(chat.avatar, jobs),
    }
}
fn message_view(message: Message, jobs: &mut Vec<(String, Asset)>) -> MessageView {
    MessageView {
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
