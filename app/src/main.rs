use line_wasm_poc::{
    backend::{FakeBackend, LineBackend},
    controller::{ActionQueue, AppState, Panel},
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
    ui.global::<State>()
        .on_has_text(|text| !text.trim().is_empty());
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
            .filter_map(|s| {
                serde_json::from_str::<line_wasm_poc::client::desktop::assets::Asset>(s).ok()
            })
            .chain(self.state.friends.iter().filter_map(|f| f.avatar.clone()))
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
            if let Ok(raw) =
                serde_json::from_value::<line_wasm_poc::client::desktop::Message>(m.body.clone())
            {
                for asset in
                    raw.image
                        .into_iter()
                        .chain(raw.sender_avatar)
                        .chain(raw.reactions.into_iter().filter_map(|r| r.asset))
                        .chain(raw.rich.emoji.iter().filter_map(
                            line_wasm_poc::client::desktop::assets::Asset::inline_emoji,
                        ))
                {
                    if self.requested_assets.insert(asset.key()) {
                        self.sender.send(Intent::Asset(asset));
                    }
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
            .filter_map(|m| {
                serde_json::from_value::<line_wasm_poc::client::desktop::Message>(m.body.clone())
                    .ok()
            })
            .filter_map(|m| m.image)
            .find(|a| a.key() == key)
            .and_then(|a| a.original());
        if let Some(asset) = asset {
            let photos = self
                .state
                .rows()
                .iter()
                .filter_map(|m| {
                    serde_json::from_value::<line_wasm_poc::client::desktop::Message>(
                        m.body.clone(),
                    )
                    .ok()
                })
                .filter(|m| m.rich.content_type == 1)
                .filter_map(|m| m.image)
                .map(|a| a.key())
                .collect::<Vec<_>>();
            let index = photos.iter().position(|k| k == key);
            ui.global::<State>()
                .set_viewer_has_previous(index.is_some_and(|i| i > 0));
            ui.global::<State>()
                .set_viewer_has_next(index.is_some_and(|i| i + 1 < photos.len()));
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
    fn apply(&mut self, ui: &AppWindow, d: Delta) {
        match &d {
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
                }
                ui.set_auth_error(
                    "Sign-in did not finish. Your cached conversations are still available.".into(),
                );
            }
            Delta::Connection(Connection::Online) if ui.get_auth_busy() => {
                self.state.panel = Panel::Chats;
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
                self.presenter.invalidate_assets();
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
                self.presenter.invalidate_assets();
                if self.viewer_asset.as_ref() == Some(key) {
                    ui.set_viewer_image(self.presenter.images[key].clone());
                    ui.set_viewer_loading(false);
                    ui.set_viewer_status(
                        format!(
                            "{}{} × {}",
                            if ui.get_demo() {
                                "Synthetic original · "
                            } else {
                                ""
                            },
                            pixels.width,
                            pixels.height
                        )
                        .into(),
                    );
                }
            }
            Delta::Pref { key, value } => match key.as_str() {
                "text_size" => ui.set_text_size(value.as_i64().unwrap() as i32),
                "enter_sends" => ui.set_enter_sends(value.as_bool().unwrap()),
                "desktop" => {
                    let s = ui.global::<State>();
                    s.set_background_mode(value["background"].as_bool().unwrap_or(false));
                    s.set_autostart(value["autostart"].as_bool().unwrap_or(false));
                    s.set_notifications_enabled(value["notifications"].as_bool().unwrap_or(false));
                    s.set_notification_preview(value["preview"].as_i64().unwrap_or(1) as i32);
                    s.set_notification_sound(value["sound"].as_bool().unwrap_or(false));
                }
                _ => {}
            },
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
        self.state.apply(d);
        if self.state.selected.is_none() && !self.state.chat_order.is_empty() {
            self.select(ui, 0);
        } else {
            self.render(ui);
            if let Some((anchor, caret)) = editor {
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
        let member_profile = matches!(d.state.panel, Panel::MemberProfile(_));
        let members = matches!(d.state.panel, Panel::Members | Panel::MemberProfile(_));
        d.state.panel = match panel {
            PanelState::None => {
                if ui.get_section() == 1 {
                    Panel::Friends
                } else {
                    Panel::Chats
                }
            }
            PanelState::Settings => Panel::Settings,
            PanelState::Profile => Panel::Profile(ui.get_profile_id().to_string()),
            PanelState::Members => Panel::Members,
            PanelState::MemberProfile => Panel::MemberProfile(ui.get_profile_id().to_string()),
            PanelState::Viewer => Panel::Viewer,
            PanelState::SignIn => Panel::SignIn,
        };
        d.render(&ui);
        match panel {
            PanelState::Settings => ui.invoke_focus_settings_close(),
            PanelState::SignIn => ui.invoke_focus_auth_start(),
            PanelState::Members if member_profile => ui.invoke_focus_member_list(),
            PanelState::Members | PanelState::MemberProfile | PanelState::Profile => {
                ui.invoke_focus_profile_close()
            }
            PanelState::None if members => ui.invoke_focus_members_header(),
            _ => {}
        }
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
        d.render(&ui);
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
        if action == "cancel" || kind == "resend" {
            ui.invoke_focus_history();
            ui.set_recovery_focus_id(id.clone());
            ui.set_recovery_focus_action(kind);
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
            d.state.composer = format!("{}\n{}", ui.get_draft(), m.body["body"].as_str().unwrap());
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
        d.sender.send(Intent::Pref {key:"desktop".into(),value:serde_json::json!({"background":s.get_background_mode(),"autostart":s.get_autostart(),"notifications":s.get_notifications_enabled(),"preview":s.get_notification_preview(),"sound":s.get_notification_sound(),"mention_exception":s.get_notification_mention_exception(),"group_events":s.get_notification_group_events()})});
        d.sender.send(Intent::Pref {
            key: "text_size".into(),
            value: ui.get_text_size().into(),
        });
        d.sender.send(Intent::Pref {
            key: "enter_sends".into(),
            value: ui.get_enter_sends().into(),
        });
    });
    callback!(on_browse, |ui, d, section| {
        d.state.panel = if section == 1 {
            Panel::Friends
        } else {
            Panel::Chats
        };
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
        if let Some(chat) = &d.state.selected {
            if !matches!(d.state.panel, Panel::Members | Panel::MemberProfile(_)) {
                ui.set_member_query("".into());
            }
            d.state.panel = Panel::Members;
            d.sender.send(Intent::Members(chat.clone()));
            d.render(&ui);
            ui.invoke_focus_profile_close();
        }
    });
    callback!(on_filter_members, |ui, d| {
        d.render(&ui);
    });
    callback!(on_open_profile, |ui, d, id| {
        d.state.panel = Panel::Profile(id.to_string());
        d.render(&ui);
        ui.invoke_show_profile();
    });
    callback!(on_open_member, |ui, d, id| {
        d.state.panel = Panel::MemberProfile(id.to_string());
        d.render(&ui);
        ui.invoke_focus_profile_close();
    });
    callback!(on_start_friend_chat, |ui, d| {
        if let Panel::Profile(id) | Panel::MemberProfile(id) = &d.state.panel {
            let id = ChatId(id.clone());
            d.reveal = Some(id.clone());
            let intent = d.state.select(id);
            d.sender.send(intent);
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
            .filter_map(|m| {
                serde_json::from_value::<line_wasm_poc::client::desktop::Message>(m.body.clone())
                    .ok()
            })
            .filter(|m| m.rich.content_type == 1)
            .filter_map(|m| m.image)
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
        let raw: line_wasm_poc::client::desktop::Message =
            serde_json::from_value(m.body.clone()).unwrap();
        if let Some(asset) = raw.attachment {
            ui.global::<State>().set_file_save_id(id.clone());
            ui.global::<State>().set_file_save_status("".into());
            ui.global::<State>().set_file_saving(true);
            d.sender.send(Intent::File {
                asset,
                name: raw.rich.file_name.unwrap_or_else(|| "attachment".into()),
            });
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
    global_callback!(on_row_geometry, |_ui, d, id, top, height| {
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
    });
    global_callback!(on_history_scrolled, |ui, d| {
        if ui.get_history_y() >= -30.
            && !ui.global::<State>().get_older_start_known()
            && let Some((chat, before)) = d.reading.borrow().clone()
        {
            d.sender.send(Intent::Page {
                chat,
                kind: PageKind::Older { before },
            });
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
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )? == debug_launch::Intent::VirtualAutomation;
    let snapshot = if snapshot_path.is_some() {
        Some(snapshot::install()?)
    } else {
        let mut selector = slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into());
        if automation {
            selector = selector.with_winit_window_attributes_hook(|a| a.with_active(false));
        }
        selector.select()?;
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
        Worker::start_with(move || {
            let backend =
                LineBackend::open_local(&locations.session, &locations.wasm, &locations.cache)?;
            let mid = backend.local_mid();
            let root = locations.session.parent().unwrap().join("accounts");
            Ok((backend, StoreLocation::Account { root, mid }))
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
    if snapshot.is_none() {
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
            desktop.update(|d| d.apply(&ui, delta));
        }
    }
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let d = desktop.clone();
    let worker = Rc::new(worker);
    let w = worker.clone();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            if let Some(ui) = weak.upgrade() {
                d.update(|d| {
                    d.background.apply(&ui);
                    d.observe_reading(&ui);
                    d.drain_notifications(&ui);
                });
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
                for delta in w.deltas.try_iter() {
                    d.update(|d| d.apply(&ui, delta));
                }
            }
        },
    );
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
            while let Ok(delta) = worker.deltas.try_recv() {
                desktop.update(|d| d.apply(&ui, delta));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
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
    timer.stop();
    drop(worker);
    Ok(())
}

#[cfg(test)]
mod tests;
