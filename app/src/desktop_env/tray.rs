//! StatusNotifierItem and dbusmenu; all desktop calls run on the tray worker.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};
const OPEN: i32 = 1;
const MUTE: i32 = 2;
const QUIT: i32 = 3;
const MENU_ITEMS: [i32; 3] = [OPEN, MUTE, QUIT];
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Available(bool),
    Open,
    Mute,
    Quit,
}
enum Command {
    Event(Event),
    Refresh,
    Stop,
    HostChanged,
}
struct Item {
    commands: SyncSender<Command>,
}
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "Communications"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        crate::desktop_env::desktop_integration::APP_ID
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "LINE"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        crate::desktop_env::desktop_integration::APP_ID
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/MenuBar").unwrap()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.commands.try_send(Command::Event(Event::Open));
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.commands.try_send(Command::Event(Event::Open));
    }
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
}
struct Menu {
    commands: SyncSender<Command>,
    muted: Arc<AtomicBool>,
}
type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);
impl Menu {
    fn properties(&self, id: i32) -> Properties {
        let label = match id {
            OPEN => "Open",
            MUTE => {
                if self.muted.load(Ordering::Relaxed) {
                    "Unmute"
                } else {
                    "Mute"
                }
            }
            QUIT => "Quit",
            _ => "",
        };
        let mut properties = HashMap::from([
            ("label".into(), Str::from(label).into()),
            ("enabled".into(), true.into()),
            ("visible".into(), true.into()),
        ]);
        if id == 0 {
            properties.insert("children-display".into(), Str::from("submenu").into());
        }
        if id == MUTE {
            properties.insert("toggle-type".into(), Str::from("checkmark").into());
            properties.insert(
                "toggle-state".into(),
                i32::from(self.muted.load(Ordering::Relaxed)).into(),
            );
        }
        properties
    }
    fn activate(&self, id: i32, event: &str) {
        if event != "clicked" {
            return;
        }
        let action = match id {
            OPEN => Event::Open,
            MUTE => Event::Mute,
            QUIT => Event::Quit,
            _ => return,
        };
        let _ = self.commands.try_send(Command::Event(action));
    }
}
#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Layout) {
        let children = if parent_id == 0 && recursion_depth != 0 {
            MENU_ITEMS
                .into_iter()
                .filter_map(|id| {
                    Value::from((id, self.properties(id), Vec::<OwnedValue>::new()))
                        .try_to_owned()
                        .ok()
                })
                .collect()
        } else {
            Vec::new()
        };
        (1, (parent_id, self.properties(parent_id), children))
    }
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, Properties)> {
        ids.into_iter()
            .map(|id| (id, self.properties(id)))
            .collect()
    }
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        self.properties(id)
            .remove(name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu property".into()))
    }
    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        self.activate(id, event_id);
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event, _, _) in events {
            self.activate(id, &event);
        }
        Vec::new()
    }
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
}
fn register(connection: &zbus::blocking::Connection) -> bool {
    let Ok(watcher) = zbus::blocking::Proxy::new(
        connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    ) else {
        return false;
    };
    watcher
        .call::<_, _, ()>("RegisterStatusNotifierItem", &("/StatusNotifierItem",))
        .is_ok()
        && watcher
            .get_property::<bool>("IsStatusNotifierHostRegistered")
            .unwrap_or(false)
}
fn watch_host(
    connection: &zbus::blocking::Connection,
    commands: SyncSender<Command>,
) -> Vec<JoinHandle<()>> {
    let rules = [
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus")
            .unwrap()
            .member("NameOwnerChanged")
            .unwrap()
            .arg(0, "org.kde.StatusNotifierWatcher")
            .unwrap()
            .build(),
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.kde.StatusNotifierWatcher")
            .unwrap()
            .build(),
    ];
    rules
        .into_iter()
        .filter_map(|rule| {
            let messages =
                zbus::blocking::MessageIterator::for_match_rule(rule, connection, Some(16)).ok()?;
            let commands = commands.clone();
            std::thread::Builder::new()
                .name("desktop-tray-host".into())
                .spawn(move || {
                    for message in messages.flatten() {
                        let header = message.header();
                        let member = header.member().map(|member| member.as_str());
                        if matches!(
                            member,
                            Some(
                                "NameOwnerChanged"
                                    | "StatusNotifierHostRegistered"
                                    | "StatusNotifierHostUnregistered"
                            )
                        ) {
                            let _ = commands.try_send(Command::HostChanged);
                        }
                    }
                })
                .ok()
        })
        .collect()
}
pub(crate) struct Tray {
    commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Tray {
    pub(crate) fn start(handler: impl Fn(Event) + Send + 'static) -> Self {
        let (commands, receiver) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let muted = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let mute = muted.clone();
        let methods = commands.clone();
        let host_events = commands.clone();
        let worker = std::thread::Builder::new()
            .name("desktop-tray".into())
            .spawn(move || {
                let connection = (|| {
                    let connection = zbus::blocking::connection::Builder::session()?
                        .method_timeout(Duration::from_secs(2))
                        .serve_at(
                            "/StatusNotifierItem",
                            Item {
                                commands: methods.clone(),
                            },
                        )?
                        .serve_at(
                            "/MenuBar",
                            Menu {
                                commands: methods,
                                muted: mute,
                            },
                        )?
                        .build()?;
                    let watcher = zbus::blocking::Proxy::new(
                        &connection,
                        "org.kde.StatusNotifierWatcher",
                        "/StatusNotifierWatcher",
                        "org.kde.StatusNotifierWatcher",
                    )?;
                    watcher.call::<_, _, ()>(
                        "RegisterStatusNotifierItem",
                        &("/StatusNotifierItem",),
                    )?;
                    let available = watcher
                        .get_property::<bool>("IsStatusNotifierHostRegistered")
                        .unwrap_or(false);
                    if !available {
                        let _ = connection.close();
                        return Err(zbus::Error::Failure("No tray host".into()));
                    }
                    Ok::<_, zbus::Error>(connection)
                })();
                let Ok(connection) = connection else {
                    handler(Event::Available(false));
                    return;
                };
                let listeners = watch_host(&connection, host_events);
                if !register(&connection) {
                    handler(Event::Available(false));
                    let _ = connection.close();
                    for listener in listeners {
                        let _ = listener.join();
                    }
                    return;
                }
                handler(Event::Available(true));
                let mut revision = 1u32;
                while let Ok(command) = receiver.recv() {
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    match command {
                        Command::Event(event) => handler(event),
                        Command::Refresh => {
                            revision = revision.wrapping_add(1);
                            let _ = connection.emit_signal(
                                None::<&str>,
                                "/MenuBar",
                                "com.canonical.dbusmenu",
                                "LayoutUpdated",
                                &(revision, 0i32),
                            );
                        }
                        Command::Stop => break,
                        Command::HostChanged => {
                            if !register(&connection) {
                                handler(Event::Available(false));
                                break;
                            }
                        }
                    }
                }
                let _ = connection.close();
                for listener in listeners {
                    let _ = listener.join();
                }
            })
            .expect("tray thread could not start");
        Self {
            commands,
            stop,
            muted,
            worker: Some(worker),
        }
    }
    pub(crate) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
        let _ = self.commands.try_send(Command::Refresh);
    }
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.commands.try_send(Command::Stop);
    }
    pub(crate) fn shutdown(mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            std::thread::spawn(move || {
                let _ = worker.join();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Watcher {
        host: Arc<AtomicBool>,
        registered: Arc<std::sync::Mutex<Option<(String, String)>>>,
    }
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            self.host.load(Ordering::Relaxed)
        }
        fn register_status_notifier_item(
            &self,
            service: &str,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            *self.registered.lock().unwrap() =
                Some((header.sender().unwrap().to_string(), service.into()));
        }
    }
    #[test]
    #[ignore = "requires a private dbus-run-session"]
    fn synthetic_tray_menu_and_host_loss_contract() {
        assert_eq!(
            std::env::var("LINE_DESKTOP_TEST_BUS").as_deref(),
            Ok("1"),
            "contract tests require LINE_DESKTOP_TEST_BUS=1 on a private session bus"
        );
        let host = Arc::new(AtomicBool::new(true));
        let registered = Arc::new(std::sync::Mutex::new(None));
        let server = zbus::blocking::connection::Builder::session()
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                Watcher {
                    host: host.clone(),
                    registered: registered.clone(),
                },
            )
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .build()
            .unwrap();
        let (sender, events) = mpsc::channel();
        let tray = Tray::start(move |event| {
            let _ = sender.send(event);
        });
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Available(true)
        ));
        let (service, path) = registered.lock().unwrap().clone().unwrap();
        assert_eq!(path, "/StatusNotifierItem");
        let item = zbus::blocking::Proxy::new(
            &server,
            service.as_str(),
            path.as_str(),
            "org.kde.StatusNotifierItem",
        )
        .unwrap();
        assert_eq!(
            item.get_property::<String>("Id").unwrap(),
            crate::desktop_env::desktop_integration::APP_ID
        );
        item.call::<_, _, ()>("Activate", &(0i32, 0i32)).unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Open
        ));
        let menu = zbus::blocking::Proxy::new(
            &server,
            service.as_str(),
            "/MenuBar",
            "com.canonical.dbusmenu",
        )
        .unwrap();
        let (_, layout): (u32, Layout) = menu
            .call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
            .unwrap();
        assert_eq!(layout.2.len(), 3);
        menu.call::<_, _, ()>("Event", &(2i32, "clicked", OwnedValue::from(0i32), 0u32))
            .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Mute
        ));
        tray.set_muted(true);
        let mut properties: Vec<(i32, Properties)> = menu
            .call("GetGroupProperties", &(vec![2i32], Vec::<String>::new()))
            .unwrap();
        assert_eq!(
            String::try_from(properties[0].1.remove("label").unwrap()).unwrap(),
            "Unmute"
        );
        menu.call::<_, _, ()>("Event", &(3i32, "clicked", OwnedValue::from(0i32), 0u32))
            .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Quit
        ));
        host.store(false, Ordering::Relaxed);
        server
            .emit_signal(
                None::<&str>,
                "/StatusNotifierWatcher",
                "org.kde.StatusNotifierWatcher",
                "StatusNotifierHostUnregistered",
                &(),
            )
            .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Available(false)
        ));
        let stopped = std::time::Instant::now();
        tray.shutdown();
        assert!(stopped.elapsed() < Duration::from_secs(3));
        let _ = server.close();
    }
}
