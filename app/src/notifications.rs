use crate::attention_policy::{self as policy, Context, EventKind, Preferences, PreviewLevel};
use crate::*;
pub(crate) struct Notifications {
    pub service: attention::Attention,
    events: std::sync::mpsc::Receiver<attention::Event>,
}
impl Notifications {
    pub fn start() -> Self {
        let service = attention::Attention::start();
        service.probe();
        let (tx, events) = std::sync::mpsc::channel();
        service.on_event(move |event| {
            let _ = tx.send(event);
            crate::event_loop::wake();
        });
        Self { service, events }
    }
    pub fn test(&self, ui: &AppWindow) {
        let prefs = preferences(ui);
        let text = policy::redact(
            prefs.preview,
            "LINE preview",
            "日本語と emoji 🐈 · Test notification",
        );
        ui.global::<State>()
            .set_notification_status("Sending test notification…".into());
        self.service.probe();
        self.service.notify(attention::Notice {
            chat: "notification-test".into(),
            summary: text.summary,
            body: text.body,
            sound: prefs.sound,
            avatar: None,
            last_message: String::new(),
        });
    }
    pub fn incoming(&self, ui: &AppWindow, state: &AppState, m: &MessageInput) {
        let global = ui.global::<State>();
        let muted = state.chats.get(&m.chat_id).is_some_and(|c| c.chat.muted);
        let prefs = preferences(ui);
        let actions = policy::decide(
            &prefs,
            Context {
                active: global.get_window_active(),
                current: state.selected.as_ref() == Some(&m.chat_id),
                at_latest: ui.get_following(),
                muted,
                ..Default::default()
            },
            if m.mentioned {
                EventKind::Mention
            } else if matches!(m.body, Content::System { .. }) {
                EventKind::GroupEvent
            } else {
                EventKind::Message
            },
        );
        if actions.notify && !global.get_notifications_paused() {
            let title = state
                .chats
                .get(&m.chat_id)
                .map(|c| presenter::chat_title(&c.chat, state))
                .unwrap_or("LINE");
            let text = policy::redact(
                prefs.preview,
                title,
                &crate::system_events::text(&m.body, state, &m.sender_id)
                    .unwrap_or_else(|| kind_label(&m.body).1),
            );
            self.service.notify(attention::Notice {
                chat: m.chat_id.0.clone(),
                summary: text.summary,
                body: text.body,
                sound: actions.sound,
                avatar: None,
                last_message: m.server_id.as_str().to_owned(),
            });
        }
    }
    pub fn drain(&self, ui: &AppWindow, state: &mut AppState, sender: &Sender) {
        for event in self.events.try_iter() {
            match event {
                attention::Event::Availability(available) => {
                    ui.global::<State>().set_notification_available(available);
                    if !available {
                        ui.global::<State>()
                            .set_notification_status("Notification service not available".into());
                    }
                }
                attention::Event::OpenChat { chat, token } => {
                    let _ = token;
                    sender.send(state.select(ChatId(chat)));
                    let _ = ui.show();
                    ui.set_show_conversation(true);
                }
                attention::Event::MarkRead { chat, message } => {
                    if let Ok(id) = ServerId::new(message) {
                        sender.send(Intent::MarkRead {
                            chat: ChatId(chat),
                            id,
                        });
                    }
                }
                attention::Event::Delivered(chat) if chat == "notification-test" => ui
                    .global::<State>()
                    .set_notification_status("Test notification sent".into()),
                attention::Event::Delivered(chat) | attention::Event::Closed(chat) => {
                    let _ = chat;
                }
            }
        }
    }
}
pub fn preferences(ui: &AppWindow) -> Preferences {
    let s = ui.global::<State>();
    Preferences {
        enabled: s.get_notifications_enabled(),
        preview: match s.get_notification_preview() {
            0 => PreviewLevel::NameAndText,
            2 => PreviewLevel::NewMessage,
            _ => PreviewLevel::NameOnly,
        },
        sound: s.get_notification_sound(),
        mention_exception: s.get_notification_mention_exception(),
        group_events: s.get_notification_group_events(),
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        self.service.withdraw_all();
    }
}
