//! A store thread schedules work; a network thread owns the backend.
use crate::{backend::*, store::*};
use serde_json::Value;
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Fatal,
    Local,
    Connecting { attempt: u32 },
    Online,
    Offline { retry_at: Instant, cause: NetCause },
    AuthExpired,
    SessionInvalid,
    SigningIn,
}
/// Retry policy and scheduler resolution, injectable for offline tests.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub retry_initial: Duration,
    pub retry_max: Duration,
    pub poll: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            retry_initial: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            poll: Duration::from_millis(100),
        }
    }
}
impl Timing {
    fn delay(self, attempt: u32) -> Duration {
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
#[derive(Clone)]
pub enum Intent {
    SearchChats {
        sub: u64,
        query: String,
    },
    ChatList {
        sub: u64,
        limit: usize,
    },
    Window(WindowSub),
    MoreChats,
    SignIn,
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
    },
    Unsend(Key),
    Retry(Key),
    Resend(Key),
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
#[derive(Clone)]
pub enum Delta {
    Fatal,
    Reset,
    Pair(PairEvent),
    PairFailed {
        cancelled: bool,
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
        error: Option<String>,
    },
    MoreChats(bool),
    Notify(crate::store::MessageInput),
    Friends(Vec<crate::store::Friend>),
    Members {
        chat: ChatId,
        rows: Vec<crate::store::Member>,
    },
    MediaFailed {
        key: String,
        failure: crate::client::desktop::assets::Failure,
    },
    File {
        name: String,
        bytes: Arc<Vec<u8>>,
    },
    Asset {
        key: String,
        pixels: std::sync::Arc<crate::client::desktop::assets::Pixels>,
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
    Error(String),
    StoreSaved,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Refresh {
    Stream,
    Send(Key),
    Friends,
    Contact(String),
    Notice(String),
    GroupInfo(ChatId),
    Members(ChatId),
    ChatList,
    MoreChats,
    Receipts(ChatId),
    Older(ChatId, Cursor),
    Newer(ChatId, Cursor),
    Details(ChatId),
    Recent(ChatId),
    Read(ChatId),
}
#[derive(Default)]
struct Lanes {
    users: VecDeque<Intent>,
    completed: VecDeque<Completion>,
    live: VecDeque<StreamEvent>,
    refresh: BTreeSet<Refresh>,
    stopped: bool,
}
#[derive(Default)]
struct Inbox {
    pair_cancel: Arc<AtomicBool>,
    lanes: Mutex<Lanes>,
    wake: Condvar,
}
#[derive(Clone)]
pub struct Sender(Arc<Inbox>);
impl Sender {
    pub fn cancel_sign_in(&self) {
        self.0.pair_cancel.store(true, Ordering::Relaxed);
    }
    pub fn send(&self, intent: Intent) {
        let mut q = self.0.lanes.lock().unwrap();
        q.users.push_back(intent);
        self.0.wake.notify_all();
    }
}
impl Inbox {
    fn live(&self, event: StreamEvent) -> bool {
        self.live_with_cancel(event, None)
    }
    fn live_with_cancel(&self, event: StreamEvent, cancel: Option<&AtomicBool>) -> bool {
        let mut q = self.lanes.lock().unwrap();
        while q.live.len() == 256
            && !q.stopped
            && !cancel.is_some_and(|stop| stop.load(Ordering::Relaxed))
        {
            q = self.wake.wait(q).unwrap();
        }
        if q.stopped || cancel.is_some_and(|stop| stop.load(Ordering::Relaxed)) {
            return false;
        }
        q.live.push_back(event);
        self.wake.notify_all();
        true
    }
    fn refresh(&self, r: Refresh) {
        self.lanes.lock().unwrap().refresh.insert(r);
        self.wake.notify_all();
    }
}
type Completion = Box<dyn FnOnce(&mut Engine) -> Result<()> + Send>;
type NetJob = Box<dyn FnOnce(&mut dyn Backend) -> Completion + Send>;

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
pub struct Worker {
    wake: DeltaWake,
    cancel: Arc<AtomicBool>,
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
    pub fn start(backend: impl Backend + 'static, location: StoreLocation) -> Self {
        Self::start_with_timing(location, move || Ok(backend), Timing::default())
    }
    /// Open the store before initializing credentials, WASM, or asset caches.
    pub fn start_with<B: Backend + 'static>(
        location: StoreLocation,
        open: impl FnOnce() -> anyhow::Result<B> + Send + 'static,
    ) -> Self {
        Self::start_with_timing(location, open, Timing::default())
    }
    pub fn start_timed(
        backend: impl Backend + 'static,
        location: StoreLocation,
        timing: Timing,
    ) -> Self {
        Self::start_with_timing(location, move || Ok(backend), timing)
    }
    pub fn start_with_timing<B: Backend + 'static>(
        location: StoreLocation,
        open: impl FnOnce() -> anyhow::Result<B> + Send + 'static,
        timing: Timing,
    ) -> Self {
        let inbox = Arc::new(Inbox::default());
        let sender = Sender(inbox.clone());
        let (tx, deltas) = mpsc::channel();
        let wake = DeltaWake::default();
        let tx = DeltaSender {
            tx,
            wake: wake.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let join = thread::spawn(move || {
            let fatal = tx.clone();
            let worker_inbox = inbox.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> anyhow::Result<()> {
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
                                    let fatal = tx.clone();
                                    let net_cancel = worker_cancel.clone();
                                    thread::Builder::new().name("line-net".into()).spawn(
                                        move || {
                                            let outcome = std::panic::catch_unwind(
                                                std::panic::AssertUnwindSafe(
                                                    || -> anyhow::Result<()> {
                                                        let Ok(first) = rx.recv() else {
                                                            return Ok(());
                                                        };
                                                        crate::client::desktop::set_cancellation_token(net_cancel);
                                                        let mut backend = open()?;
                                                        let mut job = first;
                                                        loop {
                                                            let completed = job(&mut backend);
                                                            let mut q = inbox.lanes.lock().unwrap();
                                                            if q.stopped {
                                                                break;
                                                            }
                                                            q.completed.push_back(completed);
                                                            inbox.wake.notify_all();
                                                            drop(q);
                                                            match rx.recv() {
                                                                Ok(next) => job = next,
                                                                Err(_) => break,
                                                            }
                                                        }
                                                        Ok(())
                                                    },
                                                ),
                                            );
                                            if !matches!(outcome, Ok(Ok(()))) {
                                                fatal.send(Delta::Fatal);
                                            }
                                        },
                                    )?;
                                    jobs
                                },
                                net_pending: 0,
                                generation: 0,
                                in_flight: BTreeSet::new(),
                                media: None,
                                waiting_assets: VecDeque::new(),
                                waiting_retries: Vec::new(),
                                active_refresh: None,
                                force_list: true,
                                force_window: false,
                                search: String::new(),
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
                                seq: 0,
                                stream: None,
                                assets_stop: worker_cancel,
                                list_sweep: vec![],
                                list_cursor: None,
                                list_more: false,
                                continuous: true,
                                retry_after: Default::default(),
                                retry_attempts: Default::default(),
                                timing,
                                attempt: 0,
                                reconnect_at: None,
                            };
                            e.run()?;
                        }
                        Err(e) => return Err(e.into()),
                    }
                    Ok(())
                },
            ));
            worker_inbox.lanes.lock().unwrap().stopped = true;
            worker_inbox.wake.notify_all();
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
        self.cancel.store(true, Ordering::Relaxed);
        self.sender.cancel_sign_in();
        self.sender.send(Intent::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
struct Engine {
    timing: Timing,
    attempt: u32,
    reconnect_at: Option<Instant>,
    retry_attempts: std::collections::BTreeMap<Refresh, u32>,
    retry_after: std::collections::BTreeMap<Refresh, Instant>,
    continuous: bool,
    list_sweep: Vec<ChatInput>,
    list_cursor: Option<String>,
    list_more: bool,
    net: mpsc::Sender<NetJob>,
    net_pending: usize,
    generation: u64,
    in_flight: BTreeSet<Key>,
    media: Option<AssetRequests>,
    waiting_assets: VecDeque<(
        crate::client::desktop::assets::Asset,
        crate::client::desktop::assets::Purpose,
    )>,
    waiting_retries: Vec<String>,
    active_refresh: Option<Refresh>,
    force_list: bool,
    force_window: bool,
    search: String,
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
    stream: Option<(Arc<AtomicBool>, thread::JoinHandle<()>)>,
    assets_stop: Arc<AtomicBool>,
}
type Result<T> = std::result::Result<T, StoreError>;
impl Engine {
    fn rpc<T: Send + 'static>(
        &mut self,
        run: impl FnOnce(&mut dyn Backend) -> T + Send + 'static,
        apply: impl FnOnce(&mut Engine, T) -> Result<()> + Send + 'static,
    ) {
        self.rpc_completion(run, move |engine, result, current| {
            if current {
                apply(engine, result)
            } else {
                Ok(())
            }
        });
    }
    // The FIFO network queue completes mutations before sign-in can swap stores.
    fn rpc_completion<T: Send + 'static>(
        &mut self,
        run: impl FnOnce(&mut dyn Backend) -> T + Send + 'static,
        apply: impl FnOnce(&mut Engine, T, bool) -> Result<()> + Send + 'static,
    ) {
        self.net_pending += 1;
        let cancel = self.assets_stop.clone();
        let generation = self.generation;
        let _ = self.net.send(Box::new(move |backend| {
            crate::client::desktop::set_cancellation_token(cancel);
            let result = run(backend);
            Box::new(move |engine| {
                engine.net_pending -= 1;
                let current = generation == engine.generation;
                apply(engine, result, current)
            })
        }));
    }
    fn start_assets(&mut self) {
        let inbox = self.inbox.clone();
        let tx = self.tx.clone();
        let stop = self.assets_stop.clone();
        self.rpc(
            move |backend| {
                backend.start_assets(
                    stop,
                    Arc::new(move |event| {
                        if let StreamEvent::Asset(c) = event {
                            return tx.send(asset_delta(c));
                        }
                        inbox.live(event)
                    }),
                )
            },
            |_, ()| Ok(()),
        );
    }

    fn emit(&self, d: Delta) {
        let _ = self.tx.send(d);
    }
    fn connection(&mut self, c: Connection) {
        self.connection = c.clone();
        self.emit(Delta::Connection(c));
    }
    fn error(&mut self, e: Error) {
        let connection = match e {
            Error::Auth => Connection::AuthExpired,
            Error::SessionInvalid => Connection::SessionInvalid,
            Error::Offline(cause) => Connection::Offline {
                retry_at: Instant::now() + self.timing.delay(self.attempt.max(1)),
                cause,
            },
            _ => return,
        };
        if matches!(
            connection,
            Connection::AuthExpired | Connection::SessionInvalid
        ) {
            self.reconnect_at = None;
        }
        self.continuous = false;
        self.connection(connection);
    }
    fn rpc_failed(&mut self, r: Refresh, e: Error) {
        self.retry_later(r);
        self.error(e);
    }
    fn priority(&self, r: &Refresh) -> u8 {
        let opened = self.window.as_ref().map(|w| &w.chat);
        match r {
            Refresh::Read(chat) if Some(chat) == opened => 0,
            Refresh::Recent(chat) if Some(chat) == opened => 1,
            Refresh::Send(_) => 2,
            Refresh::ChatList | Refresh::MoreChats => 3,
            Refresh::Read(_) | Refresh::Older(..) | Refresh::Newer(..) | Refresh::Receipts(_) => 4,
            Refresh::Stream
            | Refresh::Friends
            | Refresh::Members(_)
            | Refresh::Notice(_)
            | Refresh::Contact(_)
            | Refresh::GroupInfo(_) => 5,
            Refresh::Recent(_) => 6,
            Refresh::Details(_) => 7,
        }
    }
    fn run(&mut self) -> Result<()> {
        self.seq = self.store.prefs()?.request_seq;
        self.emit(Delta::Connection(Connection::Local));
        self.emit(Delta::Profile(self.store.account()?.profile));
        self.emit_friends()?;
        self.emit(Delta::Prefs(self.store.prefs()?));
        for row in self.store.all_pending()? {
            if row.delivery == Delivery::Pending {
                self.store.mark_failed(row.key, Delivery::Uncertain)?;
            }
        }
        // The local subscription snapshot is emitted before any RPC.
        self.list = Some((1, 100));
        self.publish()?;
        if let Some(chat) = self.store.prefs()?.last_chat {
            self.open(WindowSub {
                sub: 1,
                chat,
                anchor: Anchor::Unread,
                before: 80,
                after: 80,
            })?;
        }
        self.start_assets();
        self.online()?;
        loop {
            enum Work {
                User(Intent),
                Live(StreamEvent),
                Refresh(Refresh),
                Reconnect,
                Completed(Completion),
            }
            let work = {
                let mut q = self.inbox.lanes.lock().unwrap();
                loop {
                    if let Some(i) = q.users.pop_front() {
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
                        && self.connection == Connection::Online
                        && let Some(r) = q
                            .refresh
                            .iter()
                            .filter(|r| {
                                self.retry_after.get(r).is_none_or(|t| *t <= Instant::now())
                            })
                            .min_by_key(|r| self.priority(r))
                            .cloned()
                    {
                        q.refresh.remove(&r);
                        self.retry_after.remove(&r);
                        break Work::Refresh(r);
                    }
                    if self.net_pending == 0
                        && (matches!(self.connection,Connection::Offline {retry_at,..} if Instant::now()>=retry_at)
                            || self.reconnect_at.is_some_and(|at| Instant::now() >= at))
                    {
                        break Work::Reconnect;
                    }
                    q = self.inbox.wake.wait_timeout(q, self.timing.poll).unwrap().0;
                }
            };
            let writes = self.store.writes();
            let result = match work {
                Work::User(Intent::Stop) => break,
                Work::User(i) => self.intent(i),
                Work::Live(e) => self.live(e),
                Work::Refresh(r) => {
                    self.active_refresh = Some(r.clone());
                    self.refresh(r)
                }
                Work::Reconnect => self.online(),
                Work::Completed(done) => done(self),
            };
            if self.net_pending == 0
                && let Some(r) = self.active_refresh.take()
                && r != Refresh::Stream
                && !self.retry_after.contains_key(&r)
            {
                self.retry_attempts.remove(&r);
            }
            match result {
                Err(error) => self.emit(Delta::Error(error.to_string())),
                Ok(()) if self.store.writes() != writes => self.emit(Delta::StoreSaved),
                Ok(()) => {}
            }
        }
        self.inbox.lanes.lock().unwrap().stopped = true;
        self.inbox.wake.notify_all();
        self.assets_stop.store(true, Ordering::Relaxed);
        self.stop_stream();
        Ok(())
    }
    fn stop_stream(&mut self) {
        if let Some((stop, j)) = self.stream.take() {
            {
                let _q = self.inbox.lanes.lock().unwrap();
                stop.store(true, Ordering::Relaxed);
                self.inbox.wake.notify_all();
            }
            j.thread().unpark();
            drop(j);
        }
    }
    fn start_stream(&mut self) -> Result<()> {
        self.stop_stream();
        let stop = Arc::new(AtomicBool::new(false));
        let inbox = self.inbox.clone();
        let deliver_stop = stop.clone();
        let deliver: Deliver = Arc::new(move |e| inbox.live_with_cancel(e, Some(&deliver_stop)));
        let revision = self.store.sync_revision()?;
        let net_stop = stop.clone();
        let net_deliver = deliver.clone();
        self.rpc(
            move |backend| backend.stream(revision, net_stop, net_deliver),
            move |engine, result| {
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
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
                        if !stream_stop.load(Ordering::Relaxed) {
                            deliver(StreamEvent::Exited);
                        }
                    })
                    .map_err(StoreError::Io)?;
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
        self.reconnect_at = None;
        self.attempt = self.attempt.saturating_add(1);
        self.connection(Connection::Connecting {
            attempt: self.attempt,
        });
        self.rpc(
            move |backend| backend.profile(),
            move |engine, result| {
                match result {
                    Ok(p) => {
                        let account = engine.store.account()?;
                        let own_mid = if account.own_mid.is_empty() {
                            engine.store_mid.as_deref().unwrap_or_default()
                        } else {
                            &account.own_mid
                        };
                        if !own_mid.is_empty() && own_mid != p.mid {
                            engine.connection(Connection::SessionInvalid);
                            return Ok(());
                        }
                        if account.own_mid.is_empty()
                            && engine.store_mid.as_deref() != Some(p.mid.as_str())
                            && let Some(root) = &engine.account_root
                        {
                            use sha2::{Digest, Sha256};
                            let mut target =
                                Store::open(&root.join(format!("{:x}", Sha256::digest(&p.mid))))?;
                            for row in target.all_pending()? {
                                if row.delivery == Delivery::Pending {
                                    target.mark_failed(row.key, Delivery::Uncertain)?;
                                }
                            }
                            let keys = engine.store.carry_local_to(&mut target)?;
                            engine.store = target;
                            engine.store_mid = Some(p.mid.clone());
                            engine
                                .inbox
                                .lanes
                                .lock()
                                .unwrap()
                                .refresh
                                .retain(|r| !matches!(r, Refresh::Send(_)));
                            for key in keys {
                                engine.inbox.refresh(Refresh::Send(key));
                            }
                            engine.emit(Delta::Prefs(engine.store.prefs()?));
                            if engine.window.is_none()
                                && let Some(chat) = engine.store.prefs()?.last_chat
                            {
                                engine.open(WindowSub {
                                    sub: 1,
                                    chat,
                                    anchor: Anchor::Unread,
                                    before: 80,
                                    after: 80,
                                })?;
                            }
                        }
                        engine.store.set_account(&p.mid, p.clone())?;
                        if let Some(root) = &engine.account_root {
                            std::fs::create_dir_all(root)?;
                            std::fs::write(root.join("last-mid.tmp"), &p.mid)?;
                            std::fs::rename(root.join("last-mid.tmp"), root.join("last-mid"))?;
                        }
                        engine.seq = engine.store.prefs()?.request_seq;
                        engine.emit(Delta::Profile(p));
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
                    }
                    Err(e) => {
                        engine.reconnect_at =
                            Some(Instant::now() + engine.timing.delay(engine.attempt));
                        engine.error(e);
                    }
                };
                Ok(())
            },
        );
        Ok(())
    }
    fn next_seq(&mut self) -> Result<u32> {
        self.seq = self
            .seq
            .checked_add(1)
            .filter(|seq| *seq <= 0x7fff_ffff)
            .ok_or_else(|| StoreError::Invalid("request sequence exhausted".into()))?;
        let mut prefs = self.store.prefs()?;
        prefs.request_seq = self.seq;
        self.store.set_prefs(&prefs)?;
        Ok(self.seq)
    }
    fn open(&mut self, w: WindowSub) -> Result<()> {
        if self.store.chat(&w.chat)?.is_none() {
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
        self.inbox.refresh(Refresh::Recent(w.chat.clone()));
        self.window = Some(w);
        self.force_window = true;
        self.publish()
    }
    // Friends always reach the UI in store order, so a refresh does not reshuffle rows.
    fn emit_friends(&mut self) -> Result<()> {
        let rows = self.store.contacts()?.into_iter().collect::<Vec<_>>();
        for row in &rows {
            self.emit(Delta::Contact(row.clone()));
        }
        self.emit(Delta::Friends(
            rows.into_iter()
                .filter(|r: &crate::store::Friend| r.group || r.profile.relation == Some(0))
                .collect(),
        ));
        Ok(())
    }
    fn intent(&mut self, i: Intent) -> Result<()> {
        match i {
            Intent::SignIn => {
                if self.connection == Connection::SigningIn {
                    return Ok(());
                }
                self.generation += 1;
                self.stop_stream();
                self.inbox.pair_cancel.store(false, Ordering::Relaxed);
                self.connection(Connection::SigningIn);
                let tx = self.tx.clone();
                let cancel = self.inbox.pair_cancel.clone();
                self.rpc(
                    move |backend| {
                        backend.sign_in(
                            cancel,
                            Arc::new(move |e| {
                                tx.send(Delta::Pair(e));
                            }),
                        )
                    },
                    move |engine, result| {
                        match result {
                            Ok(()) => {
                                engine.generation += 1;
                                {
                                    let mut lanes = engine.inbox.lanes.lock().unwrap();
                                    lanes.live.clear();
                                    lanes.refresh.clear();
                                }
                                engine.inbox.wake.notify_all();
                                engine.list_sweep.clear();
                                engine.list_cursor = None;
                                engine.list_more = false;
                                engine.retry_after.clear();
                                for row in engine.store.all_pending()? {
                                    if row.delivery == Delivery::Pending {
                                        engine.store.mark_failed(row.key, Delivery::Uncertain)?;
                                    }
                                }
                                engine.store = Store::open_in_memory()?;
                                engine.store_mid = None;
                                engine.window = None;
                                engine.chat_keys.clear();
                                engine.message_keys.clear();
                                engine.emit(Delta::Reset);
                                engine.start_assets();
                                engine.online()?;
                                engine.publish()?;
                            }
                            Err(_) => {
                                engine.connection(Connection::AuthExpired);
                                engine.emit(Delta::PairFailed {
                                    cancelled: engine.inbox.pair_cancel.load(Ordering::Relaxed),
                                });
                            }
                        }
                        Ok(())
                    },
                );
            }
            Intent::Contact(mid) => {
                if let Some(row) = self.store.contacts()?.into_iter().find(|v| v.id == mid) {
                    self.emit(Delta::Contact(row));
                }
                self.inbox.refresh(Refresh::Contact(mid));
            }
            Intent::GroupInfo(chat) => {
                self.emit(Delta::GroupInfo {
                    chat: chat.clone(),
                    data: self.store.full_chat(&chat)?,
                });
                self.inbox.refresh(Refresh::GroupInfo(chat.clone()));
                self.inbox.refresh(Refresh::Details(chat));
            }
            Intent::ProfileAction(action) => {
                let seq = self.next_seq()?;
                let net_action = action.clone();
                self.rpc_completion(
                    move |b| b.profile_action(&net_action, Value::Null, seq),
                    move |engine, result, _| {
                        let error = result.err().map(|_| {
                            match action {
                                ProfileAction::Rename { .. } => "Couldn't rename.",
                                ProfileAction::AddFriend { .. } => "Couldn't add friend.",
                                ProfileAction::Mute { .. } => "Couldn't change notifications.",
                                ProfileAction::Leave { .. } => "Couldn't leave group.",
                            }
                            .to_owned()
                        });
                        if error.is_none() {
                            match &action {
                                ProfileAction::Rename { mid, name }
                                | ProfileAction::AddFriend { mid, chat: name } => {
                                    if let Some(raw) =
                                        engine.store.contacts()?.into_iter().find(|v| v.id == *mid)
                                    {
                                        let mut row = raw;
                                        if matches!(&action, ProfileAction::Rename { .. }) {
                                            if row.profile.original_name.is_none() {
                                                row.profile.original_name = Some(row.name.clone());
                                            }
                                            row.profile.overridden_name = Some(name.clone());
                                            row.name = name.clone();
                                        } else {
                                            row.profile.relation = Some(0);
                                        }
                                        engine.store.save_contacts(std::slice::from_ref(&row))?;
                                        engine.emit(Delta::Contact(row));
                                        engine.emit_friends()?;
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
                self.emit_friends()?;
                self.inbox.refresh(Refresh::Friends);
            }
            Intent::Members(chat) => {
                self.emit(Delta::Members {
                    rows: self.store.members(&chat)?.into_iter().collect::<Vec<_>>(),
                    chat: chat.clone(),
                });
                self.inbox.refresh(Refresh::Members(chat));
            }
            Intent::Asset(asset) => {
                self.asset(asset, crate::client::desktop::assets::Purpose::Thumbnail)?
            }
            Intent::Original(asset) => self.asset(
                asset,
                crate::client::desktop::assets::Purpose::Original { request: 0 },
            )?,
            Intent::File { asset, name } => self.asset(
                asset,
                crate::client::desktop::assets::Purpose::File { request: 0, name },
            )?,
            Intent::RetryAsset(key) => {
                if let Some(media) = &self.media {
                    (media.retry)(key);
                } else {
                    self.waiting_retries.push(key);
                }
            }
            Intent::SearchChats { sub, query } => {
                self.list = Some((sub, 100));
                self.search = query;
                self.force_list = true;
                self.publish()?;
            }
            Intent::ChatList { sub, limit } => {
                self.list = Some((sub, limit));
                self.search.clear();
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
                if !self.store.stored_page(&chat, &kind)? {
                    self.defer_page(chat, kind);
                }
            }
            Intent::Send { chat, text } => {
                if self.store.chat(&chat)?.is_some_and(|c| c.left) {
                    return Ok(());
                }
                let seq = self.next_seq()?;
                let (key, _) = self.store.insert_pending(
                    &chat,
                    LocalId(format!("{}-{seq}", now_ms())),
                    seq,
                    Content::text(&text),
                    now_ms(),
                )?;
                self.store.clear_draft(&chat)?;
                self.publish()?;
                self.emit(Delta::Submitted {
                    chat: chat.clone(),
                    text,
                });
                self.inbox.refresh(Refresh::Send(key));
            }
            Intent::Resend(key) => {
                let old = self.store.message(key)?;
                if self.in_flight.contains(&key)
                    || !matches!(
                        old.delivery,
                        Delivery::Rejected { .. } | Delivery::Uncertain
                    )
                {
                    return Ok(());
                }
                let seq = self.next_seq()?;
                let (new, _) = self.store.insert_pending(
                    &old.chat_id,
                    LocalId(format!("{}-{seq}", now_ms())),
                    seq,
                    old.body,
                    now_ms(),
                )?;
                self.store.discard(key)?;
                self.publish()?;
                self.inbox.refresh(Refresh::Send(new));
            }
            Intent::Retry(key) => {
                if self.in_flight.contains(&key) || !self.store.message(key)?.delivery.is_failed() {
                    return Ok(());
                }
                let seq = self.next_seq()?;
                self.store.retry(key, seq)?;
                self.publish()?;
                self.inbox.refresh(Refresh::Send(key));
            }
            Intent::Discard(key) => {
                if self.in_flight.contains(&key) || !self.store.message(key)?.delivery.is_failed() {
                    return Ok(());
                }
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
                if self.connection == Connection::SigningIn {
                    self.emit(Delta::UnsendFailed(key));
                    return Ok(());
                }
                let row = self.store.message(key)?;
                if row.outgoing
                    && !row.body.is_withdrawn()
                    && let Some(id) = row.server_id
                {
                    let seq = self.next_seq()?;
                    let net_id = id.clone();
                    self.rpc_completion(
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
                let window = self.window.take();
                let result = self.publish();
                self.window = window;
                result?;
            }
            Intent::Prefs(mut prefs) => {
                let stored = self.store.prefs()?;
                prefs.last_chat = stored.last_chat;
                prefs.request_seq = stored.request_seq;
                self.store.set_settings(&prefs)?;
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
    fn send(&mut self, key: Key) -> Result<()> {
        let m = self.store.message(key)?;
        let text = m.body.as_text();
        let seq = m
            .request_seq
            .ok_or_else(|| StoreError::Invalid("send sequence missing".into()))?;
        let chat = m.chat_id.clone();
        let text = text.to_owned();
        self.in_flight.insert(key);
        self.rpc_completion(
            move |backend| backend.send(&chat, &text, seq),
            move |engine, result, current| {
                engine.in_flight.remove(&key);
                match result {
                    Ok(row) => {
                        // Apply queued live operations before the result, preserving an echo-first RPC.
                        if current {
                            loop {
                                let event = engine.inbox.lanes.lock().unwrap().live.pop_front();
                                engine.inbox.wake.notify_all();
                                match event {
                                    Some(e) => engine.live(e)?,
                                    None => break,
                                }
                            }
                        }
                        engine.store.link_sent(seq, row.server_id, row.created_ms)?;
                    }
                    Err(e) => {
                        let delivery = match &e {
                            Error::Rejected(reason) => Delivery::Rejected {
                                reason: reason.clone(),
                            },
                            _ => Delivery::Uncertain,
                        };
                        engine.store.mark_failed(key, delivery)?;
                        if current && !matches!(e, Error::Rejected(_)) {
                            engine.error(e);
                        }
                    }
                };
                engine.publish()
            },
        );
        Ok(())
    }
    fn load_chats(&mut self, next: bool) -> Result<()> {
        if !next && self.list_more {
            self.inbox.refresh(Refresh::MoreChats);
            return Ok(());
        }

        let cursor = if next { self.list_cursor.clone() } else { None };
        self.rpc(
            move |backend| backend.chats(cursor),
            move |engine, result| {
                match result {
                    Ok(mut page) => {
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
                        for m in page.previews {
                            engine.store.apply_live(&m.chat_id, &m, false, None)?;
                        }
                        for c in &page.chats {
                            if c.title.trim().is_empty() || (c.is_group && c.member_count.is_none())
                            {
                                engine.inbox.refresh(Refresh::Details(c.id.clone()));
                            }
                        }
                        if page.more {
                            engine.inbox.refresh(Refresh::MoreChats);
                        }
                        engine.emit(Delta::MoreChats(page.more));
                        engine.publish()?;
                    }
                    Err(e) => {
                        engine.rpc_failed(
                            if next {
                                Refresh::MoreChats
                            } else {
                                Refresh::ChatList
                            },
                            e,
                        );
                    }
                };
                Ok(())
            },
        );
        Ok(())
    }
    fn resolve_senders(&self, rows: &[MessageInput]) -> Result<()> {
        let known = self
            .store
            .contacts()?
            .into_iter()
            .map(|c| c.id)
            .collect::<BTreeSet<_>>();
        for row in rows {
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
        self.rpc(
            move |backend| backend.messages(&net_chat, &net_kind),
            move |engine, result| {
                match result {
                    Ok(page) => {
                        engine.resolve_senders(&page.rows)?;
                        let recent = matches!(page.kind, PageKind::Recent);
                        engine.store.apply_page(
                            &chat,
                            page.kind,
                            &page.rows,
                            page.reaches_start,
                        )?;
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
                    }
                    Err(e) => {
                        engine.rpc_failed(Self::page_refresh(chat, kind), e);
                    }
                };
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
            PageKind::Recent => Refresh::Recent(chat),
            PageKind::Older { before } => Refresh::Older(chat, before),
            PageKind::Newer { after } => Refresh::Newer(chat, after),
        }
    }
    fn retry_later(&mut self, r: Refresh) {
        let attempt = self.retry_attempts.entry(r.clone()).or_default();
        *attempt = attempt.saturating_add(1);
        self.retry_after
            .insert(r.clone(), Instant::now() + self.timing.delay(*attempt));
        self.inbox.refresh(r);
    }
    fn refresh(&mut self, r: Refresh) -> Result<()> {
        if self.connection == Connection::SigningIn {
            return Ok(());
        }
        match r {
            Refresh::Stream => self.start_stream()?,
            Refresh::Send(key) => self.send(key)?,
            Refresh::Notice(mid) => {
                let net = mid.clone();
                self.rpc(
                    move |b| b.notice(&net),
                    move |engine, result| {
                        match result {
                            Ok(data) => engine.emit(Delta::Notice { mid, data }),
                            Err(e) => engine.rpc_failed(Refresh::Notice(mid), e),
                        };
                        Ok(())
                    },
                );
            }
            Refresh::Contact(mid) => {
                let net = mid.clone();
                self.rpc(
                    move |b| b.contact(&net),
                    move |engine, result| {
                        match result {
                            Ok(row) => {
                                if row.profile.relation == Some(2) {
                                    engine.inbox.refresh(Refresh::Notice(mid.clone()));
                                }
                                engine.store.save_contacts(std::slice::from_ref(&row))?;
                                engine.emit(Delta::Contact(row));
                            }
                            // A contact the server returns without a profile (e.g. a deleted
                            // account) is a final answer; retrying would loop forever.
                            Err(Error::Decode) => {}
                            Err(e) => engine.rpc_failed(Refresh::Contact(mid), e),
                        };
                        Ok(())
                    },
                );
            }
            Refresh::GroupInfo(chat) => {
                let net = chat.clone();
                self.rpc(
                    move |b| b.full_chat(&net),
                    move |engine, result| {
                        match result {
                            Ok(data) => {
                                engine.store.save_full_chat(&chat, data.clone())?;
                                if let Some(members) = data
                                    .pointer("/extra/groupExtra/memberMids")
                                    .and_then(Value::as_object)
                                {
                                    let own = engine.store.account()?.own_mid;
                                    if !own.is_empty() {
                                        engine.store.set_chat_membership(
                                            &chat,
                                            !members.contains_key(&own),
                                        )?;
                                        engine.publish()?;
                                    }
                                }
                                engine.emit(Delta::GroupInfo { chat, data });
                            }
                            Err(e) => engine.rpc_failed(Refresh::GroupInfo(chat), e),
                        }
                        Ok(())
                    },
                );
            }
            Refresh::Friends => {
                self.rpc(
                    move |backend| backend.friends(),
                    move |engine, result| {
                        match result {
                            Ok(mut rows) => {
                                for row in &mut rows {
                                    if !row.group && row.profile.relation.is_none() {
                                        row.profile.relation = Some(0);
                                    }
                                }
                                engine.store.save_contacts(&rows)?;
                                engine.emit_friends()?;
                            }
                            Err(e) => engine.rpc_failed(Refresh::Friends, e),
                        };
                        Ok(())
                    },
                );
            }
            Refresh::Members(chat) => {
                let net_chat = chat.clone();
                self.rpc(
                    move |backend| backend.members(&net_chat),
                    move |engine, result| {
                        match result {
                            Ok(mut rows) => {
                                let existing = engine.store.contacts()?;
                                for member in &mut rows {
                                    if !member.resolved
                                        && let Some(old) =
                                            existing.iter().find(|c| c.id == member.contact.id)
                                    {
                                        let invited = member.contact.profile.invited;
                                        member.contact = old.clone();
                                        member.contact.profile.invited = invited;
                                    }
                                }
                                let contacts = rows
                                    .iter()
                                    .filter(|m| !m.is_self && m.resolved)
                                    .filter_map(|m| {
                                        let mut contact = m.contact.clone();
                                        contact.profile.invited = false;
                                        let old = existing.iter().find(|v| v.id == contact.id);
                                        if old.is_some_and(|v| v.profile.relation == Some(0)) {
                                            return None;
                                        }
                                        Some(contact)
                                    })
                                    .collect::<Vec<_>>();
                                engine.store.save_contacts(&contacts)?;
                                engine.store.save_members(&chat, &rows)?;
                                engine.emit(Delta::Members { chat, rows });
                            }
                            Err(e) => engine.rpc_failed(Refresh::Members(chat), e),
                        };
                        Ok(())
                    },
                );
            }
            Refresh::ChatList => self.load_chats(false)?,
            Refresh::MoreChats => self.load_chats(true)?,
            Refresh::Older(chat, before) => self.page(chat, PageKind::Older { before })?,
            Refresh::Newer(chat, after) => self.page(chat, PageKind::Newer { after })?,
            Refresh::Receipts(chat) => {
                let net_chat = chat.clone();
                self.rpc(
                    move |backend| backend.receipts(&net_chat),
                    move |engine, result| {
                        match result {
                            Ok(ids) => {
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
                            }
                            Err(e) => {
                                engine.rpc_failed(Refresh::Receipts(chat), e);
                            }
                        };
                        Ok(())
                    },
                );
            }
            Refresh::Recent(chat) => self.page(chat, PageKind::Recent)?,
            Refresh::Details(chat) => {
                let net_chat = chat.clone();
                self.rpc(
                    move |backend| backend.details(&net_chat),
                    move |engine, result| {
                        match result {
                            Ok(rows) => {
                                engine.store.upsert_details(&rows)?;
                                engine.publish()?;
                            }
                            Err(e) => {
                                engine.rpc_failed(Refresh::Details(chat), e);
                            }
                        };
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
                    let seq = self.next_seq()?;
                    self.rpc(
                        move |backend| backend.mark_read(&net_chat, &net_id, seq),
                        move |engine, result| {
                            match result {
                                Ok(()) => {
                                    engine.store.confirm_read(&chat, &id)?;
                                }
                                Err(e) => {
                                    engine.rpc_failed(Refresh::Read(chat), e);
                                }
                            };
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
        if let Some(media) = &self.media {
            (media.request)(asset, purpose);
        } else {
            self.waiting_assets.push_back((asset, purpose));
        }
        Ok(())
    }
    fn live(&mut self, e: StreamEvent) -> Result<()> {
        if self.connection == Connection::SigningIn {
            match e {
                StreamEvent::Connection(_)
                | StreamEvent::Connecting(_)
                | StreamEvent::Retry { .. }
                | StreamEvent::Exited
                | StreamEvent::Raw(_) => return Ok(()),
                StreamEvent::RefreshToken { reply } => {
                    let _ = reply.send(Err(Error::Auth));
                    return Ok(());
                }
                StreamEvent::Authorize { reply, .. } => {
                    let _ = reply.send(Err(crate::client::desktop::assets::Failure::Network));
                    return Ok(());
                }
                StreamEvent::Resync { complete, .. } => {
                    let _ = complete.send(false);
                    return Ok(());
                }
                _ => {}
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
            StreamEvent::Asset(c) => self.emit(asset_delta(c)),
            StreamEvent::Authorize { asset, reply } => {
                self.rpc(
                    move |backend| {
                        let _ = reply.send(backend.authorize(&asset));
                    },
                    |_, ()| Ok(()),
                );
            }
            StreamEvent::Operation(op) => {
                if op.revision > self.store.sync_revision()? {
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
                    self.store.apply_operation(&op, self.continuous)?;
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
                if raw["revision"]
                    .as_i64()
                    .or_else(|| raw["revision"].as_str()?.parse().ok())
                    .is_some_and(|r| r > self.store.sync_revision().unwrap_or(0))
                {
                    let id = raw["param1"].as_str().unwrap_or_default().to_owned();
                    match raw["type"].as_u64() {
                        Some(4 | 49) => self.inbox.refresh(Refresh::Contact(id)),
                        Some(41)
                            if self
                                .store
                                .chat(&ChatId(id.clone()))?
                                .is_some_and(|c| c.is_group) =>
                        {
                            self.store.remove_chat(&ChatId(id))?;
                            self.publish()?;
                            self.inbox.refresh(Refresh::ChatList);
                        }
                        Some(121 | 122 | 127 | 128) => {
                            if matches!(raw["type"].as_u64(), Some(127))
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
                    move |engine, result| {
                        match result {
                            Ok(op) => engine.live(StreamEvent::Operation(op))?,
                            Err(e) => engine.error(e),
                        };
                        Ok(())
                    },
                );
            }
            StreamEvent::RefreshToken { reply } => {
                let stop = self.assets_stop.clone();
                self.rpc(
                    move |backend| backend.refresh_token(&stop),
                    move |_, result| {
                        let _ = reply.send(result);
                        Ok(())
                    },
                );
            }
            StreamEvent::Retry { attempt, error } => {
                self.attempt = self.attempt.max(attempt);
                self.error(error);
            }
            StreamEvent::Connecting(attempt) => {
                if matches!(
                    self.connection,
                    Connection::Offline { .. } | Connection::Connecting { .. }
                ) {
                    self.connection(Connection::Connecting {
                        attempt: self.attempt.max(attempt),
                    });
                }
            }
            StreamEvent::Connection(Ok(())) => {
                self.retry_attempts.remove(&Refresh::Stream);
                self.attempt = 0;
                let reconnect = self.connection != Connection::Online;
                self.connection(Connection::Online);
                if reconnect {
                    self.inbox.refresh(Refresh::ChatList);
                    if let Some(w) = &self.window {
                        self.inbox.refresh(Refresh::Recent(w.chat.clone()));
                    }
                }
            }
            StreamEvent::Connection(Err(e)) => self.error(e),
            StreamEvent::Exited => {
                self.continuous = false;
                if !matches!(
                    self.connection,
                    Connection::AuthExpired | Connection::SessionInvalid | Connection::Fatal
                ) {
                    self.retry_later(Refresh::Stream);
                }
            }
        };
        Ok(())
    }
    fn resync(&mut self, revision: Option<i64>, complete: mpsc::SyncSender<bool>) -> Result<()> {
        let chat = self.window.as_ref().map(|w| w.chat.clone());
        self.rpc(
            move |backend| {
                chat.map(|chat| {
                    let result = backend.messages(&chat, &PageKind::Recent);
                    (chat, result)
                })
            },
            move |engine, result| {
                if let Some((chat, result)) = result {
                    match result {
                        Ok(page) => {
                            engine.store.apply_page(
                                &chat,
                                page.kind,
                                &page.rows,
                                page.reaches_start,
                            )?;
                        }
                        Err(e) => {
                            engine.rpc_failed(Refresh::Recent(chat), e);
                            let _ = complete.send(false);
                            return Ok(());
                        }
                    }
                }
                engine.inbox.refresh(Refresh::ChatList);
                for c in engine.store.chat_list(usize::MAX, 0)? {
                    if engine.window.as_ref().is_none_or(|w| w.chat != c.chat.id) {
                        engine.inbox.refresh(Refresh::Recent(c.chat.id));
                    }
                }
                if let Some(revision) = revision {
                    engine.store.set_sync_revision(revision)?;
                }
                engine.continuous = true;
                engine.publish()?;
                let _ = complete.send(true);
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
                || !changes.chats_removed.is_empty())
        {
            let all = if self.search.is_empty() {
                self.store.chat_list(limit, 0)?
            } else {
                self.store.search_chats(&self.search)?
            };
            let mut rows = all
                .iter()
                .filter(|row| {
                    self.force_list
                        || changes.chats_changed.contains(&row.chat.id)
                        || !self.chat_keys.contains(&row.chat.id)
                })
                .cloned()
                .collect::<Vec<_>>();
            // Selected chats can fall outside the loaded list. Their metadata
            // (especially drafts) still needs a row delta without expanding order.
            for id in &changes.chats_changed {
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
                if let Some(id) = row.reply.as_ref().map(|r| r.message_id.as_str())
                    && let Some(target) = self.store.message_by_server(&ServerId(id.into()))?
                {
                    replies.push(target);
                }
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
        Ok(Payload::Image(pixels)) => Delta::Asset { key: c.key, pixels },
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
        self.inbox.lanes.lock().unwrap().stopped = true;
        self.inbox.wake.notify_all();
        self.assets_stop.store(true, Ordering::Relaxed);
        self.stop_stream();
    }
}
