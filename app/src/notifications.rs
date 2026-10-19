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
    pub fn test(&self, ui: &AppWindow, state: &AppState) {
        let prefs = preferences(&state.prefs.desktop);
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
            last_message: String::new(),
        });
    }
    pub fn incoming(&self, ui: &AppWindow, state: &AppState, m: &MessageInput) {
        let global = ui.global::<State>();
        let muted = state.chats.get(&m.chat_id).is_some_and(|c| c.chat.muted);
        let prefs = preferences(&state.prefs.desktop);
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
                .map(|c| presenter::chats::chat_title(&c.chat, state))
                .unwrap_or("LINE");
            let text = policy::redact(
                prefs.preview,
                title,
                &presenter::messages::compact_text(&m.body, state, &m.sender_id),
            );
            self.service.notify(attention::Notice {
                chat: m.chat_id.0.clone(),
                summary: text.summary,
                body: text.body,
                sound: actions.sound,
                last_message: m.server_id.as_str().to_owned(),
            });
        }
    }
    pub(crate) fn events(&self) -> Vec<attention::Event> {
        self.events.try_iter().collect()
    }
}

pub(crate) fn handle(event: attention::Event, ui: &AppWindow, desktop: &mut Desktop) {
    match event {
        attention::Event::Availability(available) => {
            ui.global::<State>().set_notification_available(available);
            if !available {
                ui.global::<State>()
                    .set_notification_status("Notification service not available".into());
            }
        }
        attention::Event::OpenChat { chat } => {
            desktop.open_chat(ui, ChatId(chat));
            let _ = ui.show();
            ui.set_show_conversation(true);
        }
        attention::Event::MarkRead { chat, message } => {
            if let Ok(id) = ServerId::new(message) {
                desktop.sender.send(Intent::MarkRead {
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

fn preferences(prefs: &DesktopPrefs) -> Preferences {
    Preferences {
        enabled: prefs.notifications,
        preview: match prefs.preview {
            0 => PreviewLevel::NameAndText,
            2 => PreviewLevel::NewMessage,
            _ => PreviewLevel::NameOnly,
        },
        sound: prefs.sound,
        mention_exception: prefs.mention_exception,
        group_events: prefs.group_events,
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        self.service.withdraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_notification_opens_preserve_both_persisted_drafts() {
        std::thread::spawn(|| {
            use line::{
                backend::FakeBackend,
                engine::{StoreLocation, Worker},
            };
            let worker = Worker::start(
                StoreLocation::Memory,
                {
                    let mut fake = Some(FakeBackend::canned());
                    move || Ok(fake.take().unwrap())
                },
                line::engine::WorkerOptions::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let chat_a = ChatId("alice".into());
            let chat_b = ChatId("weekend".into());
            let mut desktop = Desktop {
                clipboard_root: std::env::temp_dir(),
                history: Default::default(),
                shown_composer: String::new(),
                reveal: None,
                viewer_asset: None,
                destination: viewer::Destination::Dialog,
                notifications: None,
                background: Default::default(),
                assets: Default::default(),
                state: AppState::default(),
                presenter: Default::default(),
                sender: worker.sender.clone(),
                query: String::new(),
                category: 0,
            };
            let receive = |reason: &str| {
                worker
                    .deltas
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap_or_else(|error| {
                        panic!("{reason}: worker did not deliver the expected delta: {error}")
                    })
            };
            desktop.sender.send(desktop.state.select(chat_a.clone()));
            while desktop.state.connection != Connection::Online || !desktop.state.reaches_end {
                desktop.state.apply(receive("loading canned chat A"));
            }
            assert!(
                desktop.state.chats.contains_key(&chat_a),
                "canned chat A is missing"
            );
            assert!(
                desktop.state.chats.contains_key(&chat_b),
                "canned chat B is missing"
            );
            let draft_b = Draft {
                text: "persisted draft in B".into(),
                cursor: 6,
                anchor: 2,
                reply: None,
                updated: 0,
            };
            desktop.sender.send(Intent::Draft {
                chat: chat_b.clone(),
                draft: draft_b.clone(),
            });
            while desktop.state.chats[&chat_b].draft.as_ref() != Some(&draft_b) {
                desktop
                    .state
                    .apply(receive("waiting for B's draft to be persisted"));
            }
            desktop.render(&ui);
            ui.set_draft("unsent draft in A".into());
            ui.set_editor_caret(8);
            ui.set_editor_anchor(3);
            let (events, receiver) = std::sync::mpsc::channel();
            desktop.notifications = Some(Notifications {
                service: attention::Attention::disconnected(),
                events: receiver,
            });
            events
                .send(attention::Event::OpenChat {
                    chat: chat_b.0.clone(),
                })
                .unwrap();
            // C is A again: both destinations exist in the fake's canned data.
            events
                .send(attention::Event::OpenChat {
                    chat: chat_a.0.clone(),
                })
                .unwrap();
            desktop.drain_notifications(&ui);
            assert_eq!(
                desktop.state.selected,
                Some(chat_a.clone()),
                "the second queued OpenChat did not select C"
            );
            assert_eq!(
                ui.get_draft().as_str(),
                "unsent draft in A",
                "the composer did not follow the second selection"
            );
            // Consume the queued selection acknowledgements through A's draft.
            // Otherwise B's earlier acknowledgement can hide a later overwrite.
            loop {
                let delta = receive("waiting for A's queued selection draft");
                let arrived = matches!(&delta, Delta::Draft { chat, .. } if chat == &chat_a);
                desktop.apply_batch(&ui, [delta]);
                if arrived {
                    break;
                }
            }
            // Window requests are ordered behind both notification draft saves.
            // Read each draft back from the engine, without another UI capture.
            for (chat, text, cursor, anchor) in [
                (chat_b, "persisted draft in B", 6, 2),
                (chat_a, "unsent draft in A", 8, 3),
            ] {
                desktop.sender.send(desktop.state.select(chat.clone()));
                loop {
                    if let Delta::Draft {
                        chat: loaded,
                        draft,
                    } = receive(&format!("reading persisted draft for {}", chat.0))
                        && loaded == chat
                    {
                        let draft = draft.unwrap_or_else(|| {
                            panic!("draft for {} was erased by queued OpenChat events", chat.0)
                        });
                        assert_eq!(
                            draft.text, text,
                            "queued OpenChat overwrote draft text for {}",
                            chat.0
                        );
                        assert_eq!(
                            draft.cursor, cursor,
                            "queued OpenChat changed draft caret for {}",
                            chat.0
                        );
                        assert_eq!(
                            draft.anchor, anchor,
                            "queued OpenChat changed draft anchor for {}",
                            chat.0
                        );
                        break;
                    }
                }
            }
        })
        .join()
        .unwrap();
    }
}
