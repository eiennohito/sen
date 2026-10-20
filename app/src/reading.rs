//! Reading decisions use logical content coordinates, independent of Slint.
use line::{
    controller::AppState,
    engine::{Connection, Intent},
    store::*,
};
use line_desktop_ui::ReadingGeometry;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Reading {
    pub(crate) jump_target: Option<ServerId>,
    pub(crate) highlight_timer: slint::Timer,
    pub(crate) latest_requested: bool,
    pub(crate) opening_history: bool,
    pub(crate) divider_top: Option<f32>,
    pub(crate) return_anchor: Option<(Key, f32)>,
    pub(crate) geometry: BTreeMap<Key, (f32, f32)>,
    pub(crate) gap_geometry: BTreeMap<Key, (f32, f32)>,
    pub(crate) requested_gaps: BTreeSet<(u64, Key)>,
    pub(crate) restore_ready: bool,
    pub(crate) exposed: BTreeMap<ChatId, ServerId>,
    pub(crate) paging_anchor: Option<Key>,
    pub(crate) restore_anchor: Option<(Key, f32)>,
}

impl Reading {
    pub(crate) fn reset_for_chat(&mut self) {
        self.latest_requested = false;
        self.opening_history = true;
        self.divider_top = None;
        self.return_anchor = None;
        self.jump_target = None;
        self.highlight_timer.stop();
        self.restore_anchor = None;
        self.geometry.clear();
        self.gap_geometry.clear();
        self.requested_gaps.clear();
        self.paging_anchor = None;
    }
    pub(crate) fn jump_to(&mut self, key: Key, offset: f32) {
        self.restore_anchor = Some((key, offset));
        self.restore_ready = self.geometry.contains_key(&key);
        if !self.restore_ready {
            self.geometry.clear();
        }
    }
}

#[derive(Default)]
pub(crate) struct Effects {
    pub intents: Vec<Intent>,
    pub show: Option<(f32, bool)>,
}

pub(crate) fn observe(reading: &mut Reading, state: &AppState, view: ReadingGeometry) -> Effects {
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

pub(crate) fn exposure(
    reading: &mut Reading,
    state: &AppState,
    view: ReadingGeometry,
) -> Vec<Intent> {
    let mut intents = Vec::new();
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
        let rows = state.rows();
        let exposed = rows
            .iter()
            .filter(|m| {
                reading
                    .geometry
                    .get(&m.key)
                    .is_some_and(|(y, h)| *y >= top && y + h <= bottom)
            })
            .filter(|m| !m.outgoing && matches!(m.delivery, Delivery::Sent | Delivery::Read))
            .filter_map(|m| m.server_id.clone())
            .max();
        let tail_visible = rows
            .iter()
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
                .max()
        } else {
            exposed
        };
        if let Some(id) = marker
            && state.chats.get(&chat).is_some_and(|c| c.unread_count > 0)
            && reading.exposed.get(&chat) != Some(&id)
        {
            reading.exposed.insert(chat.clone(), id.clone());
            intents.push(Intent::MarkRead { chat, id });
        }
    }
    intents
}

pub(crate) fn restore(reading: &mut Reading) -> Option<f32> {
    let mut jump = None;
    if reading.restore_ready
        && let Some((key, offset)) = reading.restore_anchor
        && let Some((top, _)) = reading.geometry.get(&key)
    {
        jump = Some(offset - top);
        reading.restore_anchor = None;
        reading.restore_ready = false;
    }
    jump
}

pub(crate) fn page_older(
    reading: &mut Reading,
    state: &mut AppState,
    view: ReadingGeometry,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    if view.y >= -30.
        && !state.reaches_start
        && let Some(chat) = state.selected.clone()
        && let Some((key, before)) = state
            .messages
            .values()
            .filter(|m| m.server_id.is_some())
            .min_by(|a, b| a.sort.cmp(&b.sort))
            .map(|m| (m.key, m.sort.clone()))
    {
        if reading.paging_anchor != Some(key) {
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
    view: ReadingGeometry,
) -> Option<(Key, f32)> {
    let top = -view.y + view.visible_top;
    rows.into_iter().find_map(|m| {
        reading
            .geometry
            .get(&m.key)
            .filter(|(y, h)| y + h > top)
            .map(|(y, _)| {
                let inset = view.visible_top;
                (m.key, y + view.y + inset)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatCategory;
    use crate::*;
    use slint::ComponentHandle;

    #[test]
    fn exposing_hidden_incoming_events_clears_the_chat_badge() {
        std::thread::spawn(|| {
            use line::backend::FakeBackend;
            use line_desktop_ui::ReadingGeometry;
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
                server_unread: 1,
                last_message_id: Some(ServerId::new("951").unwrap()),
                ..Default::default()
            });
            fake.set_page(chat.clone(), vec![hidden]);
            let worker = crate::tests::test_worker(fake);
            let mut state = AppState::default();
            worker.sender.send(state.select(chat.clone()));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while state.connection != Connection::Online
                || !state.reaches_end
                || state.messages.is_empty()
                || state.chats.get(&chat).is_none_or(|c| c.unread_count == 0)
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
            presenter.present(&ui, &state, ChatCategory::All);
            assert!(
                ui.global::<ChatsState>()
                    .get_chats()
                    .iter()
                    .any(|row| row.id == "alice" && row.unread == 1)
            );
            let intents = exposure(
                &mut Reading::default(),
                &state,
                ReadingGeometry {
                    active: true,
                    height: 400.,
                    at_bottom: true,
                    ..Default::default()
                },
            );
            assert!(
                matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.as_str() == "951")
            );
            for intent in intents {
                worker.sender.send(intent);
            }
            while state.chats[&chat].unread_count != 0 {
                state.apply(
                    worker
                        .deltas
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .unwrap(),
                );
            }
            presenter.present(&ui, &state, ChatCategory::All);
            let row = ui
                .global::<ChatsState>()
                .get_chats()
                .iter()
                .find(|row| row.id == "alice")
                .unwrap();
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
                unread_count: 2,
                ..crate::tests::chat_item(ChatInput {
                    id: chat.clone(),
                    server_unread: 2,
                    ..Default::default()
                })
            },
        );
        for (key, id) in [(Key(1), "9"), (Key(2), "10")] {
            state.messages.insert(
                key,
                StoredMessage {
                    key,
                    chat_id: chat.clone(),
                    server_id: Some(ServerId::new(id).unwrap()),
                    sort: Cursor {
                        sort_ts: key.0,
                        sort_id: id.into(),
                    },
                    sender_id: "sender".into(),
                    body: Content::default(),
                    created_at_local: 0,
                    ..crate::tests::message("")
                },
            );
        }
        let mut reading = Reading {
            geometry: [(Key(1), (20., 30.)), (Key(2), (70., 30.))].into(),
            ..Default::default()
        };
        let mut view = ReadingGeometry {
            height: 100.,
            pill_height: 10.,
            ..Default::default()
        };
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
        view.active = true;
        let intents = exposure(&mut reading, &state, view.clone());
        assert!(matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.as_str() == "9"));
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
        view.pill_height = 0.;
        let mut row = state.messages[&Key(2)].clone();
        row.outgoing = true;
        state.messages.insert(row.key, row.clone());
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
        row.outgoing = false;
        state.messages.insert(row.key, row.clone());
        row.delivery = Delivery::Pending;
        state.messages.insert(row.key, row.clone());
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
        row.delivery = Delivery::Sent;
        state.messages.insert(row.key, row.clone());
        view.modal = true;
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
        view.modal = false;
        let intents = exposure(&mut reading, &state, view.clone());
        assert!(matches!(intents.as_slice(), [Intent::MarkRead { id, .. }] if id.as_str() == "10"));
        assert!(exposure(&mut reading, &state, view.clone()).is_empty());
    }
}
