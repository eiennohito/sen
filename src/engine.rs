//! One worker, one store writer, and one ordered stream of UI deltas.
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
    Local,
    Connecting { attempt: u32 },
    Online,
    Offline { retry_at: Instant, cause: NetCause },
    AuthExpired,
    SessionInvalid,
    SigningIn,
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
    ChatList {
        sub: u64,
        limit: usize,
    },
    Window(WindowSub),
    MoreChats,
    SignIn,
    Friends,
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
    Retry(Key),
    Resend(Key),
    Discard(Key),
    Copy(Key),
    Draft {
        chat: ChatId,
        draft: Draft,
    },
    Pref {
        key: String,
        value: Value,
    },
    MarkRead {
        chat: ChatId,
        id: ServerId,
    },
    Stop,
}
#[derive(Clone)]
pub enum Delta {
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
    Profile(Value),
    MoreChats(bool),
    Notify(crate::store::MessageInput),
    Friends(Vec<crate::client::desktop::Friend>),
    Members {
        chat: ChatId,
        rows: Vec<crate::client::desktop::Member>,
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
    Pref {
        key: String,
        value: Value,
    },
    Copy {
        chat: ChatId,
        text: String,
    },
    Error(String),
    StoreSaved,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Refresh {
    Send(Key),
    Friends,
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
        Self::start_with(move || Ok((backend, location)))
    }
    pub fn start_with<B: Backend + 'static>(
        open: impl FnOnce() -> anyhow::Result<(B, StoreLocation)> + Send + 'static,
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
            let (backend, location) = match open() {
                Ok(account) => account,
                Err(_) => {
                    let _ = tx.send(Delta::Connection(Connection::SessionInvalid));
                    return;
                }
            };
            use sha2::{Digest, Sha256};
            let mut account_root = None;
            let store = match location {
                StoreLocation::Memory => Store::open_in_memory(),
                StoreLocation::Directory(p) => Store::open(&p),
                StoreLocation::Account { root, mid } => {
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
                        backend: Box::new(backend),
                        store,
                        account_root,
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
                        continuous: true,
                        retry_after: Default::default(),
                    };
                    if let Err(error) = e.run() {
                        e.emit(Delta::Error(error.to_string()));
                    }
                }
                Err(e) => {
                    let _ = tx.send(Delta::Error(e.to_string()));
                }
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
    retry_after: std::collections::BTreeMap<Refresh, Instant>,
    continuous: bool,
    list_sweep: Vec<ChatInput>,
    backend: Box<dyn Backend>,
    store: Store,
    account_root: Option<PathBuf>,
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
                retry_at: Instant::now() + Duration::from_secs(2),
                cause,
            },
            _ => return,
        };
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
            Refresh::Friends | Refresh::Members(_) => 5,
            Refresh::Recent(_) => 6,
            Refresh::Details(_) => 7,
        }
    }
    fn run(&mut self) -> Result<()> {
        crate::client::desktop::set_cancellation_token(self.assets_stop.clone());
        self.seq = self
            .store
            .get_pref("request_seq")?
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        self.emit(Delta::Connection(Connection::Local));
        self.emit(Delta::Profile(self.store.account()?.profile_json));
        for (key, value) in self.store.prefs()? {
            self.emit(Delta::Pref { key, value });
        }
        for row in self.store.all_pending()? {
            if row.delivery == Delivery::Pending {
                self.store.mark_failed(row.key, Delivery::Uncertain)?;
            }
        }
        // The local subscription snapshot is emitted before any RPC.
        self.list = Some((1, 100));
        self.publish()?;
        if let Some(v) = self.store.get_pref("last_chat")? {
            let chat = ChatId(v.as_str().unwrap().into());
            self.open(WindowSub {
                sub: 1,
                chat,
                anchor: Anchor::Unread,
                before: 80,
                after: 80,
            })?;
        }
        let inbox = self.inbox.clone();
        self.backend
            .start_assets(self.assets_stop.clone(), Arc::new(move |e| inbox.live(e)));
        self.online()?;
        loop {
            enum Work {
                User(Intent),
                Live(StreamEvent),
                Refresh(Refresh),
                Reconnect,
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
                    if self.connection == Connection::Online
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
                    if matches!(self.connection,Connection::Offline {retry_at,..} if Instant::now()>=retry_at)
                    {
                        break Work::Reconnect;
                    }
                    q = self
                        .inbox
                        .wake
                        .wait_timeout(q, Duration::from_millis(100))
                        .unwrap()
                        .0;
                }
            };
            let writes = self.store.writes();
            let result = match work {
                Work::User(Intent::Stop) => break,
                Work::User(i) => self.intent(i),
                Work::Live(e) => self.live(e),
                Work::Refresh(r) => self.refresh(r),
                Work::Reconnect => self.online(),
            };
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
            let _ = j.join();
        }
    }
    fn start_stream(&mut self) -> Result<()> {
        self.stop_stream();
        let stop = Arc::new(AtomicBool::new(false));
        let inbox = self.inbox.clone();
        let deliver_stop = stop.clone();
        let deliver: Deliver = Arc::new(move |e| inbox.live_with_cancel(e, Some(&deliver_stop)));
        let run =
            match self
                .backend
                .stream(self.store.sync_revision()?, stop.clone(), deliver.clone())
            {
                Ok(run) => run,
                Err(e) => {
                    self.error(e);
                    return Ok(());
                }
            };
        let stream_stop = stop.clone();
        let j = thread::spawn(move || {
            run();
            if !stream_stop.load(Ordering::Relaxed) {
                deliver(StreamEvent::Exited);
            }
        });
        self.stream = Some((stop, j));
        Ok(())
    }
    fn online(&mut self) -> Result<()> {
        self.connection(Connection::Connecting { attempt: 1 });
        match self.backend.profile() {
            Ok(p) => {
                let account = self.store.account()?;
                if !account.own_mid.is_empty() && account.own_mid != p.mid {
                    self.connection(Connection::SessionInvalid);
                    return Ok(());
                }
                if account.own_mid.is_empty()
                    && let Some(root) = &self.account_root
                {
                    use sha2::{Digest, Sha256};
                    let mut target =
                        Store::open(&root.join(format!("{:x}", Sha256::digest(&p.mid))))?;
                    for row in target.all_pending()? {
                        if row.delivery == Delivery::Pending {
                            target.mark_failed(row.key, Delivery::Uncertain)?;
                        }
                    }
                    let keys = self.store.carry_local_to(&mut target)?;
                    self.store = target;
                    self.inbox
                        .lanes
                        .lock()
                        .unwrap()
                        .refresh
                        .retain(|r| !matches!(r, Refresh::Send(_)));
                    for key in keys {
                        self.inbox.refresh(Refresh::Send(key));
                    }
                    for (key, value) in self.store.prefs()? {
                        self.emit(Delta::Pref { key, value });
                    }
                    if self.window.is_none()
                        && let Some(chat) = self.store.get_pref("last_chat")?
                    {
                        self.open(WindowSub {
                            sub: 1,
                            chat: ChatId(chat.as_str().unwrap().into()),
                            anchor: Anchor::Unread,
                            before: 80,
                            after: 80,
                        })?;
                    }
                }
                self.store.set_account(&p.mid, p.data.clone())?;
                self.seq = self
                    .store
                    .get_pref("request_seq")?
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                self.emit(Delta::Profile(p.data));
                self.connection(Connection::Online);
                self.inbox.refresh(Refresh::ChatList);
                for chat in self.store.pending_reads()? {
                    self.inbox.refresh(Refresh::Read(chat));
                }
                if let Some(w) = &self.window {
                    self.inbox.refresh(Refresh::Recent(w.chat.clone()));
                    self.inbox.refresh(Refresh::Receipts(w.chat.clone()));
                }
                self.start_stream()?;
            }
            Err(e) => self.error(e),
        };
        Ok(())
    }
    fn next_seq(&mut self) -> Result<u32> {
        self.seq += 1;
        self.store.set_pref("request_seq", Value::from(self.seq))?;
        Ok(self.seq)
    }
    fn open(&mut self, w: WindowSub) -> Result<()> {
        if self.store.chat(&w.chat)?.is_none() {
            self.inbox.refresh(Refresh::Details(w.chat.clone()));
        }
        self.store
            .set_pref("last_chat", Value::from(w.chat.0.clone()))?;
        if self.window.as_ref().is_none_or(|old| old.chat != w.chat) {
            self.emit(Delta::Draft {
                chat: w.chat.clone(),
                draft: self.store.load_draft(&w.chat)?,
            });
        }
        self.inbox.refresh(Refresh::Recent(w.chat.clone()));
        self.window = Some(w);
        self.publish()
    }
    // Friends always reach the UI in store order, so a refresh does not reshuffle rows.
    fn emit_friends(&mut self) -> Result<()> {
        let rows = self
            .store
            .contacts()?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.emit(Delta::Friends(rows));
        Ok(())
    }
    fn intent(&mut self, i: Intent) -> Result<()> {
        match i {
            Intent::SignIn => {
                self.stop_stream();
                self.inbox.pair_cancel.store(false, Ordering::Relaxed);
                self.connection(Connection::SigningIn);
                let tx = self.tx.clone();
                let result = self.backend.sign_in(
                    self.inbox.pair_cancel.clone(),
                    Arc::new(move |e| {
                        let _ = tx.send(Delta::Pair(e));
                    }),
                );
                crate::client::desktop::set_cancellation_token(self.assets_stop.clone());
                match result {
                    Ok(()) => {
                        {
                            let mut lanes = self.inbox.lanes.lock().unwrap();
                            lanes.live.clear();
                            lanes.refresh.clear();
                        }
                        self.inbox.wake.notify_all();
                        self.list_sweep.clear();
                        self.retry_after.clear();
                        for row in self.store.all_pending()? {
                            if row.delivery == Delivery::Pending {
                                self.store.mark_failed(row.key, Delivery::Uncertain)?;
                            }
                        }
                        self.store = Store::open_in_memory()?;
                        self.window = None;
                        self.chat_keys.clear();
                        self.message_keys.clear();
                        self.emit(Delta::Reset);
                        let inbox = self.inbox.clone();
                        self.backend.start_assets(
                            self.assets_stop.clone(),
                            Arc::new(move |e| inbox.live(e)),
                        );
                        self.online()?;
                        self.publish()?;
                    }
                    Err(_) => {
                        self.connection(Connection::AuthExpired);
                        self.emit(Delta::PairFailed {
                            cancelled: self.inbox.pair_cancel.load(Ordering::Relaxed),
                        });
                    }
                }
            }
            Intent::Friends => {
                self.emit_friends()?;
                self.inbox.refresh(Refresh::Friends);
            }
            Intent::Members(chat) => {
                self.emit(Delta::Members {
                    rows: self
                        .store
                        .members(&chat)?
                        .into_iter()
                        .map(serde_json::from_value)
                        .collect::<std::result::Result<Vec<_>, _>>()?,
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
            Intent::RetryAsset(key) => self.backend.retry_asset(&key),
            Intent::ChatList { sub, limit } => {
                self.list = Some((sub, limit));
                self.publish()?;
            }
            Intent::Window(w) => self.open(w)?,
            Intent::MoreChats => {
                if let Some((_, limit)) = &mut self.list {
                    *limit += 50;
                }
                if self.connection == Connection::Online {
                    self.load_chats(true)?;
                } else {
                    self.inbox.refresh(Refresh::MoreChats);
                }
            }
            Intent::Page { chat, kind } => {
                if self.connection == Connection::Online {
                    self.page(chat, kind)?;
                } else {
                    self.defer_page(chat, kind);
                }
            }
            Intent::Send { chat, text } => {
                let seq = self.next_seq()?;
                let (key, _) = self.store.insert_pending(
                    &chat,
                    LocalId(format!("{}-{seq}", now_ms())),
                    seq,
                    serde_json::json!({"body":text}),
                    now_ms(),
                )?;
                self.store.clear_draft(&chat)?;
                self.publish()?;
                self.emit(Delta::Submitted {
                    chat: chat.clone(),
                    text,
                });
                if self.connection == Connection::Online {
                    self.send(key)?;
                } else {
                    self.inbox.refresh(Refresh::Send(key));
                }
            }
            Intent::Resend(key) => {
                let old = self.store.message(key)?;
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
                if self.connection == Connection::Online {
                    self.send(new)?;
                } else {
                    self.inbox.refresh(Refresh::Send(new));
                }
            }
            Intent::Retry(key) => {
                let seq = self.next_seq()?;
                self.store.retry(key, seq)?;
                self.publish()?;
                if self.connection == Connection::Online {
                    self.send(key)?;
                } else {
                    self.inbox.refresh(Refresh::Send(key));
                }
            }
            Intent::Discard(key) => {
                self.inbox
                    .lanes
                    .lock()
                    .unwrap()
                    .refresh
                    .remove(&Refresh::Send(key));
                self.store.discard(key)?;
                self.publish()?;
            }
            Intent::Copy(key) => {
                let m = self.store.message(key)?;
                self.emit(Delta::Copy {
                    chat: m.chat_id,
                    text: m.body["body"].as_str().unwrap_or_default().into(),
                });
            }
            Intent::Draft { chat, draft } => {
                self.store.save_draft(&chat, &draft)?;
                self.publish()?;
            }
            Intent::Pref { key, value } => {
                self.store.set_pref(&key, value.clone())?;
                self.emit(Delta::Pref { key, value });
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
        match self.backend.send(
            &m.chat_id,
            m.body["body"].as_str().unwrap(),
            m.request_seq.unwrap(),
        ) {
            Ok(row) => {
                // Apply queued live operations before the result, preserving an echo-first RPC.
                loop {
                    let event = self.inbox.lanes.lock().unwrap().live.pop_front();
                    self.inbox.wake.notify_all();
                    match event {
                        Some(e) => self.live(e)?,
                        None => break,
                    }
                }
                self.store
                    .link_sent(m.request_seq.unwrap(), row.server_id, row.created_ms)?;
            }
            Err(e) => {
                let delivery = match &e {
                    Error::Rejected(reason) => Delivery::Rejected {
                        reason: reason.clone(),
                    },
                    _ => Delivery::Uncertain,
                };
                self.store.mark_failed(key, delivery)?;
                if !matches!(e, Error::Rejected(_)) {
                    self.error(e);
                }
            }
        };
        self.publish()
    }
    fn load_chats(&mut self, next: bool) -> Result<()> {
        match self.backend.chats(next) {
            Ok(page) => {
                if !next {
                    self.list_sweep.clear();
                }
                self.list_sweep.extend(page.chats.clone());
                self.store.upsert_chats(&self.list_sweep, !page.more)?;
                for m in page.previews {
                    self.store.apply_live(&m.chat_id, &m, false, None)?;
                }
                for c in &page.chats {
                    if c.title.is_empty() || (c.kind != 0 && c.member_count.is_none()) {
                        self.inbox.refresh(Refresh::Details(c.id.clone()));
                    }
                }
                if page.more {
                    self.inbox.refresh(Refresh::MoreChats);
                }
                self.emit(Delta::MoreChats(page.more));
                self.publish()?;
            }
            Err(e) => {
                self.rpc_failed(
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
    }
    fn page(&mut self, chat: ChatId, kind: PageKind) -> Result<()> {
        match self.backend.messages(&chat, &kind) {
            Ok(page) => {
                self.store
                    .apply_page(&chat, page.kind, &page.rows, page.reaches_start)?;
                self.inbox.refresh(Refresh::Receipts(chat.clone()));
                self.publish()?;
            }
            Err(e) => {
                self.rpc_failed(Self::page_refresh(chat, kind), e);
            }
        };
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
        self.retry_after
            .insert(r.clone(), Instant::now() + Duration::from_secs(2));
        self.inbox.refresh(r);
    }
    fn refresh(&mut self, r: Refresh) -> Result<()> {
        match r {
            Refresh::Send(key) => self.send(key)?,
            Refresh::Friends => match self.backend.friends() {
                Ok(rows) => {
                    self.store.save_contacts(
                        &rows
                            .iter()
                            .map(|r| (r.id.clone(), serde_json::to_value(r).unwrap()))
                            .collect::<Vec<_>>(),
                    )?;
                    self.emit_friends()?;
                }
                Err(e) => self.rpc_failed(Refresh::Friends, e),
            },
            Refresh::Members(chat) => match self.backend.members(&chat) {
                Ok(rows) => {
                    self.store.save_members(
                        &chat,
                        &rows
                            .iter()
                            .map(|r| (r.contact.id.clone(), serde_json::to_value(r).unwrap()))
                            .collect::<Vec<_>>(),
                    )?;
                    self.emit(Delta::Members { chat, rows });
                }
                Err(e) => self.rpc_failed(Refresh::Members(chat), e),
            },
            Refresh::ChatList => self.load_chats(false)?,
            Refresh::MoreChats => self.load_chats(true)?,
            Refresh::Older(chat, before) => self.page(chat, PageKind::Older { before })?,
            Refresh::Newer(chat, after) => self.page(chat, PageKind::Newer { after })?,
            Refresh::Receipts(chat) => match self.backend.receipts(&chat) {
                Ok(ids) => {
                    for id in ids {
                        let id = match id {
                            Some(id) => Some(id),
                            None => self
                                .store
                                .newest_confirmed(&chat)?
                                .and_then(|m| m.server_id),
                        };
                        if let Some(id) = id {
                            self.store.advance_read_receipt(&chat, id)?;
                        }
                    }
                    self.publish()?;
                }
                Err(e) => {
                    self.rpc_failed(Refresh::Receipts(chat), e);
                }
            },
            Refresh::Recent(chat) => self.page(chat, PageKind::Recent)?,
            Refresh::Details(chat) => match self.backend.details(&chat) {
                Ok(rows) => {
                    self.store.upsert_details(&rows)?;
                    self.publish()?;
                }
                Err(e) => {
                    self.rpc_failed(Refresh::Details(chat), e);
                }
            },
            Refresh::Read(chat) => {
                let id = self
                    .store
                    .chat(&chat)?
                    .unwrap()
                    .last_read_server_id
                    .unwrap();
                match self.backend.mark_read(&chat, &id) {
                    Ok(()) => {
                        self.store.confirm_read(&chat, &id)?;
                    }
                    Err(e) => {
                        self.rpc_failed(Refresh::Read(chat), e);
                    }
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
        match self.backend.media(&asset, purpose.clone()) {
            Ok(Some(payload)) => self.asset_result(crate::client::desktop::assets::Completion {
                key: asset.key(),
                purpose,
                result: Ok(payload),
            }),
            Ok(None) => {}
            Err(e) => self.error(e),
        };
        Ok(())
    }
    fn asset_result(&self, c: crate::client::desktop::assets::Completion) {
        use crate::client::desktop::assets::{Payload, Purpose};
        match c.result {
            Ok(Payload::Image(pixels)) => self.emit(Delta::Asset { key: c.key, pixels }),
            Ok(Payload::File(bytes)) => {
                if let Purpose::File { name, .. } = c.purpose {
                    self.emit(Delta::File { name, bytes });
                }
            }
            Err(failure) => self.emit(Delta::MediaFailed {
                key: c.key,
                failure,
            }),
        }
    }
    fn live(&mut self, e: StreamEvent) -> Result<()> {
        match e {
            StreamEvent::Resync { revision, complete } => {
                let result = self.resync(revision);
                let _ = complete.send(result.as_ref().copied().unwrap_or(false));
                result?;
            }
            StreamEvent::Asset(c) => self.asset_result(c),
            StreamEvent::Authorize { asset, reply } => {
                let _ = reply.send(self.backend.authorize(&asset));
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
                    self.store.apply_operation(&op, self.continuous)?;
                    if fresh
                        && !op.replay
                        && let Op::Message(m) = &op.op
                        && !m.outgoing
                    {
                        self.emit(Delta::Notify(m.clone()));
                    }
                    self.publish()?;
                }
            }
            StreamEvent::Raw(raw) => match self.backend.decode(raw) {
                Ok(op) => self.live(StreamEvent::Operation(op))?,
                Err(e) => self.error(e),
            },
            StreamEvent::Token(v) => self.backend.token(v),
            StreamEvent::Connection(Ok(())) => {
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
                    Connection::AuthExpired | Connection::SessionInvalid
                ) {
                    self.connection(Connection::Offline {
                        retry_at: Instant::now() + Duration::from_secs(2),
                        cause: NetCause::Unreachable,
                    });
                }
            }
        };
        Ok(())
    }
    fn resync(&mut self, revision: Option<i64>) -> Result<bool> {
        if let Some(w) = &self.window {
            let chat = w.chat.clone();
            match self.backend.messages(&chat, &PageKind::Recent) {
                Ok(page) => {
                    self.store
                        .apply_page(&chat, page.kind, &page.rows, page.reaches_start)?;
                }
                Err(e) => {
                    self.rpc_failed(Refresh::Recent(chat), e);
                    return Ok(false);
                }
            }
        }
        self.inbox.refresh(Refresh::ChatList);
        for c in self.store.chat_list(usize::MAX, 0)? {
            if self.window.as_ref().is_none_or(|w| w.chat != c.chat.id) {
                self.inbox.refresh(Refresh::Recent(c.chat.id));
            }
        }
        if let Some(revision) = revision {
            self.store.set_sync_revision(revision)?;
        }
        self.continuous = true;
        self.publish()?;
        Ok(true)
    }
    fn publish(&mut self) -> Result<()> {
        if let Some((sub, limit)) = self.list {
            let mut rows = self.store.chat_list(limit, 0)?;
            if self.connection != Connection::Online {
                for c in &mut rows {
                    c.chat.unread_stale = true;
                }
            }
            let keys = rows
                .iter()
                .map(|r| r.chat.id.clone())
                .collect::<BTreeSet<_>>();
            let remove = self.chat_keys.difference(&keys).cloned().collect();
            let order = rows.iter().map(|r| r.chat.id.clone()).collect();
            self.chat_keys = keys;
            self.emit(Delta::Chats {
                sub,
                upsert: rows,
                remove,
                order,
            });
        }
        if let Some(w) = &self.window {
            let window = self.store.window(&w.chat, w.anchor, w.before, w.after)?;
            let keys = window.rows.iter().map(|r| r.key).collect::<BTreeSet<_>>();
            let remove = self.message_keys.difference(&keys).copied().collect();
            self.message_keys = keys;
            let mut replies = Vec::new();
            for row in &window.rows {
                if let Some(id) = row.body["rich"]["reply"]["message_id"].as_str()
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
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
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
