//! Bounded, coalescing desktop bus worker. UI callers never perform D-Bus I/O.
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
use zbus::zvariant::{OwnedValue, Str, Value};
#[derive(Clone, Debug)]
pub(crate) struct Avatar {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub chat: String,
    pub summary: String,
    pub body: String,
    pub sound: bool,
    pub avatar: Option<Avatar>,
    pub last_message: String,
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Availability(bool),
    Delivered(String),
    Closed(String),
    OpenChat { chat: String, token: Option<String> },
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
    pub(crate) fn recording() -> Self {
        let (wake, _) = mpsc::sync_channel(1);
        Self {
            shared: Arc::new(Shared {
                latest: AtomicU64::new(0),
                pending: Mutex::new(Pending::default()),
                available: AtomicBool::new(true),
                handler: Mutex::new(None),
                ids: Mutex::new(HashMap::new()),
                closed: Mutex::new(HashSet::new()),
                stop: AtomicBool::new(false),
            }),
            wake,
            worker: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn take_recorded(&self) -> Vec<Notice> {
        std::mem::take(&mut self.shared.pending.lock().unwrap().changes)
            .into_values()
            .flatten()
            .collect()
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
            .spawn(move || {
                let mut connection = connect();
                let mut listeners = Vec::new();
                if let Some(connection) = &connection
                    && let Some(listener) = listen(connection.clone(), state.clone())
                {
                    listeners.push(listener);
                }
                state.availability(connection.as_ref().is_some_and(notification_available));
                let mut ids: HashMap<String, u32> = HashMap::new();
                let mut sounds: HashMap<String, u64> = HashMap::new();
                let started = Instant::now();
                let mut last_badge = None;
                while receiver.recv().is_ok() {
                    if state.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let closed = std::mem::take(&mut *state.closed.lock().unwrap());
                    ids.retain(|_, id| !closed.contains(id));
                    while receiver.try_recv().is_ok() {}
                    let pending = std::mem::take(&mut *state.pending.lock().unwrap());
                    if connection.is_none() {
                        connection = connect();
                        if let Some(connection) = &connection
                            && let Some(listener) = listen(connection.clone(), state.clone())
                        {
                            listeners.push(listener);
                        }
                    }
                    let Some(connection) = &connection else {
                        state.availability(false);
                        continue;
                    };
                    let count = state.latest.load(Ordering::Relaxed);
                    if last_badge != Some(count)
                        && connection
                            .emit_signal(
                                None::<&str>,
                                "/org/lineclient/Desktop",
                                "com.canonical.Unity.LauncherEntry",
                                "Update",
                                &(
                                    "application://org.lineclient.Desktop.desktop",
                                    badge_properties(count),
                                ),
                            )
                            .is_ok()
                    {
                        last_badge = Some(count);
                    }
                    if pending.probe {
                        state.availability(notification_available(connection));
                    }
                    if pending.clear {
                        for (_, id) in ids.drain() {
                            close(connection, id);
                        }
                        state.ids.lock().unwrap().clear();
                        sounds.clear();
                    }
                    for (chat, notice) in pending.changes {
                        if state.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let Some(notice) = notice else {
                            if let Some(id) = ids.remove(&chat) {
                                close(connection, id);
                                state.ids.lock().unwrap().remove(&id);
                            }
                            sounds.remove(&chat);
                            continue;
                        };
                        if ids.len() >= 512
                            && !ids.contains_key(&chat)
                            && let Some(oldest) = ids.keys().next().cloned()
                            && let Some(id) = ids.remove(&oldest)
                        {
                            close(connection, id);
                            state.ids.lock().unwrap().remove(&id);
                            sounds.remove(&oldest);
                        }
                        let now = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                        let sound = notice.sound
                            && crate::attention_policy::sound_due(sounds.get(&chat).copied(), now);
                        let previous = ids.get(&chat).copied().unwrap_or(0);
                        match notify(connection, &notice, previous, sound) {
                            Ok(id) => {
                                if previous != id {
                                    state.ids.lock().unwrap().remove(&previous);
                                }
                                ids.insert(chat.clone(), id);
                                state
                                    .ids
                                    .lock()
                                    .unwrap()
                                    .insert(id, (chat.clone(), notice.last_message));
                                if sound {
                                    sounds.insert(chat.clone(), now);
                                }
                                state.availability(true);
                                state.event(Event::Delivered(chat));
                            }
                            Err(_) => state.availability(false),
                        }
                    }
                }
                if let Some(connection) = connection {
                    let _ = connection.emit_signal(
                        None::<&str>,
                        "/org/lineclient/Desktop",
                        "com.canonical.Unity.LauncherEntry",
                        "Update",
                        &(
                            "application://org.lineclient.Desktop.desktop",
                            badge_properties(0),
                        ),
                    );
                    for (_, id) in ids {
                        close(&connection, id);
                    }
                    let _ = connection.close();
                }
                for listener in listeners {
                    let _ = listener.join();
                }
            })
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
    pub(crate) fn notify(&self, mut notice: Notice) {
        if notice.chat.is_empty() || notice.chat.len() > 256 {
            return;
        }
        if notice.avatar.as_ref().is_some_and(|avatar| {
            avatar.width == 0
                || avatar.height == 0
                || avatar.width > 256
                || avatar.height > 256
                || avatar.rgba.len() != (avatar.width * avatar.height * 4) as usize
        }) {
            notice.avatar = None;
        }
        notice.last_message = notice.last_message.chars().take(256).collect();
        notice.summary = notice.summary.chars().take(200).collect();
        notice.body = notice.body.chars().take(1_000).collect();
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.changes.len() < 512 || pending.changes.contains_key(&notice.chat) {
            pending.changes.insert(notice.chat.clone(), Some(notice));
        }
        drop(pending);
        self.wake();
    }
    pub(crate) fn withdraw(&self, chat: &str) {
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.changes.len() >= 512 && !pending.changes.contains_key(chat) {
            pending.changes.clear();
            pending.clear = true;
        }
        pending.changes.insert(chat.into(), None);
        drop(pending);
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
fn badge_properties(count: u64) -> HashMap<&'static str, OwnedValue> {
    HashMap::from([
        ("count", (count.min(i64::MAX as u64) as i64).into()),
        ("count-visible", (count > 0).into()),
    ])
}
fn notification_available(connection: &zbus::blocking::Connection) -> bool {
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "GetServerInformation",
            &(),
        )
        .and_then(|reply| {
            reply
                .body()
                .deserialize::<(String, String, String, String)>()
        })
        .is_ok()
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
fn connect() -> Option<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(Duration::from_secs(2))
        .build()
        .ok()
}
fn close(connection: &zbus::blocking::Connection, id: u32) {
    if let Ok(proxy) = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    ) {
        let _ = proxy.call_noreply("CloseNotification", &(id,));
    }
}
fn notify(
    connection: &zbus::blocking::Connection,
    notice: &Notice,
    previous: u32,
    sound: bool,
) -> zbus::Result<u32> {
    let mut hints: HashMap<&str, OwnedValue> = HashMap::from([
        (
            "desktop-entry",
            Str::from(crate::desktop_integration::APP_ID).into(),
        ),
        ("category", Str::from("im.received").into()),
        ("urgency", 1u8.into()),
        ("resident", false.into()),
        ("suppress-sound", (!sound).into()),
    ]);
    if sound {
        hints.insert("sound-name", Str::from("message-new-instant").into());
    }
    if let Some(avatar) = &notice.avatar
        && avatar.width > 0
        && avatar.height > 0
        && avatar.width <= 256
        && avatar.height <= 256
        && avatar.rgba.len() == (avatar.width * avatar.height * 4) as usize
    {
        let image = Value::from((
            avatar.width as i32,
            avatar.height as i32,
            (avatar.width * 4) as i32,
            true,
            8i32,
            4i32,
            avatar.rgba.clone(),
        ));
        if let Ok(value) = image.try_to_owned() {
            hints.insert("image-data", value);
        }
    }
    let mut actions = vec!["default", "Open chat"];
    if !notice.last_message.is_empty() {
        actions.extend(["mark-read", "Mark read"]);
    }
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "LINE",
                previous,
                crate::desktop_integration::APP_ID,
                &notice.summary,
                &escape_markup(&notice.body),
                actions,
                hints,
                -1i32,
            ),
        )?
        .body()
        .deserialize()
}
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn listen(
    connection: zbus::blocking::Connection,
    shared: Arc<Shared>,
) -> Option<std::thread::JoinHandle<()>> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.Notifications")
        .ok()?
        .interface("org.freedesktop.Notifications")
        .ok()?
        .build();
    let messages =
        zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(32)).ok()?;
    std::thread::Builder::new()
        .name("desktop-notification-actions".into())
        .spawn(move || {
            let mut tokens = HashMap::new();
            for message in messages.flatten() {
                match message.header().member().map(|member| member.as_str()) {
                    Some("ActivationToken") => {
                        if let Ok((id, token)) = message.body().deserialize::<(u32, String)>()
                            && tokens.len() < 512
                            && token.len() <= 4096
                        {
                            tokens.insert(id, token);
                        }
                    }
                    Some("ActionInvoked") => {
                        if let Ok((id, action)) = message.body().deserialize::<(u32, String)>() {
                            let known = shared.ids.lock().unwrap().get(&id).cloned();
                            if let Some((chat, last)) = known {
                                match action.as_str() {
                                    "default" => shared.event(Event::OpenChat {
                                        chat,
                                        token: tokens.remove(&id),
                                    }),
                                    "mark-read" if !last.is_empty() => {
                                        shared.event(Event::MarkRead {
                                            chat,
                                            message: last,
                                        })
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _reason)) = message.body().deserialize::<(u32, u32)>() {
                            let known = shared.ids.lock().unwrap().remove(&id);
                            let mut closed = shared.closed.lock().unwrap();
                            if closed.len() < 512 {
                                closed.insert(id);
                            }
                            tokens.remove(&id);
                            drop(closed);
                            if let Some((chat, _)) = known {
                                shared.event(Event::Closed(chat));
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badge_count_and_visibility_follow_desktop_types() {
        let mut empty = badge_properties(0);
        assert_eq!(i64::try_from(empty.remove("count").unwrap()).unwrap(), 0);
        assert!(!bool::try_from(empty.remove("count-visible").unwrap()).unwrap());
        let mut full = badge_properties(u64::MAX);
        assert_eq!(
            i64::try_from(full.remove("count").unwrap()).unwrap(),
            i64::MAX
        );
        assert!(bool::try_from(full.remove("count-visible").unwrap()).unwrap());
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use std::sync::Condvar;
    #[derive(Clone)]
    struct Recorded {
        id: u32,
        replaces: u32,
        body: String,
        sound: bool,
        suppressed: bool,
        avatar: bool,
        actions: Vec<String>,
    }
    type Calls = Arc<(Mutex<Vec<Recorded>>, Condvar)>;
    struct Fake {
        calls: Calls,
        closed: Arc<Mutex<Vec<u32>>>,
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
                avatar: hints.contains_key("image-data"),
                actions,
            });
            self.calls.1.notify_all();
            id
        }
        fn close_notification(&self, id: u32) {
            self.closed.lock().unwrap().push(id);
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
    #[ignore = "requires deliberately unavailable private bus address"]
    fn unavailable_bus_reports_failure_and_joins() {
        assert_eq!(std::env::var("LINE_DESKTOP_TEST_BUS").as_deref(), Ok("1"));
        assert!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS")
                .unwrap()
                .contains("/tmp/line-desktop-no-bus")
        );
        let attention = Attention::start();
        let (sender, events) = mpsc::channel();
        attention.on_event(move |event| {
            let _ = sender.send(event);
        });
        attention.probe();
        event(&events, |event| matches!(event, Event::Availability(false)));
        let stopped = Instant::now();
        drop(attention);
        assert!(stopped.elapsed() < Duration::from_secs(3));
    }
    #[test]
    #[ignore = "requires dbus-run-session and LINE_DESKTOP_TEST_BUS=1"]
    fn synthetic_notification_service_contract() {
        assert_eq!(
            std::env::var("LINE_DESKTOP_TEST_BUS").as_deref(),
            Ok("1"),
            "refusing to claim the real desktop notification service"
        );
        let calls: Calls = Arc::default();
        let closed = Arc::new(Mutex::new(Vec::new()));
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
            avatar: Some(Avatar {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            }),
            last_message: "42".into(),
        };
        attention.notify(notice.clone());
        let first = wait_calls(&calls, 1)[0].clone();
        assert_eq!(first.body, "&lt;b&gt;literal &amp; private&lt;/b&gt;");
        assert!(first.sound && first.avatar);
        assert!(first.actions.iter().any(|action| action == "mark-read"));
        let badge = badges.next().unwrap().unwrap();
        let (uri, mut properties): (String, HashMap<String, OwnedValue>) =
            badge.body().deserialize().unwrap();
        assert_eq!(uri, "application://org.lineclient.Desktop.desktop");
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
                "ActivationToken",
                &(first.id, "synthetic-token"),
            )
            .unwrap();
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
        assert!(
            matches!(opened,Event::OpenChat{chat,token:Some(token)} if chat=="synthetic-chat"&&token=="synthetic-token")
        );
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
        private.avatar = None;
        private.body = String::new();
        attention.notify(private);
        let third = wait_calls(&calls, 3)[2].clone();
        assert_eq!(third.replaces, 0);
        assert!(!third.avatar && third.body.is_empty());
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
