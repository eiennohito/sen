//! UI history integration. Network remains on the account worker.
use crate::*;
use history_window::{Anchor, Cursor, Direction, Page};

fn order(row: &MessageRow) -> String {
    format!("{}-{:04}-{}", row.sort_key, row.id.len(), row.id)
}
fn ids(rows: &[MessageRow]) -> Vec<String> {
    rows.iter().map(|row| row.id.to_string()).collect()
}
pub(crate) fn bytes(row: &MessageRow) -> usize {
    fn pixels(image: &slint::Image) -> usize {
        let size = image.size();
        (size.width as usize)
            .saturating_mul(size.height as usize)
            .saturating_mul(4)
    }
    [
        &row.body,
        &row.sender_name,
        &row.sender_id,
        &row.sender_initial,
        &row.sender_key,
        &row.id,
        &row.detail,
        &row.reply_preview,
        &row.reply_id,
        &row.accessible_label,
        &row.image_key,
        &row.image_failure,
        &row.image_error,
        &row.file_failure,
        &row.file_error,
        &row.sort_key,
        &row.time,
        &row.clock,
        &row.day_label,
        &row.delivered_time,
        &row.delivery,
        &row.delivery_detail,
    ]
    .iter()
    .map(|text| text.len())
    .sum::<usize>()
        + std::mem::size_of::<MessageRow>()
        + 32 * 22
        + pixels(&row.picture)
        + pixels(&row.sender_avatar)
        + row
            .reactions
            .iter()
            .map(|reaction| {
                reaction.label.len() + reaction.image_key.len() + pixels(&reaction.picture) + 128
            })
            .sum::<usize>()
        + row
            .rich_runs
            .iter()
            .map(|run| 2 * run.text.len() + run.image_key.len() + pixels(&run.picture) + 192)
            .sum::<usize>()
        + row
            .links
            .iter()
            .map(|link| link.label.len() + 64)
            .sum::<usize>()
        + row
            .rich_runs
            .iter()
            .map(|run| run.url)
            .chain(row.links.iter().map(|link| link.url))
            .filter(|url| !url.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .iter()
            .map(|url| url.len())
            .sum::<usize>()
}
fn release_oversized_pixels(row: &mut MessageRow) {
    if bytes(row) <= history_window::RETAINED_BYTES {
        return;
    }
    row.picture = Default::default();
    row.has_picture = false;
    row.image_failed = !row.image_key.is_empty();
    row.sender_avatar = Default::default();
    row.has_sender_avatar = false;
    row.reactions = ModelRc::new(VecModel::from(
        row.reactions
            .iter()
            .map(|mut reaction| {
                reaction.picture = Default::default();
                reaction.has_picture = false;
                reaction
            })
            .collect::<Vec<_>>(),
    ));
    row.rich_runs = ModelRc::new(VecModel::from(
        row.rich_runs
            .iter()
            .map(|mut run| {
                run.picture = Default::default();
                run.has_picture = false;
                run
            })
            .collect::<Vec<_>>(),
    ));
}

/// Geometry callbacks run after the layout pass and never while View is borrowed
/// by an update. One queued callback per changed row replaces per-frame timers.
fn dispatch(
    ui: slint::Weak<AppWindow>,
    action: impl FnOnce(&mut View, &AppWindow) + Send + 'static,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui.upgrade() {
            UI_STATE.with(|slot| {
                if let Some(view) = slot.borrow().as_ref() {
                    action(&mut view.borrow_mut(), &ui);
                }
            });
        }
    });
}
pub(crate) fn install(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<State>().on_row_geometry(move |id, y, height| {
        dispatch(weak.clone(), move |view, ui| {
            view.history_geometry(ui, &id, y, height)
        });
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_history_scrolled(move || {
        dispatch(weak.clone(), |view, ui| view.history_scrolled(ui));
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_latest_request(move || {
        dispatch(weak.clone(), |view, ui| view.history_latest(ui));
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_older_retry(move || {
        dispatch(weak.clone(), |view, ui| view.request_older(ui, true));
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_jump_message(move |id| {
        dispatch(weak.clone(), move |view, ui| {
            view.jump_to_message(ui, &id);
        });
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_return_to_message(move || {
        dispatch(weak.clone(), |view, ui| view.return_to_anchor(ui));
    });
}
impl View {
    fn canonical_history_id(&self, chat: &str, id: &str) -> String {
        self.aliases
            .iter()
            .find(|((conversation, _), (local, _))| conversation == chat && local == id)
            .map(|((_, server), _)| server.clone())
            .unwrap_or_else(|| id.into())
    }
    fn displayed_history_id(&self, chat: &str, id: &str) -> String {
        self.aliases
            .get(&(chat.into(), id.into()))
            .map(|(local, _)| local.clone())
            .unwrap_or_else(|| id.into())
    }

    pub(crate) fn keep_recent(&mut self, chat: &str, current: bool) {
        let source = if current {
            self.messages.as_slice()
        } else {
            self.history
                .get(chat)
                .map(|h| h.messages.as_slice())
                .unwrap_or_default()
        };
        let old = self.recent.entry(chat.into()).or_default();
        for row in source {
            if let Some(existing) = old.iter_mut().find(|item| item.id == row.id) {
                *existing = row.clone();
            } else {
                old.push(row.clone());
            }
        }
        old.sort_by_key(order);
        if old.len() > 50 {
            old.drain(..old.len() - 50);
        }
        for row in old.iter_mut() {
            release_oversized_pixels(row);
        }
        history_window::bound(old, None, |row| row.id.as_str(), bytes);
    }
    pub(crate) fn bound_history(&mut self, ui: &AppWindow, chat: &str) {
        let anchor = self
            .paging
            .borrow()
            .get(chat)
            .and_then(|window| window.anchor.as_ref().map(|a| a.id.clone()));
        let anchor = anchor.map(|id| self.canonical_history_id(chat, &id));
        let current = chat == self.current;
        let rows = if current {
            Some(&mut self.messages)
        } else {
            self.history.get_mut(chat).map(|h| &mut h.messages)
        };
        if let Some(rows) = rows {
            for row in rows.iter_mut() {
                release_oversized_pixels(row);
            }
            let first = rows.first().map(|row| row.id.clone());
            let newer =
                history_window::bound(rows, anchor.as_deref(), |row| row.id.as_str(), bytes);
            let mut windows = self.paging.borrow_mut();
            let window = windows.entry(chat.into()).or_default();
            window.newer_evicted |= newer;
            if rows.first().map(|row| row.id.clone()) != first {
                window.reached_beginning = false;
            }
            let mut displayed: Vec<String> = rows
                .iter()
                .map(|row| {
                    self.aliases
                        .get(&(chat.into(), row.id.to_string()))
                        .map(|(local, _)| local.clone())
                        .unwrap_or_else(|| row.id.to_string())
                })
                .collect();
            displayed.extend(
                self.saved
                    .outbox
                    .iter()
                    .filter(|item| item.chat == chat)
                    .map(|item| item.local_id.clone()),
            );
            window.reconcile_anchor(&displayed);
            if current {
                ui.global::<State>().set_history_budget_warning(
                    rows.iter()
                        .any(|row| bytes(row) > history_window::RETAINED_BYTES),
                );
            }
            if current && newer {
                ui.global::<State>().set_history_needs_refresh(true);
            }
        }
        while self.history.len()
            > if self.history.contains_key(&self.current) {
                8
            } else {
                7
            }
        {
            let victim = self
                .history
                .keys()
                .find(|id| **id != self.current && id.as_str() != chat)
                .cloned()
                .or_else(|| self.history.keys().find(|id| **id != self.current).cloned());
            let Some(victim) = victim else {
                break;
            };
            self.history.remove(&victim);
            self.recent.remove(&victim);
            self.paging.borrow_mut().remove(&victim);
        }
        self.recent
            .retain(|id, _| id == &self.current || self.history.contains_key(id));
        self.paging
            .borrow_mut()
            .retain(|id, _| id == &self.current || self.history.contains_key(id));
        let retained: std::collections::HashSet<String> = self
            .messages
            .iter()
            .chain(
                self.history
                    .values()
                    .flat_map(|history| history.messages.iter()),
            )
            .chain(self.recent.values().flat_map(|rows| rows.iter()))
            .map(|row| row.id.to_string())
            .collect();
        self.aliases.retain(|(_, server), _| {
            retained.contains(server)
                || self
                    .saved
                    .outbox
                    .iter()
                    .any(|item| item.server_id == *server)
        });
    }
    pub(crate) fn bound_all_histories(&mut self, ui: &AppWindow) {
        let chats: Vec<String> = self
            .history
            .keys()
            .cloned()
            .chain(std::iter::once(self.current.clone()))
            .collect();
        for chat in chats {
            self.bound_history(ui, &chat);
        }
    }
    pub(crate) fn record_recent_page(&self, chat: &str) {
        let rows = if chat == self.current {
            self.messages.as_slice()
        } else {
            self.history
                .get(chat)
                .map(|h| h.messages.as_slice())
                .unwrap_or_default()
        };
        if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
            self.paging
                .borrow_mut()
                .entry(chat.into())
                .or_default()
                .add_page(Page {
                    first_order: order(first),
                    last_order: order(last),
                    cursor: None,
                });
        }
    }
    pub(crate) fn paging_status(&self, ui: &AppWindow) {
        let windows = self.paging.borrow();
        let window = windows.get(&self.current);
        let status = if window.is_some_and(|w| w.request.is_some()) {
            "Loading older…"
        } else if window.is_some_and(|w| w.failed.is_some()) {
            "Could not load older"
        } else if window.is_some_and(|w| w.reached_beginning) {
            "Beginning of conversation"
        } else {
            ""
        };
        ui.global::<State>().set_older_status(status.into());
        ui.global::<State>()
            .set_older_failed(window.is_some_and(|w| w.failed.is_some()));
        ui.global::<State>()
            .set_return_visible(window.is_some_and(|w| w.return_anchor.is_some()));
    }
    pub(crate) fn history_geometry(&mut self, ui: &AppWindow, id: &str, y: f32, height: f32) {
        if y < 16. || height <= 1. || ui.global::<State>().get_history_viewport_height() <= 0. {
            return;
        }
        // Queued geometry from an evicted delegate is not an exposed message.
        if !ui.get_messages().iter().any(|row| row.id == id) {
            return;
        }
        let canonical = self.canonical_history_id(&self.current, id);
        if !self.messages.iter().any(|row| row.id == canonical)
            && !self.saved.outbox.iter().any(|row| row.local_id == id)
        {
            return;
        }
        let offset = self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .measured(id, y, height, ui.get_following());
        if !ui.get_history_dragging()
            && let Some(offset) = offset
        {
            ui.set_history_y(offset);
        }
        self.unread_page(ui);
        self.history_exposure(ui);
    }
    pub(crate) fn history_exposure(&mut self, ui: &AppWindow) {
        let exposed = self
            .paging
            .borrow()
            .get(&self.current)
            .map(|window| {
                window.exposed(
                    ui.get_history_y(),
                    ui.global::<State>().get_history_viewport_height(),
                )
            })
            .unwrap_or_default();
        self.read_exposure(ui, exposed);
    }
    pub(crate) fn history_scrolled(&mut self, ui: &AppWindow) {
        let visible = self.with_outbox(&self.current, self.messages.clone());
        let row_ids = ids(&visible);
        let shift = {
            let mut windows = self.paging.borrow_mut();
            let window = windows.entry(self.current.clone()).or_default();
            window.capture(ui.get_history_y());
            window.slide(&row_ids, ui.get_following())
        };
        if shift {
            self.render(ui);
        }
        let viewport = ui.global::<State>().get_history_viewport_height();
        if !ui.get_history_dragging() && !ui.get_following() && -ui.get_history_y() <= viewport * 2.
        {
            let range = self
                .paging
                .borrow_mut()
                .entry(self.current.clone())
                .or_default()
                .range(&row_ids, false);
            if range.start == 0 {
                self.request_older(ui, false);
            }
        }
        let at_retained_end = self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .range(&row_ids, false)
            .end
            == row_ids.len();
        if at_retained_end
            && !ui.get_history_dragging()
            && !ui.get_following()
            && ui.global::<State>().get_history_extent() + ui.get_history_y() < viewport * 2.
        {
            self.request_newer(ui);
        }
        self.history_exposure(ui);
    }
    pub(crate) fn request_older(&mut self, ui: &AppWindow, retry: bool) {
        if self.current.is_empty() || (self.demo && !self.demo_paging) {
            return;
        }
        let failed = if retry {
            self.paging
                .borrow()
                .get(&self.current)
                .and_then(|w| w.failed.clone())
        } else {
            None
        };
        if !retry
            && self
                .paging
                .borrow()
                .get(&self.current)
                .is_some_and(|w| w.failed.is_some())
        {
            return;
        }
        let cursor = failed.map(|request| request.cursor).or_else(|| {
            self.messages.first().map(|row| Cursor {
                message: row.id.to_string(),
                delivered_time: row
                    .delivered_time
                    .parse()
                    .unwrap_or_else(|_| row.sort_key.parse().unwrap_or(0)),
            })
        });
        if let Some(cursor) = cursor {
            self.start_page(ui, cursor, Direction::Older);
        }
    }
    fn start_page(&mut self, ui: &AppWindow, cursor: Cursor, direction: Direction) {
        if !self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .begin(cursor.clone(), direction)
        {
            return;
        }
        self.paging_status(ui);
        if self.demo {
            let current = self.current.clone();
            let weak = ui.as_weak();
            let first = cursor
                .message
                .strip_prefix("paged-")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let fail = first == 900 && !self.demo_page_failure;
            if fail {
                self.demo_page_failure = true;
            }
            slint::Timer::single_shot(std::time::Duration::from_millis(80), move || {
                if let Some(ui) = weak.upgrade() {
                    UI_STATE.with(|slot| {
                        if let Some(view) = slot.borrow().as_ref() {
                            let mut view = view.borrow_mut();
                            if fail {
                                if let Some(window) = view.paging.borrow_mut().get_mut(&current) {
                                    window.failed(&cursor.message);
                                }
                                view.paging_status(&ui);
                            } else {
                                let start = first.saturating_sub(50);
                                view.older_arrived(
                                    &ui,
                                    &current,
                                    demo::paged_rows(start..first),
                                    start == 0,
                                );
                            }
                        }
                    });
                }
            });
        } else if !self.request(
            ui,
            account::Command::Older {
                chat: self.current.clone(),
                before_message: cursor.message.clone(),
                delivered_time: cursor.delivered_time,
            },
        ) {
            if let Some(window) = self.paging.borrow_mut().get_mut(&self.current) {
                window.failed(&cursor.message);
            }
            self.paging_status(ui);
        }
    }
    fn request_newer(&mut self, ui: &AppWindow) {
        let next = self.messages.last().and_then(|last| {
            self.paging
                .borrow()
                .get(&self.current)
                .filter(|w| w.newer_evicted && w.request.is_none())
                .and_then(|w| w.next_newer(&order(last)))
        });
        match next {
            Some(Some(cursor)) => self.start_page(ui, cursor, Direction::Newer),
            Some(None) => {
                let chat = self.current.clone();
                self.paging
                    .borrow_mut()
                    .entry(chat.clone())
                    .or_default()
                    .begin(
                        Cursor {
                            message: "recent-range".into(),
                            delivered_time: 0,
                        },
                        Direction::Newer,
                    );
                if self.demo {
                    self.older_arrived(ui, &chat, demo::paged_rows(950..1000), false);
                } else {
                    self.load(ui, false);
                }
            }
            None => {}
        }
    }
    pub(crate) fn older_arrived(
        &mut self,
        ui: &AppWindow,
        chat: &str,
        mut rows: Vec<MessageRow>,
        beginning: bool,
    ) {
        let request = self
            .paging
            .borrow_mut()
            .get_mut(chat)
            .and_then(|window| window.request.take());
        let Some(request) = request else {
            return;
        };
        let discovering = self.unread_discovering(chat);
        let first_new = rows.first().map(|row| row.id.to_string());
        if discovering {
            self.unread_older(ui, chat, &rows, beginning);
        }
        let old = if chat == self.current {
            &mut self.messages
        } else {
            let Some(history) = self.history.get_mut(chat) else {
                return;
            };
            &mut history.messages
        };
        if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
            self.paging
                .borrow_mut()
                .entry(chat.into())
                .or_default()
                .add_page(Page {
                    first_order: order(first),
                    last_order: order(last),
                    cursor: (request.cursor.message != "recent-range").then_some(request.cursor),
                });
        }
        for row in rows.drain(..) {
            if !old.iter().any(|previous| previous.id == row.id) {
                old.push(row);
            }
        }
        old.sort_by_key(order);
        if request.direction == Direction::Older {
            self.paging
                .borrow_mut()
                .entry(chat.into())
                .or_default()
                .reached_beginning = beginning;
        }
        if discovering
            && chat == self.current
            && let Some(first) = first_new
        {
            ui.set_following(false);
            self.paging
                .borrow_mut()
                .entry(chat.into())
                .or_default()
                .reveal(
                    &ids(&self.messages),
                    &first,
                    ui.global::<State>().get_history_viewport_height() / 3.,
                );
        }
        self.bound_history(ui, chat);
        if chat == self.current {
            let displayed = self.with_outbox(chat, self.messages.clone());
            self.paging
                .borrow_mut()
                .entry(chat.into())
                .or_default()
                .slide(&ids(&displayed), ui.get_following());
            self.paging_status(ui);
            self.render(ui);
            if discovering {
                self.unread_page(ui);
            }
        }
    }
    pub(crate) fn history_latest(&mut self, ui: &AppWindow) {
        self.deferred_counted.clear();
        ui.set_following(true);
        ui.set_new_count(0);
        ui.global::<State>().set_history_needs_refresh(false);
        let needs_fetch = self
            .paging
            .borrow()
            .get(&self.current)
            .is_some_and(|window| window.newer_evicted);
        self.paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .reset_latest();
        if needs_fetch {
            if let Some(recent) = self.recent.get(&self.current) {
                self.messages = recent.clone();
            }
            if self.demo_paging {
                self.messages = demo::paged_rows(950..1000);
            }
        }
        self.render(ui);
        self.paging_status(ui);
        if !self.demo {
            self.load(ui, needs_fetch);
        }
    }
    pub(crate) fn scroll_to_message(&mut self, ui: &AppWindow, id: &str, screen_y: f32) -> bool {
        let id = self.displayed_history_id(&self.current, id);
        let all = self.with_outbox(&self.current, self.messages.clone());
        if !self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .reveal(&ids(&all), &id, screen_y)
        {
            return false;
        }
        ui.set_following(false);
        self.render(ui);
        if let Some(offset) = self
            .paging
            .borrow()
            .get(&self.current)
            .and_then(|window| window.offset_for(&id, screen_y))
        {
            ui.set_history_y(offset);
        }
        true
    }
    pub(crate) fn jump_to_message(&mut self, ui: &AppWindow, id: &str) -> bool {
        let anchor = self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .capture(ui.get_history_y());
        let visible = self.scroll_to_message(
            ui,
            id,
            ui.global::<State>().get_history_viewport_height() / 3.,
        );
        if visible {
            self.paging
                .borrow_mut()
                .entry(self.current.clone())
                .or_default()
                .return_anchor = anchor;
            ui.global::<State>()
                .set_highlight_message(self.displayed_history_id(&self.current, id).into());
            ui.global::<State>().set_jump_error("".into());
            self.paging_status(ui);
        } else {
            ui.global::<State>()
                .set_jump_error("This message isn’t in the current history".into());
        }
        visible
    }
    pub(crate) fn return_to_anchor(&mut self, ui: &AppWindow) {
        let anchor = self
            .paging
            .borrow_mut()
            .entry(self.current.clone())
            .or_default()
            .return_anchor
            .take();
        if let Some(Anchor { id, screen_y }) = anchor {
            self.scroll_to_message(ui, &id, screen_y);
        }
        ui.global::<State>().set_highlight_message("".into());
        self.paging_status(ui);
    }
    pub(crate) fn demo_paged_history(&mut self, ui: &AppWindow) {
        self.demo_paging = true;
        self.demo_page_failure = false;
        self.messages = demo::paged_rows(950..1000);
        self.paging
            .borrow_mut()
            .insert(self.current.clone(), Default::default());
        self.keep_recent(&self.current.clone(), true);
        self.record_recent_page(&self.current);
        ui.global::<State>().set_paging_demo(true);
        ui.set_following(true);
        self.paging_status(ui);
        self.render(ui);
        ui.invoke_latest();
    }
}
