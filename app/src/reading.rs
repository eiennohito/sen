//! Reading decisions use logical content coordinates, independent of Slint.
use line::{
    controller::AppState,
    engine::{Connection, Intent},
    store::*,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Reading {
    pub(crate) jump_target: Option<ServerId>,
    pub(crate) highlight_timer: slint::Timer,
    pub(crate) latest_requested: bool,
    pub(crate) opening_history: bool,
    pub(crate) divider_top: Option<f32>,
    pub(crate) return_anchor: Option<(Key, f32, bool)>,
    pub(crate) geometry: BTreeMap<Key, (f32, f32)>,
    pub(crate) gap_geometry: BTreeMap<Key, (f32, f32)>,
    pub(crate) requested_gaps: BTreeSet<(u64, Key)>,
    pub(crate) restore_ready: bool,
    pub(crate) exposed: BTreeMap<ChatId, ServerId>,
    pub(crate) paging_anchor: Option<Key>,
    pub(crate) restore_anchor: Option<(Key, f32)>,
    pub(crate) oldest: Option<(ChatId, Cursor)>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Geometry {
    pub y: f32,
    pub at_bottom: bool,
    pub visible_top: f32,
    pub height: f32,
    pub pill_height: f32,
    pub active: bool,
    pub modal: bool,
    pub dragging: bool,
    pub interacting: bool,
    pub narrow: bool,
    pub conversation: bool,
    pub older_start_known: bool,
}

#[derive(Default)]
pub(crate) struct Effects {
    pub intents: Vec<Intent>,
    pub show: Option<(f32, bool)>,
    pub jump: Option<f32>,
}

pub(crate) fn observe(reading: &mut Reading, state: &AppState, view: Geometry) -> Effects {
    let mut effects = Effects::default();
    let top = -view.y + view.visible_top;
    let bottom = -view.y + view.height;
    if state.connection == Connection::Online {
        for gap in &state.gaps {
            if reading
                .gap_geometry
                .get(&gap.after)
                .is_some_and(|(y, h)| y + h >= top && *y <= bottom)
                && reading.requested_gaps.insert((state.window_sub, gap.after))
            {
                let row = &state.messages[&gap.after];
                effects.intents.push(Intent::Page {
                    chat: row.chat_id.clone(),
                    kind: PageKind::Older {
                        before: row.sort.clone(),
                    },
                });
            }
        }
    }

    if reading.opening_history
        && state.unread_divider.is_some()
        && let Some(top) = reading.divider_top
    {
        effects.show = Some((-top + view.visible_top, false));
        reading.opening_history = false;
    } else if reading.opening_history && state.reaches_end {
        effects.show = Some((0., true));
        reading.opening_history = false;
    }
    effects
}

pub(crate) fn exposure(reading: &mut Reading, state: &AppState, view: Geometry) -> Vec<Intent> {
    let mut effects = Effects::default();
    if view.active
        && !view.modal
        && !view.dragging
        && !view.interacting
        && (!view.narrow || view.conversation)
        && state.connection == Connection::Online
        && let Some(chat) = state.selected.clone()
    {
        let top = -view.y + view.visible_top;
        let bottom = -view.y + view.height - view.pill_height;
        let exposed = state
            .rows()
            .iter()
            .filter(|m| {
                reading
                    .geometry
                    .get(&m.key)
                    .is_some_and(|(y, h)| *y >= top && y + h <= bottom)
            })
            .filter(|m| !m.outgoing && matches!(m.delivery, Delivery::Sent | Delivery::Read))
            .filter_map(|m| m.server_id.clone())
            .max_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));
        let tail_visible = state
            .rows()
            .into_iter()
            .rev()
            .find(|m| line::system_events::displayable(&m.body))
            .is_none_or(|m| {
                reading
                    .geometry
                    .get(&m.key)
                    .is_some_and(|(y, h)| y + h > top && y + h <= bottom)
            });
        let marker = if view.at_bottom
            && tail_visible
            && view.height > 0.
            && state.reaches_end
            && !state.window_loading
            && !reading.opening_history
        {
            state
                .messages
                .values()
                .filter(|m| !m.outgoing && matches!(m.delivery, Delivery::Sent | Delivery::Read))
                .filter_map(|m| m.server_id.clone())
                .chain(
                    state
                        .chats
                        .get(&chat)
                        .and_then(|c| c.preview.as_ref())
                        .filter(|m| {
                            !m.outgoing && matches!(m.delivery, Delivery::Sent | Delivery::Read)
                        })
                        .and_then(|m| m.server_id.clone()),
                )
                .max_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)))
        } else {
            exposed
        };
        if let Some(id) = marker
            && state
                .chats
                .get(&chat)
                .is_some_and(|c| c.chat.unread_count > 0)
            && reading.exposed.get(&chat) != Some(&id)
        {
            reading.exposed.insert(chat.clone(), id.clone());
            effects.intents.push(Intent::MarkRead { chat, id });
        }
    }
    effects.intents
}

pub(crate) fn restore(reading: &mut Reading) -> Option<f32> {
    let mut effects = Effects::default();
    if reading.restore_ready
        && let Some((key, offset)) = reading.restore_anchor
        && let Some((top, _)) = reading.geometry.get(&key)
    {
        effects.jump = Some(offset - top);
        reading.restore_anchor = None;
        reading.restore_ready = false;
    }
    effects.jump
}

pub(crate) fn page_older(
    reading: &mut Reading,
    state: &mut AppState,
    view: Geometry,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if view.y >= -30.
        && !view.older_start_known
        && let Some((chat, before)) = reading.oldest.clone()
    {
        if let Some(key) = state
            .rows()
            .into_iter()
            .find(|m| m.server_id.is_some())
            .map(|m| m.key)
            && reading.paging_anchor != Some(key)
        {
            let offset = reading
                .geometry
                .get(&key)
                .map(|(y, _)| y + view.y)
                .unwrap_or(0.);
            reading.paging_anchor = Some(key);
            reading.restore_anchor = Some((key, offset));
            reading.restore_ready = false;
            reading.requested_gaps.clear();
            let intent = state.subscribe_window(Anchor::Key(key), 80, 80);
            intents.push(intent);
        }
        intents.push(Intent::Page {
            chat,
            kind: PageKind::Older { before },
        });
    }
    intents
}

pub(crate) fn return_anchor(
    reading: &Reading,
    rows: Vec<&StoredMessage>,
    view: Geometry,
    following: bool,
) -> Option<(Key, f32, bool)> {
    let top = -view.y + view.visible_top;
    rows.into_iter().find_map(|m| {
        reading
            .geometry
            .get(&m.key)
            .filter(|(y, h)| y + h > top)
            .map(|(y, _)| {
                let inset = view.visible_top;
                (m.key, y + view.y + inset, following)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposing_hidden_incoming_events_clears_the_chat_badge() {
        std::thread::spawn(|| {
            use line::{
                backend::FakeBackend,
                engine::{StoreLocation, Worker},
            };
            use slint::Model;
            let fake = FakeBackend::canned();
            let chat = ChatId("alice".into());
            let mut hidden = FakeBackend::message("alice", 951, "", false);
            hidden.body = Content::System {
                kind: Some("OA_UNKNOWN".into()),
                params: None,
                text: None,
            };
            fake.add_chat(ChatInput {
                id: chat.clone(),
                title: "Official hidden events".into(),
                unread_count: 1,
                last_message_id: Some(ServerId("951".into())),
                ..Default::default()
            });
            fake.set_page(chat.clone(), vec![hidden]);
            let worker = Worker::start(fake, StoreLocation::Memory);
            let mut state = AppState::default();
            worker.sender.send(state.select(chat.clone()));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.connection != Connection::Online
                || !state.reaches_end
                || state.messages.is_empty()
                || state
                    .chats
                    .get(&chat)
                    .is_none_or(|c| c.chat.unread_count == 0)
            {
                state.apply(
                    worker
                        .deltas
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .unwrap(),
                );
            }
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut presenter = crate::presenter::Presenter::default();
            presenter.install(&ui);
            presenter.present(&ui, &state, "", 0);
            assert!(
                ui.get_chats()
                    .iter()
                    .any(|row| row.id == "alice" && row.unread == 1)
            );
            let intents = exposure(
                &mut Reading::default(),
                &state,
                Geometry {
                    active: true,
                    height: 400.,
                    at_bottom: true,
                    ..Default::default()
                },
            );
            assert!(matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.0 == "951"));
            for intent in intents {
                worker.sender.send(intent);
            }
            while state.chats[&chat].chat.unread_count != 0 {
                state.apply(
                    worker
                        .deltas
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .unwrap(),
                );
            }
            presenter.present(&ui, &state, "", 0);
            let row = ui.get_chats().iter().find(|row| row.id == "alice").unwrap();
            assert_eq!(row.unread, 0);
            assert!(!row.unread_unknown);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn exposure_requires_active_unobscured_rows_and_advances_numeric_ids_once() {
        let chat = ChatId("chat".into());
        let mut state = AppState {
            connection: Connection::Online,
            selected: Some(chat.clone()),
            ..Default::default()
        };
        state.chats.insert(
            chat.clone(),
            ChatListItem {
                chat: ChatRecord {
                    id: chat.clone(),
                    unread_count: 2,
                    ..Default::default()
                },
                preview: None,
                activity_ms: 0,
                failed_count: 0,
                draft: None,
            },
        );
        for (key, id) in [(Key(1), "9"), (Key(2), "10")] {
            state.messages.insert(
                key,
                StoredMessage {
                    key,
                    chat_id: chat.clone(),
                    server_id: Some(ServerId(id.into())),
                    local_id: None,
                    request_seq: None,
                    sort: Cursor {
                        sort_ts: key.0,
                        sort_id: id.into(),
                    },
                    sender_id: "sender".into(),
                    outgoing: false,
                    body: Content::default(),
                    reactions: vec![],
                    reactions_rev: 0,
                    content_rev: None,
                    delivery: Delivery::Sent,
                    created_at_local: 0,
                    mentioned: false,
                    reply: None,
                    skip_badge: false,
                },
            );
        }
        let mut reading = Reading {
            geometry: [(Key(1), (20., 30.)), (Key(2), (70., 30.))].into(),
            ..Default::default()
        };
        let mut view = Geometry {
            height: 100.,
            pill_height: 10.,
            ..Default::default()
        };
        assert!(exposure(&mut reading, &state, view).is_empty());
        view.active = true;
        let intents = exposure(&mut reading, &state, view);
        assert!(matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.0 == "9"));
        assert!(exposure(&mut reading, &state, view).is_empty());
        view.pill_height = 0.;
        state.messages.get_mut(&Key(2)).unwrap().outgoing = true;
        assert!(exposure(&mut reading, &state, view).is_empty());
        state.messages.get_mut(&Key(2)).unwrap().outgoing = false;
        state.messages.get_mut(&Key(2)).unwrap().delivery = Delivery::Pending;
        assert!(exposure(&mut reading, &state, view).is_empty());
        state.messages.get_mut(&Key(2)).unwrap().delivery = Delivery::Sent;
        view.modal = true;
        assert!(exposure(&mut reading, &state, view).is_empty());
        view.modal = false;
        let intents = exposure(&mut reading, &state, view);
        assert!(matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.0 == "10"));
        assert!(exposure(&mut reading, &state, view).is_empty());
    }
}
