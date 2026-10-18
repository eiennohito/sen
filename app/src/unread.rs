//! Read exposure is driven by geometry and window/layer events, never a timer.
use crate::*;
use std::collections::{BTreeSet, HashSet};
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SavedUnread {
    pub count: Option<u64>,
    pub last_read: String,
}
#[derive(Default)]
pub(crate) struct Entry {
    pub count: Option<u64>,
    pub stale: bool,
    pub last_read: String,
    counted_through: String,
    known: BTreeSet<(usize, String)>,
    pub divider: Option<String>,
    pub jump: bool,
    pub(crate) pending: bool,
    pub(crate) queued: bool,
    pub search_remaining: Option<u64>,
    search_started: bool,
    pub boundary_unknown: bool,
    ranks: std::collections::BTreeMap<(usize, String), u64>,
    scanned: u64,
}
#[derive(Default)]
pub(crate) struct Unread {
    pub chats: HashMap<String, Entry>,
    pub exposed: Vec<String>,
}
fn key(id: &str) -> (usize, String) {
    (id.len(), id.into())
}
impl Unread {
    pub fn restore(&mut self, saved: &HashMap<String, SavedUnread>) {
        self.chats = saved
            .iter()
            .map(|(chat, s)| {
                (
                    chat.clone(),
                    Entry {
                        count: s.count,
                        last_read: s.last_read.clone(),
                        stale: true,
                        ..Default::default()
                    },
                )
            })
            .collect();
    }
    pub fn saved(&self) -> HashMap<String, SavedUnread> {
        self.chats
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    SavedUnread {
                        count: e.count,
                        last_read: e.last_read.clone(),
                    },
                )
            })
            .collect()
    }
    pub fn server(&mut self, chat: &str, count: Option<u64>, last: Option<&str>) {
        self.server_through(chat, count, last, None);
    }
    pub fn server_through(
        &mut self,
        chat: &str,
        count: Option<u64>,
        last: Option<&str>,
        through: Option<&str>,
    ) {
        let e = self.chats.entry(chat.into()).or_default();
        e.stale = false;
        if let Some(id) = through {
            e.counted_through = id.into();
        }
        if count == Some(0) && !e.pending {
            e.known.clear();
        }
        if let Some(last) = last
            && key(last) > key(&e.last_read)
        {
            e.last_read = last.into();
            e.known.retain(|(_, id)| key(id) > key(last));
        }
        if !e.pending {
            e.count = count.map(|n| n.max(e.known.len() as u64));
        }
    }
    pub fn enter(&mut self, chat: &str) {
        self.exposed.clear();
        let e = self.chats.entry(chat.into()).or_default();
        e.divider = None;
        e.jump = e.count.is_some_and(|n| n > 0);
        e.search_remaining = None;
        e.search_started = false;
        e.boundary_unknown = false;
        e.ranks.clear();
        e.scanned = 0;
    }
    pub fn page(&mut self, chat: &str, rows: &[MessageRow]) {
        let e = self.chats.entry(chat.into()).or_default();
        let incoming: Vec<_> = rows
            .iter()
            .filter(|m| !m.outgoing && !m.skip_badge)
            .collect();
        if e.known.is_empty()
            && let Some(count) = e.count
        {
            for row in incoming.iter().rev().take(count.min(200) as usize) {
                if key(&row.id) > key(&e.last_read) {
                    e.known.insert(key(&row.id));
                }
            }
        }
        if e.jump && !e.search_started && (!incoming.is_empty() || e.count == Some(0)) {
            e.search_started = true;
            for (rank, row) in incoming
                .iter()
                .rev()
                .take(e.count.unwrap_or(0).min(incoming.len() as u64) as usize)
                .enumerate()
            {
                e.ranks.insert(key(&row.id), rank as u64);
            }
            while e.ranks.len() > 400 {
                e.ranks.pop_last();
            }
            e.scanned = incoming.len() as u64;
            if let Some(count) = e.count
                && count > incoming.len() as u64
            {
                e.search_remaining = Some(count - incoming.len() as u64);
                e.divider = None;
            }
        }
        if e.divider.is_none()
            && e.search_remaining.is_none()
            && let Some(count) = e
                .count
                .filter(|count| *count > 0 && *count <= incoming.len() as u64)
        {
            e.divider = incoming
                .iter()
                .rev()
                .nth((count - 1) as usize)
                .map(|row| row.id.to_string());
        }
    }
    pub fn incoming(&mut self, chat: &str, ids: impl Iterator<Item = String>) {
        let e = self.chats.entry(chat.into()).or_default();
        e.stale = false;
        for id in ids {
            if key(&id) > key(&e.last_read)
                && e.known.insert(key(&id))
                && key(&id) > key(&e.counted_through)
            {
                e.count = e.count.map(|n| n.saturating_add(1));
                e.scanned = e.scanned.saturating_add(1);
                for rank in e.ranks.values_mut() {
                    *rank = rank.saturating_add(1);
                }
                e.ranks.insert(key(&id), 0);
            }
        }
        while e.ranks.len() > 400 {
            e.ranks.pop_first();
        }
        while e.known.len() > 200 {
            e.known.pop_first();
        }
    }
    pub fn expose(&mut self, chat: &str, id: &str) -> bool {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let e = self.chats.entry(chat.into()).or_default();
        if e.stale || key(id) <= key(&e.last_read) || (e.count == Some(0) && e.known.is_empty()) {
            return false;
        }
        let covers_known = e.known.first().is_some_and(|first| key(id) >= *first);
        e.last_read = id.into();
        e.known.retain(|item| *item > key(id));
        e.count = e
            .ranks
            .get(&key(id))
            .copied()
            .or_else(|| covers_known.then_some(e.known.len() as u64));
        e.pending = true;
        e.queued = false;
        true
    }
    pub fn elsewhere(&mut self, chat: &str, id: &str) {
        let e = self.chats.entry(chat.into()).or_default();
        if key(id) > key(&e.last_read) {
            e.last_read = id.into();
        }
        e.known.retain(|item| *item > key(id));
        e.count = Some(e.known.len() as u64);
        e.stale = false;
        e.pending = false;
        e.search_remaining = None;
        e.jump = false;
        e.boundary_unknown = false;
    }
    pub fn older(&mut self, chat: &str, rows: &[MessageRow], beginning: bool) -> bool {
        let Some(e) = self.chats.get_mut(chat) else {
            return false;
        };
        let Some(remaining) = e.search_remaining else {
            return false;
        };
        let incoming: Vec<_> = rows
            .iter()
            .filter(|m| !m.outgoing && !m.skip_badge)
            .collect();
        for (offset, row) in incoming
            .iter()
            .rev()
            .take(remaining.min(incoming.len() as u64) as usize)
            .enumerate()
        {
            e.ranks
                .insert(key(&row.id), e.scanned.saturating_add(offset as u64));
        }
        e.scanned = e.scanned.saturating_add(incoming.len() as u64);
        while e.ranks.len() > 400 {
            e.ranks.pop_last();
        }
        if remaining <= incoming.len() as u64 {
            e.divider = incoming
                .iter()
                .rev()
                .nth(remaining.saturating_sub(1) as usize)
                .map(|m| m.id.to_string());
            e.search_remaining = None;
            e.jump = true;
            false
        } else if beginning {
            e.search_remaining = None;
            e.boundary_unknown = true;
            e.jump = false;
            e.divider = None;
            false
        } else {
            e.search_remaining = Some(remaining - incoming.len() as u64);
            true
        }
    }
    pub fn badge(&self) -> u64 {
        self.chats
            .values()
            .filter(|e| e.count.is_some_and(|n| n > 0) || !e.known.is_empty())
            .count() as u64
    }
}
#[derive(Default)]
struct Eligibility {
    active: bool,
    visible: bool,
    modal: bool,
    loading: bool,
    has_exposed: bool,
}
impl Eligibility {
    fn allowed(&self) -> bool {
        self.active && self.visible && !self.modal && !self.loading && self.has_exposed
    }
}
thread_local! {static READ_GATE:RefCell<Option<std::sync::Arc<account::ReadGate>>>=const{RefCell::new(None)};}
pub(crate) fn set_gate(gate: std::sync::Arc<account::ReadGate>) {
    READ_GATE.with(|slot| *slot.borrow_mut() = Some(gate));
}
pub(crate) fn revoke_now() {
    READ_GATE.with(|slot| {
        if let Some(gate) = slot.borrow().as_ref() {
            gate.revoke();
        }
    });
}
pub(crate) fn install(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<State>().on_read_state_changed(move || {
        if let Some(ui) = weak.upgrade() {
            let state = ui.global::<State>();
            if !state.get_window_active()
                || state.get_modal_open()
                || ui.get_loading_history()
                || !ui.get_has_chat()
                || (state.get_narrow() && !ui.get_show_conversation())
            {
                revoke_now();
            }
        }
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                UI_STATE.with(|slot| {
                    if let Some(view) = slot.borrow().as_ref() {
                        let mut view = view.borrow_mut();
                        view.history_exposure(&ui);
                    }
                });
            }
        });
    });
}
impl View {
    pub(crate) fn unread_ui(&mut self, ui: &AppWindow) {
        for (index, id) in self.ids.iter().enumerate() {
            if let Some(e) = self.unread.chats.get(id) {
                self.rows[index].unread = e.count.unwrap_or(0).min(i32::MAX as u64) as i32;
                self.rows[index].unread_unknown = e.stale || e.count.is_none();
            }
        }
        let count = self.unread.badge();
        let state = ui.global::<State>();
        state.set_unread_locating(self.unread_discovering(&self.current));
        state.set_unread_boundary_unknown(
            self.unread
                .chats
                .get(&self.current)
                .is_some_and(|e| e.boundary_unknown),
        );
        state.set_unread_conversations(count.min(i32::MAX as u64) as i32);
        state.set_unread_stale(
            self.unread
                .chats
                .values()
                .any(|e| e.stale && e.count.is_some_and(|n| n > 0)),
        );
        let saved = self.unread.saved();
        if saved != self.saved.unread {
            self.saved.unread = saved;
            if let Some(storage) = &self.storage {
                storage.save(self.saved.clone());
            }
        }
        if let Some(attention) = &self.attention {
            attention.set_unread_count(if state.get_unread_stale() { 0 } else { count });
            let _ = attention.available();
        }
        // Preserve the pressed row/model until the deferred visual update drains.
        if !ui.get_list_pressed() {
            self.filter(ui);
        }
    }
    pub(crate) fn unread_page(&mut self, ui: &AppWindow) {
        self.unread.page(&self.current, &self.messages);
        if let Some(e) = self.unread.chats.get(&self.current) {
            ui.global::<State>()
                .set_unread_divider(e.divider.clone().unwrap_or_default().into());
        }
        let jump = self
            .unread
            .chats
            .get(&self.current)
            .filter(|e| e.jump)
            .and_then(|e| e.divider.clone());
        if let Some(id) = jump
            && self.scroll_to_message(
                ui,
                &id,
                ui.global::<State>().get_history_viewport_height() / 3.,
            )
        {
            self.unread.chats.get_mut(&self.current).unwrap().jump = false;
        }
        let discovering = self.unread_discovering(&self.current);
        ui.global::<State>().set_unread_locating(discovering);
        ui.global::<State>().set_unread_boundary_unknown(
            self.unread
                .chats
                .get(&self.current)
                .is_some_and(|e| e.boundary_unknown),
        );
        self.unread_ui(ui);
        if discovering
            && !ui.get_loading_history()
            && self
                .paging
                .borrow()
                .get(&self.current)
                .is_none_or(|window| window.failed.is_none())
        {
            self.request_older(ui, true);
        }
    }
    pub(crate) fn unread_discovering(&self, chat: &str) -> bool {
        self.unread
            .chats
            .get(chat)
            .is_some_and(|e| e.search_remaining.is_some())
    }
    pub(crate) fn unread_older(
        &mut self,
        ui: &AppWindow,
        chat: &str,
        rows: &[MessageRow],
        beginning: bool,
    ) -> bool {
        let more = self.unread.older(chat, rows, beginning);
        if chat == self.current {
            let e = &self.unread.chats[chat];
            ui.global::<State>()
                .set_unread_divider(e.divider.clone().unwrap_or_default().into());
            ui.global::<State>().set_unread_locating(more);
            ui.global::<State>()
                .set_unread_boundary_unknown(e.boundary_unknown);
        }
        more
    }
    pub(crate) fn read_exposure(&mut self, ui: &AppWindow, exposed: Vec<String>) {
        self.unread.exposed = exposed;
        self.retain_previews(ui);
        let state = ui.global::<State>();
        let eligibility = Eligibility {
            active: state.get_window_active(),
            visible: ui.get_has_chat() && (!state.get_narrow() || ui.get_show_conversation()),
            modal: state.get_modal_open() || ui.get_migration_needed(),
            loading: ui.get_loading_history()
                || !ui.get_history_error().is_empty()
                || self.unread_discovering(&self.current),
            has_exposed: !self.unread.exposed.is_empty(),
        };
        if let Some(priority) = &self.asset_priority {
            let exposed: HashSet<_> = self.unread.exposed.iter().map(String::as_str).collect();
            let mut keys = self
                .messages
                .iter()
                .filter(|m| eligibility.visible && exposed.contains(m.id.as_str()))
                .flat_map(|m| {
                    [m.sender_key.to_string(), m.image_key.to_string()]
                        .into_iter()
                        .chain(m.rich_runs.iter().map(|run| run.image_key.to_string()))
                        .chain(
                            m.reactions
                                .iter()
                                .map(|reaction| reaction.image_key.to_string()),
                        )
                })
                .collect::<Vec<_>>();
            keys.extend(self.preview_priority_keys(ui));
            priority.set(
                if eligibility.visible {
                    self.current.clone()
                } else {
                    String::new()
                },
                keys,
            );
        }
        let visible = self.preview_priority_keys(ui);
        self.preview_requested.retain(|key| visible.contains(key));
        let keys: Vec<_> = self
            .preview_keys(ui)
            .into_iter()
            .filter(|key| !self.preview_requested.contains(key))
            .collect();
        if !self.demo_preview_loading
            && !keys.is_empty()
            && self.commands.as_ref().is_some_and(|tx| {
                tx.try_send(account::Command::PreviewAssets { keys: keys.clone() })
                    .is_ok()
            })
        {
            self.preview_requested.extend(keys);
        }
        let allowed = eligibility.allowed();
        if let Some(gate) = &self.read_gate {
            if allowed {
                gate.allow();
            } else if self.read_activity != Some(false) {
                gate.revoke();
                for e in self.unread.chats.values_mut() {
                    e.queued = false;
                }
            }
        }
        if self.read_activity != Some(allowed) {
            self.read_activity = Some(allowed);
            if !self.demo {
                self.request(ui, account::Command::ReadActivity { allowed });
            }
        }
        if !allowed {
            return;
        }
        let exposed: HashSet<_> = self.unread.exposed.iter().map(String::as_str).collect();
        let highest = self
            .messages
            .iter()
            .filter(|m| {
                exposed.contains(m.id.as_str())
                    && m.id.bytes().all(|b| b.is_ascii_digit())
                    && !m.outgoing
                    && !m.skip_badge
            })
            .max_by_key(|m| key(&m.id))
            .map(|m| m.id.to_string());
        let advanced = highest
            .as_deref()
            .is_some_and(|id| self.unread.expose(&self.current, id));
        if advanced {
            if let Some(attention) = &self.attention {
                attention.withdraw(&self.current);
            }
            self.unread_ui(ui);
            self.persist(ui);
        }
        let marker = self
            .unread
            .chats
            .get(&self.current)
            .filter(|e| e.pending && !e.queued)
            .map(|e| e.last_read.clone());
        if let Some(up_to_message) = marker {
            let queued = self.demo
                || self.request(
                    ui,
                    account::Command::MarkRead {
                        chat: self.current.clone(),
                        up_to_message,
                        eligibility_epoch: self.read_gate.as_ref().map_or(0, |gate| gate.epoch()),
                    },
                );
            if queued {
                self.unread.chats.get_mut(&self.current).unwrap().queued = true;
            }
        }
    }
}
impl View {
    pub(crate) fn apply_read_ranges(
        &mut self,
        ui: &AppWindow,
        ranges: &[line_wasm_poc::client::desktop::ReadRange],
    ) {
        let apply = |chat: &str, rows: &mut Vec<MessageRow>| {
            for row in rows
                .iter_mut()
                .filter(|row| row.outgoing && row.id.bytes().all(|b| b.is_ascii_digit()))
            {
                let readers: HashSet<_> = ranges
                    .iter()
                    .filter(|range| {
                        range.chat == chat
                            && !range.open_end
                            && range.reader != row.sender_id.as_str()
                            && key(&range.start) < key(&row.id)
                            && key(&range.end) >= key(&row.id)
                    })
                    .map(|range| range.reader.as_str())
                    .collect();
                if !readers.is_empty() {
                    row.delivery = "Read".into();
                    row.delivery_detail = format!("Read by {}", readers.len()).into();
                }
            }
        };
        apply(&self.current, &mut self.messages);
        for (chat, history) in &mut self.history {
            apply(chat, &mut history.messages);
        }
        self.render(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eligibility_requires_every_condition() {
        let good = Eligibility {
            active: true,
            visible: true,
            has_exposed: true,
            ..Default::default()
        };
        assert!(good.allowed());
        for bad in [
            Eligibility {
                active: false,
                ..good
            },
            Eligibility {
                visible: false,
                ..good
            },
            Eligibility {
                modal: true,
                ..good
            },
            Eligibility {
                loading: true,
                ..good
            },
            Eligibility {
                has_exposed: false,
                ..good
            },
        ] {
            assert!(!bad.allowed());
        }
    }
    #[test]
    fn stale_restart_does_not_mark_and_live_ids_reconcile() {
        let mut m = Unread::default();
        m.restore(&HashMap::from([(
            "chat".into(),
            SavedUnread {
                count: Some(3),
                last_read: "5".into(),
            },
        )]));
        assert!(!m.expose("chat", "9"));
        m.server("chat", Some(3), Some("5"));
        m.incoming("chat", ["6", "7", "8"].into_iter().map(str::to_owned));
        assert!(m.expose("chat", "7"));
        assert_eq!(m.chats["chat"].count, Some(1));
        assert_eq!(m.badge(), 1);
        m.elsewhere("chat", "8");
        assert_eq!(m.badge(), 0);
    }
}

#[cfg(test)]
mod controller_tests {
    use super::*;
    #[test]
    fn exposed_read_requires_active_loaded_visible_uncovered_conversation() {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        let mut view = View {
            current: "chat".into(),
            commands: Some(tx.into()),
            ..Default::default()
        };
        view.ids.push("chat".into());
        view.rows.push(ChatRow {
            title: "Synthetic".into(),
            ..Default::default()
        });
        view.messages = ["11", "12"]
            .into_iter()
            .map(|id| MessageRow {
                id: id.into(),
                body: "Synthetic".into(),
                ..Default::default()
            })
            .collect();
        view.unread.server("chat", Some(2), Some("10"));
        view.unread.page("chat", &view.messages);
        ui.set_has_chat(true);
        ui.global::<State>().set_narrow(false);
        ui.set_loading_history(true);
        view.read_exposure(&ui, vec!["11".into()]);
        ui.global::<State>().set_window_active(true);
        view.read_exposure(&ui, vec!["11".into()]);
        ui.set_loading_history(false);
        ui.set_settings_open(true);
        view.read_exposure(&ui, vec!["11".into()]);
        ui.set_settings_open(false);
        ui.global::<State>().set_narrow(true);
        ui.set_show_conversation(false);
        view.read_exposure(&ui, vec!["11".into()]);
        assert!(
            rx.try_iter()
                .all(|c| !matches!(c, account::Command::MarkRead { .. }))
        );
        ui.set_show_conversation(true);
        view.read_exposure(&ui, Vec::new());
        assert_eq!(view.unread.chats["chat"].count, Some(2));
        view.read_exposure(&ui, vec!["11".into()]);
        assert!(rx.try_iter().any(
            |c| matches!(c,account::Command::MarkRead{ref up_to_message,..} if up_to_message=="11")
        ));
        assert_eq!(view.unread.chats["chat"].count, Some(1));
        assert_eq!(view.unread.chats["chat"].divider.as_deref(), Some("11"));
        ui.global::<State>().set_window_active(false);
        view.read_exposure(&ui, vec!["12".into()]);
        assert_eq!(view.unread.chats["chat"].count, Some(1));
        assert!(
            rx.try_iter()
                .all(|c| !matches!(c, account::Command::MarkRead { .. }))
        );
    }
    #[test]
    fn server_snapshot_includes_older_live_operation_without_double_count() {
        let mut state = Unread::default();
        state.server_through("chat", Some(3), Some("5"), Some("8"));
        state.incoming("chat", ["6", "7", "8"].into_iter().map(str::to_owned));
        assert_eq!(state.chats["chat"].count, Some(3));
        state.incoming("chat", ["9"].into_iter().map(str::to_owned));
        assert_eq!(state.chats["chat"].count, Some(4));
        state.incoming("chat", ["9"].into_iter().map(str::to_owned));
        assert_eq!(state.chats["chat"].count, Some(4));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn page(start: i64, end: i64) -> Vec<MessageRow> {
        (start..=end)
            .map(|n| MessageRow {
                id: n.to_string().into(),
                ..Default::default()
            })
            .collect()
    }
    #[test]
    fn first_unread_is_discovered_across_bounded_older_pages() {
        let mut state = Unread::default();
        state.server_through("chat", Some(120), Some("100"), Some("220"));
        state.enter("chat");
        state.page("chat", &page(171, 220));
        assert_eq!(state.chats["chat"].search_remaining, Some(70));
        assert!(state.chats["chat"].divider.is_none());
        assert!(state.older("chat", &page(122, 170), false));
        assert_eq!(state.chats["chat"].search_remaining, Some(21));
        assert!(!state.older("chat", &page(73, 121), false));
        assert_eq!(state.chats["chat"].divider.as_deref(), Some("101"));
        state.incoming("chat", ["221".to_owned()].into_iter());
        assert!(state.expose("chat", "101"));
        assert_eq!(state.chats["chat"].count, Some(120));
        assert!(state.chats["chat"].ranks.len() <= 400);
        assert_eq!(state.chats["chat"].divider.as_deref(), Some("101"));
    }
    #[test]
    fn inconsistent_count_at_beginning_is_unknown_without_false_divider() {
        let mut state = Unread::default();
        state.server("chat", Some(500), None);
        state.enter("chat");
        state.page("chat", &page(51, 100));
        assert!(!state.older("chat", &page(1, 50), true));
        assert!(state.chats["chat"].boundary_unknown);
        assert!(state.chats["chat"].divider.is_none());
    }
}

#[cfg(test)]
mod cached_boundary_tests {
    use super::*;
    #[test]
    fn three_hundred_cached_unread_rows_keep_true_boundary_beyond_known_cap() {
        let mut state = Unread::default();
        state.server("chat", Some(300), Some("100"));
        state.enter("chat");
        let rows: Vec<_> = (101..=400)
            .map(|n| MessageRow {
                id: n.to_string().into(),
                ..Default::default()
            })
            .collect();
        state.page("chat", &rows);
        assert_eq!(state.chats["chat"].divider.as_deref(), Some("101"));
        assert!(state.chats["chat"].search_remaining.is_none());
        assert!(state.expose("chat", "101"));
        assert_eq!(state.chats["chat"].count, Some(299));
    }
}

#[cfg(test)]
mod invalid_identity_tests {
    use super::*;
    #[test]
    fn synthetic_or_unavailable_ids_never_become_server_read_markers() {
        let mut state = Unread::default();
        state.server("chat", Some(1), None);
        for id in ["", "demo-25", "unavailable-fingerprint"] {
            assert!(!state.expose("chat", id));
        }
        assert_eq!(state.chats["chat"].last_read, "");
    }
}

#[cfg(test)]
mod badge_metadata_tests {
    use super::*;
    #[test]
    fn authoritative_skip_flag_counts_system_and_skips_ordinary_rows() {
        let mut state = Unread::default();
        state.server("chat", Some(2), Some("100"));
        state.enter("chat");
        let rows = vec![
            MessageRow {
                id: "101".into(),
                kind: MessageKind::System,
                ..Default::default()
            },
            MessageRow {
                id: "102".into(),
                skip_badge: true,
                ..Default::default()
            },
            MessageRow {
                id: "103".into(),
                ..Default::default()
            },
        ];
        state.page("chat", &rows);
        assert_eq!(state.chats["chat"].divider.as_deref(), Some("101"));
        assert!(state.expose("chat", "101"));
        assert_eq!(state.chats["chat"].count, Some(1));
        assert!(!state.chats["chat"].known.contains(&key("102")));
    }
}
