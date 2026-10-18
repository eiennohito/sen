//! Account RPCs stay on one worker; bounded asset fetchers own downloads and decoding.
use line_wasm_poc::client::desktop::{
    Chat, Client, Message, SendFailure,
    assets::{Asset, Completion, Failure, Failures, Payload, Pixels, Priority, Purpose},
    send_failure,
    sync::{self, SyncState},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionFailure {
    Network,
    Session,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListRequest {
    #[default]
    Refresh,
    More,
    Details,
}
impl ListRequest {
    pub fn command(self) -> Command {
        match self {
            Self::Refresh => Command::Chats { next: false },
            Self::More => Command::Chats { next: true },
            Self::Details => Command::ChatDetails,
        }
    }
}

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
    ReadActivity {
        allowed: bool,
    },
    SaveFile {
        chat: String,
        message_id: String,
        request: u64,
    },
    RetainSendRequests {
        request_seqs: Vec<u32>,
    },
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
        eligibility_epoch: u64,
    },
    MarkReadExplicit {
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
    RetryImage {
        key: String,
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
        request_seq: u32,
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
    pub last_message_id: Option<String>,
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
pub struct EmojiView {
    pub span: line_wasm_poc::client::desktop::rich::InlineEmoji,
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
    pub decryption_failed: bool,
    pub emoji: Vec<EmojiView>,
    pub mentioned: bool,
    pub skip_badge: bool,
}
#[allow(dead_code)]
pub enum Update {
    FileBytes {
        request: u64,
        name: String,
        bytes: Arc<Vec<u8>>,
    },
    FileFailed {
        request: u64,
        failure: Failure,
    },
    Older {
        chat: String,
        before_message: String,
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
    OwnEcho {
        chat: String,
        request_seq: u32,
        server_id: String,
    },
    Withdrawn {
        chat: String,
        message_id: String,
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
        failure: Failure,
    },
    ConnectionFailed(ConnectionFailure),
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
    ChatsFailed(ListRequest),
    ImageFailed {
        key: String,
        failure: Failure,
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

#[derive(Default)]
pub struct ReadGate {
    allowed: AtomicBool,
    epoch: AtomicU64,
}
impl ReadGate {
    pub fn revoke(&self) {
        self.allowed.store(false, Ordering::SeqCst);
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn allow(&self) {
        self.allowed.store(true, Ordering::SeqCst);
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.allowed.load(Ordering::SeqCst) && self.epoch() == epoch
    }
}

#[derive(Clone)]
pub struct CommandSender {
    tx: SyncSender<Command>,
    wake: Option<std::thread::Thread>,
}
impl CommandSender {
    pub fn try_send(&self, command: Command) -> Result<(), mpsc::TrySendError<Command>> {
        self.tx.try_send(command)?;
        if let Some(wake) = &self.wake {
            wake.unpark();
        }
        Ok(())
    }
}
#[cfg(test)]
impl From<SyncSender<Command>> for CommandSender {
    fn from(tx: SyncSender<Command>) -> Self {
        Self { tx, wake: None }
    }
}

enum AssetEvent {
    Asset {
        epoch: u64,
        completion: Completion,
    },
    MediaToken {
        epoch: u64,
        asset: Asset,
        reply: SyncSender<Result<Option<String>, Failure>>,
    },
}

#[derive(Clone)]
struct AssetSender {
    tx: SyncSender<AssetEvent>,
    wake: std::thread::Thread,
}
impl AssetSender {
    fn send(&self, mut event: AssetEvent, stop: &AtomicBool, global_stop: &AtomicBool) -> bool {
        loop {
            if stop.load(Ordering::Relaxed) || global_stop.load(Ordering::Relaxed) {
                return false;
            }
            match self.tx.try_send(event) {
                Ok(()) => {
                    self.wake.unpark();
                    return true;
                }
                Err(mpsc::TrySendError::Full(value)) => {
                    event = value;
                    std::thread::park_timeout(Duration::from_millis(20));
                }
                Err(_) => return false,
            }
        }
    }
}

struct Inbox {
    rx: mpsc::Receiver<Command>,
    asset_rx: mpsc::Receiver<AssetEvent>,
    asset_tx: AssetSender,
    push_tx: CommandSender,
    cancel: Arc<AtomicBool>,
    pairing_stop: Arc<AtomicBool>,
    exposure_gate: Arc<ReadGate>,
    fetch_priority: Arc<Priority>,
}

enum Work {
    Command(Command),
    Asset(AssetEvent),
}
fn next_work(
    rx: &mpsc::Receiver<Command>,
    assets: &mpsc::Receiver<AssetEvent>,
    pending: &mut Option<Command>,
) -> Option<Work> {
    rx.try_recv()
        .ok()
        .or_else(|| pending.take())
        .map(Work::Command)
        .or_else(|| assets.try_recv().ok().map(Work::Asset))
}

pub struct Worker {
    pub commands: CommandSender,
    stop: Arc<AtomicBool>,
    pub pairing_cancel: Arc<AtomicBool>,
    pub read_gate: Arc<ReadGate>,
    pub asset_priority: Arc<Priority>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.commands.try_send(Command::Stop);
        if let Some(wake) = &self.commands.wake {
            wake.unpark();
        }
        self.pairing_cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Worker {
    fn spawn(run: impl FnOnce(Inbox) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        let (asset_tx, asset_rx) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = stop.clone();
        let pairing_cancel = Arc::new(AtomicBool::new(false));
        let pairing_stop = pairing_cancel.clone();
        let read_gate = Arc::new(ReadGate::default());
        let exposure_gate = read_gate.clone();
        let asset_priority = Arc::new(Priority::default());
        let fetch_priority = asset_priority.clone();
        let push_tx = tx.clone();
        let thread = std::thread::spawn(move || {
            let wake = std::thread::current();
            run(Inbox {
                rx,
                asset_rx,
                asset_tx: AssetSender {
                    tx: asset_tx,
                    wake: wake.clone(),
                },
                push_tx: CommandSender {
                    tx: push_tx,
                    wake: Some(wake),
                },
                cancel,
                pairing_stop,
                exposure_gate,
                fetch_priority,
            });
        });
        Self {
            commands: CommandSender {
                tx,
                wake: Some(thread.thread().clone()),
            },
            stop,
            pairing_cancel,
            read_gate,
            asset_priority,
            thread: Some(thread),
        }
    }
}

pub fn start_demo(
    failure: Arc<std::sync::atomic::AtomicU8>,
    publish: impl Fn(Update) + Send + 'static,
) -> Worker {
    let now = crate::present::fixed_now();
    Worker::spawn(move |inbox| {
        while let Ok(command) = inbox.rx.recv() {
            if inbox.cancel.load(Ordering::Relaxed) || matches!(command, Command::Stop) {
                break;
            }
            let Command::Send {
                id,
                text,
                local_id,
                request_seq,
                ..
            } = command
            else {
                continue;
            };
            let outcome = failure.swap(0, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(if outcome == 0 { 120 } else { 1000 }));
            match outcome {
                1 => publish(Update::SendUncertain { id, local_id }),
                2 => publish(Update::SendRejected {
                    id,
                    local_id,
                    reason: "Message was not accepted. You can retry.".into(),
                }),
                _ => {
                    let timestamp = now.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
                    let server_id = format!("demo-{request_seq}");
                    publish(Update::Sent {
                        id: id.clone(),
                        local_id,
                        server_id: server_id.clone(),
                    });
                    publish(Update::Incoming {
                        chat: id,
                        messages: vec![MessageView {
                            id: server_id,
                            body: text,
                            timestamp,
                            delivered_time: timestamp,
                            outgoing: true,
                            sender_id: "self".into(),
                            sender_name: String::new(),
                            sender_key: String::new(),
                            sticker: false,
                            photo: false,
                            image_key: String::new(),
                            reactions: Vec::new(),
                            rich: Default::default(),
                            decryption_failed: false,
                            emoji: Vec::new(),
                            mentioned: false,
                            skip_badge: false,
                        }],
                        replay: false,
                    });
                }
            }
        }
    })
}

pub fn start(paths: Paths, publish: impl Fn(Update) + Send + 'static) -> Worker {
    Worker::spawn(move |inbox| {
        let Inbox {
            rx,
            asset_rx,
            asset_tx,
            push_tx,
            cancel,
            pairing_stop,
            exposure_gate,
            fetch_priority,
        } = inbox;
        line_wasm_poc::client::desktop::set_cancellation_token(cancel.clone());
        let Some(mut client) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else {
            return;
        };
        publish(Update::Connected(client.account_name.clone()));
        let mut sync_epoch = 0;
        let mut sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch));
        let mut sync_connected = false;
        let mut marker_epochs = std::collections::HashMap::<String, u64>::new();
        let mut markers = line_wasm_poc::client::desktop::read::Markers::default();
        let mut pending = Some(Command::Chats { next: false });
        let mut jobs = Jobs::default();
        let failures = Failures::default();
        let mut fetchers = Some(start_fetchers(
            &client,
            &asset_tx,
            &cancel,
            &fetch_priority,
            &failures,
            sync_epoch,
        ));
        while !cancel.load(Ordering::Relaxed) {
            let mut command = rx.try_recv().ok().or_else(|| pending.take());

            if command.is_none()
                && sync_connected
                && exposure_gate.accepts(exposure_gate.epoch())
                && let Some((chat, up_to_message)) = markers.due(Instant::now())
            {
                if marker_epochs
                    .get(&chat)
                    .is_none_or(|epoch| !exposure_gate.accepts(*epoch))
                {
                    markers.cancel(&chat);
                    marker_epochs.remove(&chat);
                    continue;
                }
                match client.mark_read(&chat, &up_to_message) {
                    Ok(()) => {
                        markers.confirmed(&chat, &up_to_message);
                        marker_epochs.remove(&chat);
                        publish(Update::ReadMarked {
                            chat,
                            up_to_message,
                        });
                    }
                    Err(_) => publish(Update::ReadFailed { chat }),
                }
            }
            let asset_event = if command.is_none() {
                match next_work(&rx, &asset_rx, &mut pending) {
                    Some(Work::Command(next)) => {
                        command = Some(next);
                        None
                    }
                    Some(Work::Asset(event)) => Some(event),
                    None => None,
                }
            } else {
                None
            };
            if let Some(command) = command {
                // Prefer user commands over outstanding images from an old view.
                if worker_asset_command(
                    &command,
                    fetchers.as_ref().unwrap(),
                    sync_epoch,
                    &mut sync_connected,
                    &publish,
                ) {
                    continue;
                }
                match command {
                    Command::Stop => break,
                    Command::RetryImage { .. } => unreachable!(),
                    Command::ReadActivity { .. } => {},
                    Command::MarkReadExplicit { chat,up_to_message } => match client.mark_read(&chat,&up_to_message) {Ok(())=>{markers.confirmed(&chat,&up_to_message);publish(Update::ReadMarked{chat,up_to_message});},Err(_)=>publish(Update::ReadFailed{chat})},
                    Command::SaveFile { chat,message_id,request } => match client.file_asset(&chat,&message_id) {
                        Ok((name,asset)) => {
                            let accepted = fetchers.as_ref().unwrap().enqueue(asset, Some(chat), Purpose::File { request, name });
                            if !accepted { publish(Update::FileFailed { request, failure: Failure::Cancelled }); }
                        },
                        Err(error) => publish(Update::FileFailed { request, failure: Failure::from_error(&error) }),
                    },
                    Command::RetainSendRequests { request_seqs } => { let _ = client.retain_send_requests(&request_seqs); },
                    Command::Older { chat, before_message, delivered_time } => match client.older(&chat, &before_message, delivered_time) {
                        Ok((messages,reached_beginning)) => { jobs.chat = Some(chat.clone()); publish_echoes(&chat,&messages,&publish); publish(Update::Older { chat, before_message, messages: messages.into_iter().map(|m| message_view(m,&mut jobs)).collect(), reached_beginning }); },
                        Err(_) => publish(Update::OlderFailed { chat, before_message }),
                    },
                    Command::MarkRead { chat, up_to_message, eligibility_epoch } => { if exposure_gate.accepts(eligibility_epoch) {marker_epochs.insert(chat.clone(),eligibility_epoch);markers.expose(chat,up_to_message,Instant::now());} },
                    Command::Receipts { chat } => { if let Ok(ranges) = client.read_ranges(&chat) { publish(Update::ReadRanges(ranges)); } },
                    Command::Push { epoch, event } => { if epoch != sync_epoch { continue; } match event {
                        sync::Event::State(_) => unreachable!(),
                        sync::Event::Token(token) => client.replace_token(token),
                        sync::Event::Resync { complete, .. } => {
                            match client.resync() {
                                Ok((chats, pages)) => {
                                    publish(Update::Chats { chats: chats.into_iter().map(|c| chat_view(c, &mut jobs)).collect(), more: client.more_chats });
                                    for (chat, messages) in pages { jobs.chat = Some(chat.clone()); publish_echoes(&chat, &messages, &publish); publish(Update::Incoming { chat, messages: messages.into_iter().map(|m| message_view(m, &mut jobs)).collect(), replay: true }); }
                                    let _ = complete.send(true);
                                },
                                Err(_) => { let _ = complete.send(false); }
                            }
                        },
                        sync::Event::Operation(operation) => {
                            if matches!(operation["type"].as_u64(),Some(64|65)) {
                                if let (Some(chat),Some(message))=(operation["param1"].as_str(),operation["param2"].as_str()) { let _ = client.withdraw(message); publish(Update::Withdrawn { chat:chat.into(),message_id:message.into() }); }
                                pending=Some(Command::Chats{next:false});
                                continue;
                            }

                            if let Some((chat, request_seq, server_id)) = client.own_echo(&operation) { publish(Update::OwnEcho { chat, request_seq, server_id }); }

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
                                    jobs.chat = Some(chat.clone());
                                    publish(Update::Incoming { chat, messages: vec![message_view(message, &mut jobs)], replay: false });
                                    pending = Some(Command::Chats { next: false });
                                },
                                None => { pending = Some(Command::Chats { next: false }); }
                            }
                        }
                    } },
                    Command::Original { chat, key, request } => {
                        let result = client.messages(&chat).and_then(|messages| {
                            messages.into_iter().filter_map(|m| m.image).find(|asset| asset.key() == key).and_then(|asset| asset.original()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "photo is no longer loaded").into())
                        });
                        match result {
                            Ok(asset) => { if !fetchers.as_ref().unwrap().enqueue(asset, Some(chat), Purpose::Original { request }) { publish(Update::OriginalFailed { request, failure: Failure::Cancelled }); } },
                            Err(error) => publish(Update::OriginalFailed { request, failure: Failure::from_error(&error) }),
                        }
                    }
                    command @ (Command::Reconnect | Command::Reauthenticate) => {
                        sync_epoch = sync_epoch.wrapping_add(1);
                        drop(fetchers.take());
                        drop(sync_guard.take());
                        if matches!(command, Command::Reauthenticate) && !reauthenticate(&paths, &cancel, &pairing_stop, &publish) { fetchers = Some(start_fetchers(&client, &asset_tx, &cancel, &fetch_priority, &failures, sync_epoch)); sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch)); continue; }
                        let Some(reconnected) = connect(&paths, &rx, &cancel, &pairing_stop, &publish) else { break; };
                        client = reconnected;
                        sync_guard = Some(start_sync(&paths, &push_tx, &cancel, sync_epoch));
                        jobs = Jobs::default();
                        fetchers = Some(start_fetchers(&client, &asset_tx, &cancel, &fetch_priority, &failures, sync_epoch));
                        publish(Update::Connected(client.account_name.clone()));
                        pending = Some(Command::Chats { next: false });
                    }

                    Command::ChatDetails => match client.chat_details() {
                        Ok(chats) => publish(Update::Chats { chats: chats.into_iter().map(|c| chat_view(c, &mut jobs)).collect(), more: client.more_chats }),
                        Err(_) => publish(Update::ChatsFailed(ListRequest::Details)),
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
                        Err(_) => publish(Update::ChatsFailed(if next { ListRequest::More } else { ListRequest::Refresh })),
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
                        Ok(messages) => {
                            jobs.chat = Some(id.clone());
                            publish_echoes(&id, &messages, &publish);
                            pending = Some(Command::Receipts { chat: id.clone() });
                            publish(Update::Messages {
                                id, generation, messages: messages.into_iter().map(|m| message_view(m, &mut jobs)).collect(),
                            });
                        },
                        Err(_) => publish(Update::Error {
                            message: "Couldn’t get recent messages. Messages already shown are still available. Retry.".into(),
                            generation: Some(generation),
                        }),
                    },
                    Command::Send {
                        request_seq,
                        id,
                        generation,
                        text,
                        local_id,
                    } => match client.send_text_with_request(&id, &text, request_seq) {
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
                for (asset, chat) in jobs.entries.drain(..) {
                    queue_thumbnail(fetchers.as_ref().unwrap(), asset, chat, &publish);
                }
                jobs.chat = None;
            } else if let Some(event) = asset_event {
                match event {
                    AssetEvent::Asset { epoch, completion } => {
                        if epoch == sync_epoch {
                            publish_asset(completion, &publish);
                        }
                    }
                    AssetEvent::MediaToken {
                        epoch,
                        asset,
                        reply,
                    } => {
                        let token = if epoch == sync_epoch {
                            client
                                .media_token(&asset)
                                .map_err(|error| Failure::from_error(&error))
                        } else {
                            Err(Failure::Cancelled)
                        };
                        let _ = reply.try_send(token);
                    }
                }
            } else if let Some(wait) = (sync_connected
                && exposure_gate.accepts(exposure_gate.epoch()))
            .then(|| markers.wait(Instant::now()))
            .flatten()
            {
                std::thread::park_timeout(wait);
            } else {
                std::thread::park();
            }
        }
    })
}

trait AssetQueue {
    fn retry(&self, key: &str) -> bool;
    fn failure(&self, key: &str) -> Option<Failure>;
    fn reconnected(&self);
    fn thumbnail(&self, asset: Asset, chat: Option<String>) -> bool;
}
impl AssetQueue for line_wasm_poc::client::desktop::assets::Fetchers {
    fn retry(&self, key: &str) -> bool {
        self.retry(key)
    }
    fn failure(&self, key: &str) -> Option<Failure> {
        self.failure(key)
    }
    fn reconnected(&self) {
        self.reconnected();
    }
    fn thumbnail(&self, asset: Asset, chat: Option<String>) -> bool {
        self.enqueue(asset, chat, Purpose::Thumbnail)
    }
}

fn worker_asset_command(
    command: &Command,
    fetchers: &impl AssetQueue,
    epoch: u64,
    connected: &mut bool,
    publish: &impl Fn(Update),
) -> bool {
    match command {
        Command::RetryImage { key } => {
            if !fetchers.retry(key)
                && let Some(failure) = fetchers.failure(key)
            {
                publish(Update::ImageFailed {
                    key: key.clone(),
                    failure,
                });
            }
        }
        Command::Push {
            epoch: event_epoch,
            event: sync::Event::State(state),
        } => {
            if *event_epoch == epoch {
                let online = matches!(state, SyncState::Connected);
                if online && !*connected {
                    fetchers.reconnected();
                }
                *connected = online;
                publish(Update::Sync(state.clone()));
            }
        }
        _ => return false,
    }
    true
}

fn queue_thumbnail(
    fetchers: &impl AssetQueue,
    asset: Asset,
    chat: Option<String>,
    publish: &impl Fn(Update),
) {
    let key = asset.key();
    if !fetchers.thumbnail(asset, chat) {
        let failure = fetchers.failure(&key).unwrap_or(Failure::Network);
        publish(Update::ImageFailed { key, failure });
    }
}

fn start_fetchers(
    client: &Client,
    tx: &AssetSender,
    cancel: &Arc<AtomicBool>,
    priority: &Arc<Priority>,
    failures: &Failures,
    epoch: u64,
) -> line_wasm_poc::client::desktop::assets::Fetchers {
    let tx = tx.clone();
    let auth_tx = tx.clone();
    let global_stop = cancel.clone();
    let auth_stop = cancel.clone();
    client.asset_fetchers(
        cancel.clone(),
        priority.clone(),
        failures.clone(),
        move |asset, stop| {
            let (reply, rx) = mpsc::sync_channel(1);
            if !auth_tx.send(
                AssetEvent::MediaToken {
                    epoch,
                    asset: asset.clone(),
                    reply,
                },
                stop,
                &auth_stop,
            ) {
                return Err(Failure::Cancelled);
            }
            loop {
                if stop.load(Ordering::Relaxed) || auth_stop.load(Ordering::Relaxed) {
                    return Err(Failure::Cancelled);
                }
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(token) => return token,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::Cancelled),
                }
            }
        },
        move |completion, stop| {
            tx.send(AssetEvent::Asset { epoch, completion }, stop, &global_stop);
        },
    )
}

pub(crate) fn publish_asset(completion: Completion, publish: &impl Fn(Update)) {
    let Completion {
        key,
        purpose,
        result,
    } = completion;
    match (purpose, result) {
        (Purpose::Thumbnail, Ok(Payload::Image(pixels))) => publish(Update::Image { key, pixels }),
        (Purpose::Original { request }, Ok(Payload::Image(pixels))) => {
            publish(Update::Original { request, pixels })
        }
        (Purpose::File { request, name }, Ok(Payload::File(bytes))) => publish(Update::FileBytes {
            request,
            name,
            bytes,
        }),
        (purpose, Err(failure)) => match purpose {
            Purpose::Thumbnail => publish(Update::ImageFailed { key, failure }),
            Purpose::Original { request } => publish(Update::OriginalFailed { request, failure }),
            Purpose::File { request, .. } => publish(Update::FileFailed { request, failure }),
        },
        (purpose, Ok(_)) => publish_asset(
            Completion {
                key,
                purpose,
                result: Err(Failure::Decode),
            },
            publish,
        ),
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
    tx: &CommandSender,
    global_stop: &Arc<AtomicBool>,
    epoch: u64,
) -> SyncGuard {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let global_stop = global_stop.clone();
    let paths = paths.clone();
    let tx = tx.clone();
    let thread = std::thread::spawn(move || {
        line_wasm_poc::client::desktop::set_cancellation_token(thread_stop.clone());
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
            publish(Update::ConnectionFailed(ConnectionFailure::Session));
            attempt = false;
        }
        match rx.recv() {
            Ok(Command::Reconnect) => attempt = true,
            Ok(Command::Reauthenticate) => {
                if reauthenticate(paths, stop, pairing_cancel, publish) {
                    attempt = true;
                }
            }
            Ok(Command::Original { request, .. }) => publish(Update::OriginalFailed {
                request,
                failure: Failure::Network,
            }),
            Ok(Command::Send { id, local_id, .. }) => publish(Update::SendRejected {
                id,
                local_id,
                reason: "Connection is unavailable. Try again after reconnecting.".into(),
            }),
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

fn publish_echoes(chat: &str, messages: &[Message], publish: &impl Fn(Update)) {
    for message in messages.iter().filter(|m| m.outgoing) {
        if let Some(request_seq) = message.request_seq {
            publish(Update::OwnEcho {
                chat: chat.into(),
                request_seq,
                server_id: message.id.clone(),
            });
        }
    }
}

#[derive(Default)]
struct Jobs {
    chat: Option<String>,
    entries: Vec<(Asset, Option<String>)>,
}
fn asset_key(asset: Option<Asset>, jobs: &mut Jobs) -> String {
    if let Some(asset) = asset {
        let key = asset.key();
        jobs.entries.push((asset, jobs.chat.clone()));
        key
    } else {
        String::new()
    }
}
fn chat_view(chat: Chat, jobs: &mut Jobs) -> ChatView {
    ChatView {
        id: chat.id,
        title: chat.title,
        preview: chat.preview,
        timestamp: chat.timestamp_ms,
        avatar_key: asset_key(chat.avatar, jobs),
        unread: chat.unread,
        muted: chat.muted,
        last_read_message: chat.last_read_message,
        last_message_id: chat.last_message_id,
        member_count: chat.member_count,
        outgoing_preview: chat.outgoing_preview,
        group: chat.group,
    }
}
fn message_view(message: Message, jobs: &mut Jobs) -> MessageView {
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
        emoji: message
            .rich
            .emoji
            .iter()
            .map(|span| EmojiView {
                span: span.clone(),
                image_key: asset_key(Asset::inline_emoji(span), jobs),
            })
            .collect(),
        rich: message.rich,
        decryption_failed: message.decryption_failed,
        mentioned: message.mentioned,
        // Legacy cached text stays visible offline, but unknown unread metadata
        // must not participate in exposure/counting. Never persist this UI flag.
        skip_badge: message.skip_badge || message.metadata_version < 2,
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
    fn asset_backpressure_preserves_worker_command_capacity_during_an_rpc() {
        let (ready, rpc_entered) = mpsc::channel();
        let (finish_rpc, rpc) = mpsc::channel();
        let (processed, commands_done) = mpsc::channel();
        let worker = Worker::spawn(move |inbox| {
            ready
                .send((inbox.asset_tx.clone(), inbox.cancel.clone()))
                .unwrap();
            rpc.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut pending = None;
            for index in 0..8 {
                let Some(Work::Command(command)) =
                    next_work(&inbox.rx, &inbox.asset_rx, &mut pending)
                else {
                    panic!("asset traffic ran ahead of UI/sync commands");
                };
                match (index, command) {
                    (0, Command::Send { .. })
                    | (1, Command::Push { .. })
                    | (2..=7, Command::ReadActivity { .. }) => {}
                    _ => panic!("worker command order changed"),
                }
            }
            assert!(matches!(
                next_work(&inbox.rx, &inbox.asset_rx, &mut pending),
                Some(Work::Asset(_))
            ));
            processed.send(()).unwrap();
        });
        let (assets, cancel) = rpc_entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let (delivered, backlog) = mpsc::channel();
        let producers: Vec<_> = (0..3)
            .map(|producer| {
                let assets = assets.clone();
                let cancel = cancel.clone();
                let delivered = delivered.clone();
                std::thread::spawn(move || {
                    let stop = AtomicBool::new(false);
                    for index in 0..64 {
                        let event = if index == 0 {
                            let (reply, _rx) = mpsc::sync_channel(1);
                            AssetEvent::MediaToken {
                                epoch: 0,
                                asset: Asset::attachment(
                                    &serde_json::json!({"id":"42","contentType":14}),
                                )
                                .unwrap(),
                                reply,
                            }
                        } else {
                            AssetEvent::Asset {
                                epoch: 0,
                                completion: Completion {
                                    key: format!("{producer}-{index}"),
                                    purpose: Purpose::Thumbnail,
                                    result: Ok(Payload::Image(Arc::new(Pixels {
                                        width: 1,
                                        height: 1,
                                        rgba: vec![0; 4],
                                    }))),
                                },
                            }
                        };
                        if !assets.send(event, &stop, &cancel) {
                            break;
                        }
                        delivered.send(()).unwrap();
                    }
                })
            })
            .collect();
        for _ in 0..8 {
            backlog.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        assert!(
            worker
                .commands
                .try_send(Command::Send {
                    request_seq: 1,
                    id: "chat".into(),
                    generation: 0,
                    text: "synthetic".into(),
                    local_id: "local".into(),
                })
                .is_ok()
        );
        assert!(
            worker
                .commands
                .try_send(Command::Push {
                    epoch: 0,
                    event: sync::Event::State(SyncState::Connected)
                })
                .is_ok()
        );
        for _ in 0..6 {
            assert!(
                worker
                    .commands
                    .try_send(Command::ReadActivity { allowed: false })
                    .is_ok()
            );
        }
        assert!(matches!(
            worker.commands.try_send(Command::Stop),
            Err(mpsc::TrySendError::Full(_))
        ));
        finish_rpc.send(()).unwrap();
        commands_done.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(worker);
        for producer in producers {
            producer.join().unwrap();
        }
    }

    #[test]
    fn ui_and_asset_senders_wake_the_idle_worker() {
        let (ready, started) = mpsc::channel();
        let (handled, events) = mpsc::channel();
        let worker = Worker::spawn(move |inbox| {
            ready.send(inbox.asset_tx.clone()).unwrap();
            let mut pending = None;
            let mut remaining = 2;
            while remaining > 0 {
                if let Some(work) = next_work(&inbox.rx, &inbox.asset_rx, &mut pending) {
                    handled.send(matches!(work, Work::Command(_))).unwrap();
                    remaining -= 1;
                } else {
                    std::thread::park();
                }
            }
        });
        let assets = started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            worker
                .commands
                .try_send(Command::ReadActivity { allowed: false })
                .is_ok()
        );
        assert!(events.recv_timeout(Duration::from_secs(1)).unwrap());
        assert!(assets.send(
            AssetEvent::Asset {
                epoch: 0,
                completion: Completion {
                    key: "synthetic".into(),
                    purpose: Purpose::Thumbnail,
                    result: Err(Failure::Network)
                }
            },
            &worker.stop,
            &worker.stop
        ));
        assert!(!events.recv_timeout(Duration::from_secs(1)).unwrap());
    }

    #[test]
    fn legacy_cached_text_remains_visible_without_assuming_unread_metadata() {
        let mut message: Message = serde_json::from_value(serde_json::json!({
            "id":"123", "body":"cached text", "timestamp_ms":1,
            "outgoing":false, "image":null,"reactions":[],"encrypted":false,
            "decryption_failed":false
        }))
        .unwrap();
        let legacy = message_view(message.clone(), &mut Jobs::default());
        assert_eq!(legacy.body, "cached text");
        assert!(legacy.skip_badge);
        assert!(!message.skip_badge);
        message.metadata_version = 2;
        assert!(!message_view(message, &mut Jobs::default()).skip_badge);
    }

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
            Update::ConnectionFailed(ConnectionFailure::Session)
        ));
        assert!(worker.commands.try_send(Command::Reconnect).is_ok());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Update::ConnectionFailed(ConnectionFailure::Session)
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
    #[test]
    fn read_revocation_is_immediate_even_when_command_queue_is_full() {
        let gate = ReadGate::default();
        gate.allow();
        let old = gate.epoch();
        let (tx, _rx) = mpsc::sync_channel(1);
        assert!(tx.try_send(Command::Stop).is_ok());
        gate.revoke();
        assert!(
            tx.try_send(Command::ReadActivity { allowed: false })
                .is_err()
        );
        assert!(!gate.accepts(old));
        gate.allow();
        assert!(!gate.accepts(old));
        assert!(gate.accepts(gate.epoch()));
    }
}

#[cfg(test)]
mod media_worker_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Assets {
        failures: Mutex<std::collections::HashMap<String, Failure>>,
        reconnects: std::sync::atomic::AtomicUsize,
    }
    impl AssetQueue for Assets {
        fn retry(&self, key: &str) -> bool {
            if key == "racing" {
                self.failures
                    .lock()
                    .unwrap()
                    .insert(key.into(), Failure::Http(404));
            }
            false
        }
        fn failure(&self, key: &str) -> Option<Failure> {
            self.failures.lock().unwrap().get(key).copied()
        }
        fn reconnected(&self) {
            self.reconnects.fetch_add(1, Ordering::Relaxed);
        }
        fn thumbnail(&self, _asset: Asset, _chat: Option<String>) -> bool {
            false
        }
    }

    #[test]
    fn worker_retry_preserves_current_class_and_ignores_missing_records() {
        let mut worker = Worker::spawn(move |inbox| {
            let assets = Assets::default();
            assets
                .failures
                .lock()
                .unwrap()
                .insert("permanent".into(), Failure::Decode);
            assets
                .failures
                .lock()
                .unwrap()
                .insert("network".into(), Failure::Network);
            let events = std::cell::RefCell::new(Vec::new());
            let mut connected = false;
            for _ in 0..4 {
                let command = inbox.rx.recv().unwrap();
                assert!(worker_asset_command(
                    &command,
                    &assets,
                    2,
                    &mut connected,
                    &|event| events.borrow_mut().push(event)
                ));
            }
            let found: Vec<_> = events
                .into_inner()
                .into_iter()
                .map(|event| match event {
                    Update::ImageFailed { key, failure } => (key, failure),
                    _ => panic!("unexpected worker event"),
                })
                .collect();
            assert_eq!(
                found,
                [
                    ("permanent".into(), Failure::Decode),
                    ("network".into(), Failure::Network),
                    ("racing".into(), Failure::Http(404))
                ]
            );
        });
        for key in ["permanent", "network", "missing", "racing"] {
            worker
                .commands
                .try_send(Command::RetryImage { key: key.into() })
                .unwrap();
        }
        worker.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn worker_connected_dispatch_retries_only_on_current_epoch_transitions() {
        let mut worker = Worker::spawn(move |inbox| {
            let assets = Assets::default();
            let mut connected = false;
            for expected in [0, 1, 1, 1, 2] {
                let command = inbox.rx.recv().unwrap();
                assert!(worker_asset_command(
                    &command,
                    &assets,
                    2,
                    &mut connected,
                    &|_| {}
                ));
                assert_eq!(assets.reconnects.load(Ordering::Relaxed), expected);
            }
        });
        for (epoch, state) in [
            (1, SyncState::Connected),
            (2, SyncState::Connected),
            (2, SyncState::Connected),
            (2, SyncState::Offline),
            (2, SyncState::Connected),
        ] {
            worker
                .commands
                .try_send(Command::Push {
                    epoch,
                    event: sync::Event::State(state),
                })
                .unwrap();
        }
        worker.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn refused_visible_thumbnail_reports_a_recoverable_failure() {
        let assets = Assets::default();
        let asset = Asset::attachment(&serde_json::json!({"id":"42", "contentType":14})).unwrap();
        let key = asset.key();
        let events = std::cell::RefCell::new(Vec::new());
        queue_thumbnail(&assets, asset, Some("open".into()), &|event| {
            events.borrow_mut().push(event)
        });
        assert!(
            matches!(events.borrow_mut().pop(), Some(Update::ImageFailed { key: found, failure: Failure::Network }) if found == key)
        );
    }
}
