//! Notification policy adapters. Network access remains exclusively in the account worker.
use crate::{
    attention::{Avatar, Event, Notice},
    attention_policy::{self as policy, Context, EventKind, Preferences, PreviewLevel},
    *,
};

pub(crate) fn preferences(ui: &AppWindow) -> Preferences {
    let state = ui.global::<State>();
    Preferences {
        enabled: state.get_notifications_enabled(),
        preview: match state.get_notification_preview() {
            0 => PreviewLevel::NameAndText,
            2 => PreviewLevel::NewMessage,
            _ => PreviewLevel::NameOnly,
        },
        sound: state.get_notification_sound(),
        mention_exception: state.get_notification_mention_exception(),
        group_events: state.get_notification_group_events(),
    }
}
impl View {
    pub(crate) fn apply_notification_settings(&self, ui: &AppWindow) {
        let preferences = &self.saved.notifications;
        let state = ui.global::<State>();
        state.set_notifications_enabled(preferences.enabled);
        state.set_notification_preview(match preferences.preview {
            PreviewLevel::NameAndText => 0,
            PreviewLevel::NameOnly => 1,
            PreviewLevel::NewMessage => 2,
        });
        state.set_notification_sound(preferences.sound);
        state.set_notification_mention_exception(preferences.mention_exception);
        state.set_notification_group_events(preferences.group_events);
    }
    pub(crate) fn capture_notification_settings(&mut self, ui: &AppWindow) {
        let next = preferences(ui);
        if next != self.saved.notifications {
            if let Some(attention) = &self.attention {
                if !next.enabled
                    || next.preview != self.saved.notifications.preview
                    || (!next.mention_exception && self.saved.notifications.mention_exception)
                    || (!next.group_events && self.saved.notifications.group_events)
                {
                    attention.withdraw_notifications();
                }
                attention.probe();
            }
            self.saved.notifications = next;
        }
    }
    pub(crate) fn notification_update(&mut self, ui: &AppWindow, event: &account::Update) {
        match event {
            account::Update::Incoming {
                chat,
                messages,
                replay: false,
            } => self.attention_event(ui, chat, messages),
            account::Update::SendRejected { id, .. } | account::Update::SendFailed { id, .. } => {
                self.attention_notice(
                    ui,
                    id,
                    "Message delivery needs attention.",
                    "",
                    EventKind::SendFailure,
                )
            }
            account::Update::Sync(sync) => {
                use line_wasm_poc::client::desktop::sync::SyncState;
                let next = match sync {
                    SyncState::Connected => 1,
                    SyncState::Reconnecting { .. } => 2,
                    SyncState::Offline => 3,
                    SyncState::AuthExpired => 4,
                };
                if next != self.notification_connection {
                    if next == 1 && self.notification_connection > 1 {
                        ui.global::<State>()
                            .set_transient_notice("Reconnected".into());
                    }
                    if next > 1 {
                        self.attention_notice(
                            ui,
                            "account",
                            match next {
                                2 => "Reconnecting to LINE.",
                                3 => "LINE is offline.",
                                _ => "Sign in again to reconnect.",
                            },
                            "",
                            EventKind::Account,
                        );
                    }
                    self.notification_connection = next;
                }
            }
            account::Update::Withdrawn { chat, .. } => {
                if let Some(attention) = &self.attention {
                    attention.withdraw(chat);
                }
            }
            account::Update::ReadMarked {
                chat,
                up_to_message,
            } => {
                if self
                    .notification_reads
                    .get(chat)
                    .is_some_and(|requested| requested == up_to_message)
                {
                    self.notification_reads.remove(chat);
                    self.unread.elsewhere(chat, up_to_message);
                    self.unread_ui(ui);
                    if let Some(attention) = &self.attention {
                        attention.withdraw(chat);
                    }
                }
            }
            account::Update::SessionChanged => {
                if let Some(attention) = &self.attention {
                    attention.withdraw_all();
                }
                self.notification_connection = 0;
                self.notification_through.clear();
            }
            _ => {}
        }
    }
    pub(crate) fn attention_event(
        &mut self,
        ui: &AppWindow,
        chat: &str,
        messages: &[account::MessageView],
    ) {
        if self.demo && !ui.global::<State>().get_demo_notifications() {
            return;
        }
        let through = self
            .notification_through
            .get(chat)
            .cloned()
            .unwrap_or_default();
        let key = |id: &str| (id.trim_start_matches('0').len(), id.to_owned());
        let newest = messages
            .iter()
            .map(|message| &message.id)
            .max_by_key(|id| key(id));
        if let Some(newest) = newest
            && key(newest) > key(&through)
        {
            if self.notification_through.len() >= 512
                && !self.notification_through.contains_key(chat)
                && let Some(oldest) = self.notification_through.keys().next().cloned()
            {
                self.notification_through.remove(&oldest);
            }
            self.notification_through
                .insert(chat.into(), newest.clone());
        }
        let old = if chat == self.current {
            Some(&self.messages)
        } else {
            self.history.get(chat).map(|h| &h.messages)
        };
        // Updates to old rows include reactions and withdrawal; they never alert.
        let Some(message) = messages.iter().rev().find(|message| {
            key(&message.id) > key(&through)
                && !message.outgoing
                && !message.rich.withdrawn
                && !old.is_some_and(|rows| rows.iter().any(|row| row.id == message.id))
                && !self
                    .recent
                    .get(chat)
                    .is_some_and(|rows| rows.iter().any(|row| row.id == message.id))
        }) else {
            return;
        };
        let kind = if message.mentioned {
            EventKind::Mention
        } else if message.rich.content_type == 18 {
            EventKind::GroupEvent
        } else {
            EventKind::Message
        };
        let body = if !message.body.is_empty() {
            message.body.as_str()
        } else if message.photo {
            "Photo"
        } else if message.sticker {
            "Sticker"
        } else {
            message.rich.file_name.as_deref().unwrap_or("New message")
        };
        self.attention_notice(ui, chat, body, &message.id, kind);
    }
    fn attention_notice(
        &self,
        ui: &AppWindow,
        chat: &str,
        body: &str,
        last_message: &str,
        kind: EventKind,
    ) {
        let state = ui.global::<State>();
        let index = self.ids.iter().position(|id| id == chat);
        let row = index.and_then(|i| self.rows.get(i));
        let current = chat == self.current;
        let active = state.get_window_active();
        let context = Context {
            active,
            current,
            at_latest: ui.get_following(),
            exposed: current && !state.get_modal_open() && !self.unread.exposed.is_empty(),
            replay: false,
            muted: row.is_some_and(|row| row.muted),
        };
        self.post_notice(ui, chat, (body, last_message), kind, context, row);
    }
    fn post_notice(
        &self,
        ui: &AppWindow,
        chat: &str,
        payload: (&str, &str),
        kind: EventKind,
        context: Context,
        row: Option<&ChatRow>,
    ) {
        let (body, last_message) = payload;
        let mut preferences = preferences(ui);
        preferences.enabled &= !ui.global::<State>().get_notifications_paused();
        let actions = policy::decide(&preferences, context, kind);
        if actions.mention_indicator {
            ui.global::<State>().set_transient_notice(
                format!(
                    "Mention in {}",
                    row.map(|r| r.title.as_str())
                        .unwrap_or("another conversation")
                )
                .into(),
            );
        }
        if actions.attention {
            request_attention(ui);
        }
        if !actions.notify {
            return;
        }
        let Some(attention) = &self.attention else {
            return;
        };
        let name = row
            .map(|row| row.title.as_str())
            .unwrap_or(if chat == "account" {
                "LINE"
            } else {
                "New message"
            });
        let text = policy::redact(preferences.preview, name, body);
        let avatar = if text.avatar {
            row.and_then(|row| row.avatar.to_rgba8())
                .filter(|pixels| pixels.width() <= 256 && pixels.height() <= 256)
                .map(|pixels| Avatar {
                    width: pixels.width(),
                    height: pixels.height(),
                    rgba: pixels.as_bytes().to_vec(),
                })
        } else {
            None
        };
        attention.notify(Notice {
            chat: chat.into(),
            summary: text.summary,
            body: text.body,
            sound: actions.sound,
            avatar,
            last_message: last_message.into(),
        });
    }
    fn notification_action(&mut self, ui: &AppWindow, event: Event) {
        match event {
            Event::Availability(available) => {
                ui.global::<State>().set_notification_available(available);
                if !available {
                    ui.global::<State>()
                        .set_notification_status("Notification service not available".into());
                } else if ui.global::<State>().get_notification_status()
                    == "Notification service not available"
                {
                    ui.global::<State>().set_notification_status("".into());
                }
            }
            Event::Closed(chat) => {
                if chat == "notification-test" {
                    ui.global::<State>()
                        .set_notification_status("Test notification dismissed".into());
                }
            }
            Event::Delivered(chat) => {
                if chat == "notification-test" {
                    ui.global::<State>()
                        .set_notification_status("Test notification sent".into());
                }
            }
            Event::OpenChat { chat, token } => {
                // Winit 0.30 only accepts an external startup token when creating a window.
                // An existing Slint window uses the compositor's attention fallback.
                let _ = token;
                let _ = ui.show();
                request_attention(ui);
                use slint::winit_030::WinitWindowAccessor;
                if !ui.global::<State>().get_automation_background() {
                    ui.window().with_winit_window(|window| {
                        window.set_minimized(false);
                        window.focus_window();
                    });
                }
                if let Some(index) = self.ids.iter().position(|id| id == &chat) {
                    self.select(ui, index);
                    ui.set_show_conversation(true);
                    self.unread.enter(&chat);
                    self.unread_page(ui);
                    ui.invoke_focus_composer();
                }
            }
            Event::MarkRead { chat, message } => {
                if self.demo {
                    self.unread.elsewhere(&chat, &message);
                    self.unread_ui(ui);
                    if let Some(attention) = &self.attention {
                        attention.withdraw(&chat);
                    }
                } else if self.request(
                    ui,
                    account::Command::MarkReadExplicit {
                        chat: chat.clone(),
                        up_to_message: message.clone(),
                    },
                ) {
                    if self.notification_reads.len() < 512
                        || self.notification_reads.contains_key(&chat)
                    {
                        self.notification_reads.insert(chat, message);
                    }
                } else {
                    ui.global::<State>().set_notification_status(
                        "Could not queue Mark read. Try again from the conversation.".into(),
                    );
                }
            }
        }
    }
    fn test_notification(&self, ui: &AppWindow) {
        ui.global::<State>()
            .set_notification_status("Sending test notification…".into());
        if let Some(attention) = &self.attention {
            attention.probe();
            let text = policy::redact(
                preferences(ui).preview,
                "LINE preview",
                "日本語と emoji 🐈 · Test notification",
            );
            attention.notify(Notice {
                chat: "notification-test".into(),
                summary: text.summary,
                body: text.body,
                sound: preferences(ui).sound,
                avatar: None,
                last_message: String::new(),
            });
        } else {
            ui.global::<State>()
                .set_notification_status("Notification service not available".into());
        }
    }
    fn demo_notification(&self, ui: &AppWindow, kind: i32) {
        if !self.demo || !ui.global::<State>().get_demo_notifications() {
            return;
        }
        let context = Context {
            active: kind == 4,
            current: kind == 4,
            at_latest: kind == 4,
            exposed: kind == 4,
            replay: kind == 2,
            muted: kind == 1 || kind == 3,
        };
        let event = if kind == 3 {
            EventKind::Mention
        } else {
            EventKind::Message
        };
        self.post_notice(
            ui,
            "demo-attention",
            ("日本語と emoji 🐈 · Preview message", "synthetic-boundary"),
            event,
            context,
            self.rows.first(),
        );
    }
}
fn request_attention(ui: &AppWindow) {
    if ui.global::<State>().get_automation_background() {
        return;
    }
    use slint::winit_030::WinitWindowAccessor;
    ui.window().with_winit_window(|window| {
        window.request_user_attention(Some(
            slint::winit_030::winit::window::UserAttentionType::Informational,
        ))
    });
}
pub(crate) fn install(ui: &AppWindow, state: Rc<RefCell<View>>) {
    if let Some(attention) = &state.borrow().attention {
        let (sender, events) = std::sync::mpsc::sync_channel(32);
        attention.on_event(move |event| {
            let _ = sender.try_send(event);
        });
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                let (completed, completion) = std::sync::mpsc::sync_channel(1);
                if weak
                    .upgrade_in_event_loop(move |ui| {
                        UI_STATE.with(|slot| {
                            if let Some(view) = slot.borrow().as_ref() {
                                view.borrow_mut().notification_action(&ui, event);
                            }
                        });
                        let _ = completed.send(());
                    })
                    .is_err()
                {
                    break;
                }
                if completion
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    let weak = ui.as_weak();
    let view = state.clone();
    ui.global::<State>().on_test_notification(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().test_notification(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_demo_notification(move |kind| {
        if let Some(ui) = weak.upgrade() {
            state.borrow().demo_notification(&ui, kind);
        }
    });
}
