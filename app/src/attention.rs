//! Coalescing desktop bus worker. UI callers never perform D-Bus I/O.
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Instant,
};
use zbus::zvariant::OwnedValue;

mod dbus;
use dbus::{close, connect, listen, notification_available, notify};
#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub chat: String,
    pub summary: String,
    pub body: String,
    pub sound: bool,
    pub last_message: String,
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Availability(bool),
    Delivered(String),
    Closed(String),
    OpenChat { chat: String },
    MarkRead { chat: String, message: String },
}
#[derive(Default)]
struct Pending {
    changes: HashMap<String, Option<Notice>>,
    clear: bool,
    probe: bool,
}
type Handler = Box<dyn Fn(Event) + Send>;
struct Shared {
    latest: AtomicU64,
    pending: Mutex<Pending>,
    available: AtomicBool,
    handler: Mutex<Option<Handler>>,
    ids: Mutex<HashMap<u32, (String, String)>>,
    closed: Mutex<HashSet<u32>>,
    stop: AtomicBool,
}
impl Shared {
    fn event(&self, event: Event) {
        if let Some(handler) = self.handler.lock().unwrap().as_ref() {
            handler(event);
        }
    }
    fn availability(&self, value: bool) {
        self.available.store(value, Ordering::Relaxed);
        self.event(Event::Availability(value));
    }
}
pub(crate) struct Attention {
    shared: Arc<Shared>,
    wake: SyncSender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Attention {
    #[cfg(test)]
    pub(crate) fn disconnected() -> Self {
        let (wake, _) = mpsc::sync_channel(1);
        Self {
            shared: Arc::new(Shared {
                latest: AtomicU64::new(0),
                pending: Mutex::new(Pending::default()),
                available: AtomicBool::new(false),
                handler: Mutex::new(None),
                ids: Mutex::new(HashMap::new()),
                closed: Mutex::new(HashSet::new()),
                stop: AtomicBool::new(false),
            }),
            wake,
            worker: None,
        }
    }
    pub(crate) fn start() -> Self {
        let (wake, receiver) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            latest: AtomicU64::new(0),
            pending: Mutex::new(Pending::default()),
            available: AtomicBool::new(false),
            handler: Mutex::new(None),
            ids: Mutex::new(HashMap::new()),
            closed: Mutex::new(HashSet::new()),
            stop: AtomicBool::new(false),
        });
        let state = shared.clone();
        let worker = std::thread::Builder::new()
            .name("desktop-attention".into())
            .spawn(move || Worker::new(state).run(receiver))
            .expect("desktop worker thread could not start");
        Self {
            shared,
            wake,
            worker: Some(worker),
        }
    }
    fn wake(&self) {
        let _ = self.wake.try_send(());
    }
    pub(crate) fn on_event(&self, handler: impl Fn(Event) + Send + 'static) {
        *self.shared.handler.lock().unwrap() = Some(Box::new(handler));
        self.shared.event(Event::Availability(self.available()));
    }
    pub(crate) fn set_unread_count(&self, count: u64) {
        self.shared.latest.store(count, Ordering::Relaxed);
        self.wake();
    }
    pub(crate) fn available(&self) -> bool {
        self.shared.available.load(Ordering::Relaxed)
    }
    pub(crate) fn probe(&self) {
        self.shared.pending.lock().unwrap().probe = true;
        self.wake();
    }
    pub(crate) fn notify(&self, notice: Notice) {
        if notice.chat.is_empty() {
            return;
        }
        self.shared
            .pending
            .lock()
            .unwrap()
            .changes
            .insert(notice.chat.clone(), Some(notice));
        self.wake();
    }
    pub(crate) fn withdraw(&self, chat: &str) {
        self.shared
            .pending
            .lock()
            .unwrap()
            .changes
            .insert(chat.into(), None);
        self.wake();
    }
    pub(crate) fn withdraw_notifications(&self) {
        let mut pending = self.shared.pending.lock().unwrap();
        pending.changes.clear();
        pending.clear = true;
        drop(pending);
        self.wake();
    }
    pub(crate) fn withdraw_all(&self) {
        self.withdraw_notifications();
        self.set_unread_count(0);
    }
}
struct Worker {
    state: Arc<Shared>,
    connection: Option<zbus::blocking::Connection>,
    listeners: Vec<std::thread::JoinHandle<()>>,
    ids: HashMap<String, u32>,
    sounds: HashMap<String, u64>,
    started: Instant,
    last_badge: Option<u64>,
}
impl Worker {
    fn new(state: Arc<Shared>) -> Self {
        let connection = connect();
        let mut listeners = Vec::new();
        if let Some(connection) = &connection
            && let Some(listener) = listen(connection.clone(), state.clone())
        {
            listeners.push(listener);
        }
        state.availability(connection.as_ref().is_some_and(notification_available));
        Self {
            state,
            connection,
            listeners,
            ids: HashMap::new(),
            sounds: HashMap::new(),
            started: Instant::now(),
            last_badge: None,
        }
    }
    fn connect(&mut self) {
        self.connection = connect();
        if let Some(connection) = &self.connection
            && let Some(listener) = listen(connection.clone(), self.state.clone())
        {
            self.listeners.push(listener);
        }
    }
    fn run(mut self, receiver: mpsc::Receiver<()>) {
        while receiver.recv().is_ok() {
            if self.state.stop.load(Ordering::Relaxed) {
                break;
            }
            let closed = std::mem::take(&mut *self.state.closed.lock().unwrap());
            self.ids.retain(|_, id| !closed.contains(id));
            while receiver.try_recv().is_ok() {}
            self.process();
        }
        self.shutdown();
    }
    fn process(&mut self) {
        let pending = std::mem::take(&mut *self.state.pending.lock().unwrap());
        if self.connection.is_none() {
            self.connect();
        }
        let Some(connection) = self.connection.clone() else {
            self.state.availability(false);
            return;
        };
        self.update_badge(&connection);
        if pending.probe {
            self.state.availability(notification_available(&connection));
        }
        if pending.clear {
            for (_, id) in self.ids.drain() {
                close(&connection, id);
            }
            self.state.ids.lock().unwrap().clear();
            self.sounds.clear();
        }
        self.apply_changes(&connection, pending.changes);
    }
    fn update_badge(&mut self, connection: &zbus::blocking::Connection) {
        let count = self.state.latest.load(Ordering::Relaxed);
        if self.last_badge != Some(count)
            && connection
                .emit_signal(
                    None::<&str>,
                    crate::desktop_integration::OBJECT,
                    "com.canonical.Unity.LauncherEntry",
                    "Update",
                    &(
                        crate::desktop_integration::APPLICATION_URI,
                        badge_properties(count),
                    ),
                )
                .is_ok()
        {
            self.last_badge = Some(count);
        }
    }
    fn apply_changes(
        &mut self,
        connection: &zbus::blocking::Connection,
        changes: HashMap<String, Option<Notice>>,
    ) {
        for (chat, notice) in changes {
            if self.state.stop.load(Ordering::Relaxed) {
                break;
            }
            let Some(notice) = notice else {
                if let Some(id) = self.ids.remove(&chat) {
                    close(connection, id);
                    self.state.ids.lock().unwrap().remove(&id);
                }
                self.sounds.remove(&chat);
                continue;
            };
            let now = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let sound = notice.sound
                && crate::attention_policy::sound_due(self.sounds.get(&chat).copied(), now);
            let previous = self.ids.get(&chat).copied().unwrap_or(0);
            match notify(connection, &notice, previous, sound) {
                Ok(id) => {
                    if previous != id {
                        self.state.ids.lock().unwrap().remove(&previous);
                    }
                    self.ids.insert(chat.clone(), id);
                    self.state
                        .ids
                        .lock()
                        .unwrap()
                        .insert(id, (chat.clone(), notice.last_message));
                    if sound {
                        self.sounds.insert(chat.clone(), now);
                    }
                    self.state.availability(true);
                    self.state.event(Event::Delivered(chat));
                }
                Err(_) => self.state.availability(false),
            }
        }
    }
    fn shutdown(self) {
        if let Some(connection) = self.connection {
            let _ = connection.emit_signal(
                None::<&str>,
                crate::desktop_integration::OBJECT,
                "com.canonical.Unity.LauncherEntry",
                "Update",
                &(
                    crate::desktop_integration::APPLICATION_URI,
                    badge_properties(0),
                ),
            );
            for (_, id) in self.ids {
                close(&connection, id);
            }
            let _ = connection.close();
        }
        for listener in self.listeners {
            let _ = listener.join();
        }
    }
}

fn badge_properties(count: u64) -> HashMap<&'static str, OwnedValue> {
    HashMap::from([
        ("count", (count.min(i64::MAX as u64) as i64).into()),
        ("count-visible", (count > 0).into()),
    ])
}
impl Drop for Attention {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod service_tests {
    use super::*;
    use std::{sync::Condvar, time::Duration};
    #[derive(Clone)]
    struct Recorded {
        id: u32,
        replaces: u32,
        body: String,
        sound: bool,
        suppressed: bool,
        actions: Vec<String>,
    }
    type Calls = Arc<(Mutex<Vec<Recorded>>, Condvar)>;
    struct Fake {
        calls: Calls,
        closed: mpsc::Sender<u32>,
    }
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Fake {
        fn get_server_information(&self) -> (String, String, String, String) {
            (
                "Synthetic daemon".into(),
                "LINE tests".into(),
                "1".into(),
                "1.2".into(),
            )
        }
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            _application: &str,
            replaces: u32,
            _icon: &str,
            _summary: &str,
            body: &str,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            _timeout: i32,
        ) -> u32 {
            let mut calls = self.calls.0.lock().unwrap();
            let id = if replaces == 0 {
                calls.len() as u32 + 10
            } else {
                replaces
            };
            calls.push(Recorded {
                id,
                replaces,
                body: body.into(),
                sound: hints.contains_key("sound-name"),
                suppressed: hints
                    .get("suppress-sound")
                    .and_then(|value| bool::try_from(value).ok())
                    .unwrap_or(false),
                actions,
            });
            self.calls.1.notify_all();
            id
        }
        fn close_notification(&self, id: u32) {
            self.closed.send(id).unwrap();
        }
    }
    fn wait_calls(calls: &Calls, count: usize) -> Vec<Recorded> {
        let mut records = calls.0.lock().unwrap();
        while records.len() < count {
            let (next, result) = calls
                .1
                .wait_timeout(records, Duration::from_secs(3))
                .unwrap();
            records = next;
            assert!(!result.timed_out(), "notification worker timed out");
        }
        records.clone()
    }
    fn event(receiver: &mpsc::Receiver<Event>, predicate: impl Fn(&Event) -> bool) -> Event {
        for _ in 0..20 {
            let event = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
            if predicate(&event) {
                return event;
            }
        }
        panic!("expected notification event not delivered");
    }
    #[test]
    #[ignore = "requires a private dbus-run-session"]
    fn synthetic_notification_service_contract() {
        assert_eq!(
            std::env::var("LINE_DESKTOP_TEST_BUS").as_deref(),
            Ok("1"),
            "contract tests require LINE_DESKTOP_TEST_BUS=1 on a private session bus"
        );
        let calls: Calls = Arc::default();
        let (closed, closes) = mpsc::channel();
        let server = zbus::blocking::connection::Builder::session()
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                Fake {
                    calls: calls.clone(),
                    closed: closed.clone(),
                },
            )
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .build()
            .unwrap();
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("com.canonical.Unity.LauncherEntry")
            .unwrap()
            .member("Update")
            .unwrap()
            .build();
        let mut badges =
            zbus::blocking::MessageIterator::for_match_rule(rule, &server, Some(8)).unwrap();
        let attention = Attention::start();
        let (sender, events) = mpsc::channel();
        attention.on_event(move |event| {
            let _ = sender.send(event);
        });
        event(&events, |event| matches!(event, Event::Availability(true)));
        attention.set_unread_count(2);
        let notice = Notice {
            chat: "synthetic-chat".into(),
            summary: "Synthetic name".into(),
            body: "<b>literal & private</b>".into(),
            sound: true,
            last_message: "42".into(),
        };
        attention.notify(notice.clone());
        let first = wait_calls(&calls, 1)[0].clone();
        assert_eq!(first.body, "&lt;b&gt;literal &amp; private&lt;/b&gt;");
        assert!(first.sound);
        assert!(first.actions.iter().any(|action| action == "mark-read"));
        let badge = badges.next().unwrap().unwrap();
        let (uri, mut properties): (String, HashMap<String, OwnedValue>) =
            badge.body().deserialize().unwrap();
        assert_eq!(uri, crate::desktop_integration::APPLICATION_URI);
        assert_eq!(
            i64::try_from(properties.remove("count").unwrap()).unwrap(),
            2
        );
        attention.notify(notice.clone());
        let second = wait_calls(&calls, 2)[1].clone();
        assert_eq!(second.replaces, first.id);
        assert!(!second.sound && second.suppressed);
        server
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "ActionInvoked",
                &(first.id, "default"),
            )
            .unwrap();
        let opened = event(&events, |event| matches!(event, Event::OpenChat { .. }));
        assert!(matches!(opened, Event::OpenChat { chat } if chat == "synthetic-chat"));
        server
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "ActionInvoked",
                &(first.id, "mark-read"),
            )
            .unwrap();
        assert!(
            matches!(event(&events,|event|matches!(event,Event::MarkRead{..})),Event::MarkRead{chat,message} if chat=="synthetic-chat"&&message=="42")
        );
        server
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "NotificationClosed",
                &(first.id, 2u32),
            )
            .unwrap();
        event(&events, |event| matches!(event, Event::Closed(_)));
        let mut private = notice;
        private.body = String::new();
        attention.notify(private);
        let third = wait_calls(&calls, 3)[2].clone();
        assert_eq!(third.replaces, 0);
        assert!(third.body.is_empty());
        attention.withdraw("synthetic-chat");
        assert_eq!(
            closes.recv_timeout(Duration::from_secs(3)).unwrap(),
            third.id
        );
        let stopped = Instant::now();
        drop(attention);
        assert!(
            stopped.elapsed() < Duration::from_secs(3),
            "desktop worker did not join promptly"
        );
        drop(badges);
        let _ = server.close();
    }
}
