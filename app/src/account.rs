//! A single account worker owns all tokens, WASM objects, requests and decoding.
use line_wasm_poc::client::desktop::{
    Chat, Client, Message, SendFailure,
    assets::{Asset, Pixels},
    send_failure,
    sync::{self, SyncState},
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
// Extended contracts are consumed by S8/S10/S11 in subsequent sessions.
#[allow(dead_code)]
pub enum Command {
    Stop,
    Older {
        chat: String,
        before_message: String,
        delivered_time: i64,
    },
    Push {
        epoch: u64,
        event: sync::Event,
    },
    Reconnect,
    Friends,
    ChatDetails,
    MarkRead {
        chat: String,
        up_to_message: String,
    },
    Receipts {
        chat: String,
    },
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

#[allow(dead_code)]
pub struct ChatView {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub timestamp: i64,
    pub avatar_key: String,
    pub unread: Option<u64>,
    pub muted: Option<bool>,
    pub last_read_message: Option<String>,
    pub member_count: Option<usize>,
    pub outgoing_preview: bool,
    pub group: bool,
}
#[allow(dead_code)]
pub struct FriendView {
    pub id: String,
    pub name: String,
    pub status: String,
    pub group: bool,
    pub avatar_key: String,
    pub member_count: Option<usize>,
}
pub struct ReactionView {
    pub label: String,
    pub image_key: String,
}
#[allow(dead_code)]
pub struct MessageView {
    pub id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_key: String,
    pub sticker: bool,
    pub photo: bool,
    pub body: String,
    pub timestamp: i64,
    pub delivered_time: i64,
    pub outgoing: bool,
    pub image_key: String,
    pub reactions: Vec<ReactionView>,
    pub rich: line_wasm_poc::client::desktop::rich::RichContent,
}
#[allow(dead_code)]
pub enum Update {
    Older {
        chat: String,
        messages: Vec<MessageView>,
        reached_beginning: bool,
    },
    OlderFailed {
        chat: String,
        before_message: String,
    },
    Sync(SyncState),
    Incoming {
        chat: String,
        messages: Vec<MessageView>,
        replay: bool,
    },
    ReadElsewhere {
        chat: String,
        up_to_message: String,
    },
    ReadMarked {
        chat: String,
        up_to_message: String,
    },
    ReadFailed {
        chat: String,
    },
    ReadRanges(Vec<line_wasm_poc::client::desktop::ReadRange>),
    ChatMeta {
        chat: String,
        title: String,
        preview: String,
        timestamp: i64,
        unread: Option<u64>,
        muted: Option<bool>,
    },
    Receipt {
        chat: String,
        up_to_message: String,
        reader_count: Option<usize>,
    },
    SendRejected {
        id: String,
        local_id: String,
        reason: String,
    },
    SendUncertain {
        id: String,
        local_id: String,
    },
    Friends(Vec<FriendView>),
    FriendsFailed,
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
        let _ = self.commands.try_send(Command::Stop);
        self.pairing_cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(paths: Paths, publish: impl Fn(Update) + Send + 'static) -> Worker {
    let (tx, rx) = mpsc::sync_channel(8);
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    let pairing_cancel = Arc::new(AtomicBool::new(false));
    let pairing_stop = pairing_cancel.clone();
    let push_tx = tx.clone();
    std::thread::spawn(move || {
        let Some(mut client) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else {
            return;
        };
        publish(Update::Connected(client.account_name.clone()));
        let mut sync_epoch = 0;
        let mut sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch));
        let mut markers = line_wasm_poc::client::desktop::read::Markers::default();
        let mut pending = Some(Command::Chats { next: false });
        let mut jobs: Vec<(String, Asset)> = Vec::new();
        let mut failed_images = std::collections::HashMap::<String, Instant>::new();
        while !cancel.load(Ordering::Relaxed) {
            if let Some((chat, up_to_message)) = markers.due(Instant::now()) {
                match client.mark_read(&chat, &up_to_message) {
                    Ok(()) => {
                        markers.confirmed(&chat, &up_to_message);
                        publish(Update::ReadMarked {
                            chat,
                            up_to_message,
                        });
                    }
                    Err(_) => publish(Update::ReadFailed { chat }),
                }
            }
            let command = rx.try_recv().ok().or_else(|| pending.take());
            if let Some(command) = command {
                // Prefer user commands over outstanding images from an old view.
                match command {
                    Command::Stop => break,
                    Command::Older { chat, before_message, delivered_time } => match client.older(&chat, &before_message, delivered_time) {
                        Ok((messages,reached_beginning)) => publish(Update::Older { chat, messages: messages.into_iter().map(|m| message_view(m,&mut jobs)).collect(), reached_beginning }),
                        Err(_) => publish(Update::OlderFailed { chat, before_message }),
                    },
                    Command::MarkRead { chat, up_to_message } => markers.expose(chat, up_to_message, Instant::now()),
                    Command::Receipts { chat } => { if let Ok(ranges) = client.read_ranges(&chat) { publish(Update::ReadRanges(ranges)); } },
                    Command::Push { epoch, event } => { if epoch != sync_epoch { continue; } match event {
                        sync::Event::State(state) => publish(Update::Sync(state)),
                        sync::Event::Token(token) => client.replace_token(token),
                        sync::Event::Resync { complete, .. } => {
                            match client.resync() {
                                Ok((chats, pages)) => {
                                    publish(Update::Chats { chats: chats.into_iter().map(|c| chat_view(c, &mut jobs)).collect(), more: client.more_chats });
                                    for (chat, messages) in pages { publish(Update::Incoming { chat, messages: messages.into_iter().map(|m| message_view(m, &mut jobs)).collect(), replay: true }); }
                                    let _ = complete.send(true);
                                },
                                Err(_) => { let _ = complete.send(false); }
                            }
                        },
                        sync::Event::Operation(operation) => {
                            if operation["type"].as_u64() == Some(40) {
                                if let (Some(chat), Some(message)) = (operation["param1"].as_str(),operation["param2"].as_str()) { publish(Update::ReadElsewhere { chat: chat.into(), up_to_message: message.into() }); }
                                continue;
                            }
                            if operation["type"].as_u64() == Some(55) {
                                if let (Some(chat), Some(message)) = (operation["param1"].as_str(),operation["param3"].as_str()) { publish(Update::Receipt { chat: chat.into(), up_to_message: message.into(), reader_count: None }); pending=Some(Command::Receipts { chat: chat.into() }); }
                                continue;
                            }
                            match client.operation(&operation) {
                                Some((chat, message)) => {
                                    publish(Update::Incoming { chat, messages: vec![message_view(message, &mut jobs)], replay: false });
                                    pending = Some(Command::Chats { next: false });
                                },
                                None => { pending = Some(Command::Chats { next: false }); }
                            }
                        }
                    } },
                    Command::Original { chat, key, request } => {
                        let result = client.messages(&chat).and_then(|messages| {
                            let asset = messages.into_iter().filter_map(|m| m.image).find(|asset| asset.key() == key).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "photo is no longer loaded"))?;
                            client.original_image(&asset)
                        });
                        match result { Ok(pixels) => publish(Update::Original { request, pixels }), Err(_) => publish(Update::OriginalFailed { request }) }
                    }
                    command @ (Command::Reconnect | Command::Reauthenticate) => {
                        sync_epoch = sync_epoch.wrapping_add(1);
                        drop(sync_guard.take());
                        if matches!(command, Command::Reauthenticate) && !reauthenticate(&paths, &cancel, &pairing_stop, &publish) { sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch)); continue; }
                        let Some(reconnected) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else { break; };
                        client = reconnected;
                        sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch));
                        jobs.clear(); failed_images.clear();
                        publish(Update::Connected(client.account_name.clone()));
                        pending = Some(Command::Chats { next: false });
                    }

                    Command::ChatDetails => match client.chat_details() {
                        Ok(chats) => publish(Update::Chats { chats: chats.into_iter().map(|c| chat_view(c, &mut jobs)).collect(), more: client.more_chats }),
                        Err(_) => publish(Update::ChatsFailed),
                    },
                    Command::Friends => match client.friends() {
                        Ok(friends) => publish(Update::Friends(friends.into_iter().map(|friend| FriendView {
                            id: friend.id, name: friend.name, status: friend.status, group: friend.group, member_count: friend.member_count,
                            avatar_key: asset_key(friend.avatar, &mut jobs),
                        }).collect())),
                        Err(_) => publish(Update::FriendsFailed),
                    },
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
                            pending = Some(Command::ChatDetails);
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
                        Ok(messages) => {
                            pending = Some(Command::Receipts { chat: id.clone() });
                            publish(Update::Messages {
                                id, generation, messages: messages.into_iter().map(|m| message_view(m, &mut jobs)).collect(),
                            });
                        },
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
                        Err(error) => match send_failure(&error) {
                            SendFailure::Rejected => publish(Update::SendRejected { id, local_id, reason: "Message was not accepted. You can retry.".into() }),
                            SendFailure::Uncertain => publish(Update::SendUncertain { id, local_id }),
                        },
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
                if let Some(wait) = markers.wait(Instant::now()) {
                    match rx.recv_timeout(wait) {
                        Ok(command) => pending = Some(command),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(command) => pending = Some(command),
                        Err(_) => break,
                    }
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

struct SyncGuard {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for SyncGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn start_sync(
    paths: &Paths,
    tx: &SyncSender<Command>,
    global_stop: &Arc<AtomicBool>,
    epoch: u64,
) -> SyncGuard {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let global_stop = global_stop.clone();
    let paths = paths.clone();
    let tx = tx.clone();
    let thread = std::thread::spawn(move || {
        let publish = |event| {
            let mut command = Command::Push { epoch, event };
            loop {
                if thread_stop.load(Ordering::Relaxed) || global_stop.load(Ordering::Relaxed) {
                    return false;
                }
                match tx.try_send(command) {
                    Ok(()) => return true,
                    Err(mpsc::TrySendError::Full(value)) => {
                        command = value;
                        std::thread::park_timeout(Duration::from_millis(20));
                    }
                    Err(_) => return false,
                }
            }
        };
        if sync::receive(&paths.state, &paths.wasm, &thread_stop, publish).is_err()
            && !thread_stop.load(Ordering::Relaxed)
        {
            let _ = tx.try_send(Command::Push {
                epoch,
                event: sync::Event::State(SyncState::Offline),
            });
        }
    });
    SyncGuard {
        stop,
        thread: Some(thread),
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
        match rx.recv() {
            Ok(Command::Reconnect) => attempt = true,
            Ok(Command::Reauthenticate) => {
                if reauthenticate(paths, stop, pairing_cancel, publish) {
                    attempt = true;
                }
            }
            Ok(Command::Original { request, .. }) => publish(Update::OriginalFailed { request }),
            Ok(Command::Send { id, local_id, .. }) => publish(Update::SendFailed { id, local_id }),
            Ok(Command::Stop) | Err(_) => return None,
            Ok(_) => {}
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
        unread: chat.unread,
        muted: chat.muted,
        last_read_message: chat.last_read_message,
        member_count: chat.member_count,
        outgoing_preview: chat.outgoing_preview,
        group: chat.group,
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
        delivered_time: message.delivered_time,
        outgoing: message.outgoing,
        image_key: asset_key(message.image, jobs),
        rich: message.rich,
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
