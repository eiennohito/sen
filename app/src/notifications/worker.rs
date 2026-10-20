//! Coalescing desktop bus worker. UI callers never perform D-Bus I/O.
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Instant,
};
use zbus::zvariant::OwnedValue;

pub(crate) const TEST_CHAT: &str = "notification-test";

use super::dbus::{close, connect, listen, notification_available, notify};
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
    TestDelivered,
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
#[derive(Default)]
pub(super) struct Shared {
    latest: AtomicU64,
    pending: Mutex<Pending>,
    available: AtomicBool,
    handler: Mutex<Option<Handler>>,
    pub(super) ids: Mutex<HashMap<u32, (String, String)>>,
    stop: AtomicBool,
}
impl Shared {
    pub(super) fn event(&self, event: Event) {
        if let Some(handler) = self.handler.lock().unwrap().as_ref() {
            handler(event);
        }
    }
    fn report_availability(&self, value: bool) {
        self.available.store(value, Ordering::Relaxed);
        self.event(Event::Availability(value));
    }
    fn availability(&self, value: bool) {
        let handler = self.handler.lock().unwrap();
        if self.available.swap(value, Ordering::Relaxed) != value
            && let Some(handler) = handler.as_ref()
        {
            handler(Event::Availability(value));
        }
    }
}
pub(crate) struct Attention {
    shared: Arc<Shared>,
    wake: SyncSender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Attention {
    pub(crate) fn start() -> Self {
        let (wake, receiver) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared::default());
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
        let mut slot = self.shared.handler.lock().unwrap();
        handler(Event::Availability(
            self.shared.available.load(Ordering::Relaxed),
        ));
        *slot = Some(Box::new(handler));
    }
    pub(crate) fn set_unread_count(&self, count: u64) {
        if self.shared.latest.swap(count, Ordering::Relaxed) != count {
            self.wake();
        }
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
            self.state.report_availability(false);
            return;
        };
        self.update_badge(&connection);
        if pending.probe {
            self.state
                .report_availability(notification_available(&connection));
        }
        if pending.clear {
            let ids = std::mem::take(&mut *self.state.ids.lock().unwrap());
            for id in ids.into_keys() {
                close(&connection, id);
            }
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
                    crate::desktop_env::desktop_integration::OBJECT,
                    "com.canonical.Unity.LauncherEntry",
                    "Update",
                    &(
                        crate::desktop_env::desktop_integration::APPLICATION_URI,
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
                let id = self.id_for(&chat);
                if let Some(id) = id {
                    close(connection, id);
                    self.state.ids.lock().unwrap().remove(&id);
                }
                self.sounds.remove(&chat);
                continue;
            };
            let now = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let sound = notice.sound
                && crate::notifications::policy::sound_due(self.sounds.get(&chat).copied(), now);
            let previous = self.id_for(&chat).unwrap_or(0);
            match notify(connection, &notice, previous, sound) {
                Ok(id) => {
                    if previous != id {
                        self.state.ids.lock().unwrap().remove(&previous);
                    }
                    self.state
                        .ids
                        .lock()
                        .unwrap()
                        .insert(id, (chat.clone(), notice.last_message));
                    if sound {
                        self.sounds.insert(chat.clone(), now);
                    }
                    self.state.availability(true);
                    if chat == TEST_CHAT {
                        self.state.event(Event::TestDelivered);
                    }
                }
                Err(_) if chat == TEST_CHAT => self.state.report_availability(false),
                Err(_) => self.state.availability(false),
            }
        }
    }
    fn id_for(&self, chat: &str) -> Option<u32> {
        self.state
            .ids
            .lock()
            .unwrap()
            .iter()
            .find_map(|(id, (owner, _))| (owner == chat).then_some(*id))
    }
    fn shutdown(mut self) {
        if let Some(connection) = self.connection.take() {
            self.state.latest.store(0, Ordering::Relaxed);
            self.update_badge(&connection);
            let ids = std::mem::take(&mut *self.state.ids.lock().unwrap());
            for id in ids.into_keys() {
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
                "閃 tests".into(),
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
    fn unchanged_unread_count_does_not_wake_desktop_services() {
        let (wake, receiver) = mpsc::sync_channel(1);
        let attention = Attention {
            shared: Arc::default(),
            wake,
            worker: None,
        };
        attention.set_unread_count(2);
        receiver.try_recv().unwrap();
        attention.set_unread_count(2);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        attention.set_unread_count(0);
        receiver.try_recv().unwrap();
    }
    #[test]
    #[ignore = "requires a private dbus-run-session"]
    fn unavailable_service_reports_each_probe_and_test_outcome() {
        assert_eq!(std::env::var("SEN_TEST_BUS").as_deref(), Ok("1"));
        let attention = Attention::start();
        let (sender, events) = mpsc::channel();
        attention.on_event(move |event| sender.send(event).unwrap());
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Availability(false)
        ));
        for _ in 0..2 {
            attention.probe();
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(3)).unwrap(),
                Event::Availability(false)
            ));
            attention.notify(Notice {
                chat: TEST_CHAT.into(),
                summary: "Test".into(),
                body: String::new(),
                sound: false,
                last_message: String::new(),
            });
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(3)).unwrap(),
                Event::Availability(false)
            ));
        }
    }
    #[test]
    #[ignore = "requires a private dbus-run-session"]
    fn synthetic_notification_service_contract() {
        assert_eq!(
            std::env::var("SEN_TEST_BUS").as_deref(),
            Ok("1"),
            "contract tests require SEN_TEST_BUS=1 on a private session bus"
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
        assert_eq!(
            uri,
            crate::desktop_env::desktop_integration::APPLICATION_URI
        );
        assert_eq!(
            i64::try_from(properties.remove("count").unwrap()).unwrap(),
            2
        );
        attention.notify(notice.clone());
        let second = wait_calls(&calls, 2)[1].clone();
        assert_eq!(second.replaces, first.id);
        assert!(!second.sound && second.suppressed);
        let invoke = |id, action: &str| {
            server
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/Notifications",
                    "org.freedesktop.Notifications",
                    "ActionInvoked",
                    &(id, action),
                )
                .unwrap();
        };
        invoke(first.id, "default");
        let opened = event(&events, |event| matches!(event, Event::OpenChat { .. }));
        assert!(matches!(opened, Event::OpenChat { chat } if chat == "synthetic-chat"));
        invoke(first.id, "mark-read");
        assert!(
            matches!(event(&events,|event|matches!(event,Event::MarkRead{..})),Event::MarkRead{chat,message} if chat=="synthetic-chat"&&message=="42")
        );
        let mut sentinel = notice.clone();
        sentinel.chat = TEST_CHAT.into();
        attention.notify(sentinel);
        let sentinel_id = wait_calls(&calls, 3)[2].id;
        assert!(
            matches!(
                events.recv_timeout(Duration::from_secs(3)).unwrap(),
                Event::TestDelivered
            ),
            "successful notifications repeated availability or delivered ordinary chat events"
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
        invoke(first.id, "default");
        invoke(sentinel_id, "default");
        assert!(
            matches!(event(&events, |event| matches!(event, Event::OpenChat { .. })),
            Event::OpenChat { chat } if chat == TEST_CHAT)
        );
        let mut private = notice;
        private.body = String::new();
        attention.notify(private);
        let third = wait_calls(&calls, 4)[3].clone();
        assert_eq!(third.replaces, 0);
        assert!(third.body.is_empty());
        attention.withdraw("synthetic-chat");
        assert_eq!(
            closes.recv_timeout(Duration::from_secs(3)).unwrap(),
            third.id
        );
        attention.withdraw_all();
        assert_eq!(attention.shared.latest.load(Ordering::Relaxed), 0);
        assert_eq!(
            closes.recv_timeout(Duration::from_secs(3)).unwrap(),
            sentinel_id
        );
        loop {
            let badge = badges.next().unwrap().unwrap();
            let (_, mut properties): (String, HashMap<String, OwnedValue>) =
                badge.body().deserialize().unwrap();
            if i64::try_from(properties.remove("count").unwrap()).unwrap() == 0 {
                assert!(!bool::try_from(properties.remove("count-visible").unwrap()).unwrap());
                break;
            }
        }
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
