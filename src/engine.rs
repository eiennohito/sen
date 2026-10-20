//! A store thread schedules work; a network thread owns the backend.
use crate::client::http::CancelToken;
use crate::client::{NetworkActivity, operation::OperationType};
use crate::{backend::*, store::*};
use serde_json::Value;
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Fatal,
    Local,
    Connecting,
    Online,
    Offline,
    SignedOut { reason: SignOutReason },
    SigningIn,
}
struct Health {
    failed_since: Option<Instant>,
}
impl Health {
    fn new() -> Self {
        Self { failed_since: None }
    }
    fn success(&mut self) {
        self.failed_since = None;
    }
    fn failure(&mut self, now: Instant) {
        self.failed_since.get_or_insert(now);
    }
    fn offline(&self, now: Instant, threshold: Duration) -> bool {
        self.failed_since
            .is_some_and(|since| now.saturating_duration_since(since) >= threshold)
    }
}
/// Account facts have provenance; Connection is only the presentation/transport status.
#[derive(Clone, Debug)]
enum AccountSession {
    Opening {
        epoch: u64,
    },
    NoSession,
    SessionUnusable {
        reason: SignOutReason,
    },
    Ready,
    SigningIn {
        epoch: u64,
        previous: Box<AccountSession>,
    },
}
impl AccountSession {
    fn signing_in(&self) -> bool {
        matches!(self, Self::SigningIn { .. })
    }
}
/// Retry policy and health clock, injectable for offline tests.
#[derive(Clone)]
pub struct Timing {
    pub retry_initial: Duration,
    pub retry_max: Duration,
    pub offline_after: Duration,
    pub health_now: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            retry_initial: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            offline_after: Duration::from_secs(15),
            health_now: Arc::new(Instant::now),
        }
    }
}
impl Timing {
    fn delay(&self, attempt: u32) -> Duration {
        self.retry_initial
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1).min(31)))
            .min(self.retry_max)
    }
}
#[derive(Clone, Debug)]
pub struct WindowSub {
    pub sub: u64,
    pub chat: ChatId,
    pub anchor: Anchor,
    pub before: usize,
    pub after: usize,
}
impl WindowSub {
    pub fn unread(sub: u64, chat: ChatId) -> Self {
        Self {
            sub,
            chat,
            anchor: Anchor::Unread,
            before: 80,
            after: 80,
        }
    }
}
#[derive(Clone)]
pub enum Intent {
    ChatList {
        sub: u64,
        limit: usize,
    },
    Window(WindowSub),
    MoreChats,
    SignIn,
    CancelSignIn,
    Friends,
    Contact(String),
    GroupInfo(ChatId),
    ProfileAction(ProfileAction),
    Members(ChatId),
    Asset(crate::client::desktop::assets::Asset),
    Original(crate::client::desktop::assets::Asset),
    File {
        asset: crate::client::desktop::assets::Asset,
        name: String,
    },
    RetryAsset(String),
    Page {
        chat: ChatId,
        kind: PageKind,
    },
    Send {
        chat: ChatId,
        text: String,
        reply: Option<ReplyReference>,
    },
    React {
        key: Key,
        kind: u32,
    },
    StickerPackages,
    SendAttachments {
        chat: ChatId,
        items: Vec<crate::attachments::Attachment>,
    },
    SendSticker {
        chat: ChatId,
        sticker: Box<Content>,
        reply: Option<ReplyReference>,
    },
    CreateChat {
        name: String,
        targets: Vec<String>,
    },
    Unsend(Key),
    Retry(Key),
    Discard(Key),
    Copy(Key),
    Draft {
        chat: ChatId,
        draft: Draft,
    },
    Prefs(Prefs),
    MarkRead {
        chat: ChatId,
        id: ServerId,
    },
    Stop,
}
impl Intent {
    fn name(&self) -> &'static str {
        match self {
            Self::ChatList { .. } => "chat-list",
            Self::Window(_) => "window",
            Self::MoreChats => "more-chats",
            Self::SignIn => "sign-in",
            Self::CancelSignIn => "cancel-sign-in",
            Self::Friends => "friends",
            Self::Contact(_) => "contact",
            Self::GroupInfo(_) => "group-info",
            Self::ProfileAction(_) => "profile-action",
            Self::Members(_) => "members",
            Self::Asset(_) => "asset",
            Self::Original(_) => "original",
            Self::File { .. } => "file",
            Self::RetryAsset(_) => "retry-asset",
            Self::Page { .. } => "page",
            Self::Send { .. } => "send",
            Self::SendAttachments { .. } => "send-attachments",
            Self::React { .. } => "react",
            Self::StickerPackages => "sticker-packages",
            Self::SendSticker { .. } => "send-sticker",
            Self::CreateChat { .. } => "create-chat",
            Self::Unsend(_) => "unsend",
            Self::Retry(_) => "retry",
            Self::Discard(_) => "discard",
            Self::Copy(_) => "copy",
            Self::Draft { .. } => "draft",
            Self::Prefs(_) => "prefs",
            Self::MarkRead { .. } => "mark-read",
            Self::Stop => "stop",
        }
    }
}
fn account_allows(intent: &Intent, ready: bool, known: bool) -> bool {
    match intent {
        Intent::SignIn
        | Intent::CancelSignIn
        | Intent::Prefs(_)
        | Intent::ChatList { .. }
        | Intent::Friends
        | Intent::Contact(_)
        | Intent::GroupInfo(_)
        | Intent::Members(_)
        | Intent::Window(_)
        | Intent::Page { .. }
        | Intent::Asset(_)
        | Intent::Original(_)
        | Intent::File { .. }
        | Intent::Draft { .. }
        | Intent::Copy(_)
        | Intent::Discard(_)
        | Intent::MoreChats
        | Intent::Stop => true,
        Intent::ProfileAction(_)
        | Intent::RetryAsset(_)
        | Intent::React { .. }
        | Intent::StickerPackages
        | Intent::CreateChat { .. }
        | Intent::Unsend(_)
        | Intent::Retry(_)
        | Intent::MarkRead { .. } => known,
        Intent::Send { .. } | Intent::SendAttachments { .. } | Intent::SendSticker { .. } => ready,
    }
}
#[derive(Clone, Debug)]
pub enum CreateFailure {
    InvalidSelection,
    Local,
    Rejected,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageFailure {
    Full,
    ReadOnly,
    Permission,
    Io,
}
impl StorageFailure {
    pub fn classify(error: &StoreError) -> Option<Self> {
        use rusqlite::ErrorCode;
        match error {
            StoreError::Sql(e) => match e.sqlite_error_code()? {
                ErrorCode::DiskFull => Some(Self::Full),
                ErrorCode::ReadOnly => Some(Self::ReadOnly),
                ErrorCode::PermissionDenied => Some(Self::Permission),
                ErrorCode::SystemIoFailure | ErrorCode::CannotOpen => Some(Self::Io),
                _ => None,
            },
            StoreError::Io(e) => Some(if e.raw_os_error() == Some(libc::ENOSPC) {
                Self::Full
            } else if e.raw_os_error() == Some(libc::EROFS) {
                Self::ReadOnly
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                Self::Permission
            } else {
                Self::Io
            }),
            _ => None,
        }
    }
}
#[derive(Clone)]
pub enum Delta {
    UploadFinished(Key),
    UploadProgress {
        key: Key,
        fraction: f32,
    },
    ReactionFailed(Key),
    StickerPackages(Vec<StickerPackage>),
    StickerPackagesFailed,
    CreatedChat(std::result::Result<ChatId, CreateFailure>),
    Fatal,
    Reset,
    Pair(PairEvent),
    PairFailed {
        reason: crate::client::error::SignInFailure,
    },
    Submitted {
        chat: ChatId,
        text: String,
    },
    Connection(Connection),
    Profile(crate::store::Profile),
    Contact(crate::store::Friend),
    Notice {
        mid: String,
        data: Value,
    },
    GroupInfo {
        chat: ChatId,
        data: Value,
    },
    ProfileAction {
        action: ProfileAction,
        error: bool,
    },
    MoreChats(bool),
    RefreshFailed {
        request: Refresh,
        error: Failure,
    },
    RefreshRecovered(Refresh),
    Notify(crate::store::MessageInput),
    Contacts(Vec<crate::store::Friend>),
    Members {
        chat: ChatId,
        rows: Vec<crate::store::Member>,
    },
    AssetEvicted {
        key: String,
    },
    MediaFailed {
        key: String,
        failure: crate::client::desktop::assets::Failure,
    },
    File {
        name: String,
        bytes: Arc<Vec<u8>>,
    },
    Original {
        key: String,
        path: std::path::PathBuf,
        source: std::path::PathBuf,
    },
    Asset {
        key: String,
        path: std::path::PathBuf,
    },
    Chats {
        sub: u64,
        upsert: Vec<ChatListItem>,
        remove: Vec<ChatId>,
        order: Vec<ChatId>,
    },
    Window {
        sub: u64,
        upsert: Vec<StoredMessage>,
        remove: Vec<Key>,
        gaps: Vec<Gap>,
        reaches_start: bool,
        reaches_end: bool,
        replies: Vec<StoredMessage>,
        unread_anchor: Option<Key>,
    },
    Draft {
        chat: ChatId,
        draft: Option<Draft>,
    },
    Prefs(Prefs),
    Copy {
        chat: ChatId,
        text: String,
    },
    UnsendFailed(Key),
    StorageFailed(StorageFailure),
    InternalError(&'static str),
    StoreSaved,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refresh {
    Session,
    Stream,
    Profile,
    Send(Key),
    Friends,
    Contact(String),
    Notice(String),
    GroupInfo(ChatId),
    Members(ChatId),
    ChatList,
    MoreChats,
    Receipts(ChatId),
    Around(ChatId, ServerId),
    Older(ChatId, Cursor),
    Details(ChatId),
    Recent(ChatId),
    Read(ChatId),
}
impl Refresh {
    fn name(&self) -> &'static str {
        match self {
            Self::Session => "refresh-session",
            Self::Stream => "refresh-stream",
            Self::Profile => "refresh-profile",
            Self::Send(_) => "send",
            Self::Friends => "refresh-friends",
            Self::Contact(_) => "refresh-contact",
            Self::Notice(_) => "refresh-notice",
            Self::GroupInfo(_) => "refresh-group-info",
            Self::Members(_) => "refresh-members",
            Self::ChatList => "refresh-chat-list",
            Self::MoreChats => "refresh-more-chats",
            Self::Receipts(_) => "refresh-receipts",
            Self::Around(_, _) => "refresh-around",
            Self::Older(_, _) => "refresh-older",
            Self::Details(_) => "refresh-details",
            Self::Recent(_) => "refresh-recent",
            Self::Read(_) => "refresh-read",
        }
    }
}

#[derive(Default)]
struct Lanes {
    users: VecDeque<Intent>,
    completed: VecDeque<Completion>,
    live: VecDeque<StreamEvent>,
    refresh: BTreeSet<Refresh>,
    stopped: bool,
}
struct Inbox {
    pair_cancel: Mutex<Arc<CancelToken>>,
    lanes: Mutex<Lanes>,
    wake: Condvar,
}
#[derive(Clone)]
pub struct Sender(Arc<Inbox>);
impl Sender {
    pub fn cancel_sign_in(&self) {
        self.0
            .pair_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel();
        self.send(Intent::CancelSignIn);
    }
    pub fn send(&self, intent: Intent) {
        let mut q = self
            .0
            .lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        q.users.push_back(intent);
        self.0.notify();
    }
}
impl Inbox {
    fn notify(&self) {
        self.wake.notify_all();
    }
    fn live(&self, event: StreamEvent) -> bool {
        self.live_with_cancel(event, None)
    }
    fn live_with_cancel(&self, event: StreamEvent, cancel: Option<&CancelToken>) -> bool {
        let mut q = self.lanes.lock().unwrap();
        while q.live.len() == 256 && !q.stopped && !cancel.is_some_and(|stop| stop.is_cancelled()) {
            q = self.wake.wait(q).unwrap();
        }
        if q.stopped || cancel.is_some_and(|stop| stop.is_cancelled()) {
            return false;
        }
        q.live.push_back(event);
        self.notify();
        true
    }
    fn refresh(&self, r: Refresh) {
        self.lanes.lock().unwrap().refresh.insert(r);
        self.notify();
    }
}
type Completion = Box<dyn FnOnce(&mut Engine) -> Result<()> + Send>;
type OpenBackend = dyn FnMut() -> anyhow::Result<Box<dyn Backend>>;
type NetJob = Box<
    dyn FnOnce(
            crate::backend::Result<(&mut Option<Box<dyn Backend>>, &mut OpenBackend)>,
        ) -> Completion
        + Send,
>;
type UploadTask = Box<dyn FnOnce() + Send>;
fn upload_workers() -> mpsc::Sender<UploadTask> {
    let (tx, rx) = mpsc::channel::<UploadTask>();
    let rx = Arc::new(Mutex::new(rx));
    for index in 0..2 {
        let rx = rx.clone();
        thread::Builder::new()
            .name(format!("line-upload-{index}"))
            .spawn(move || {
                loop {
                    let task = rx.lock().unwrap().recv();
                    match task {
                        Ok(task) => task(),
                        Err(_) => break,
                    }
                }
            })
            .expect("upload worker");
    }
    tx
}
fn upload_completed(inbox: &Inbox, completed: Completion) {
    let mut lanes = inbox.lanes.lock().unwrap();
    if !lanes.stopped {
        lanes.completed.push_back(completed);
        inbox.notify();
    }
}

pub enum StoreLocation {
    Memory,
    Directory(PathBuf),
    Account { root: PathBuf, mid: Option<String> },
}
type DeltaWake = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;
#[derive(Clone)]
struct DeltaSender {
    tx: mpsc::Sender<Delta>,
    wake: DeltaWake,
}
impl DeltaSender {
    fn send(&self, delta: Delta) -> bool {
        if self.tx.send(delta).is_err() {
            return false;
        }
        if let Some(wake) = self.wake.lock().unwrap().as_ref() {
            wake();
        }
        true
    }
}
#[derive(Default)]
pub struct WorkerOptions {
    pub timing: Timing,
}
pub struct Worker {
    wake: DeltaWake,
    cancel: Arc<CancelToken>,
    pub sender: Sender,
    pub deltas: mpsc::Receiver<Delta>,
    join: Option<thread::JoinHandle<()>>,
}
impl Worker {
    /// Install a nonblocking event-loop wakeup, including already queued deltas.
    pub fn on_deltas(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.lock().unwrap() = Some(Box::new(wake));
        if let Some(wake) = self.wake.lock().unwrap().as_ref() {
            wake();
        }
    }
    /// Open the store before initializing credentials, WASM, or asset caches.
    pub fn start<B: Backend + 'static>(
        location: StoreLocation,
        mut open: impl FnMut() -> anyhow::Result<B> + Send + 'static,
        options: WorkerOptions,
    ) -> Self {
        crate::reporting::install_panic_hook();
        let timing = options.timing;
        let inbox = Arc::new(Inbox {
            pair_cancel: Mutex::default(),
            lanes: Mutex::default(),
            wake: Condvar::new(),
        });
        let sender = Sender(inbox.clone());
        let (tx, deltas) = mpsc::channel();
        let wake = DeltaWake::default();
        let tx = DeltaSender {
            tx,
            wake: wake.clone(),
        };
        let cancel = Arc::new(CancelToken::new());
        let worker_cancel = cancel.clone();
        let join = thread::spawn(move || {
            let fatal = tx.clone();
            let worker_inbox = inbox.clone();
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
                    use sha2::{Digest, Sha256};
                    let mut account_root = None;
                    let mut store_mid = None;
                    let store = match location {
                        StoreLocation::Memory => Store::open_in_memory(),
                        StoreLocation::Directory(p) => Store::open(&p),
                        StoreLocation::Account { root, mid } => {
                            let mid =
                                mid.or_else(|| std::fs::read_to_string(root.join("last-mid")).ok());
                            store_mid = mid.clone();
                            let store = if let Some(mid) = mid {
                                Store::open(&root.join(format!("{:x}", Sha256::digest(mid))))
                            } else {
                                Store::open_in_memory()
                            };
                            account_root = Some(root);
                            store
                        }
                    };
                    match store {
                        Ok(store) => {
                            let mut e = Engine {
                                net: {
                                    let (jobs, rx) = mpsc::channel::<NetJob>();
                                    let inbox = inbox.clone();
                                    let net_cancel = worker_cancel.clone();
                                    thread::Builder::new().name("line-net".into()).spawn(
                                        move || {
                                            crate::client::set_request_cancel(net_cancel);
                                            let mut backend = None;
                                            let mut open = move || {
                                                open().map(|value| {
                                                    Box::new(value) as Box<dyn Backend>
                                                })
                                            };
                                            while let Ok(job) = rx.recv() {
                                                let completed = job(Ok((&mut backend, &mut open)));
                                                let mut q = inbox.lanes.lock().unwrap();
                                                if q.stopped {
                                                    break;
                                                }
                                                q.completed.push_back(completed);
                                                inbox.notify();
                                            }
                                        },
                                    )?;
                                    jobs
                                },
                                local_previews: None,
                                uploads: upload_workers(),
                                net_pending: 0,
                                generation: Arc::new(AtomicU64::new(0)),
                                health: Health::new(),
                                in_flight: BTreeSet::new(),
                                reaction_flights: Default::default(),
                                group_requests: Default::default(),
                                media: None,
                                waiting_assets: VecDeque::new(),
                                waiting_retries: Vec::new(),
                                refresh_failures: BTreeSet::new(),
                                refused_refreshes: BTreeSet::new(),
                                account: AccountSession::Opening { epoch: 0 },
                                operation: "startup".into(),
                                force_list: true,
                                force_window: false,
                                store,
                                account_root,
                                store_mid,
                                inbox,
                                tx,
                                connection: Connection::Local,
                                list: None,
                                window: None,
                                chat_keys: BTreeSet::new(),
                                message_keys: BTreeSet::new(),
                                seq: initial_request_seq()?,
                                stream: None,
                                assets_stop: worker_cancel,
                                list_sweep: vec![],
                                list_cursor: None,
                                list_more: false,
                                continuous: true,
                                retry_after: Default::default(),
                                retry_attempts: Default::default(),
                                timing,
                                #[cfg(any(test, feature = "demo"))]
                                waiters: Vec::new(),
                            };
                            e.run()?;
                        }
                        Err(e) => return Err(e.into()),
                    }
                    Ok(())
                }));
            worker_inbox
                .lanes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stopped = true;
            worker_inbox.notify();
            if let Ok(Err(error)) = &outcome {
                crate::reporting::log("engine-start", "fatal", error.as_ref());
            }
            if !matches!(outcome, Ok(Ok(()))) {
                let _ = fatal.send(Delta::Fatal);
            }
        });
        Self {
            wake,
            cancel,
            sender,
            deltas,
            join: Some(join),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.sender.cancel_sign_in();
        self.sender.send(Intent::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
enum ReactionFlight {
    InFlight,
    Remove,
    Set(u32),
}
struct Engine {
    local_previews: Option<(
        tempfile::TempDir,
        crate::client::desktop::assets::ImageCache,
    )>,
    #[cfg(any(test, feature = "demo"))]
    waiters: Vec<harness::EngineWaiter>,
    timing: Timing,
    retry_attempts: std::collections::BTreeMap<Refresh, u32>,
    retry_after: std::collections::BTreeMap<Refresh, Instant>,
    continuous: bool,
    list_sweep: Vec<ChatInput>,
    list_cursor: Option<String>,
    list_more: bool,
    net: mpsc::Sender<NetJob>,
    uploads: mpsc::Sender<UploadTask>,
    net_pending: usize,
    generation: Arc<AtomicU64>,
    health: Health,
    in_flight: BTreeSet<(String, Key)>,
    reaction_flights: std::collections::BTreeMap<Key, ReactionFlight>,
    group_requests: std::collections::BTreeMap<(String, Vec<String>), u32>,
    media: Option<AssetRequests>,
    waiting_assets: VecDeque<(
        crate::client::desktop::assets::Asset,
        crate::client::desktop::assets::Purpose,
    )>,
    waiting_retries: Vec<String>,
    refresh_failures: BTreeSet<Refresh>,
    refused_refreshes: BTreeSet<ChatId>,
    account: AccountSession,
    operation: String,
    force_list: bool,
    force_window: bool,
    store: Store,
    account_root: Option<PathBuf>,
    store_mid: Option<String>,
    inbox: Arc<Inbox>,
    tx: DeltaSender,
    connection: Connection,
    list: Option<(u64, usize)>,
    window: Option<WindowSub>,
    chat_keys: BTreeSet<ChatId>,
    message_keys: BTreeSet<Key>,
    seq: u32,
    stream: Option<(Arc<CancelToken>, thread::JoinHandle<()>)>,
    assets_stop: Arc<CancelToken>,
}
type Result<T> = std::result::Result<T, StoreError>;
fn reject_pending(
    mid: &str,
    in_flight: &BTreeSet<(String, Key)>,
    store: &mut Store,
    reason: Refusal,
) -> Result<()> {
    for row in store.all_pending()? {
        if row.delivery == Delivery::Pending && !in_flight.contains(&(mid.to_owned(), row.key)) {
            store.mark_failed(
                row.key,
                Delivery::Rejected {
                    reason: reason.clone(),
                },
            )?;
        }
    }
    Ok(())
}
impl Engine {
    fn known(&self) -> bool {
        matches!(self.account, AccountSession::Ready)
            && self.store_mid.as_ref().is_some_and(|mid| !mid.is_empty())
    }

    fn rpc<T: Send + 'static>(
        &mut self,
        run: impl FnOnce(&mut dyn Backend) -> std::result::Result<T, Failure> + Send + 'static,
        apply: impl FnOnce(&mut Engine, std::result::Result<T, Failure>, bool) -> Result<()>
        + Send
        + 'static,
    ) {
        let operation = self.operation.clone();
        let cancel = self.assets_stop.clone();
        self.net_pending += 1;
        let generation = self.generation.load(Ordering::SeqCst);
        let live_generation = self.generation.clone();
        let job: NetJob = Box::new(move |backend| {
            crate::reporting::set_operation(&operation);
            crate::client::set_request_cancel(cancel);
            let activity = NetworkActivity::default();
            // Cancellation still produces a completion so the UI leaves its busy state.
            let result = if generation != live_generation.load(Ordering::SeqCst) {
                Err(Failure::SignedOut(SignOutReason::NoSession))
            } else {
                match backend {
                    Ok((backend, open)) => {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if backend.is_none() {
                                *backend = Some(
                                    open()
                                        .map_err(|error| crate::client::error::classify(&error))?,
                                );
                            }
                            let backend = backend.as_mut().expect("opened backend");
                            backend.set_network_activity(activity.clone());
                            run(backend.as_mut())
                        }))
                        .unwrap_or_else(|_| Err(Failure::Bug("network job panicked".into())))
                    }
                    // A dead worker completes on the store thread, which already has a boundary.
                    Err(error) => Err(error),
                }
            };
            let network = activity.touched();
            Box::new(move |engine| {
                engine.operation = operation;
                engine.net_pending -= 1;
                let current = generation == engine.generation.load(Ordering::SeqCst);
                if current && network {
                    engine.network_outcome(&result);
                }
                if let Err(error) = &result
                    && (matches!(error, Failure::Bug(_)) || current && engine.known())
                {
                    engine.error(error.clone());
                }
                apply(engine, result, current)
            })
        });
        if let Err(error) = self.net.send(job) {
            let done = (error.0)(Err(Failure::Bug("network worker stopped".into())));
            self.inbox.lanes.lock().unwrap().completed.push_back(done);
            self.inbox.notify();
        }
    }
    fn refresh_rpc<T: Send + 'static>(
        &mut self,
        request: Refresh,
        run: impl FnOnce(&mut dyn Backend) -> std::result::Result<T, Failure> + Send + 'static,
        apply: impl FnOnce(&mut Engine, T) -> Result<()> + Send + 'static,
    ) {
        self.rpc(run, move |engine, result, current| {
            if !current {
                return Ok(());
            }
            match result {
                Ok(value) => {
                    engine.retry_attempts.remove(&request);
                    if engine.refresh_failures.remove(&request) {
                        engine.emit(Delta::RefreshRecovered(request));
                    }
                    apply(engine, value)
                }
                Err(error) => {
                    engine.rpc_failed(request, error);
                    Ok(())
                }
            }
        });
    }
    fn start_assets(&mut self) {
        if !self.known() {
            return;
        }
        let inbox = self.inbox.clone();
        let stop = Arc::new(CancelToken::new());
        self.rpc(
            move |backend| backend.start_assets(stop, Arc::new(move |event| inbox.live(event))),
            |_, result, _| {
                if let Err(error) = result
                    // rpc reports Bug failures before invoking this completion.
                    && !matches!(error, Failure::Bug(_))
                {
                    crate::reporting::log("start-assets", "startup", &error);
                }
                Ok(())
            },
        );
    }

    fn report_store_error(&self, error: StoreError) {
        let storage = StorageFailure::classify(&error);
        let kind = if storage.is_some() {
            "local-write"
        } else {
            match &error {
                StoreError::Invalid(_) => "invalid store operation",
                StoreError::Sql(rusqlite::Error::QueryReturnedNoRows) => "unknown key",
                StoreError::Json(_) => "stored data decode",
                _ => "store invariant",
            }
        };
        crate::reporting::log(&self.operation, kind, &error);
        if let Some(failure) = storage {
            self.emit(Delta::StorageFailed(failure));
        } else {
            self.emit(Delta::InternalError(kind));
        }
    }

    fn emit(&self, d: Delta) {
        let _ = self.tx.send(d);
    }
    fn connection(&mut self, c: Connection) {
        self.connection = c.clone();
        self.emit(Delta::Connection(c));
    }
    fn present_account(&mut self, report: bool) -> Result<()> {
        match &self.account {
            AccountSession::Ready => {
                self.start_assets();
                self.online()?;
            }
            AccountSession::SessionUnusable { reason } => {
                let reason = reason.clone();
                if report && let SignOutReason::Unavailable(reason) = &reason {
                    self.emit(Delta::PairFailed {
                        reason: reason.clone(),
                    });
                }
                self.connection(Connection::SignedOut { reason });
            }
            AccountSession::NoSession => self.connection(Connection::SignedOut {
                reason: SignOutReason::NoSession,
            }),
            AccountSession::Opening { .. } => self.connection(Connection::Local),
            AccountSession::SigningIn { .. } => {}
        }
        Ok(())
    }
    fn network_outcome<T>(&mut self, result: &std::result::Result<T, Failure>) {
        match result {
            Ok(_) => self.network_success(),
            Err(Failure::NotDelivered | Failure::Unanswered) => {
                self.health.failure((self.timing.health_now)())
            }
            Err(Failure::Refused(_) | Failure::SignedOut(_)) => self.network_success(),
            Err(Failure::Bug(_)) => {}
        }
    }
    fn network_success(&mut self) {
        self.health.success();
        if self.connection == Connection::Offline {
            self.connection(if self.known() {
                Connection::Online
            } else {
                Connection::Local
            });
        }
    }
    fn error(&mut self, error: Failure) {
        match error {
            Failure::NotDelivered | Failure::Unanswered => {}
            Failure::SignedOut(reason) => {
                if matches!(self.connection, Connection::SignedOut { .. }) {
                    return;
                }
                self.generation.fetch_add(1, Ordering::SeqCst);
                self.stop_stream();
                if let Some(media) = self.media.take() {
                    media.stop.cancel();
                }
                self.continuous = false;
                self.inbox.lanes.lock().unwrap().refresh.clear();
                self.retry_after.clear();
                self.waiting_assets.clear();
                self.waiting_retries.clear();
                self.account = AccountSession::SessionUnusable {
                    reason: reason.clone(),
                };
                self.connection(Connection::SignedOut { reason });
                let result = (|| -> Result<()> {
                    reject_pending(
                        &self.account_mid(),
                        &self.in_flight,
                        &mut self.store,
                        Refusal::SignedOut,
                    )?;
                    self.publish()
                })();
                if let Err(error) = result {
                    self.report_store_error(error);
                }
            }
            Failure::Bug(ref message) => {
                crate::reporting::log(&self.operation, "bug", &Failure::Bug(message.clone()));
                self.emit(Delta::InternalError("network protocol failure"));
            }
            Failure::Refused(_) => {}
        }
    }
    fn account_mid(&self) -> String {
        self.store_mid.clone().unwrap_or_default()
    }
    fn rpc_failed(&mut self, r: Refresh, error: Failure) {
        if error.retry_read() {
            self.retry_later(r);
        } else if matches!(error, Failure::Refused(_) | Failure::Bug(_)) {
            self.refresh_failures.insert(r.clone());
            self.emit(Delta::RefreshFailed { request: r, error });
        }
    }
    fn priority(&self, r: &Refresh) -> u8 {
        let opened = self.window.as_ref().map(|w| &w.chat);
        match r {
            Refresh::Session | Refresh::Send(_) => 0,
            Refresh::Read(chat) if Some(chat) == opened => 1,
            Refresh::Recent(chat) if Some(chat) == opened => 2,
            Refresh::ChatList | Refresh::MoreChats => 3,
            Refresh::Read(_) | Refresh::Around(..) | Refresh::Older(..) | Refresh::Receipts(_) => 4,
            Refresh::Stream
            | Refresh::Profile
            | Refresh::Friends
            | Refresh::Members(_)
            | Refresh::Notice(_)
            | Refresh::Contact(_)
            | Refresh::GroupInfo(_) => 5,
            Refresh::Recent(_) => 6,
            Refresh::Details(_) => 7,
        }
    }
    fn can_refresh(&self, r: &Refresh) -> bool {
        matches!(r, Refresh::Session) && matches!(self.account, AccountSession::Opening { .. })
            || self.known()
            || matches!(r, Refresh::Profile) && matches!(self.account, AccountSession::Ready)
    }
    fn probe_session(&mut self) {
        let probe_gen = self.generation.load(Ordering::SeqCst);
        self.rpc(
            |backend| backend.session(),
            move |engine, result, _| {
                let fact = match result {
                    Ok(true) => AccountSession::Ready,
                    Ok(false) => AccountSession::NoSession,
                    Err(Failure::SignedOut(reason)) => AccountSession::SessionUnusable { reason },
                    Err(error @ (Failure::NotDelivered | Failure::Unanswered)) => {
                        let opening = matches!(engine.account, AccountSession::Opening { epoch } if epoch == probe_gen);
                        let signing_in = matches!(&engine.account, AccountSession::SigningIn { previous, .. } if matches!(previous.as_ref(), AccountSession::Opening { epoch } if *epoch == probe_gen));
                        if opening || signing_in {
                            if opening {
                                engine.connection(Connection::Offline);
                            }
                            engine.rpc_failed(Refresh::Session, error);
                        }
                        return Ok(());
                    }
                    Err(error) => AccountSession::SessionUnusable {
                        reason: SignOutReason::Unavailable(error.sign_in_reason()),
                    },
                };
                engine.retry_attempts.remove(&Refresh::Session);
                if engine.refresh_failures.remove(&Refresh::Session) {
                    engine.emit(Delta::RefreshRecovered(Refresh::Session));
                }
                match &mut engine.account {
                    AccountSession::Opening { epoch } if *epoch == probe_gen => {
                        engine.account = fact;
                        engine.present_account(true)?;
                    }
                    AccountSession::SigningIn { previous, .. }
                        if matches!(previous.as_ref(), AccountSession::Opening { epoch } if *epoch == probe_gen) => {
                        **previous = fact;
                    }
                    _ => {}
                }
                Ok(())
            },
        );
    }

    fn run(&mut self) -> Result<()> {
        if let Some(accounts) = &self.account_root
            && let Some(data) = accounts.parent()
        {
            // Retain retry sources from every account sharing this clipboard directory.
            // If any store cannot be read, leave the files alone.
            if let Err(error) = self
                .store
                .prune_clipboard(&data.join("clipboard-attachments"), accounts)
            {
                crate::reporting::log("clipboard-prune", "cleanup", &error);
            }
        }
        self.emit(Delta::Connection(Connection::Local));
        self.emit(Delta::Profile(self.store.account()?.profile));
        self.emit_contacts()?;
        self.emit(Delta::Prefs(self.store.prefs()?));
        self.store.recover_sends()?;
        // The local subscription snapshot is emitted before any RPC.
        self.list = Some((1, 100));
        self.publish()?;
        if let Some(chat) = self.store.prefs()?.last_chat {
            self.open(WindowSub::unread(1, chat))?;
        }
        self.probe_session();
        loop {
            #[cfg(any(test, feature = "demo"))]
            {
                let waiters = std::mem::take(&mut self.waiters);
                for waiter in waiters {
                    if !waiter(self) {
                        self.waiters.push(waiter);
                    }
                }
            }
            enum Work {
                User(Intent),
                Live(StreamEvent),
                Refresh(Refresh),
                Completed(Completion),
                Health,
            }
            let work = {
                let mut q = self.inbox.lanes.lock().unwrap();
                loop {
                    if self.known()
                        && self
                            .health
                            .offline((self.timing.health_now)(), self.timing.offline_after)
                        && self.connection != Connection::Offline
                    {
                        break Work::Health;
                    }
                    let user = q.users.pop_front();
                    if let Some(i) = user {
                        break Work::User(i);
                    }
                    if let Some(e) = q.live.pop_front() {
                        self.inbox.wake.notify_all();
                        break Work::Live(e);
                    }
                    if let Some(done) = q.completed.pop_front() {
                        break Work::Completed(done);
                    }
                    if self.net_pending == 0
                        && let Some(r) = q
                            .refresh
                            .iter()
                            .filter(|r| {
                                self.can_refresh(r)
                                    && self.retry_after.get(r).is_none_or(|t| *t <= Instant::now())
                            })
                            .min_by_key(|r| self.priority(r))
                            .cloned()
                    {
                        q.refresh.remove(&r);
                        self.retry_after.remove(&r);
                        break Work::Refresh(r);
                    }
                    let now = Instant::now();
                    let health = self
                        .health
                        .failed_since
                        .filter(|_| self.known() && self.connection != Connection::Offline)
                        .map(|since| {
                            now + (since + self.timing.offline_after)
                                .saturating_duration_since((self.timing.health_now)())
                        });
                    let retry = (self.net_pending == 0)
                        .then(|| {
                            q.refresh
                                .iter()
                                .filter(|r| self.can_refresh(r))
                                .filter_map(|r| self.retry_after.get(r))
                                .min()
                                .copied()
                        })
                        .flatten();
                    q = match health.into_iter().chain(retry).min() {
                        Some(deadline) => {
                            self.inbox
                                .wake
                                .wait_timeout(q, deadline.saturating_duration_since(now))
                                .unwrap()
                                .0
                        }
                        None => self.inbox.wake.wait(q).unwrap(),
                    };
                }
            };
            let writes = self.store.writes();
            let result = match work {
                Work::User(Intent::Stop) => break,
                Work::User(i) => {
                    self.operation = i.name().into();
                    self.intent(i)
                }
                Work::Live(e) => {
                    self.operation = "live".into();
                    self.live(e)
                }
                Work::Refresh(r) => {
                    self.operation = r.name().into();
                    self.refresh(r)
                }
                Work::Completed(done) => done(self),
                Work::Health => {
                    self.connection(Connection::Offline);
                    Ok(())
                }
            };
            match result {
                Err(error) => {
                    self.report_store_error(error);
                }
                Ok(()) if self.store.writes() != writes => self.emit(Delta::StoreSaved),
                Ok(()) => {}
            }
        }
        Ok(())
    }
    fn stop_stream(&mut self) {
        if let Some((stop, j)) = self.stream.take() {
            {
                let _q = self
                    .inbox
                    .lanes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                stop.cancel();
                self.inbox.notify();
            }
            drop(j);
        }
    }
    fn start_stream(&mut self) -> Result<()> {
        self.stop_stream();
        let stop = Arc::new(CancelToken::new());
        let inbox = self.inbox.clone();
        let deliver_stop = stop.clone();
        let deliver: Deliver = Arc::new(move |e| inbox.live_with_cancel(e, Some(&deliver_stop)));
        let revision = self.store.sync_revision()?;
        let net_stop = stop.clone();
        let net_deliver = deliver.clone();
        self.rpc(
            move |backend| backend.stream(revision, net_stop, net_deliver),
            move |engine, result, current| {
                if !current {
                    return Ok(());
                }
                let run = match result {
                    Ok(run) => run,
                    Err(e) => {
                        engine.rpc_failed(Refresh::Stream, e);
                        return Ok(());
                    }
                };
                let stream_stop = stop.clone();
                let j = thread::Builder::new()
                    .name("line-sync".into())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
                        if !stream_stop.is_cancelled() {
                            if result.is_err() {
                                deliver(StreamEvent::Retry {
                                    error: Failure::Bug("stream worker panicked".into()),
                                });
                            }
                            deliver(StreamEvent::Exited);
                        }
                    })
                    .map_err(|_| StoreError::Invalid("sync worker could not start".into()))?;
                engine.stream = Some((stop, j));
                engine
                    .inbox
                    .lanes
                    .lock()
                    .unwrap()
                    .refresh
                    .remove(&Refresh::Stream);
                engine.retry_after.remove(&Refresh::Stream);
                Ok(())
            },
        );
        Ok(())
    }
    fn online(&mut self) -> Result<()> {
        if !matches!(self.account, AccountSession::Ready) {
            return Ok(());
        }
        if self.refresh_failures.remove(&Refresh::Session) {
            self.emit(Delta::RefreshRecovered(Refresh::Session));
        }
        if self.connection == Connection::Local {
            self.connection(Connection::Connecting);
        }

        self.refresh_rpc(
            Refresh::Profile,
            move |backend| backend.profile(),
            move |engine, p| {
                let account = engine.store.account()?;
                let own_mid = if account.own_mid.is_empty() {
                    engine.store_mid.as_deref().unwrap_or_default()
                } else {
                    &account.own_mid
                };
                if !own_mid.is_empty() && own_mid != p.mid {
                    engine.error(Failure::SignedOut(SignOutReason::IdentityChanged));
                    return Ok(());
                }
                if account.own_mid.is_empty()
                    && engine.store_mid.as_deref() != Some(p.mid.as_str())
                    && let Some(root) = &engine.account_root
                {
                    use sha2::{Digest, Sha256};
                    let directory = root.join(format!("{:x}", Sha256::digest(&p.mid)));
                    let first_identity =
                        engine.store_mid.is_none() && !directory.join("store.sqlite3").exists();
                    let target = if first_identity {
                        engine.store.persist_to(&directory)?
                    } else {
                        let mut target = Store::open(&directory)?;
                        reject_pending(&p.mid, &engine.in_flight, &mut target, Refusal::AppClosed)?;
                        target
                    };
                    engine.store = target;
                    engine.store_mid = Some(p.mid.clone());
                    engine
                        .inbox
                        .lanes
                        .lock()
                        .unwrap()
                        .refresh
                        .retain(|r| !matches!(r, Refresh::Send(_)));
                    if first_identity {
                        for row in engine.store.all_pending()? {
                            if row.delivery == Delivery::Pending {
                                engine.inbox.refresh(Refresh::Send(row.key));
                            }
                        }
                    }
                    engine.emit(Delta::Prefs(engine.store.prefs()?));
                    if engine.window.is_none()
                        && let Some(chat) = engine.store.prefs()?.last_chat
                    {
                        engine.open(WindowSub::unread(1, chat))?;
                    }
                }
                engine.store_mid = Some(p.mid.clone());
                engine.store.set_account(&p.mid, p.clone())?;
                if let Some(root) = &engine.account_root {
                    std::fs::create_dir_all(root)?;
                    std::fs::write(root.join("last-mid.tmp"), &p.mid)?;
                    std::fs::rename(root.join("last-mid.tmp"), root.join("last-mid"))?;
                }
                engine.emit(Delta::Profile(p));
                engine.account = AccountSession::Ready;
                if engine.media.is_none() {
                    engine.start_assets();
                }
                engine.connection(Connection::Online);
                engine.inbox.refresh(Refresh::ChatList);
                for chat in engine.store.pending_reads()? {
                    engine.inbox.refresh(Refresh::Read(chat));
                }
                if let Some(w) = &engine.window {
                    engine.inbox.refresh(Refresh::Recent(w.chat.clone()));
                    engine.inbox.refresh(Refresh::Receipts(w.chat.clone()));
                }
                engine.start_stream()?;
                Ok(())
            },
        );
        Ok(())
    }
    fn next_seq(&mut self) -> u32 {
        self.seq = (self.seq + 1) & 0x7fff_ffff;
        self.seq
    }
    fn send_reaction(
        &mut self,
        key: Key,
        id: ServerId,
        actor: String,
        previous: Option<ReactionKind>,
        next: Option<u32>,
    ) -> Result<()> {
        let seq = self.next_seq();
        self.reaction_flights.insert(key, ReactionFlight::InFlight);
        let net_id = id.clone();
        self.rpc(
            move |backend| backend.react(&net_id, next, seq),
            move |engine, result, current| {
                let queued = engine
                    .reaction_flights
                    .remove(&key)
                    .and_then(|flight| match flight {
                        ReactionFlight::InFlight => None,
                        ReactionFlight::Remove => Some(None),
                        ReactionFlight::Set(kind) => Some(Some(kind)),
                    });
                let sent = next.map(ReactionKind::Predefined);
                let failed = result.is_err();
                if failed && current {
                    let row = engine.store.message(key)?;
                    let stored = row
                        .reactions
                        .iter()
                        .find(|r| r.actor == actor)
                        .map(|r| r.kind.clone());
                    if stored == sent {
                        engine.store.own_reaction(key, &actor, previous.clone())?;
                    }
                }
                if current
                    && let Some(desired) = queued
                    && (failed || desired != next)
                {
                    engine.store.own_reaction(
                        key,
                        &actor,
                        desired.map(ReactionKind::Predefined),
                    )?;
                    engine.send_reaction(
                        key,
                        id,
                        actor,
                        if failed { previous } else { sent },
                        desired,
                    )?;
                }
                engine.publish()?;
                if failed {
                    engine.emit(Delta::ReactionFailed(key));
                }
                Ok(())
            },
        );
        Ok(())
    }
    fn open(&mut self, w: WindowSub) -> Result<()> {
        self.chat_changed(&w.chat);
        let exists = self.store.chat(&w.chat)?.is_some();
        let friend = self.store.contact(&w.chat.0)?.is_some_and(|f| !f.group);
        if !exists && !friend {
            self.inbox.refresh(Refresh::Details(w.chat.clone()));
        }
        let mut prefs = self.store.prefs()?;
        prefs.last_chat = Some(w.chat.clone());
        self.store.set_prefs(&prefs)?;
        if self.window.as_ref().is_none_or(|old| old.chat != w.chat) {
            self.emit(Delta::Draft {
                chat: w.chat.clone(),
                draft: self.store.load_draft(&w.chat)?,
            });
        }
        if exists || !friend || self.connection == Connection::Online {
            self.inbox.refresh(Refresh::Recent(w.chat.clone()));
        }
        self.window = Some(w);
        self.force_window = true;
        self.publish()
    }
    // Contacts reach the UI in store order, so a refresh does not reshuffle friends.
    fn emit_contacts(&mut self) -> Result<()> {
        self.emit(Delta::Contacts(self.store.contacts()?));
        Ok(())
    }
    fn intent(&mut self, i: Intent) -> Result<()> {
        if !account_allows(
            &i,
            matches!(self.account, AccountSession::Ready),
            self.known(),
        ) {
            return Ok(());
        }
        match i {
            Intent::SignIn => {
                if self.account.signing_in() {
                    return Ok(());
                }
                self.generation.fetch_add(1, Ordering::SeqCst);
                reject_pending(
                    &self.account_mid(),
                    &self.in_flight,
                    &mut self.store,
                    Refusal::SignedOut,
                )?;
                self.stop_stream();
                self.waiting_assets.clear();
                self.waiting_retries.clear();
                if let Some(media) = self.media.take() {
                    media.stop.cancel();
                }
                let cancel = Arc::new(CancelToken::new());
                *self.inbox.pair_cancel.lock().unwrap() = cancel.clone();
                let previous = std::mem::replace(&mut self.account, AccountSession::NoSession);
                let reuse = matches!(
                    previous,
                    AccountSession::Opening { .. }
                        | AccountSession::NoSession
                        | AccountSession::SessionUnusable {
                            reason: SignOutReason::Unavailable(_)
                        }
                );
                self.account = AccountSession::SigningIn {
                    epoch: self.generation.load(Ordering::SeqCst),
                    previous: Box::new(previous),
                };
                self.connection(Connection::SigningIn);
                self.emit(Delta::Pair(PairEvent::Stage(
                    crate::client::pairing::Stage::Preparing,
                )));
                let inbox = self.inbox.clone();
                let generation = self.generation.load(Ordering::SeqCst);
                self.rpc(
                    move |backend| {
                        if reuse && matches!(backend.session(), Ok(true)) { return Ok(false); }
                        if cancel.is_cancelled() {
                            return Err(Failure::SignedOut(SignOutReason::Unavailable(crate::client::error::SignInFailure::Cancelled)));
                        }
                        backend.sign_in(
                            cancel,
                            Arc::new(move |e| {
                                let mut lanes = inbox.lanes.lock().unwrap();
                                lanes.completed.push_back(Box::new(move |engine| {
                                    if engine.generation.load(Ordering::SeqCst) == generation
                                        && engine.account.signing_in()
                                    {
                                        engine.emit(Delta::Pair(e));
                                    }
                                    Ok(())
                                }));
                                inbox.notify();
                            }),
                        ).map(|()| true)
                    },
                    move |engine, result, _| {
                        // Successful install is authoritative even when Cancel raced.
                        // A reused startup session also survives cancellation.
                        if result.is_err() && !matches!(engine.account, AccountSession::SigningIn { epoch, .. } if epoch == generation) { return Ok(()); }
                        match result {
                            Ok(paired) => {
                                if !paired && engine.known() { return Ok(()); }
                                engine.account = AccountSession::Ready;
                                engine.health.success();
                                engine.connection(Connection::Connecting);
                                engine.generation.fetch_add(1, Ordering::SeqCst);
                                if !paired {
                                    return engine.online();
                                }
                                {
                                    let mut lanes = engine.inbox.lanes.lock().unwrap();
                                    lanes.live.clear();
                                    lanes.refresh.clear();
                                }
                                engine.inbox.notify();
                                engine.list_sweep.clear();
                                engine.list_cursor = None;
                                engine.list_more = false;
                                engine.retry_after.clear();
                                engine.store = Store::open_in_memory()?;
                                engine.store_mid = None;
                                engine.refused_refreshes.clear();
                                engine.window = None;
                                engine.chat_keys.clear();
                                engine.message_keys.clear();
                                engine.emit(Delta::Reset);
                                engine.account = AccountSession::Ready;
                                engine.online()?;
                                engine.publish()?;
                            }
                            Err(error) => {
                                use crate::client::error::SignInFailure;
                                let reason = if let Failure::SignedOut(SignOutReason::Unavailable(reason)) = error { reason } else { SignInFailure::Internal };
                                let previous = match std::mem::replace(&mut engine.account, AccountSession::NoSession) {
                                    AccountSession::SigningIn { previous, .. } => *previous,
                                    other => other,
                                };
                                engine.account = if matches!(previous, AccountSession::Ready) {
                                    AccountSession::SessionUnusable { reason: SignOutReason::Unavailable(reason.clone()) }
                                } else { previous };
                                engine.connection(match &engine.account {
                                    AccountSession::NoSession => Connection::SignedOut { reason: SignOutReason::NoSession },
                                    AccountSession::SessionUnusable { reason } => Connection::SignedOut { reason: reason.clone() },
                                    _ => Connection::SignedOut { reason: SignOutReason::NoSession },
                                });
                                engine.emit(Delta::PairFailed { reason });
                            }
                        }
                        Ok(())
                    },
                );
            }
            Intent::CancelSignIn => {
                if self.account.signing_in() {
                    self.inbox.pair_cancel.lock().unwrap().cancel();
                    let previous =
                        match std::mem::replace(&mut self.account, AccountSession::NoSession) {
                            AccountSession::SigningIn { previous, .. } => *previous,
                            other => other,
                        };
                    self.account = previous;
                    self.present_account(false)?;
                    self.emit(Delta::PairFailed {
                        reason: crate::client::error::SignInFailure::Cancelled,
                    });
                }
            }
            Intent::Contact(mid) => {
                if let Some(row) = self.store.contact(&mid)? {
                    self.emit(Delta::Contact(row));
                }
                self.inbox.refresh(Refresh::Contact(mid));
            }
            Intent::GroupInfo(chat) => {
                self.chat_changed(&chat);
                self.emit(Delta::GroupInfo {
                    chat: chat.clone(),
                    data: self.store.full_chat(&chat)?,
                });
                self.inbox.refresh(Refresh::GroupInfo(chat.clone()));
                self.inbox.refresh(Refresh::Details(chat));
            }
            Intent::ProfileAction(action) => {
                let seq = self.next_seq();
                let net_action = action.clone();
                self.rpc(
                    move |b| b.profile_action(&net_action, seq),
                    move |engine, result, _| {
                        let error = result.is_err();
                        if !error {
                            match &action {
                                ProfileAction::Rename { mid, name }
                                | ProfileAction::AddFriend { mid, chat: name } => {
                                    if let Some(raw) = engine.store.contact(mid)? {
                                        let mut row = raw;
                                        if matches!(&action, ProfileAction::Rename { .. }) {
                                            if row.profile.original_name.is_none() {
                                                row.profile.original_name = row.name.clone();
                                            }
                                            row.profile.overridden_name = Some(name.clone());
                                            row.name = Some(name.clone());
                                        } else {
                                            row.profile.relation = Some(0);
                                        }
                                        engine.store.save_contacts(std::slice::from_ref(&row))?;
                                        engine.emit_contacts()?;
                                    }
                                    engine.inbox.refresh(Refresh::Contact(mid.clone()));
                                }
                                ProfileAction::Mute { chat, disabled } => {
                                    if let Some(mut row) = engine.store.chat(chat)? {
                                        row.muted = *disabled;
                                        engine.store.upsert_details(&[row])?;
                                    }
                                    let mut full = engine.store.full_chat(chat)?;
                                    if full.is_object() {
                                        full["notificationDisabled"] = (*disabled).into();
                                        engine.store.save_full_chat(chat, full.clone())?;
                                        engine.emit(Delta::GroupInfo {
                                            chat: chat.clone(),
                                            data: full,
                                        });
                                    }
                                }
                                ProfileAction::Leave { chat } => {
                                    if let Some(mut row) = engine.store.chat(chat)? {
                                        row.left = true;
                                        engine.store.upsert_details(&[row])?;
                                    }
                                }
                            }
                            engine.publish()?;
                        }
                        engine.emit(Delta::ProfileAction { action, error });
                        Ok(())
                    },
                );
            }
            Intent::Friends => {
                self.emit_contacts()?;
                self.inbox.refresh(Refresh::Friends);
            }
            Intent::Members(chat) => {
                self.chat_changed(&chat);
                self.emit(Delta::Members {
                    rows: self.store.members(&chat)?.into_iter().collect::<Vec<_>>(),
                    chat: chat.clone(),
                });
                self.inbox.refresh(Refresh::Members(chat));
            }
            Intent::Asset(asset) => {
                self.asset(asset, crate::client::desktop::assets::Purpose::Thumbnail)?
            }
            Intent::Original(asset) => {
                self.asset(asset, crate::client::desktop::assets::Purpose::Original)?
            }
            Intent::File { asset, name } => self.asset(
                asset,
                crate::client::desktop::assets::Purpose::File { name },
            )?,
            Intent::RetryAsset(key) => {
                if let Some(media) = &self.media {
                    (media.retry)(key);
                } else {
                    self.waiting_retries.push(key);
                }
            }
            Intent::ChatList { sub, limit } => {
                self.list = Some((sub, limit));
                self.force_list = true;
                self.publish()?;
            }
            Intent::Window(w) => self.open(w)?,
            Intent::MoreChats => {
                if let Some((_, limit)) = &mut self.list {
                    *limit += 50;
                }
                self.force_list = true;
                self.publish()?;
                self.inbox.refresh(Refresh::MoreChats);
            }
            Intent::Page { chat, kind } => {
                self.chat_changed(&chat);
                if !self.store.stored_page(&chat, &kind)? {
                    self.defer_page(chat, kind);
                }
            }
            Intent::React { key, kind } => {
                if !(2..=7).contains(&kind) {
                    return Ok(());
                }
                let row = self.store.message(key)?;
                let Some(id) = row.server_id else {
                    return Ok(());
                };
                if row.body.is_withdrawn() {
                    return Ok(());
                }
                let actor = self.store.account()?.own_mid;
                let previous = row
                    .reactions
                    .iter()
                    .find(|r| r.actor == actor)
                    .map(|r| r.kind.clone());
                let next = if previous == Some(ReactionKind::Predefined(kind)) {
                    None
                } else {
                    Some(kind)
                };
                self.store
                    .own_reaction(key, &actor, next.map(ReactionKind::Predefined))?;
                self.publish()?;
                if let Some(queued) = self.reaction_flights.get_mut(&key) {
                    *queued = match next {
                        None => ReactionFlight::Remove,
                        Some(kind) => ReactionFlight::Set(kind),
                    };
                } else {
                    self.send_reaction(key, id, actor, previous, next)?;
                }
            }
            Intent::StickerPackages => {
                self.emit(Delta::StickerPackages(self.store.sticker_packages()?));
                self.rpc(
                    |backend| backend.sticker_packages(),
                    |engine, result, current| {
                        if !current {
                            return Ok(());
                        }
                        match result {
                            Ok(rows) => {
                                engine.store.save_sticker_packages(&rows)?;
                                engine.emit(Delta::StickerPackages(rows));
                            }
                            Err(_) => engine.emit(Delta::StickerPackagesFailed),
                        }
                        Ok(())
                    },
                );
            }
            Intent::CreateChat { name, targets } => {
                let request = (|| {
                    let own = self
                        .store
                        .account()
                        .map_err(|e| {
                            self.report_store_error(e);
                            CreateFailure::Local
                        })?
                        .own_mid;
                    let mut targets = targets
                        .into_iter()
                        .filter(|id| id != &own)
                        .collect::<Vec<_>>();
                    targets.sort();
                    targets.dedup();
                    if targets.len() < 2 {
                        return Err(CreateFailure::InvalidSelection);
                    }
                    let identity = (own, targets.clone());
                    let seq = if let Some(seq) = self.group_requests.get(&identity) {
                        *seq
                    } else {
                        let seq = self.next_seq();
                        self.group_requests.insert(identity.clone(), seq);
                        seq
                    };
                    Ok((targets, identity, seq))
                })();
                let (targets, identity, seq) = match request {
                    Ok(request) => request,
                    Err(error) => {
                        self.emit(Delta::CreatedChat(Err(error)));
                        return Ok(());
                    }
                };
                self.rpc(
                    move |backend| backend.create_chat(name.trim(), &targets, seq),
                    move |engine, result, current| {
                        let outcome = if !current {
                            engine.inbox.refresh(Refresh::ChatList);
                            Err(CreateFailure::Uncertain)
                        } else {
                            match result {
                                Ok(chat) => {
                                    let saved = (|| {
                                        engine.store.upsert_details(std::slice::from_ref(&chat))?;
                                        engine.publish()
                                    })();
                                    engine.inbox.refresh(Refresh::Details(chat.id.clone()));
                                    match saved {
                                        Ok(()) => {
                                            engine.group_requests.remove(&identity);
                                            Ok(chat.id)
                                        }
                                        Err(error) => {
                                            engine.report_store_error(error);
                                            engine.inbox.refresh(Refresh::ChatList);
                                            Err(CreateFailure::Uncertain)
                                        }
                                    }
                                }
                                Err(Failure::Refused(_)) => {
                                    engine.group_requests.remove(&identity);
                                    Err(CreateFailure::Rejected)
                                }
                                Err(_) => {
                                    engine.inbox.refresh(Refresh::ChatList);
                                    Err(CreateFailure::Uncertain)
                                }
                            }
                        };
                        engine.emit(Delta::CreatedChat(outcome));
                        Ok(())
                    },
                );
            }
            Intent::SendAttachments { chat, items } => {
                if self.store.chat(&chat)?.is_some_and(|c| c.left) {
                    return Ok(());
                }
                for mut item in items.into_iter().filter(|item| !item.too_large()) {
                    if !item.preview.is_empty() {
                        let asset = crate::client::desktop::assets::Asset::local(item.clone());
                        let result = (|| -> anyhow::Result<_> {
                            if self.local_previews.is_none() {
                                let root = self
                                    .account_root
                                    .as_ref()
                                    .map(|root| root.join("images"))
                                    .unwrap_or_else(|| {
                                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                                            .join("target/tmp")
                                    });
                                std::fs::create_dir_all(&root)?;
                                let directory = tempfile::Builder::new()
                                    .prefix("local-previews-")
                                    .tempdir_in(root)?;
                                let cache = crate::client::desktop::assets::ImageCache::new(
                                    directory.path(),
                                    &self.store.account()?.own_mid,
                                )?;
                                self.local_previews = Some((directory, cache));
                            }
                            self.local_previews
                                .as_mut()
                                .unwrap()
                                .1
                                .store(&asset, &item.preview)
                        })();
                        match result {
                            Ok(path) => self.emit(Delta::Asset {
                                key: asset.key(),
                                path,
                            }),
                            Err(error) => crate::reporting::log(
                                "attachment-preview",
                                "decode",
                                error.as_ref(),
                            ),
                        }
                    }
                    item.preview.clear();
                    let key = self.enqueue(&chat, || item.content(), None)?;
                    self.inbox.refresh(Refresh::Send(key));
                }
                self.publish()?;
            }
            Intent::SendSticker {
                chat,
                sticker,
                reply,
            } => {
                if !matches!(*sticker, Content::Sticker { .. })
                    || self.store.chat(&chat)?.is_some_and(|c| c.left)
                {
                    return Ok(());
                }
                let key = self.enqueue(&chat, || *sticker, reply)?;
                if let Some(mut draft) = self.store.load_draft(&chat)? {
                    draft.reply = None;
                    self.store.save_draft(&chat, &draft)?;
                }
                self.submitted(chat, String::new(), key)?;
            }
            Intent::Send { chat, text, reply } => {
                if self.store.chat(&chat)?.is_some_and(|c| c.left) {
                    return Ok(());
                }
                let key = self.enqueue(&chat, || Content::text(&text), reply)?;
                self.store.clear_draft(&chat)?;
                self.submitted(chat, text, key)?;
            }
            Intent::Retry(key) => {
                if self.in_flight.contains(&(self.account_mid(), key))
                    || !self.store.message(key)?.delivery.is_failed()
                {
                    return Ok(());
                }
                let seq = self.next_seq();
                self.emit(Delta::UploadFinished(key));
                self.store.retry(key, seq)?;
                self.publish()?;
                self.inbox.refresh(Refresh::Send(key));
            }
            Intent::Discard(key) => {
                if self.in_flight.contains(&(self.account_mid(), key))
                    || !self.store.message(key)?.delivery.is_failed()
                {
                    return Ok(());
                }
                self.emit(Delta::UploadFinished(key));
                self.cleanup_clipboard(key)?;
                self.store.discard(key)?;
                self.inbox
                    .lanes
                    .lock()
                    .unwrap()
                    .refresh
                    .remove(&Refresh::Send(key));
                self.publish()?;
            }
            Intent::Unsend(key) => {
                if self.account.signing_in() {
                    self.emit(Delta::UnsendFailed(key));
                    return Ok(());
                }
                let row = self.store.message(key)?;
                if row.outgoing
                    && !row.body.is_withdrawn()
                    && let Some(id) = row.server_id
                {
                    let seq = self.next_seq();
                    let net_id = id.clone();
                    self.rpc(
                        move |backend| backend.unsend(&net_id, seq),
                        move |engine, result, _current| {
                            match result {
                                Ok(()) => {
                                    engine.store.withdraw(&id)?;
                                    engine.publish()?;
                                }
                                Err(_) => engine.emit(Delta::UnsendFailed(key)),
                            };
                            Ok(())
                        },
                    );
                }
            }
            Intent::Copy(key) => {
                let m = self.store.message(key)?;
                self.emit(Delta::Copy {
                    chat: m.chat_id,
                    text: m.body.as_text().into(),
                });
            }
            Intent::Draft { chat, draft } => {
                self.store.save_draft(&chat, &draft)?;
                self.publish()?;
            }
            Intent::Prefs(mut prefs) => {
                let stored = self.store.prefs()?;
                prefs.last_chat = stored.last_chat;
                self.store.set_prefs(&prefs)?;
                self.emit(Delta::Prefs(prefs));
            }
            Intent::MarkRead { chat, id } => {
                self.store.mark_read(&chat, id.clone())?;
                self.inbox.refresh(Refresh::Read(chat));
                self.publish()?;
            }
            Intent::Stop => {}
        };
        Ok(())
    }
    fn enqueue(
        &mut self,
        chat: &ChatId,
        content: impl FnOnce() -> Content,
        reply: Option<ReplyReference>,
    ) -> Result<Key> {
        let seq = self.next_seq();
        let key = self.store.insert_pending(
            chat,
            LocalId(format!("{}-{seq}", now_ms())),
            seq,
            content(),
            now_ms(),
            reply,
        )?;
        Ok(key)
    }
    fn submitted(&mut self, chat: ChatId, text: String, key: Key) -> Result<()> {
        self.publish()?;
        self.emit(Delta::Submitted { chat, text });
        self.inbox.refresh(Refresh::Send(key));
        Ok(())
    }
    fn send(&mut self, key: Key) -> Result<()> {
        if !self.known() {
            return Ok(());
        }
        let message = self.store.message(key)?;
        if message.delivery != Delivery::Pending {
            return Ok(());
        }
        let seq = message
            .request_seq
            .ok_or_else(|| StoreError::Invalid("send sequence missing".into()))?;
        let account = self.account_mid();
        let address = (account.clone(), key);
        let attachment = message.body.local_attachment().cloned();
        self.in_flight.insert(address.clone());
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(item) = attachment {
            let chat = message.chat_id;
            self.rpc(
                move |backend| backend.prepare_attachment(chat, item, seq),
                move |engine, result, current| {
                    let upload = match result {
                        Ok(upload) if current => upload,
                        Ok(_) => {
                            return engine.sent(
                                address,
                                seq,
                                Err(Failure::SignedOut(SignOutReason::NoSession)),
                                generation,
                            );
                        }
                        Err(error) => return engine.sent(address, seq, Err(error), generation),
                    };
                    let inbox = engine.inbox.clone();
                    let live_generation = engine.generation.clone();
                    let progress_account = account.clone();
                    let failed_address = address.clone();
                    let operation = engine.operation.clone();
                    let work = Box::new(move || {
                        if generation != live_generation.load(Ordering::SeqCst) {
                            upload_completed(
                                &inbox,
                                Box::new(move |engine| {
                                    engine.sent(
                                        address,
                                        seq,
                                        Err(Failure::SignedOut(SignOutReason::NoSession)),
                                        generation,
                                    )
                                }),
                            );
                            return;
                        }
                        let activity = NetworkActivity::default();
                        let mut last_percent = None;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            upload(activity.clone(), &mut |fraction| {
                                let percent = (fraction.clamp(0., 1.) * 100.) as u32;
                                if last_percent == Some(percent) {
                                    return;
                                }
                                last_percent = Some(percent);
                                let account = progress_account.clone();
                                upload_completed(
                                    &inbox,
                                    Box::new(move |engine| {
                                        if engine.account_mid() == account {
                                            engine.emit(Delta::UploadProgress { key, fraction });
                                        }
                                        Ok(())
                                    }),
                                );
                            })
                        }))
                        .unwrap_or_else(|_| Err(Failure::Bug("upload job panicked".into())));
                        let network = activity.touched();
                        upload_completed(
                            &inbox,
                            Box::new(move |engine| {
                                engine.operation = operation;
                                if network {
                                    engine.network_outcome(&result);
                                }
                                if let Err(error) = &result
                                    && (matches!(error, Failure::Bug(_))
                                        || generation == engine.generation.load(Ordering::SeqCst))
                                {
                                    engine.error(error.clone());
                                }
                                match result {
                                    Ok(finish)
                                        if generation
                                            == engine.generation.load(Ordering::SeqCst) =>
                                    {
                                        engine.rpc(
                                            move |backend| finish(backend),
                                            move |engine, result, _| {
                                                engine.sent(address, seq, result, generation)
                                            },
                                        );
                                    }
                                    Ok(_) => {
                                        return engine.sent(
                                            address,
                                            seq,
                                            Err(Failure::SignedOut(SignOutReason::NoSession)),
                                            generation,
                                        );
                                    }
                                    Err(error) => {
                                        return engine.sent(address, seq, Err(error), generation);
                                    }
                                }
                                Ok(())
                            }),
                        );
                    });
                    engine.emit(Delta::UploadProgress { key, fraction: 0. });
                    if engine.uploads.send(work).is_err() {
                        engine.error(Failure::Bug("upload worker stopped".into()));
                        engine.sent(
                            failed_address,
                            seq,
                            Err(Failure::Bug("upload worker stopped".into())),
                            generation,
                        )?;
                    }
                    Ok(())
                },
            );
        } else {
            self.rpc(
                move |backend| {
                    if matches!(message.body, Content::Sticker { .. }) {
                        backend.send_sticker(
                            &message.chat_id,
                            &message.body,
                            seq,
                            message.reply.as_deref(),
                        )
                    } else {
                        backend.send(
                            &message.chat_id,
                            message.body.as_text(),
                            seq,
                            message.reply.as_deref(),
                        )
                    }
                },
                move |engine, result, _| engine.sent(address, seq, result, generation),
            );
        }
        Ok(())
    }
    fn cleanup_clipboard(&self, key: Key) -> Result<()> {
        let row = self.store.message(key)?;
        if let Some(item) = row.body.local_attachment() {
            item.delete_clipboard();
        }
        Ok(())
    }
    fn sent(
        &mut self,
        address: (String, Key),
        seq: u32,
        result: std::result::Result<MessageInput, Failure>,
        generation: u64,
    ) -> Result<()> {
        self.in_flight.remove(&address);
        let (account, key) = address;
        let active = self.account_mid() == account;
        let same_session = generation == self.generation.load(Ordering::SeqCst) && self.known();
        if active {
            self.emit(Delta::UploadFinished(key));
        }
        if active && same_session && result.as_ref().is_err_and(|failure| failure.retry_send()) {
            self.retry_later(Refresh::Send(key));
            return Ok(());
        }
        if active
            && same_session
            && let Ok(row) = &result
            && self
                .store
                .chat(&row.chat_id)?
                .is_none_or(|chat| chat.title.is_empty())
        {
            self.inbox.refresh(Refresh::Details(row.chat_id.clone()));
        }
        let mut other;
        let store = if active {
            &mut self.store
        } else {
            use sha2::{Digest, Sha256};
            let root = self
                .account_root
                .as_ref()
                .ok_or_else(|| StoreError::Invalid("account store root missing".into()))?;
            other = Store::open(&root.join(format!("{:x}", Sha256::digest(&account))))?;
            &mut other
        };
        match result {
            Ok(row) => {
                let pending = store.message(key)?;
                if let Some(item) = pending.body.local_attachment() {
                    item.delete_clipboard();
                }
                store.link_sent(seq, row.server_id, row.created_ms)?;
            }
            Err(error) => {
                // A late echo may already have confirmed this row.
                if store.message(key)?.server_id.is_none() {
                    let delivery = match error {
                        Failure::Refused(reason) => Delivery::Rejected { reason },
                        Failure::SignedOut(_) | Failure::NotDelivered => Delivery::Rejected {
                            reason: Refusal::SignedOut,
                        },
                        Failure::Unanswered => Delivery::Rejected {
                            reason: Refusal::Unanswered,
                        },
                        Failure::Bug(_) => Delivery::Rejected {
                            reason: Refusal::Internal,
                        },
                    };
                    store.mark_failed(key, delivery)?;
                }
            }
        }
        if active { self.publish() } else { Ok(()) }
    }

    fn load_chats(&mut self, next: bool) -> Result<()> {
        if !next && self.list_more {
            self.inbox.refresh(Refresh::MoreChats);
            return Ok(());
        }

        let cursor = if next { self.list_cursor.clone() } else { None };
        self.refresh_rpc(
            if next {
                Refresh::MoreChats
            } else {
                Refresh::ChatList
            },
            move |backend| backend.chats(cursor),
            move |engine, mut page| {
                for chat in &mut page.chats {
                    if chat.title.trim().is_empty()
                        && let Some(old) = engine.store.chat(&chat.id)?
                    {
                        chat.title = old.title;
                        chat.avatar = old.avatar;
                        chat.member_count = old.member_count;
                    }
                }
                if !next {
                    engine.list_sweep.clear();
                }
                engine.list_cursor = page.cursor;
                engine.list_more = page.more;
                engine.list_sweep.extend(page.chats.clone());
                engine.store.upsert_chats(&engine.list_sweep, !page.more)?;
                engine.resolve_senders(&page.previews)?;
                for m in page.previews {
                    engine.store.apply_live(&m.chat_id, &m, false)?;
                }
                for c in &page.chats {
                    if c.title.trim().is_empty() || (c.is_group && c.member_count.is_none()) {
                        engine.inbox.refresh(Refresh::Details(c.id.clone()));
                    }
                }
                if page.more {
                    engine.inbox.refresh(Refresh::MoreChats);
                }
                engine.emit(Delta::MoreChats(page.more));
                engine.publish()?;
                Ok(())
            },
        );
        Ok(())
    }
    fn resolve_senders(&self, rows: &[MessageInput]) -> Result<()> {
        let known = self.store.contact_ids()?;
        for row in rows {
            if let Some(event) = crate::system_events::parse(&row.body) {
                for mid in event.mids() {
                    if !known.contains(mid) {
                        self.inbox.refresh(Refresh::Contact(mid.into()));
                    }
                }
            }
            if !row.outgoing && !row.sender_id.is_empty() && !known.contains(&row.sender_id) {
                self.inbox.refresh(Refresh::Contact(row.sender_id.clone()));
            }
        }
        Ok(())
    }
    fn page(&mut self, chat: ChatId, kind: PageKind) -> Result<()> {
        if self.store.stored_page(&chat, &kind)? {
            return Ok(());
        }
        let net_chat = chat.clone();
        let net_kind = kind.clone();
        self.refresh_rpc(
            Self::page_refresh(chat.clone(), kind.clone()),
            move |backend| backend.messages(&net_chat, &net_kind),
            move |engine, page| {
                engine.resolve_senders(&page.rows)?;
                let recent = matches!(page.kind, PageKind::Recent);
                engine
                    .store
                    .apply_page(&chat, page.kind, &page.rows, page.reaches_start)?;
                if recent {
                    // Page RPCs have no revision; a quiet refresh covers operations
                    // already processed, including an actorless reaction operation.
                    let revision = engine.store.sync_revision()?;
                    for row in &page.rows {
                        engine.store.set_reactions(
                            &row.server_id,
                            row.reactions.clone(),
                            revision,
                        )?;
                    }
                }
                engine.inbox.refresh(Refresh::Receipts(chat.clone()));
                engine.publish()?;
                Ok(())
            },
        );
        Ok(())
    }
    fn defer_page(&self, chat: ChatId, kind: PageKind) {
        self.inbox.refresh(Self::page_refresh(chat, kind));
    }
    fn page_refresh(chat: ChatId, kind: PageKind) -> Refresh {
        match kind {
            PageKind::Around { id } => Refresh::Around(chat, id),
            PageKind::Recent => Refresh::Recent(chat),
            PageKind::Older { before } => Refresh::Older(chat, before),
        }
    }
    fn retry_later(&mut self, r: Refresh) {
        let attempt = self.retry_attempts.entry(r.clone()).or_default();
        *attempt = attempt.saturating_add(1);
        self.retry_after
            .insert(r.clone(), Instant::now() + self.timing.delay(*attempt));
        self.inbox.refresh(r);
    }
    fn chat_changed(&mut self, chat: &ChatId) {
        self.refused_refreshes.remove(chat);
    }
    fn refresh(&mut self, r: Refresh) -> Result<()> {
        if r == Refresh::Session {
            self.probe_session();
            return Ok(());
        }
        if r == Refresh::Profile {
            return self.online();
        }
        if !self.known() {
            return Ok(());
        }
        match r {
            Refresh::Stream => self.start_stream()?,
            Refresh::Session | Refresh::Profile => unreachable!("handled above"),
            Refresh::Send(key) => self.send(key)?,
            Refresh::Notice(mid) => {
                let net = mid.clone();
                self.refresh_rpc(
                    Refresh::Notice(mid.clone()),
                    move |b| b.notice(&net),
                    move |engine, data| {
                        engine.emit(Delta::Notice { mid, data });
                        Ok(())
                    },
                );
            }
            Refresh::Contact(mid) => {
                let net = mid.clone();
                self.refresh_rpc(
                    Refresh::Contact(mid.clone()),
                    move |b| b.contact(&net),
                    move |engine, data| {
                        if let Some(row) = data {
                            if row.profile.relation == Some(2) {
                                engine.inbox.refresh(Refresh::Notice(mid.clone()));
                            }
                            engine.store.save_contacts(std::slice::from_ref(&row))?;
                            engine.emit(Delta::Contact(row));
                        }
                        Ok(())
                    },
                );
            }
            Refresh::GroupInfo(chat) => {
                let net = chat.clone();
                self.refresh_rpc(
                    Refresh::GroupInfo(chat.clone()),
                    move |b| b.full_chat(&net),
                    move |engine, data| {
                        engine.store.save_full_chat(&chat, data.clone())?;
                        if let Some(members) = data
                            .pointer("/extra/groupExtra/memberMids")
                            .and_then(Value::as_object)
                        {
                            let own = engine.store.account()?.own_mid;
                            if !own.is_empty() {
                                engine
                                    .store
                                    .set_chat_membership(&chat, !members.contains_key(&own))?;
                                engine.publish()?;
                            }
                        }
                        engine.emit(Delta::GroupInfo { chat, data });
                        Ok(())
                    },
                );
            }
            Refresh::Friends => {
                self.refresh_rpc(
                    Refresh::Friends,
                    move |backend| backend.friends(),
                    move |engine, mut rows| {
                        for row in &mut rows {
                            if !row.group && row.profile.relation.is_none() {
                                row.profile.relation = Some(0);
                            }
                        }
                        engine.store.save_contacts(&rows)?;
                        engine.emit_contacts()?;
                        Ok(())
                    },
                );
            }
            Refresh::Members(chat) => {
                let net_chat = chat.clone();
                self.refresh_rpc(
                    Refresh::Members(chat.clone()),
                    move |backend| backend.members(&net_chat),
                    move |engine, mut rows| {
                        for member in &mut rows {
                            if member.contact.name.is_none()
                                && let Some(old) = engine.store.contact(&member.contact.id)?
                            {
                                let invited = member.contact.profile.invited;
                                member.contact = old;
                                member.contact.profile.invited = invited;
                            }
                        }
                        let mut contacts = Vec::new();
                        for member in &rows {
                            if member.is_self || member.contact.name.is_none() {
                                continue;
                            }
                            if engine
                                .store
                                .contact(&member.contact.id)?
                                .is_some_and(|v| v.profile.relation == Some(0))
                            {
                                continue;
                            }
                            let mut contact = member.contact.clone();
                            contact.profile.invited = false;
                            contacts.push(contact);
                        }
                        engine.store.save_contacts(&contacts)?;
                        engine.store.save_members(&chat, &rows)?;
                        engine.emit(Delta::Members { chat, rows });
                        Ok(())
                    },
                );
            }
            Refresh::ChatList => self.load_chats(false)?,
            Refresh::MoreChats => self.load_chats(true)?,
            Refresh::Around(chat, id) => self.page(chat, PageKind::Around { id })?,
            Refresh::Older(chat, before) => self.page(chat, PageKind::Older { before })?,
            Refresh::Receipts(chat) => {
                let net_chat = chat.clone();
                self.refresh_rpc(
                    Refresh::Receipts(chat.clone()),
                    move |backend| backend.receipts(&net_chat),
                    move |engine, ids| {
                        for id in ids {
                            let id = match id {
                                Some(id) => Some(id),
                                None => engine
                                    .store
                                    .newest_confirmed(&chat)?
                                    .and_then(|m| m.server_id),
                            };
                            if let Some(id) = id {
                                engine.store.advance_read_receipt(&chat, id)?;
                            }
                        }
                        engine.publish()?;
                        Ok(())
                    },
                );
            }
            Refresh::Recent(chat) => self.page(chat, PageKind::Recent)?,
            Refresh::Details(chat) => {
                let net_chat = chat.clone();
                self.refresh_rpc(
                    Refresh::Details(chat.clone()),
                    move |backend| backend.details(&net_chat),
                    move |engine, rows| {
                        engine.store.upsert_details(&rows)?;
                        engine.publish()?;
                        Ok(())
                    },
                );
            }
            Refresh::Read(chat) => {
                let id = self
                    .store
                    .chat(&chat)?
                    .and_then(|c| c.last_read_server_id)
                    .ok_or_else(|| StoreError::Invalid("read marker missing".into()))?;
                {
                    let net_chat = chat.clone();
                    let net_id = id.clone();
                    let seq = self.next_seq();
                    self.refresh_rpc(
                        Refresh::Read(chat.clone()),
                        move |backend| backend.mark_read(&net_chat, &net_id, seq),
                        move |engine, ()| {
                            engine.store.confirm_read(&chat, &id)?;
                            Ok(())
                        },
                    );
                }
            }
        };
        Ok(())
    }
    fn asset(
        &mut self,
        asset: crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<()> {
        if !self.known() {
            let mid = self.store.account()?.own_mid;
            let mid = if mid.is_empty() {
                self.store_mid.clone().unwrap_or_default()
            } else {
                mid
            };
            if mid.is_empty() {
                return Ok(());
            }
            let key = asset.key();
            let response_purpose = purpose.clone();
            self.rpc(
                move |backend| backend.cached_media(&mid, &asset, purpose),
                move |engine, result, current| {
                    if !current {
                        return Ok(());
                    }
                    use crate::client::desktop::assets::{Completion, Failure};
                    engine.emit(asset_delta(Completion {
                        network: false,
                        key,
                        purpose: response_purpose,
                        result: result.ok().flatten().ok_or(Failure::Network),
                    }));
                    Ok(())
                },
            );
        } else if let Some(media) = &self.media {
            (media.request)(asset, purpose);
        } else {
            self.waiting_assets.push_back((asset, purpose));
        }
        Ok(())
    }
    fn live(&mut self, e: StreamEvent) -> Result<()> {
        if !self.known() {
            if let StreamEvent::Retry {
                error: error @ Failure::Bug(_),
                ..
            }
            | StreamEvent::Connection(Err(error @ Failure::Bug(_))) = &e
            {
                self.error(error.clone());
            }
            match e {
                StreamEvent::AssetRequests(media) => {
                    media.stop.cancel();
                    return Ok(());
                }
                StreamEvent::Connection(_)
                | StreamEvent::Connecting
                | StreamEvent::Retry { .. }
                | StreamEvent::Exited
                | StreamEvent::Raw(_) => return Ok(()),
                StreamEvent::RefreshToken { reply } => {
                    let _ = reply
                        .try_send(Err(Failure::SignedOut(SignOutReason::AuthenticationFailed)));
                    return Ok(());
                }
                StreamEvent::Authorize { reply, .. } => {
                    let _ = reply.try_send(Err(crate::client::desktop::assets::Failure::Network));
                    return Ok(());
                }
                StreamEvent::Resync { complete, .. } => {
                    let _ = complete.try_send(Err(Failure::SignedOut(SignOutReason::NoSession)));
                    return Ok(());
                }
                StreamEvent::Asset(_) | StreamEvent::Operation(_) => return Ok(()),
            }
        }
        match e {
            StreamEvent::Resync { revision, complete } => {
                self.resync(revision, complete)?;
            }
            StreamEvent::AssetRequests(media) => {
                for (asset, purpose) in self.waiting_assets.drain(..) {
                    (media.request)(asset, purpose);
                }
                for key in self.waiting_retries.drain(..) {
                    (media.retry)(key);
                }
                self.media = Some(media);
            }
            StreamEvent::Asset(c) => {
                if c.network {
                    match &c.result {
                        Ok(_) => self.network_success(),
                        Err(failure) => {
                            self.network_outcome::<()>(&Err(Failure::from(*failure)));
                        }
                    }
                }
                self.emit(asset_delta(c));
            }
            StreamEvent::Authorize { renew, reply } => {
                self.rpc(
                    move |backend| backend.authorize(renew),
                    move |_, result, _| {
                        let _ = reply.try_send(
                            result.map_err(|_| crate::client::desktop::assets::Failure::Network),
                        );
                        Ok(())
                    },
                );
            }
            StreamEvent::Operation(op) => {
                if op.revision > self.store.sync_revision()? {
                    match &op.op {
                        Op::Message(m) => self.chat_changed(&m.chat_id),
                        Op::Recent(chat) | Op::Read { chat, .. } | Op::Receipt { chat, .. } => {
                            self.chat_changed(chat)
                        }
                        Op::Withdraw(id) | Op::Reaction { id, .. } => {
                            if let Some(row) = self.store.message_by_server(id)? {
                                self.chat_changed(&row.chat_id);
                            }
                        }
                        Op::Checkpoint => {}
                    }
                    let fresh = if let Op::Message(m) = &op.op {
                        if self.store.chat(&m.chat_id)?.is_none() {
                            self.inbox.refresh(Refresh::Details(m.chat_id.clone()));
                        }
                        !self.store.contains_server(&m.server_id)?
                    } else {
                        false
                    };
                    if let Op::Recent(chat) = &op.op {
                        self.inbox.refresh(Refresh::Recent(chat.clone()));
                    }
                    if let Op::Message(m) = &op.op {
                        self.resolve_senders(std::slice::from_ref(m.as_ref()))?;
                    }
                    let clipboard = if let Op::Message(message) = &op.op
                        && message.outgoing
                        && message.request_seq.is_some()
                    {
                        self.store
                            .all_pending()?
                            .into_iter()
                            .find(|row| {
                                row.chat_id == message.chat_id
                                    && row.request_seq == message.request_seq
                            })
                            .and_then(|row| {
                                row.body
                                    .local_attachment()
                                    .filter(|item| item.clipboard)
                                    .cloned()
                            })
                    } else {
                        None
                    };
                    self.store.apply_operation(&op, self.continuous)?;
                    if let Some(item) = clipboard {
                        item.delete_clipboard();
                    }
                    if fresh
                        && !op.replay
                        && let Op::Message(m) = &op.op
                        && !m.outgoing
                    {
                        self.emit(Delta::Notify(m.as_ref().clone()));
                    }
                    self.publish()?;
                }
            }
            StreamEvent::Raw(raw) => {
                let data = &raw.data;
                if crate::client::wire_number(&data["revision"])
                    .is_some_and(|r: i64| r > self.store.sync_revision().unwrap_or(0))
                    && let Some(id) = data["param1"].as_str().filter(|id| !id.is_empty())
                {
                    let id = id.to_owned();
                    self.chat_changed(&ChatId(id.clone()));
                    match raw.kind {
                        OperationType::AddContact | OperationType::UpdateContact => {
                            self.inbox.refresh(Refresh::Contact(id))
                        }
                        OperationType::SendChatRemoved
                            if self
                                .store
                                .chat(&ChatId(id.clone()))?
                                .is_some_and(|c| c.is_group) =>
                        {
                            self.store.remove_chat(&ChatId(id))?;
                            self.publish()?;
                            self.inbox.refresh(Refresh::ChatList);
                        }
                        OperationType::UpdateChat
                        | OperationType::NotifiedUpdateChat
                        | OperationType::DeleteSelfFromChat
                        | OperationType::NotifiedDeleteSelfFromChat => {
                            if raw.kind == OperationType::DeleteSelfFromChat
                                && let Some(mut row) = self.store.chat(&ChatId(id.clone()))?
                            {
                                row.left = true;
                                self.store.upsert_details(&[row])?;
                                self.publish()?;
                            }
                            self.inbox.refresh(Refresh::Members(ChatId(id.clone())));
                            self.inbox.refresh(Refresh::Details(ChatId(id.clone())));
                            self.inbox.refresh(Refresh::GroupInfo(ChatId(id)));
                        }
                        _ => {}
                    }
                }

                self.rpc(
                    move |backend| backend.decode(raw),
                    move |engine, result, current| {
                        if current && let Ok(operation) = result {
                            engine.live(StreamEvent::Operation(operation))?;
                        }
                        Ok(())
                    },
                );
            }
            StreamEvent::RefreshToken { reply } => {
                self.rpc(
                    move |backend| backend.refresh_token(),
                    move |_, result, _| {
                        let _ = reply.try_send(result);
                        Ok(())
                    },
                );
            }
            StreamEvent::Retry { error } => {
                self.network_outcome::<()>(&Err(error.clone()));
                if !error.retry_read() && matches!(error, Failure::Refused(_) | Failure::Bug(_)) {
                    self.rpc_failed(Refresh::Stream, error.clone());
                }
                self.error(error);
            }
            StreamEvent::Connecting => {}
            StreamEvent::Connection(Ok(())) => {
                self.continuous = true;
                self.retry_attempts.remove(&Refresh::Stream);
                if self.refresh_failures.remove(&Refresh::Stream) {
                    self.emit(Delta::RefreshRecovered(Refresh::Stream));
                }
                self.network_success();
            }
            StreamEvent::Connection(Err(e)) => {
                self.network_outcome::<()>(&Err(e.clone()));
                self.error(e);
            }
            StreamEvent::Exited => {
                self.continuous = false;
                self.stop_stream();
                self.retry_later(Refresh::Stream);
            }
        };
        Ok(())
    }
    fn resync(
        &mut self,
        revision: Option<i64>,
        complete: async_channel::Sender<std::result::Result<(), Failure>>,
    ) -> Result<()> {
        // Suppression applies only to Recent refreshes initiated by this resync.
        let chats: Vec<_> = self
            .store
            .chat_list(usize::MAX, 0)?
            .into_iter()
            .map(|c| c.chat.id)
            .chain(self.window.as_ref().map(|w| w.chat.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|chat| !self.refused_refreshes.contains(chat))
            .collect();
        let chat = self
            .window
            .as_ref()
            .map(|w| w.chat.clone())
            .filter(|chat| chats.contains(chat));
        let requested_chat = chat.clone();
        self.rpc(
            move |backend| {
                chat.map(|chat| {
                    backend
                        .messages(&chat, &PageKind::Recent)
                        .map(|page| (chat, page))
                })
                .transpose()
            },
            move |engine, result, current| {
                if !current {
                    return Ok(());
                }
                match result {
                    Ok(Some((chat, page))) => {
                        engine.store.apply_page(
                            &chat,
                            page.kind,
                            &page.rows,
                            page.reaches_start,
                        )?;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(chat) = requested_chat {
                            if error.suppress_resync_refresh() {
                                engine.refused_refreshes.insert(chat.clone());
                            }
                            engine.rpc_failed(Refresh::Recent(chat), error.clone());
                        }
                        let _ = complete.try_send(Err(error));
                        return Ok(());
                    }
                }
                engine.inbox.refresh(Refresh::ChatList);
                for chat in chats {
                    if engine.window.as_ref().is_none_or(|w| w.chat != chat) {
                        engine.inbox.refresh(Refresh::Recent(chat));
                    }
                }
                if let Some(revision) = revision {
                    engine.store.set_sync_revision(revision)?;
                }
                engine.continuous = true;
                engine.publish()?;
                let _ = complete.try_send(Ok(()));
                Ok(())
            },
        );
        Ok(())
    }
    fn publish(&mut self) -> Result<()> {
        let changes = self.store.take_changes();
        if let Some((sub, limit)) = self.list
            && (self.force_list
                || !changes.chats_changed.is_empty()
                || !changes.drafts_changed.is_empty()
                || !changes.chats_removed.is_empty())
        {
            let all = self.store.chat_list(limit, 0)?;
            let mut rows = all
                .iter()
                .filter(|row| {
                    self.force_list
                        || changes.chats_changed.contains(&row.chat.id)
                        || changes.drafts_changed.contains(&row.chat.id)
                        || !self.chat_keys.contains(&row.chat.id)
                })
                .cloned()
                .collect::<Vec<_>>();
            // Selected chats can fall outside the loaded list. Their metadata
            // (especially drafts) still needs a row delta without expanding order.
            for id in changes.chats_changed.union(&changes.drafts_changed) {
                if !all.iter().any(|row| &row.chat.id == id)
                    && let Some(row) = self.store.chat_item(id)?
                {
                    rows.push(row);
                }
            }
            if self.connection != Connection::Online {
                for c in &mut rows {
                    c.chat.unread_stale = true;
                }
            }
            let keys = all
                .iter()
                .map(|r| r.chat.id.clone())
                .collect::<BTreeSet<_>>();
            let remove = self.chat_keys.difference(&keys).cloned().collect();
            let order = all.iter().map(|r| r.chat.id.clone()).collect();
            self.force_list = false;
            self.chat_keys = keys;
            self.emit(Delta::Chats {
                sub,
                upsert: rows,
                remove,
                order,
            });
        }
        if let Some(w) = &self.window
            && (self.force_window
                || changes.chats_changed.contains(&w.chat)
                || !changes.messages_removed.is_disjoint(&self.message_keys))
        {
            self.force_window = false;
            let window = self.store.window(&w.chat, w.anchor, w.before, w.after)?;
            let keys = window.rows.iter().map(|r| r.key).collect::<BTreeSet<_>>();
            let remove = self.message_keys.difference(&keys).copied().collect();
            self.message_keys = keys;
            let mut replies = Vec::new();
            for row in &window.rows {
                if let Some(id) = row.reply.as_ref().map(|r| &r.message_id)
                    && let Some(target) = self.store.message_by_server(id)?
                {
                    replies.push(target);
                }
            }
            if let Some(reference) = self.store.load_draft(&w.chat)?.and_then(|d| d.reply)
                && !replies.iter().any(|m| {
                    m.server_id
                        .as_ref()
                        .is_some_and(|id| id == &reference.message_id)
                })
                && let Some(target) = self.store.message_by_server(&reference.message_id)?
            {
                replies.push(target);
            }
            self.emit(Delta::Window {
                unread_anchor: self.store.unread_anchor(&w.chat)?.map(|m| m.key),
                replies,
                sub: w.sub,
                upsert: window.rows,
                remove,
                gaps: window.gaps,
                reaches_start: window.reaches_start,
                reaches_end: window.reaches_end,
            });
        };
        Ok(())
    }
}
fn asset_delta(c: crate::client::desktop::assets::Completion) -> Delta {
    use crate::client::desktop::assets::{Payload, Purpose};
    match c.result {
        Ok(Payload::Evicted) => Delta::AssetEvicted { key: c.key },
        Ok(Payload::Image(path)) => Delta::Asset { key: c.key, path },
        Ok(Payload::Original { path, source }) => Delta::Original {
            key: c.key,
            path,
            source,
        },
        Ok(Payload::File(bytes)) => Delta::File {
            name: match c.purpose {
                Purpose::File { name, .. } => name,
                _ => String::new(),
            },
            bytes,
        },
        Err(failure) => Delta::MediaFailed {
            key: c.key,
            failure,
        },
    }
}
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Drop for Engine {
    fn drop(&mut self) {
        {
            let mut lanes = self
                .inbox
                .lanes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lanes.stopped = true;
            lanes.completed.clear();
        }
        self.inbox.notify();
        self.assets_stop.cancel();
        self.stop_stream();
    }
}

#[cfg(test)]
mod tests;

fn initial_request_seq() -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    crate::fill_random(&mut bytes)?;
    Ok(u32::from_ne_bytes(bytes) & 0x7fff_ffff)
}

#[cfg(any(test, feature = "demo"))]
pub mod harness;
