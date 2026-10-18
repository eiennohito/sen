use line_wasm_poc::{
    backend::{FakeBackend, LineBackend, ProfileAction},
    controller::{ActionQueue, AppState, DetailPage, Panel},
    engine::{Connection, Delta, Intent, Sender, StoreLocation, Worker},
    store::*,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
mod account;
mod attention;
mod attention_policy;
mod autostart;
mod background;
mod debug_launch;
mod demo;
mod demo_data;
mod desktop_integration;
mod event_loop;
#[cfg(feature = "ui-debug")]
mod latency;
mod links;
mod notifications;
mod present;
mod presenter;
mod preview;
mod rich_content;
mod snapshot;
mod tray;
mod viewer;
pub use line_desktop_ui::*;
use present::*;
fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.global::<State>().on_avatar_tint(|id| {
        let hash = id.bytes().fold(0u32, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(byte as u32)
        });
        let hues = [
            0xDCCEDC, 0xC6D9E8, 0xC9DDD0, 0xE7D4BD, 0xD2D2EA, 0xE8CCD0, 0xDADDBE, 0xBEDDDA,
        ];
        let rgb = hues[hash as usize % hues.len()];
        slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    });
    ui.global::<State>()
        .on_has_text(|text| !text.trim().is_empty());
    {
        use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};
        let saved = Rc::new(RefCell::new(Vec::new()));
        let record = saved.clone();
        let weak = ui.as_weak();
        ui.global::<State>().on_remember_detail_focus(move |reset| {
            if let Some(ui) = weak.upgrade() {
                if reset {
                    record.borrow_mut().clear();
                }
                record.borrow_mut().push(
                    WindowInner::from_pub(ui.window())
                        .focus_item
                        .borrow()
                        .clone(),
                );
            }
        });
        let weak = ui.as_weak();
        ui.global::<State>().on_restore_detail_focus(move |all| {
            if let Some(ui) = weak.upgrade() {
                let item = if all {
                    let first = saved.borrow().first().cloned();
                    saved.borrow_mut().clear();
                    first
                } else {
                    saved.borrow_mut().pop()
                };
                if let Some(item) = item.and_then(|i| i.upgrade()).filter(|i| i.is_visible()) {
                    WindowInner::from_pub(ui.window()).set_focus_item(
                        &item,
                        true,
                        FocusReason::PopupActivation,
                    );
                } else if ui.get_detail_open() {
                    ui.invoke_focus_profile_close();
                } else {
                    ui.invoke_focus_composer();
                }
            }
        });
    }
    {
        use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};
        let saved = Rc::new(RefCell::new(None));
        let record = saved.clone();
        let weak = ui.as_weak();
        ui.global::<State>().on_remember_demo_focus(move || {
            if let Some(ui) = weak.upgrade() {
                *record.borrow_mut() = Some(
                    WindowInner::from_pub(ui.window())
                        .focus_item
                        .borrow()
                        .clone(),
                );
            }
        });
        let weak = ui.as_weak();
        ui.global::<State>().on_restore_demo_focus(move || {
            if let Some(ui) = weak.upgrade()
                && let Some(item) = saved.borrow_mut().take().and_then(|weak| weak.upgrade())
            {
                WindowInner::from_pub(ui.window()).set_focus_item(
                    &item,
                    true,
                    FocusReason::PopupActivation,
                );
            }
        });
    }
    macro_rules! bridge {
        ($on:ident,$invoke:ident) => {{
            let weak = ui.as_weak();
            ui.global::<State>().$on(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.$invoke();
                }
            });
        }};
    }
    bridge!(on_open_members, invoke_open_members);
    bridge!(on_filter_members, invoke_filter_members);
    bridge!(on_back_to_members, invoke_back_to_members);
    let weak = ui.as_weak();
    ui.global::<State>().on_open_member(move |id| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_member(id);
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_cycle_end(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_cycle_end();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_after_browser(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_after_browser();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_after_conversation(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_after_conversation();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_before_layer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_before_layer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_after_layer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_after_layer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_settings(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_settings();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_friend_list(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_friend_list();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_quit(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_quit();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>()
        .on_restore_editor(move |anchor, caret| {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_restore_editor(anchor, caret);
            }
        });
    let weak = ui.as_weak();
    ui.global::<State>().on_retry_message(move |id| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_retry_message(id);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_resolve_recovery(move |action| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_resolve_recovery(action);
        }
    });

    let weak = ui.as_weak();
    ui.global::<State>()
        .on_focus_reading_before_editor(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_focus_reading_before_editor();
            }
        });
    let weak = ui.as_weak();
    ui.global::<State>().on_filter_category(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_filter_category();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_switch_chat(move |delta| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_switch_chat(delta);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_chat_list(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_chat_list();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_profile(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_profile_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_profile(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_profile();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_composer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_composer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_viewer(move |key| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_show_viewer(key);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_viewer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_viewer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_account_ready(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_account_ready();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_history(move |y, latest| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_show_history(y, latest);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_latest(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_latest();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_search(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_search();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_friend_search(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_friend_search();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_settings_opener(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_settings_opener();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_settings_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_settings_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_profile_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_profile_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_auth_start(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_auth_start();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_viewer_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_viewer_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_history(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_history();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_connection_first(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_connection_first();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_sign_in_opener(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_sign_in_opener();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_reset_viewer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_reset_viewer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_history_scroll(move |delta| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_history_scroll(delta);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_history_jump(move |y| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_history_jump(y);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_demo_event(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_demo_event(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_recover_message(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_recover_message(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_settings_changed();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_send(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_send();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_more(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_more();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_retry_image(move |key| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_retry_image(key);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_draft_edited(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_draft_edited();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_search_changed(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_search_changed(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_select_chat(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_select_chat(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_cancel_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_cancel_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_start_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_start_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_photo_closed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_photo_closed();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_photo(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_photo(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_start_friend_chat(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_start_friend_chat();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_profile(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_profile(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_filter_friends(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_filter_friends();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_refresh_friends(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_refresh_friends();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_browse(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_browse(arg0);
        }
    });
    Ok(ui)
}

struct Desktop {
    latest_requested: bool,
    opening_history: bool,
    divider_top: Rc<std::cell::Cell<Option<f32>>>,
    return_anchor: Option<(Key, f32, bool)>,
    geometry: Rc<RefCell<std::collections::BTreeMap<Key, (f32, f32)>>>,
    gap_geometry: Rc<RefCell<std::collections::BTreeMap<Key, (f32, f32)>>>,
    requested_gaps: std::collections::BTreeSet<(u64, Key)>,
    restore_ready: bool,
    exposed: std::collections::BTreeMap<ChatId, ServerId>,
    paging_anchor: Option<Key>,
    restore_anchor: Option<(Key, f32)>,
    reading: Rc<RefCell<Option<(ChatId, Cursor)>>>,
    shown_composer: String,
    reveal: Option<ChatId>,
    viewer_asset: Option<String>,
    destination: viewer::Destination,
    notifications: Option<notifications::Notifications>,
    background: background::Background,
    requested_assets: std::collections::BTreeSet<String>,
    state: AppState,
    presenter: presenter::Presenter,
    sender: Sender,
    query: String,
    category: i32,
}
impl Desktop {
    fn observe_reading(&mut self, ui: &AppWindow) {
        let global = ui.global::<State>();
        let top = -ui.get_history_y() + global.get_history_visible_top();
        let bottom = -ui.get_history_y() + global.get_history_viewport_height();
        if self.state.connection == Connection::Online {
            for gap in &self.state.gaps {
                if self
                    .gap_geometry
                    .borrow()
                    .get(&gap.after)
                    .is_some_and(|(y, h)| y + h >= top && *y <= bottom)
                    && self
                        .requested_gaps
                        .insert((self.state.window_sub, gap.after))
                {
                    let row = &self.state.messages[&gap.after];
                    self.sender.send(Intent::Page {
                        chat: row.chat_id.clone(),
                        kind: PageKind::Older {
                            before: row.sort.clone(),
                        },
                    });
                }
            }
        }

        if self.opening_history
            && self.state.unread_divider.is_some()
            && let Some(top) = self.divider_top.get()
        {
            ui.invoke_show_history(-top + global.get_history_visible_top(), false);
            self.opening_history = false;
        } else if self.opening_history && self.state.reaches_end && !self.state.messages.is_empty()
        {
            ui.invoke_show_history(0., true);
            self.opening_history = false;
        }
        if global.get_window_active()
            && !global.get_modal_open()
            && !ui.get_history_dragging()
            && !global.get_history_interacting()
            && (!global.get_narrow() || ui.get_show_conversation())
            && self.state.connection == Connection::Online
            && let Some(chat) = self.state.selected.clone()
        {
            let top = -ui.get_history_y() + global.get_history_visible_top();
            let bottom = -ui.get_history_y() + global.get_history_viewport_height()
                - global.get_history_pill_height();
            let marker = self
                .state
                .rows()
                .iter()
                .filter(|m| {
                    self.geometry
                        .borrow()
                        .get(&m.key)
                        .is_some_and(|(y, h)| *y >= top && y + h <= bottom)
                })
                .filter_map(|m| m.server_id.clone())
                .max_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));
            if let Some(id) = marker
                && self
                    .state
                    .chats
                    .get(&chat)
                    .is_some_and(|c| c.chat.unread_count > 0)
                && self.exposed.get(&chat) != Some(&id)
            {
                self.exposed.insert(chat.clone(), id.clone());
                self.sender.send(Intent::MarkRead { chat, id });
            }
        }
        if self.restore_ready
            && let Some((key, offset)) = self.restore_anchor
            && let Some((top, _)) = self.geometry.borrow().get(&key)
        {
            ui.invoke_history_jump(offset - top);
            self.restore_anchor = None;
            self.restore_ready = false;
        }
    }
    fn drain_notifications(&mut self, ui: &AppWindow) {
        if let Some(n) = &self.notifications {
            n.drain(ui, &mut self.state, &self.sender);
        }
    }
    fn render(&mut self, ui: &AppWindow) {
        #[cfg(feature = "ui-debug")]
        latency::RENDERS.set(latency::RENDERS.get() + 1);
        // Text typed or set without an edit event stays; only engine changes replace it.
        if self.state.composer == self.shown_composer {
            self.state.composer = ui.get_draft().to_string();
        }
        self.shown_composer = self.state.composer.clone();
        for asset in self
            .state
            .chats
            .values()
            .filter_map(|c| c.chat.avatar.as_ref())
            .cloned()
            .chain(self.state.friends.iter().filter_map(|f| f.avatar.clone()))
            .chain(
                self.state
                    .contacts
                    .values()
                    .filter_map(|f| f.avatar.clone()),
            )
            .chain(
                self.state
                    .profile
                    .picture_path
                    .as_deref()
                    .and_then(line_wasm_poc::client::desktop::assets::Asset::avatar),
            )
            .chain(
                self.state
                    .members
                    .iter()
                    .filter_map(|m| m.contact.avatar.clone()),
            )
        {
            if self.requested_assets.insert(asset.key()) {
                self.sender.send(Intent::Asset(asset));
            }
        }

        *self.reading.borrow_mut() = self.state.selected.clone().zip(
            self.state
                .rows()
                .iter()
                .find(|m| m.server_id.is_some())
                .map(|m| m.sort.clone()),
        );
        self.background.apply(ui);
        if let Some(notifications) = &self.notifications {
            notifications.drain(ui, &mut self.state, &self.sender);
            notifications.service.set_unread_count(
                self.state
                    .chats
                    .values()
                    .filter(|c| c.chat.unread_count > 0)
                    .count() as u64,
            );
        }

        for m in self
            .state
            .messages
            .values()
            .chain(self.state.chats.values().filter_map(|c| c.preview.as_ref()))
        {
            use line_wasm_poc::client::desktop::assets::Asset;
            let media = m.body.image().cloned();
            let emoji = m.body.spans().iter().filter_map(Asset::inline_emoji);
            let reactions = m.reactions.iter().filter_map(|r| match &r.kind {
                ReactionKind::Paid(asset) => asset.clone(),
                _ => None,
            });
            for asset in media.into_iter().chain(emoji).chain(reactions) {
                if self.requested_assets.insert(asset.key()) {
                    self.sender.send(Intent::Asset(asset));
                }
            }
        }
        self.presenter
            .present(ui, &self.state, &self.query, self.category);
        // A chat opened from a profile is revealed once its row exists.
        if let Some(index) = self
            .reveal
            .as_ref()
            .and_then(|id| self.state.chat_order.iter().position(|c| c == id))
        {
            let chats = ui.get_chats();
            if let Some(position) =
                (0..chats.row_count()).find(|&i| chats.row_data(i).unwrap().index == index as i32)
            {
                ui.set_result_focus(position as i32);
                ui.invoke_reveal_chat(position as i32);
                self.reveal = None;
            }
        }
    }
    fn select(&mut self, ui: &AppWindow, index: usize) {
        if let Some(id) = self.state.chat_order.get(index).cloned() {
            if let Some(n) = &self.notifications {
                n.service.withdraw(&id.0);
            }
            self.capture_draft(ui);
            self.latest_requested = false;
            self.opening_history = true;
            self.divider_top.set(None);
            self.return_anchor = None;
            ui.global::<State>().set_return_visible(false);
            ui.global::<State>().set_highlight_message("".into());
            self.restore_anchor = None;
            self.geometry.borrow_mut().clear();
            self.gap_geometry.borrow_mut().clear();
            self.requested_gaps.clear();
            self.paging_anchor = None;
            let intent = self.state.select(id);
            self.sender.send(intent);
            ui.set_show_conversation(true);
            ui.invoke_show_history(0., false);
            self.render(ui);
        }
    }
    fn open_photo(&mut self, ui: &AppWindow, key: &str) {
        let asset = self
            .state
            .rows()
            .iter()
            .filter_map(|m| m.body.image().cloned())
            .find(|a| a.key() == key)
            .and_then(|a| a.original());
        if let Some(asset) = asset {
            let photos = self
                .state
                .rows()
                .iter()
                .filter(|m| matches!(m.body, Content::Photo { .. }))
                .filter_map(|m| m.body.image().cloned())
                .map(|a| a.key())
                .collect::<Vec<_>>();
            let index = photos.iter().position(|k| k == key);
            ui.global::<State>()
                .set_viewer_has_previous(index.is_some_and(|i| i > 0));
            ui.global::<State>()
                .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
            ui.global::<State>()
                .set_viewer_name(asset.display_name().into());
            self.viewer_asset = Some(asset.key());
            if let Some(preview) = self.presenter.images.get(key) {
                ui.set_viewer_image(preview.clone());
            }
            ui.set_viewer_status("Preview · loading original…".into());
            ui.set_viewer_loading(true);
            ui.set_viewer_error("".into());
            if !ui.get_viewer_open() {
                ui.invoke_show_viewer(key.into());
            }
            self.sender.send(Intent::Original(asset));
        }
    }
    fn capture_draft(&mut self, ui: &AppWindow) {
        self.state.composer = ui.get_draft().to_string();
        if let Some(chat) = &self.state.selected {
            self.sender.send(Intent::Draft {
                chat: chat.clone(),
                draft: Draft {
                    text: self.state.composer.clone(),
                    cursor: ui.get_editor_caret() as usize,
                    anchor: ui.get_editor_anchor() as usize,
                    updated: line_wasm_poc::engine::now_ms(),
                },
            });
        }
    }
    fn apply_batch(&mut self, ui: &AppWindow, deltas: impl IntoIterator<Item = Delta>) {
        let mut dirty = false;
        let mut window = false;
        let mut editor = None;
        for delta in deltas {
            let (changed, arrived, selection) = self.apply(ui, delta);
            dirty |= changed;
            window |= arrived;
            editor = selection.or(editor);
        }
        if self.state.selected.is_none() && !self.state.chat_order.is_empty() {
            self.select(ui, 0);
        } else if dirty {
            self.render(ui);
        }
        if let Some((anchor, caret)) = editor {
            if ui.get_draft().as_str() != self.state.composer {
                ui.set_draft(self.state.composer.clone().into());
            }
            self.shown_composer = self.state.composer.clone();
            ui.invoke_restore_editor(anchor, caret);
        }
        if window && self.latest_requested && self.state.reaches_end {
            self.latest_requested = false;
            ui.set_following(true);
            ui.invoke_latest();
        } else if window && !self.opening_history && ui.get_following() {
            ui.invoke_latest();
        }
    }
    fn apply(&mut self, ui: &AppWindow, d: Delta) -> (bool, bool, Option<(i32, i32)>) {
        // The engine may publish an unchanged window and a whole chat list on a
        // draft save. Keep that acknowledgement out of the conversation render.
        if let Delta::Window {
            sub,
            upsert,
            remove,
            gaps,
            reaches_start,
            reaches_end,
            replies,
            unread_anchor,
        } = &d
            && *sub == self.state.window_sub
            && remove.is_empty()
            && upsert.len() == self.state.messages.len()
            && upsert
                .iter()
                .all(|m| self.state.messages.get(&m.key) == Some(m))
            && gaps == &self.state.gaps
            && *reaches_start == self.state.reaches_start
            && *reaches_end == self.state.reaches_end
            && (self.state.unread_divider.is_some() || *unread_anchor == self.state.unread_divider)
            && replies.len() == self.state.reply_targets.len()
            && replies.iter().all(|m| {
                m.server_id
                    .as_ref()
                    .and_then(|id| self.state.reply_targets.get(id))
                    == Some(m)
            })
        {
            return (false, false, None);
        }
        if let Delta::Chats {
            sub,
            upsert,
            remove,
            order,
        } = &d
            && *sub == self.state.chat_sub
            && remove.is_empty()
            && *order == self.state.chat_order
            && upsert.iter().all(|item| {
                self.state.chats.get(&item.chat.id).is_some_and(|old| {
                    old.chat == item.chat
                        && old.preview == item.preview
                        && old.activity_ms == item.activity_ms
                        && old.failed_count == item.failed_count
                })
            })
        {
            for item in upsert {
                if self.state.chats[&item.chat.id].draft != item.draft {
                    self.presenter.patch_draft(&item.chat.id, &item.draft);
                }
            }
            self.state.apply(d);
            return (false, false, None);
        }
        let dirty = !matches!(
            &d,
            Delta::Asset { .. } | Delta::MediaFailed { .. } | Delta::Draft { .. }
        );
        match &d {
            Delta::UnsendFailed(key) => {
                if self.state.messages.contains_key(key) {
                    ui.set_unsend_feedback_id(key.0.to_string().into());
                }
            }
            Delta::Pair(event) => {
                use line_wasm_poc::backend::PairEvent;
                match event {
                    PairEvent::Stage(stage) => ui.set_auth_stage(stage.into()),
                    PairEvent::Qr(p) => ui.set_auth_qr(slint::Image::from_rgba8(
                        slint::SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height),
                    )),
                    PairEvent::Pin(pin) => ui.set_auth_pin(pin.into()),
                    PairEvent::Scanned => ui.set_auth_stage("Phone scanned the code".into()),
                }
            }
            Delta::PairFailed { cancelled } => {
                if *cancelled {
                    self.state.panel = Panel::Chats;
                    self.state.detail.clear();
                }
                ui.set_auth_error(
                    "Sign-in did not finish. Your cached conversations are still available.".into(),
                );
            }
            Delta::Connection(Connection::Online) if ui.get_auth_busy() => {
                self.state.panel = Panel::Chats;
                self.state.detail.clear();
            }
            Delta::Notify(m) => {
                if self.state.selected.as_ref() == Some(&m.chat_id) && !ui.get_following() {
                    ui.set_new_count(ui.get_new_count() + 1);
                }
                if let Some(notifications) = &self.notifications {
                    notifications.incoming(ui, &self.state, m);
                }
            }
            Delta::MoreChats(more) => ui.set_more_chats(*more),
            Delta::MediaFailed { key, failure } => {
                self.presenter.failures.insert(key.clone(), *failure);
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_loading(false);
                    ui.set_viewer_error(failure.to_string().into());
                }
            }
            Delta::File { name, bytes } => {
                let weak = ui.as_weak();
                let name = name.clone();
                let bytes = bytes.clone();
                let destination = self.destination.clone();
                std::thread::spawn(move || {
                    let result = viewer::save_bytes(&bytes, &name, destination);
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.global::<State>().set_file_saving(false);
                        ui.global::<State>().set_file_save_status(
                            result.err().unwrap_or_else(|| "Saved".into()).into(),
                        );
                    });
                });
            }
            Delta::Asset { key, pixels } => {
                self.presenter.failures.remove(key);
                self.presenter.images.insert(
                    key.clone(),
                    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                        &pixels.rgba,
                        pixels.width,
                        pixels.height,
                    )),
                );
                self.presenter.patch_asset(ui, &self.state, key);
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_image(self.presenter.images[key].clone());
                    ui.set_viewer_loading(false);
                    ui.set_viewer_status(
                        format!(
                            "{} · {} × {}",
                            ui.global::<State>().get_viewer_name(),
                            pixels.width,
                            pixels.height
                        )
                        .into(),
                    );
                }
            }
            Delta::Prefs(prefs) => {
                if !ui.get_demo() {
                    ui.set_text_size(prefs.text_size);
                    ui.set_enter_sends(prefs.enter_sends);
                }
                let s = ui.global::<State>();
                let p = &prefs.desktop;
                s.set_background_mode(p.background);
                s.set_autostart(p.autostart);
                s.set_notifications_enabled(p.notifications);
                s.set_notification_preview(p.preview);
                s.set_notification_sound(p.sound);
                s.set_notification_mention_exception(p.mention_exception);
                s.set_notification_group_events(p.group_events);
            }
            _ => {}
        }
        let window = matches!(d,Delta::Window {sub,..} if sub==self.state.window_sub);
        if window {
            self.restore_ready = true;
        }
        if matches!(d, Delta::Connection(Connection::Online)) {
            self.requested_gaps.clear();
        }
        let editor = match &d {
            Delta::Draft { chat, draft } if self.state.selected.as_ref() == Some(chat) => Some(
                draft
                    .as_ref()
                    .map(|d| (d.anchor as i32, d.cursor as i32))
                    .unwrap_or((0, 0)),
            ),
            _ => None,
        };
        if let Delta::Draft { chat, draft } = &d {
            if let Some(item) = self.state.chats.get_mut(chat) {
                item.draft = draft.clone();
            }
            self.presenter.patch_draft(chat, draft);
            // A delayed selection draft must not replace text entered since
            // the selection was rendered.
            if self.state.selected.as_ref() == Some(chat)
                && ui.get_draft().as_str() != self.shown_composer
            {
                return (false, false, None);
            }
        }
        self.state.apply(d);
        (dirty, window, editor)
    }
}
fn install(ui: &AppWindow, actions: Runtime, fake: Option<FakeBackend>) {
    macro_rules! callback {($name:ident, |$u:ident,$d:ident $(,$arg:ident)*| $body:block)=>{{
        let weak=ui.as_weak(); let actions=actions.clone();
        ui.$name(move |$($arg),*| {let weak=weak.clone(); actions.push(move |$d|{if let Some($u)=weak.upgrade() {$body}});});
    }};}
    macro_rules! global_callback {($name:ident, |$u:ident,$d:ident $(,$arg:ident)*| $body:block)=>{{
        let weak=ui.as_weak(); let actions=actions.clone();
        ui.global::<State>().$name(move |$($arg),*| {let weak=weak.clone(); actions.push(move |$d|{if let Some($u)=weak.upgrade() {$body}});});
    }};}
    macro_rules! focus_bridge {
        ($on:ident,$invoke:ident) => {
            global_callback!($on, |ui, _d| {
                ui.$invoke();
            });
        };
    }
    global_callback!(on_read_state_changed, |ui, d| {
        d.observe_reading(&ui);
    });
    focus_bridge!(on_focus_cycle_end, invoke_focus_cycle_end);
    focus_bridge!(on_focus_after_browser, invoke_focus_after_browser);
    focus_bridge!(on_focus_after_conversation, invoke_focus_after_conversation);
    focus_bridge!(on_focus_before_layer, invoke_focus_before_layer);
    focus_bridge!(on_focus_after_layer, invoke_focus_after_layer);
    focus_bridge!(on_focus_friend_list, invoke_focus_friend_list);
    focus_bridge!(on_focus_chat_list, invoke_focus_chat_list);
    focus_bridge!(on_focus_search, invoke_focus_search);
    focus_bridge!(on_focus_friend_search, invoke_focus_friend_search);
    focus_bridge!(on_focus_settings_opener, invoke_focus_settings_opener);
    focus_bridge!(on_focus_settings_close, invoke_focus_settings_close);
    focus_bridge!(on_focus_profile_close, invoke_focus_profile_close);
    focus_bridge!(on_focus_auth_start, invoke_focus_auth_start);
    focus_bridge!(on_focus_connection_first, invoke_focus_connection_first);
    focus_bridge!(on_focus_sign_in_opener, invoke_focus_sign_in_opener);
    focus_bridge!(on_focus_viewer_close, invoke_focus_viewer_close);
    focus_bridge!(on_focus_history, invoke_focus_history);
    focus_bridge!(on_focus_composer, invoke_focus_composer);
    focus_bridge!(
        on_focus_reading_before_editor,
        invoke_focus_reading_before_editor
    );
    global_callback!(on_request_panel, |ui, d, panel| {
        let restore_avatar = panel == PanelState::None
            && matches!(d.state.panel, Panel::Viewer)
            && ui.global::<State>().get_viewer_from_detail();
        d.state.panel = match panel {
            PanelState::None => {
                if !d.state.detail.is_empty() && matches!(d.state.panel, Panel::Viewer) {
                    Panel::Detail
                } else if ui.get_section() == 1 {
                    Panel::Friends
                } else {
                    Panel::Chats
                }
            }
            PanelState::Settings => Panel::Settings,
            PanelState::Detail => Panel::Detail,
            PanelState::Viewer => Panel::Viewer,
            PanelState::SignIn => Panel::SignIn,
        };
        d.render(&ui);
        if restore_avatar {
            ui.global::<State>().set_viewer_from_detail(false);
            ui.global::<State>().invoke_finish_detail_restore(false);
        }
        match panel {
            PanelState::Settings => ui.invoke_focus_settings_close(),
            PanelState::SignIn => ui.invoke_focus_auth_start(),
            _ => {}
        }
    });
    global_callback!(on_pop_detail, |ui, d, all| {
        d.state.pop_detail(all);
        d.render(&ui);
        ui.global::<State>().invoke_finish_detail_restore(all);
    });
    global_callback!(on_finish_detail_restore, |ui, _d, all| {
        ui.global::<State>().invoke_restore_detail_focus(all);
    });
    global_callback!(on_detail_action, |ui, d, kind, value| {
        if kind == "member-chat" {
            d.capture_draft(&ui);
            let id = ChatId(value.to_string());
            d.reveal = Some(id.clone());
            d.sender.send(d.state.select(id));
            d.state.detail.clear();
            d.state.panel = Panel::Chats;
            ui.set_section(0);
            ui.set_show_conversation(true);
            d.render(&ui);
            ui.invoke_focus_composer();
            return;
        }
        if d.state.action.is_some() {
            return;
        }
        let Some(page) = d.state.detail_page().cloned() else {
            return;
        };
        let action = match (kind.as_str(), page) {
            ("rename", DetailPage::Profile(mid)) => ProfileAction::Rename {
                mid,
                name: value.to_string(),
            },
            ("add", DetailPage::Profile(mid)) => ProfileAction::AddFriend {
                mid,
                chat: d
                    .state
                    .selected
                    .as_ref()
                    .map(|c| c.0.clone())
                    .unwrap_or_default(),
            },
            ("mute", DetailPage::GroupInfo(chat)) => ProfileAction::Mute {
                disabled: !ui.global::<State>().get_group_muted(),
                chat,
            },
            ("leave", DetailPage::GroupInfo(chat)) => ProfileAction::Leave { chat },
            _ => return,
        };
        d.state.action_error.clear();
        d.state.action = Some(action.clone());
        d.sender.send(Intent::ProfileAction(action));
        d.render(&ui);
    });
    global_callback!(on_profile_photo, |ui, d| {
        if ui.get_profile_avatar().size().width == 0 {
            return;
        }
        ui.global::<State>().invoke_remember_detail_focus(false);
        ui.global::<State>().set_viewer_from_detail(true);
        ui.global::<State>().set_viewer_has_previous(false);
        ui.global::<State>().set_viewer_has_next(false);
        ui.set_viewer_loading(false);
        ui.set_viewer_error("".into());
        d.viewer_asset = None;
        ui.set_viewer_key(format!("profile:{}", ui.get_profile_id()).into());
        ui.invoke_reset_viewer();
        ui.set_viewer_image(ui.get_profile_avatar());
        ui.set_viewer_status(ui.get_profile_name());
        d.state.panel = Panel::Viewer;
        d.render(&ui);
        ui.invoke_focus_viewer_close();
    });
    callback!(on_select_chat, |ui, d, index| {
        d.select(&ui, index as usize);
    });
    callback!(on_switch_chat, |ui, d, delta| {
        let i = ui.get_selected_chat() + delta;
        if i >= 0 && (i as usize) < d.state.chat_order.len() {
            d.select(&ui, i as usize);
        }
    });
    callback!(on_search_changed, |ui, d, query| {
        d.query = query.to_lowercase();
        d.render(&ui);
    });
    callback!(on_filter_category, |ui, d| {
        d.category = ui.get_category();
        d.render(&ui);
    });
    callback!(on_draft_edited, |ui, d| {
        d.capture_draft(&ui);
    });
    callback!(on_send, |ui, d| {
        d.state.composer = ui.get_draft().to_string();
        if let Some(intent) = d.state.submit() {
            d.sender.send(intent);
            d.render(&ui);
        }
    });
    callback!(on_retry_message, |ui, d, id| {
        let key = Key(id.parse().unwrap());
        if d.state.messages[&key].delivery == Delivery::Uncertain {
            ui.set_recovery_id(id);
            ui.set_recovery_kind("resend".into());
        } else {
            d.sender.send(Intent::Retry(key));
        }
    });
    callback!(on_resolve_recovery, |ui, d, action| {
        let id = ui.get_recovery_id();
        let kind = ui.get_recovery_kind();
        ui.set_recovery_kind("".into());
        if action == "cancel" || kind == "resend" || kind == "unsend" {
            ui.invoke_focus_history();
            ui.set_recovery_focus_id(id.clone());
            ui.set_recovery_focus_action(kind);
        }
        if action == "unsend" {
            ui.set_unsend_feedback_id("".into());
            d.sender.send(Intent::Unsend(Key(id.parse().unwrap())));
        }
        if action == "resend" {
            d.sender.send(Intent::Resend(Key(id.parse().unwrap())));
        }
        if action == "replace" {
            d.sender.send(Intent::Copy(Key(id.parse().unwrap())));
            ui.invoke_focus_composer();
        }
        if action == "append" {
            let m = &d.state.messages[&Key(id.parse().unwrap())];
            d.state.composer = format!("{}\n{}", ui.get_draft(), m.body.as_text());
            ui.set_draft(d.state.composer.clone().into());
            d.capture_draft(&ui);
            ui.invoke_restore_editor(d.state.composer.len() as i32, d.state.composer.len() as i32);
            ui.invoke_focus_composer();
        }
    });
    callback!(on_recover_message, |ui, d, id| {
        if ui.get_draft().is_empty() {
            d.sender.send(Intent::Copy(Key(id.parse().unwrap())));
        } else {
            ui.set_recovery_id(id);
            ui.set_recovery_kind("edit".into());
        }
    });
    callback!(on_start_sign_in, |ui, d| {
        ui.set_auth_error("".into());
        d.sender.send(Intent::SignIn);
    });
    callback!(on_cancel_sign_in, |ui, d| {
        d.sender.cancel_sign_in();
        ui.set_auth_stage("Cancelling sign-in…".into());
    });
    callback!(on_more, |_ui, d| {
        d.sender.send(Intent::MoreChats);
    });
    callback!(on_settings_changed, |ui, d| {
        if !ui.global::<State>().get_notifications_enabled()
            && let Some(n) = &d.notifications
        {
            n.service.withdraw_notifications();
        }
        let s = ui.global::<State>();
        d.sender.send(Intent::Prefs(Prefs {
            text_size: ui.get_text_size(),
            enter_sends: ui.get_enter_sends(),
            desktop: DesktopPrefs {
                background: s.get_background_mode(),
                autostart: s.get_autostart(),
                notifications: s.get_notifications_enabled(),
                preview: s.get_notification_preview(),
                sound: s.get_notification_sound(),
                mention_exception: s.get_notification_mention_exception(),
                group_events: s.get_notification_group_events(),
            },
            ..d.state.prefs.clone()
        }));
    });
    callback!(on_browse, |ui, d, section| {
        d.state.panel = if section == 1 {
            Panel::Friends
        } else {
            Panel::Chats
        };
        d.state.detail.clear();
        ui.set_section(section);
        if section == 1 {
            d.sender.send(Intent::Friends);
        }
        d.render(&ui);
    });
    callback!(on_refresh_friends, |_ui, d| {
        d.sender.send(Intent::Friends);
    });
    callback!(on_filter_friends, |ui, d| {
        d.render(&ui);
    });
    callback!(on_open_members, |ui, d| {
        if let Some(chat) = d.state.selected.clone() {
            let push =
                matches!(d.state.detail_page(),Some(DetailPage::GroupInfo(id)) if id == &chat);
            if !push {
                d.state.detail.clear();
            }
            ui.global::<State>().invoke_remember_detail_focus(!push);
            ui.set_member_query("".into());
            d.state
                .push_detail(DetailPage::Members(chat.clone()), "members".into());
            d.sender.send(Intent::Members(chat));
            d.render(&ui);
            ui.invoke_focus_profile_close();
        }
    });
    callback!(on_filter_members, |ui, d| {
        d.render(&ui);
    });
    callback!(on_open_profile, |ui, d, id| {
        d.state.detail.clear();
        ui.global::<State>().invoke_remember_detail_focus(true);
        let id = id.to_string();
        let group = d.state.friends.iter().any(|f| f.id == id && f.group)
            || d.state
                .chats
                .get(&ChatId(id.clone()))
                .is_some_and(|c| c.chat.is_group);
        if group {
            let chat = ChatId(id);
            if d.state.selected.as_ref() != Some(&chat) {
                d.capture_draft(&ui);
                d.sender.send(d.state.select(chat.clone()));
            }
            ui.set_show_conversation(true);
            d.state
                .push_detail(DetailPage::GroupInfo(chat.clone()), "header".into());
            d.sender.send(Intent::GroupInfo(chat.clone()));
            d.sender.send(Intent::Members(chat));
        } else {
            d.state
                .push_detail(DetailPage::Profile(id.clone()), "profile".into());
            if id != "self" {
                d.sender.send(Intent::Contact(id));
            }
        }
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    callback!(on_open_member, |ui, d, id| {
        ui.global::<State>()
            .invoke_remember_detail_focus(d.state.detail.is_empty());
        let id = id.to_string();
        d.state
            .push_detail(DetailPage::Profile(id.clone()), "member".into());
        d.sender.send(Intent::Contact(id));
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    callback!(on_start_friend_chat, |ui, d| {
        d.capture_draft(&ui);
        if let Some(DetailPage::Profile(id)) = d.state.detail_page() {
            let id = ChatId(id.clone());
            d.reveal = Some(id.clone());
            let intent = d.state.select(id);
            d.sender.send(intent);
            d.state.detail.clear();
            d.state.panel = Panel::Chats;
            ui.set_section(0);
            ui.set_show_conversation(true);
            d.render(&ui);
            ui.invoke_focus_composer();
        }
    });
    callback!(on_open_photo, |ui, d, key| {
        d.open_photo(&ui, key.as_str());
    });
    callback!(on_retry_image, |ui, d, key| {
        d.sender.send(Intent::RetryAsset(key.to_string()));
        ui.invoke_focus_history();
    });
    global_callback!(on_discard_message, |_ui, d, id| {
        d.sender.send(Intent::Discard(Key(id.parse().unwrap())));
    });
    global_callback!(on_test_notification, |ui, d| {
        if let Some(n) = &d.notifications {
            n.test(&ui);
        } else {
            ui.global::<State>()
                .set_notification_status("Notification service not available".into());
        }
    });
    global_callback!(on_jump_message, |ui, d, id| {
        let target = d
            .state
            .reply_targets
            .get(&ServerId(id.to_string()))
            .map(|m| m.key);
        if let Some(key) = target {
            let top = -ui.get_history_y() + ui.global::<State>().get_history_visible_top();
            let origin = d.state.rows().into_iter().find_map(|m| {
                d.geometry
                    .borrow()
                    .get(&m.key)
                    .filter(|(y, h)| y + h > top)
                    .map(|(y, _)| {
                        let inset = ui.global::<State>().get_history_visible_top();
                        (m.key, y + ui.get_history_y() + inset, ui.get_following())
                    })
            });
            d.return_anchor = origin;
            d.opening_history = false;
            d.restore_anchor = Some((key, ui.global::<State>().get_history_visible_top()));
            // A target already laid out is scrolled to at once; the window may not change.
            let known = d.geometry.borrow().contains_key(&key);
            d.restore_ready = known;
            if !known {
                d.geometry.borrow_mut().clear();
            }
            ui.set_following(false);
            ui.global::<State>().set_return_visible(origin.is_some());
            ui.global::<State>()
                .set_highlight_message(key.0.to_string().into());
            let intent = d.state.subscribe_window(Anchor::Key(key), 80, 80);
            d.sender.send(intent);
        }
    });
    global_callback!(on_return_to_message, |ui, d| {
        if let Some((key, offset, _)) = d.return_anchor.take() {
            // The origin is kept below the top cover, which may have changed.
            d.restore_anchor = Some((key, offset - ui.global::<State>().get_history_visible_top()));
            let known = d.geometry.borrow().contains_key(&key);
            d.restore_ready = known;
            if !known {
                d.geometry.borrow_mut().clear();
            }
            ui.set_following(false);
            ui.global::<State>().set_return_visible(false);
            ui.global::<State>().set_highlight_message("".into());
            let intent = d.state.subscribe_window(Anchor::Key(key), 80, 80);
            d.sender.send(intent);
        }
    });
    global_callback!(on_viewer_step, |ui, d, direction| {
        let photos = d
            .state
            .rows()
            .iter()
            .filter(|m| matches!(m.body, Content::Photo { .. }))
            .filter_map(|m| m.body.image().cloned())
            .map(|a| a.key())
            .collect::<Vec<_>>();
        if let Some(i) = photos
            .iter()
            .position(|k| k == ui.get_viewer_key().as_str())
        {
            let next = i as i32 + direction.signum();
            if next >= 0 && (next as usize) < photos.len() {
                let key = &photos[next as usize];
                ui.set_viewer_key(key.into());
                d.open_photo(&ui, key);
            }
        }
    });
    global_callback!(on_save_file, |ui, d, id| {
        let m = &d.state.messages[&Key(id.parse().unwrap())];
        if let Content::File {
            asset: Some(asset),
            name,
            ..
        } = &m.body
        {
            let asset = asset.clone();
            let name = name.clone().unwrap_or_else(|| "attachment".into());
            ui.global::<State>().set_file_save_id(id.clone());
            ui.global::<State>().set_file_save_status("".into());
            ui.global::<State>().set_file_saving(true);
            d.sender.send(Intent::File { asset, name });
        }
    });
    global_callback!(on_save_photo, |ui, d| {
        viewer::save_photo(&ui, d.destination.clone());
    });
    global_callback!(on_latest_request, |ui, d| {
        ui.global::<State>().set_return_visible(false);
        ui.global::<State>().set_highlight_message("".into());
        d.return_anchor = None;
        d.opening_history = false;
        if !matches!(d.state.window_anchor, Anchor::Latest) {
            d.latest_requested = true;
            d.restore_anchor = None;
            d.paging_anchor = None;
            d.requested_gaps.clear();
            let intent = d.state.subscribe_window(Anchor::Latest, 80, 0);
            d.sender.send(intent);
            ui.set_following(true);
        }
    });
    global_callback!(on_row_geometry, |ui, d, id, top, height| {
        if id == "unread" {
            d.divider_top.set(Some(top));
        } else if let Some(key) = id.strip_prefix("gap-") {
            d.gap_geometry
                .borrow_mut()
                .insert(Key(key.parse().unwrap()), (top, height));
        } else {
            d.geometry
                .borrow_mut()
                .insert(Key(id.parse().unwrap()), (top, height));
        }
        d.observe_reading(&ui);
    });
    global_callback!(on_history_scrolled, |ui, d| {
        d.observe_reading(&ui);
        if ui.get_history_y() >= -30.
            && !ui.global::<State>().get_older_start_known()
            && let Some((chat, before)) = d.reading.borrow().clone()
        {
            if let Some(key) = d
                .state
                .rows()
                .into_iter()
                .find(|m| m.server_id.is_some())
                .map(|m| m.key)
                && d.paging_anchor != Some(key)
            {
                let offset = d
                    .geometry
                    .borrow()
                    .get(&key)
                    .map(|(y, _)| y + ui.get_history_y())
                    .unwrap_or(0.);
                d.paging_anchor = Some(key);
                d.restore_anchor = Some((key, offset));
                d.restore_ready = false;
                d.requested_gaps.clear();
                let intent = d.state.subscribe_window(Anchor::Key(key), 80, 80);
                d.sender.send(intent);
            }
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Older { before },
            });
        }
    });
    let link_actions = actions.clone();
    ui.global::<State>().on_open_link(move |url| {
        link_actions.push(move |_| links::open(url.to_string()));
    });
    if let Some(fake) = fake {
        demo::install(ui, actions, fake);
    }
    rich_content::install(ui);
}

#[derive(Clone)]
struct Runtime {
    desktop: Rc<RefCell<Option<Desktop>>>,
    actions: ActionQueue<Desktop>,
    draining: Rc<std::cell::Cell<bool>>,
}
impl Runtime {
    fn push(&self, action: impl FnOnce(&mut Desktop) + 'static) {
        self.actions.push(action);
        if self.draining.get() {
            return;
        }
        let available = self.desktop.borrow().is_some();
        if available {
            self.update(|_| {});
        }
    }
    fn update<R>(&self, f: impl FnOnce(&mut Desktop) -> R) -> R {
        self.draining.set(true);
        let mut desktop = self.desktop.borrow_mut().take().unwrap();
        let result = f(&mut desktop);
        self.actions.drain(&mut desktop);
        *self.desktop.borrow_mut() = Some(desktop);
        self.draining.set(false);
        result
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = false;
    let mut snapshot_path = None;
    let mut size = (980, 720);
    let mut text_size = 15;
    let mut scene = String::new();
    let mut case = None;
    let mut dark = false;
    let mut snapshot_profile = false;
    let mut background = false;
    let mut locations = line_wasm_poc::locations::Locations::from_environment()?;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"mcp_debug\":{},\"renderer\":\"software\"}}",
                    cfg!(feature = "ui-debug")
                );
                return Ok(());
            }
            "--demo" => demo = true,
            "--demo-paging" => {
                demo = true;
                scene = "paging".into();
            }
            "--demo-previews" => {
                demo = true;
                scene = "previews".into();
            }
            "--demo-dark" => {
                demo = true;
                dark = true;
            }
            "--demo-light" => demo = true,
            "--snapshot-profile" => snapshot_profile = true,
            "--snapshot-case" => case = Some(args.next().ok_or("missing snapshot case")?),
            "--debug-background" => background = true,
            "--snapshot" => {
                snapshot_path = Some(args.next().ok_or("missing snapshot path")?);
                demo = true;
            }
            "--snapshot-size" | "--demo-size" => {
                let v = args.next().ok_or("missing size")?;
                let (w, h) = v.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                size = (w.parse()?, h.parse()?);
            }
            "--snapshot-text-size" => {
                text_size = args.next().ok_or("missing text size")?.parse()?
            }
            "--snapshot-scene" => scene = args.next().ok_or("missing scene")?,
            "--state" => locations.session = args.next().ok_or("missing state")?.into(),
            "--wasm" => locations.wasm = args.next().ok_or("missing wasm")?.into(),
            "--cache-dir" => locations.cache = args.next().ok_or("missing cache directory")?.into(),
            "--demo-rich" | "--demo-photos" | "--demo-groups" | "--demo-friends" => {
                demo = true;
                scene = arg.trim_start_matches("--demo-").into();
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    let automation = debug_launch::intent(
        background,
        cfg!(feature = "ui-debug"),
        demo,
        std::env::var("LINE_DESKTOP_AUTOMATION_SOCKET")
            .ok()
            .as_deref(),
    )? == debug_launch::Intent::VirtualAutomation;
    let snapshot = if snapshot_path.is_some() {
        Some(snapshot::install()?)
    } else {
        if automation {
            // Slint 1.18.1 supports headless in create_backend(), but not BackendSelector.
            // The owned launcher selects it before any platform initialization.
            if std::env::var("SLINT_BACKEND").as_deref() != Ok("headless-software") {
                return Err("headless automation requires the private launcher".into());
            }
            #[cfg(feature = "ui-debug")]
            {
                slint::platform::set_platform(i_slint_backend_selector::create_backend()?)?;
                i_slint_backend_selector::init_testing_backends();
            }
        } else {
            slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("software".into())
                .select()?;
        }
        None
    };
    slint::set_xdg_app_id("org.lineclient.Desktop")?;
    if snapshot.is_some() {
        fix_snapshot_clock();
    }
    let instance = if snapshot.is_none() {
        match desktop_integration::Instance::acquire_with_activation(!automation)? {
            desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    let activations = instance.as_ref().map(|i| i.take_events());
    let fake = demo.then(FakeBackend::canned);
    if let Some(fake) = &fake {
        demo::configure(fake, &scene, case.as_deref());
    }
    let worker = if let Some(fake) = &fake {
        Worker::start(fake.clone(), StoreLocation::Memory)
    } else {
        let root = locations.session.parent().unwrap().join("accounts");
        Worker::start_with(StoreLocation::Account { root, mid: None }, move || {
            LineBackend::open_local(&locations.session, &locations.wasm, &locations.cache)
        })
    };
    let ui = create_app_window()?;
    ui.set_demo(demo);
    ui.global::<State>().set_preview_demo(scene == "previews");
    if dark {
        ui.invoke_preview_dark(true);
    }
    if let Some(warning) = instance.as_ref().and_then(|i| i.warning.as_ref()) {
        ui.set_desktop_warning(warning.into());
    }
    ui.set_text_size(text_size);
    ui.global::<State>().set_automation_background(automation);
    if snapshot.is_none() && !automation {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{Ime, WindowEvent},
        };
        let weak = ui.as_weak();
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
                    WindowEvent::Focused(active) => ui.global::<State>().set_window_active(*active),
                    WindowEvent::Ime(Ime::Preedit(text, _)) => ui.set_ime_active(!text.is_empty()),
                    WindowEvent::Ime(Ime::Commit(_)) | WindowEvent::Ime(Ime::Disabled) => {
                        ui.set_ime_active(false)
                    }
                    _ => {}
                }
            }
            EventResult::Propagate
        });
    }
    let desktop = Desktop {
        requested_assets: Default::default(),
        gap_geometry: Default::default(),
        requested_gaps: Default::default(),
        restore_ready: false,
        latest_requested: false,
        opening_history: true,
        divider_top: Default::default(),
        return_anchor: None,
        geometry: Default::default(),
        exposed: Default::default(),
        paging_anchor: None,
        restore_anchor: None,
        reading: Default::default(),
        shown_composer: String::new(),
        reveal: None,
        viewer_asset: None,
        destination: if demo {
            viewer::Destination::Directory(std::env::temp_dir().join("line-desktop-demo-exports"))
        } else {
            viewer::Destination::Dialog
        },
        notifications: if snapshot.is_none() {
            Some(notifications::Notifications::start())
        } else {
            None
        },
        background: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender: worker.sender.clone(),
        query: String::new(),
        category: 0,
    };
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade()
            && ui.global::<State>().get_background_mode()
            && ui.global::<State>().get_tray_available()
        {
            let _ = ui.hide();
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        }
    });
    desktop.presenter.install(&ui);
    let desktop = Runtime {
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: ActionQueue::default(),
        draining: Default::default(),
    };
    install(&ui, desktop.clone(), fake.clone());
    // Initial synthetic rendering uses the same FIFO as interactive rendering.
    if snapshot.is_some() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while desktop.update(|d| d.state.messages.is_empty()) {
            let delta = worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))?;
            desktop.update(|d| {
                d.apply_batch(&ui, std::iter::once(delta).chain(worker.deltas.try_iter()))
            });
        }
    }
    let weak = ui.as_weak();
    let d = desktop.clone();
    let worker = Rc::new(worker);
    let w = worker.clone();
    worker.on_deltas(event_loop::wake);
    event_loop::install(move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_integration::Activation::OpenChat(chat) = e {
                        d.update(|d| {
                            let intent = d.state.select(ChatId(chat));
                            d.sender.send(intent);
                            d.render(&ui);
                            ui.set_show_conversation(true);
                        });
                    }
                }
            }
            d.update(|d| d.apply_batch(&ui, w.deltas.try_iter()));
            d.update(|d| {
                d.background.apply(&ui);
                d.observe_reading(&ui);
                d.drain_notifications(&ui);
            });
        }
    });
    ui.window()
        .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
    ui.set_show_conversation(scene != "chats" && !scene.starts_with("friends"));
    if scene == "settings" {
        desktop.update(|d| {
            d.state.panel = Panel::Settings;
            d.render(&ui);
        });
    }
    if scene == "friends" {
        ui.invoke_browse(1);
    }
    if matches!(
        scene.as_str(),
        "members" | "member-profile" | "group-header" | "room-header"
    ) {
        desktop.update(|d| d.select(&ui, 1));
    }
    if scene.starts_with("members") || scene.starts_with("member-profile") {
        ui.invoke_open_members();
    }
    if snapshot_profile || scene == "member-profile" {
        ui.invoke_browse(1);
    }
    if scene.starts_with("message-selected")
        && let Some(key) = desktop.update(|d| d.state.rows().first().map(|m| m.key))
    {
        ui.global::<State>()
            .set_highlight_message(key.0.to_string().into());
    }
    if scene == "session-expired"
        && let Some(fake) = &fake
    {
        fake.auth_expired();
    }
    if scene.ends_with("offline")
        && let Some(fake) = &fake
    {
        fake.offline(true);
    }
    if scene == "failed-send"
        && let Some(fake) = &fake
    {
        fake.fail_send();
        ui.set_draft("See you soon! 🐈".into());
        ui.invoke_send();
    }
    if snapshot_path.is_some() {
        for _ in 0..20 {
            desktop.update(|_| {});
            desktop.update(|d| d.apply_batch(&ui, worker.deltas.try_iter()));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[cfg(feature = "ui-debug")]
    if demo && std::env::var_os("LINE_UI_LATENCY").is_some() {
        let window = snapshot
            .as_ref()
            .expect("latency measurement requires --snapshot");
        desktop.update(|d| latency::run(d, &ui, window));
        return Ok(());
    }
    if snapshot_profile || scene == "member-profile" {
        ui.invoke_open_profile("alice".into());
    }
    if let (Some(window), Some(path)) = (snapshot, snapshot_path) {
        ui.show()?;
        snapshot::save(&window, &path, size)?;
        ui.hide()?;
    } else {
        ui.show()?;
        slint::run_event_loop_until_quit()?;
        ui.hide()?;
    }
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    drop(worker);
    Ok(())
}

#[cfg(test)]
mod tests;
