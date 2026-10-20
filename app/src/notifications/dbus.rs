//! Notification service calls and signal listener.
use super::worker::{Event, Notice, Shared};
use std::{collections::HashMap, sync::Arc, time::Duration};
use zbus::zvariant::{OwnedValue, Str};

pub(super) fn notification_available(connection: &zbus::blocking::Connection) -> bool {
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
pub(super) fn connect() -> Option<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(Duration::from_secs(2))
        .build()
        .ok()
}
pub(super) fn close(connection: &zbus::blocking::Connection, id: u32) {
    if let Ok(proxy) = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    ) {
        let _ = proxy.call_noreply("CloseNotification", &(id,));
    }
}
pub(super) fn notify(
    connection: &zbus::blocking::Connection,
    notice: &Notice,
    previous: u32,
    sound: bool,
) -> zbus::Result<u32> {
    let mut hints: HashMap<&str, OwnedValue> = HashMap::from([
        (
            "desktop-entry",
            Str::from(crate::desktop_env::desktop_integration::APP_ID).into(),
        ),
        ("category", Str::from("im.received").into()),
        ("urgency", 1u8.into()),
        ("resident", false.into()),
        ("suppress-sound", (!sound).into()),
    ]);
    if sound {
        hints.insert("sound-name", Str::from("message-new-instant").into());
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
                "閃",
                previous,
                crate::desktop_env::desktop_integration::APP_ID,
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
pub(super) fn listen(
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
            for message in messages.flatten() {
                match message.header().member().map(|member| member.as_str()) {
                    Some("ActionInvoked") => {
                        if let Ok((id, action)) = message.body().deserialize::<(u32, String)>() {
                            let known = shared.ids.lock().unwrap().get(&id).cloned();
                            if let Some((chat, last)) = known {
                                match action.as_str() {
                                    "default" => shared.event(Event::OpenChat { chat }),
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
                            shared.ids.lock().unwrap().remove(&id);
                        }
                    }
                    _ => {}
                }
            }
        })
        .ok()
}
