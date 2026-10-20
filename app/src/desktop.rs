use crate::*;

pub(crate) struct Desktop {
    pub(crate) clipboard_root: std::path::PathBuf,
    pub(crate) history: crate::reading::Reading,
    pub(crate) shown_composer: String,
    pub(crate) reveal: Option<ChatId>,
    pub(crate) viewer_name: String,
    pub(crate) attachments: std::sync::Arc<std::sync::Mutex<callbacks::attachments::Selection>>,
    pub(crate) viewer_asset: Option<String>,
    pub(crate) viewer_source: Option<std::path::PathBuf>,
    pub(crate) notifications: Option<notifications::Notifications>,
    pub(crate) background: desktop_env::background::Background,
    pub(crate) assets: assets::Assets,
    pub(crate) state: AppState,
    pub(crate) presenter: presenter::Presenter,
    pub(crate) sender: Sender,
    pub(crate) category: ChatCategory,
}
mod deltas;
mod reading;
mod viewer;

impl Desktop {
    pub(crate) fn show_chat(&mut self, ui: &AppWindow, id: ChatId) {
        self.reveal = Some(id.clone());
        self.open_chat(ui, id);
        self.state.detail.clear();
        self.state.panel = Panel::Chats;
        let g = ui.global::<ShellState>();
        g.set_section(Section::Chats);
        g.set_show_conversation(true);
        self.render(ui);
        ui.invoke_focus_composer();
    }

    pub(crate) fn drain_notifications(&mut self, ui: &AppWindow) {
        let events = self
            .notifications
            .as_ref()
            .map(|n| n.events())
            .unwrap_or_default();
        for event in events {
            notifications::handle(event, ui, self);
        }
    }
    pub(crate) fn request_sticker_assets(&mut self, ui: &AppWindow) {
        self.assets.stickers(
            &self.state,
            ui.global::<ComposerState>().get_sticker_package() as usize,
            &self.sender,
        );
    }
    pub(crate) fn render(&mut self, ui: &AppWindow) {
        ui.global::<ShellState>()
            .set_device_scale(ui.window().scale_factor());
        #[cfg(feature = "demo")]
        dev::rendered();
        // Text typed or set without an edit event stays; only engine changes replace it.
        if self.state.composer == self.shown_composer {
            self.state.composer = ui.global::<ComposerState>().get_draft().to_string();
        }
        self.shown_composer = self.state.composer.clone();
        self.assets.visible(
            &self.state,
            ui.global::<ComposerState>().get_sticker_package() as usize,
            &self.sender,
        );

        self.history.oldest = self.state.selected.clone().zip(
            self.state
                .rows()
                .iter()
                .find(|m| m.server_id.is_some())
                .map(|m| m.sort.clone()),
        );
        if let Some(notifications) = &self.notifications {
            notifications.service.set_unread_count(
                self.state
                    .chats
                    .values()
                    .filter(|c| c.unread_count > 0)
                    .count() as u64,
            );
        }

        self.presenter.present(ui, &self.state, self.category);
        self.presenter
            .request_status_assets(&mut self.assets, &self.sender);
        if let Some(id) = &self.history.jump_target
            && let Some(target) = self.state.reply_targets.get(id)
        {
            let key = target.key;
            self.history.jump_target = None;
            self.jump_to_message(ui, key);
        }
        // A chat opened from a profile is revealed once its row exists.
        if let Some(index) = self
            .reveal
            .as_ref()
            .and_then(|id| self.state.chat_order.iter().position(|c| c == id))
        {
            let chats = ui.global::<ChatsState>().get_chats();
            if let Some(position) =
                (0..chats.row_count()).find(|&i| chats.row_data(i).unwrap().index == index as i32)
            {
                ui.invoke_reveal_chat(position as i32);
                self.reveal = None;
            }
        }
    }
    pub(crate) fn select(&mut self, ui: &AppWindow, index: usize) {
        if let Some(id) = self.state.chat_order.get(index).cloned() {
            if let Some(n) = &self.notifications {
                n.service.withdraw(&id.0);
            }
            self.open_chat(ui, id);
            self.history.reset_for_chat();
            ui.global::<ConversationState>().set_return_visible(false);
            ui.global::<ConversationState>()
                .set_highlight_message("".into());
            ui.global::<ShellState>().set_show_conversation(true);
            ui.global::<ConversationState>()
                .invoke_show_history(0., false);
            self.render(ui);
        }
    }
    pub(crate) fn open_chat(&mut self, ui: &AppWindow, chat: ChatId) {
        self.capture_draft(ui);
        self.shown_composer = self.state.composer.clone();
        self.sender.send(self.state.select(chat));
    }
    pub(crate) fn capture_draft(&mut self, ui: &AppWindow) {
        // A selection can change before the next render updates the editor.
        if self.state.composer != self.shown_composer
            && ui.global::<ComposerState>().get_draft().as_str() == self.shown_composer
        {
            return;
        }
        self.state.composer = ui.global::<ComposerState>().get_draft().to_string();
        if let Some(chat) = &self.state.selected {
            let draft = Draft {
                reply: self.state.composer_reply.clone(),
                text: self.state.composer.clone(),
                cursor: ui.global::<ComposerState>().get_editor_caret() as usize,
                anchor: ui.global::<ComposerState>().get_editor_anchor() as usize,
                updated: line::engine::now_ms(),
            };
            self.sender.send(Intent::Draft {
                chat: chat.clone(),
                draft,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn desktop(worker: &Worker) -> Desktop {
        Desktop {
            clipboard_root: std::env::temp_dir(),
            history: Default::default(),
            shown_composer: String::new(),
            reveal: None,
            viewer_asset: None,
            viewer_source: None,
            viewer_name: String::new(),
            attachments: Default::default(),
            notifications: None,
            background: Default::default(),
            assets: Default::default(),
            state: AppState::default(),
            presenter: Default::default(),
            sender: worker.sender.clone(),
            category: ChatCategory::All,
        }
    }

    #[test]
    fn own_avatar_appears_in_rail_and_profile_when_asset_arrives() {
        std::thread::spawn(|| {
            use i_slint_backend_testing::ElementHandle;
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            ui.show().unwrap();
            ui.window().set_size(slint::LogicalSize::new(980., 720.));
            slint::platform::update_timers_and_animations();
            let mut desktop = desktop(&worker);
            desktop.presenter.install(&ui);
            desktop.state.apply(Delta::Profile(Profile {
                mid: "self".into(),
                display_name: "Me".into(),
                picture_path: Some("/own-photo".into()),
                ..Default::default()
            }));
            desktop
                .state
                .push_detail(line::controller::DetailPage::Profile("self".into()));
            let avatar = desktop
                .state
                .contact("self")
                .unwrap()
                .avatar
                .clone()
                .unwrap();
            desktop.render(&ui);
            assert_eq!(ui.global::<ShellState>().get_account_initial(), "M");
            assert_eq!(
                ui.global::<ShellState>().get_account_avatar().size().width,
                0
            );
            assert_eq!(
                ui.global::<DetailState>().get_profile_avatar().size().width,
                0
            );
            assert!(
                ElementHandle::find_by_accessible_label(&ui, "Your avatar")
                    .next()
                    .is_none()
            );
            assert!(
                !desktop.assets.request(&avatar),
                "render must request the own avatar"
            );
            loop {
                let delta = worker
                    .deltas
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap();
                if matches!(&delta, Delta::Asset { key, .. } if *key == avatar.key()) {
                    desktop.apply(&ui, delta);
                    break;
                }
            }
            let image = ui.global::<ShellState>().get_account_avatar();
            assert!(image.size().width > 0);
            assert_eq!(ui.global::<DetailState>().get_profile_avatar(), image);
            assert!(
                ElementHandle::find_by_accessible_label(&ui, "Your avatar")
                    .next()
                    .is_some()
            );
            desktop.render(&ui);
            assert_eq!(ui.global::<ShellState>().get_account_avatar(), image);
            desktop.state.apply(Delta::Profile(Profile {
                mid: "self".into(),
                display_name: "Me".into(),
                ..Default::default()
            }));
            desktop.render(&ui);
            assert_eq!(
                ui.global::<ShellState>().get_account_avatar().size().width,
                0
            );
            assert!(
                ElementHandle::find_by_accessible_label(&ui, "Your avatar")
                    .next()
                    .is_none()
            );
        })
        .join()
        .unwrap();
    }
    #[test]
    fn asset_eviction_does_not_mark_desktop_dirty() {
        std::thread::spawn(|| {
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut desktop = desktop(&worker);
            let avatar = line::client::desktop::assets::Asset::avatar("/visible-avatar").unwrap();
            assert!(desktop.assets.request(&avatar));
            let (dirty, window, editor) =
                desktop.apply(&ui, Delta::AssetEvicted { key: avatar.key() });
            assert!(!dirty, "asset eviction must wait for a natural render");
            assert!(!window);
            assert!(editor.is_none());
            assert!(
                desktop.assets.request(&avatar),
                "evicted asset must be requestable again"
            );
        })
        .join()
        .unwrap();
    }
    #[test]
    fn unchanged_empty_window_finishes_loading() {
        std::thread::spawn(|| {
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut desktop = desktop(&worker);
            desktop.state.select(ChatId("empty".into()));
            desktop.apply(
                &ui,
                Delta::Window {
                    sub: desktop.state.window_sub,
                    upsert: vec![],
                    remove: vec![],
                    gaps: vec![],
                    reaches_start: false,
                    reaches_end: false,
                    replies: vec![],
                    unread_anchor: None,
                },
            );
            assert!(
                !desktop.state.window_loading,
                "an unchanged empty window must finish loading"
            );
        })
        .join()
        .unwrap();
    }
    #[test]
    fn profile_viewer_waits_for_original_without_showing_a_circle() {
        std::thread::spawn(|| {
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut desktop = desktop(&worker);
            let avatar = line::client::desktop::assets::Asset::avatar("/own-photo").unwrap();
            desktop.state.apply(Delta::Profile(Profile {
                mid: "self".into(),
                display_name: "Me".into(),
                picture_path: Some("/own-photo".into()),
                ..Default::default()
            }));
            ui.global::<DetailState>().set_profile_id("self".into());
            ui.global::<DetailState>().set_profile_name("Me".into());
            ui.global::<DetailState>()
                .set_profile_avatar(slint::Image::from_rgba8(slint::SharedPixelBuffer::<
                    slint::Rgba8Pixel,
                >::new(2, 3)));
            desktop.open_profile_photo(&ui);
            assert_eq!(desktop.viewer_asset, Some(avatar.original().unwrap().key()));
            assert!(ui.global::<ViewerState>().get_viewer_loading());
            assert_eq!(
                (
                    ui.global::<ViewerState>().get_viewer_image().size().width,
                    ui.global::<ViewerState>().get_viewer_image().size().height
                ),
                (0, 0)
            );
            assert_eq!(
                ui.global::<ViewerState>().get_viewer_status(),
                "Loading original…"
            );
            let source = crate::tests::image_file(&slint::Image::from_rgba8(
                slint::SharedPixelBuffer::clone_from_slice(&[255u8; 4 * 6 * 4], 4, 6),
            ));
            desktop.apply(
                &ui,
                Delta::Original {
                    key: desktop.viewer_asset.clone().unwrap(),
                    path: source.clone(),
                    source: source.clone(),
                },
            );
            assert!(!ui.global::<ViewerState>().get_viewer_loading());
            assert_eq!(
                (
                    ui.global::<ViewerState>().get_viewer_image().size().width,
                    ui.global::<ViewerState>().get_viewer_image().size().height
                ),
                (4, 6)
            );
            assert_eq!(desktop.viewer_source, Some(source));
        })
        .join()
        .unwrap();
    }
    #[test]
    fn profile_viewer_without_original_keeps_an_empty_placeholder() {
        std::thread::spawn(|| {
            let worker = Worker::start(
                StoreLocation::Memory,
                || Ok(line::backend::FakeBackend::canned()),
                Default::default(),
            );
            let _window = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut desktop = desktop(&worker);
            ui.global::<DetailState>().set_profile_id("missing".into());
            ui.global::<DetailState>()
                .set_profile_avatar(slint::Image::from_rgba8(slint::SharedPixelBuffer::<
                    slint::Rgba8Pixel,
                >::new(2, 3)));
            desktop.open_profile_photo(&ui);
            assert_eq!(desktop.state.panel, Panel::Viewer);
            assert_eq!(
                (
                    ui.global::<ViewerState>().get_viewer_image().size().width,
                    ui.global::<ViewerState>().get_viewer_image().size().height
                ),
                (0, 0)
            );
            assert!(!ui.global::<ViewerState>().get_viewer_loading());
            assert!(desktop.viewer_asset.is_none());
        })
        .join()
        .unwrap();
    }
}

#[cfg(all(test, feature = "demo"))]
#[test]
fn demo_settings_scene_focuses_close() {
    std::thread::spawn(|| {
        use slint::private_unstable_api::re_exports::{AccessibleStringProperty, WindowInner};
        let window = crate::snapshot::install().unwrap();
        let ui = crate::window::create_app_window().unwrap();
        let worker = line::engine::Worker::start(
            line::engine::StoreLocation::Memory,
            || Ok(line::backend::FakeBackend::canned()),
            Default::default(),
        );
        let runtime = crate::Runtime {
            desktop: std::rc::Rc::new(std::cell::RefCell::new(Some(tests::desktop(&worker)))),
            actions: Default::default(),
            draining: Default::default(),
        };
        ui.show().unwrap();
        crate::snapshot::render(&window, (980, 720)).unwrap();
        ui.invoke_focus_search();
        crate::dev::demo::stage(&ui, &runtime, None, "settings", false);
        crate::snapshot::render(&window, (980, 720)).unwrap();
        let item = WindowInner::from_pub(ui.window())
            .focus_item
            .borrow()
            .upgrade()
            .unwrap();
        assert_eq!(
            item.accessible_string_property(AccessibleStringProperty::Label)
                .unwrap(),
            "Close settings"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn failed_cancel_restores_sign_in_back_action() {
    std::thread::spawn(|| {
        let platform = crate::snapshot::install().unwrap();
        let ui = crate::window::create_app_window().unwrap();
        let worker = Worker::start(
            StoreLocation::Memory,
            || Ok(line::backend::FakeBackend::canned()),
            Default::default(),
        );
        let mut d = tests::desktop(&worker);
        ui.show().unwrap();
        for delta in [
            Delta::PairFailed {
                reason: line::client::error::SignInFailure::Network,
            },
            Delta::Connection(Connection::Online),
        ] {
            d.state.panel = Panel::SignIn;
            d.state.connection = Connection::SigningIn;
            ui.global::<AuthState>()
                .set_auth_phase(AuthPhase::Cancelling);
            d.render(&ui);
            crate::snapshot::render(&platform, (980, 720)).unwrap();
            d.apply(&ui, delta);
            // Inspect the sign-in controls before dismissing the panel.
            ui.global::<ShellState>()
                .set_connection(ConnectionState::Offline);
            crate::snapshot::render(&platform, (980, 720)).unwrap();
            let back =
                i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, "Back")
                    .next()
                    .expect("cancel left Back labelled Cancelling");
            assert_eq!(
                back.accessible_enabled(),
                Some(true),
                "cancel left Back disabled"
            );
            assert_ne!(
                ui.global::<AuthState>().get_auth_phase(),
                AuthPhase::Cancelling
            );
        }
    })
    .join()
    .unwrap();
}
