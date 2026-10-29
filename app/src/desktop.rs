use crate::*;

pub(crate) struct Desktop {
    pub(crate) clipboard_root: std::path::PathBuf,
    pub(crate) history: crate::reading::Reading,
    pub(crate) shown_composer: String,
    pub(crate) reveal: Option<ChatId>,
    pub(crate) viewer_name: String,
    pub(crate) attachments: callbacks::attachments::Selection,
    pub(crate) viewer_asset: Option<String>,
    pub(crate) viewer_source: Option<std::path::PathBuf>,
    pub(crate) viewer_file_name: String,
    pub(crate) media_open: Option<media::MediaOpen>,
    pub(crate) notifications: Option<notifications::Notifications>,
    pub(crate) background: desktop_env::background::Background,
    pub(crate) assets: assets::Assets,
    pub(crate) state: AppState,
    pub(crate) presenter: presenter::Presenter,
    pub(crate) sender: Sender,
    pub(crate) category: ChatCategory,
}
mod deltas;
mod media;
mod reading;
mod viewer;

impl Desktop {
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

        self.presenter.present(ui, &self.state, self.category);
        self.refresh_photo_navigation(ui);
        if let Some(notifications) = &self.notifications {
            notifications
                .set_unread_count(ui.global::<ShellState>().get_unread_conversations() as u64);
        }

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
            self.open_conversation(ui, id);
        }
    }
    pub(crate) fn open_conversation(&mut self, ui: &AppWindow, chat: ChatId) {
        if self.state.selected.as_ref() != Some(&chat) {
            self.state.detail.clear();
            self.state.panel = Panel::Chats;
        }
        self.reveal = Some(chat.clone());
        let shell = ui.global::<ShellState>();
        shell.set_section(Section::Chats);
        shell.set_show_conversation(true);
        self.switch_chat(ui, chat);
        ui.invoke_focus_composer();
    }
    pub(crate) fn switch_chat(&mut self, ui: &AppWindow, chat: ChatId) {
        if let Some(n) = &self.notifications {
            n.withdraw(&chat.0);
        }
        self.capture_draft(ui);
        self.shown_composer = self.state.composer.clone();
        self.sender.send(self.state.select(chat.clone()));
        // Restore now so switching away before Delta::Draft arrives cannot save an empty draft.
        let draft = self.state.chats.get(&chat).and_then(|c| c.draft.clone());
        if let Some(draft) = &draft {
            self.state.composer = draft.text.clone();
            self.state.composer_reply = draft.reply.clone();
        }
        self.history.reset_for_chat();
        let conversation = ui.global::<ConversationState>();
        conversation.set_return_visible(false);
        conversation.set_highlight_message("".into());
        conversation.invoke_show_history(0., false);
        self.render(ui);
        let (anchor, cursor) = draft
            .map(|d| (d.anchor as i32, d.cursor as i32))
            .unwrap_or_default();
        ui.invoke_restore_editor(anchor, cursor);
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
                updated: sen::engine::now_ms(),
            };
            if let Some(item) = self.state.chats.get_mut(chat) {
                item.draft = Some(draft.clone());
            }
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

    #[test]
    fn profile_chat_positions_unread_divider_and_clears_previous_jump() {
        chat_open_positions_unread_divider("profile");
    }

    #[test]
    fn new_chat_positions_unread_divider_and_clears_previous_jump() {
        chat_open_positions_unread_divider("new chat");
    }

    #[test]
    fn member_chat_positions_unread_divider_and_clears_previous_jump() {
        chat_open_positions_unread_divider("member");
    }

    fn chat_open_positions_unread_divider(source: &'static str) {
        std::thread::spawn(move || {
            let fake = sen::backend::FakeBackend::canned();
            let chat = ChatId("unread-profile".into());
            fake.add_chat(ChatInput {
                id: chat.clone(),
                title: "Unread friend".into(),
                server_unread: 2,
                last_message_id: Some(ServerId::new("8106").unwrap()),
                ..Default::default()
            });
            fake.set_page(
                chat.clone(),
                (8101..=8106)
                    .map(|id| {
                        sen::backend::FakeBackend::message(
                            "unread-profile",
                            id,
                            "Incoming message",
                            false,
                        )
                    })
                    .collect(),
            );
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, worker) = crate::tests::test_window(fake, |d, worker, ui| {
                while d.state.connection != Connection::Online || !d.state.chats.contains_key(&chat)
                {
                    d.state.apply(
                        worker
                            .deltas
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap(),
                    );
                }
                d.history.return_anchor = Some((Key(99), 10.));
                d.history.restore_anchor = Some((Key(99), 10.));
                d.history.jump_target = Some(ServerId::new("99").unwrap());
                d.state.push_detail(DetailPage::Profile(chat.0.clone()));
                ui.global::<ConversationState>().set_return_visible(true);
                ui.global::<ConversationState>()
                    .set_highlight_message("99".into());
            });
            match source {
                "profile" => ui.global::<DetailState>().invoke_start_friend_chat(),
                "member" => ui
                    .global::<DetailState>()
                    .invoke_detail_action(DetailAction::MemberChat, chat.0.clone().into()),
                "new chat" => {
                    runtime.update(|d| {
                        d.state.friends.push(Friend {
                            id: chat.0.clone(),
                            name: Some("Unread friend".into()),
                            ..Default::default()
                        })
                    });
                    ui.global::<NewChatState>().invoke_open_new_chat();
                    ui.global::<NewChatState>()
                        .invoke_new_chat_toggle(chat.0.clone().into());
                    ui.global::<NewChatState>().invoke_new_chat_submit();
                    assert!(!ui.global::<NewChatState>().get_new_chat_open());
                }
                _ => unreachable!(),
            }
            runtime.update(|d| {
                while d.state.window_loading || d.state.messages.len() != 6 {
                    let delta = worker
                        .deltas
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    d.apply_batch(&ui, [delta]);
                }
                assert_eq!(d.state.selected.as_ref(), Some(&chat));
                assert!(d.state.detail.is_empty());
                assert!(d.history.return_anchor.is_none());
                assert!(d.history.restore_anchor.is_none());
                assert!(d.history.jump_target.is_none());
                assert!(d.state.unread_divider.is_some());
            });
            assert!(
                !ui.global::<ConversationState>()
                    .get_unread_divider()
                    .is_empty()
            );
            assert!(!ui.global::<ConversationState>().get_return_visible());
            assert!(
                ui.global::<ConversationState>()
                    .get_highlight_message()
                    .is_empty()
            );
            // The history reports the divider position through the same callback as Slint rows.
            ui.global::<ConversationState>().invoke_row_geometry(
                HistoryRowKind::Unread,
                "".into(),
                320.,
                24.,
            );
            assert_eq!(ui.global::<ConversationState>().get_history_y(), -320.);
            assert!(!ui.global::<ConversationState>().get_following());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn group_info_from_friends_keeps_friends_section() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |d, _, ui| {
                    d.state.friends.push(Friend {
                        id: "weekend".into(),
                        group: true,
                        ..Default::default()
                    });
                    d.state.panel = Panel::Friends;
                    ui.global::<ShellState>().set_section(Section::Friends);
                });
            ui.global::<DetailState>()
                .invoke_open_profile("weekend".into());
            assert_eq!(ui.global::<ShellState>().get_section(), Section::Friends);
            runtime.update(|d| {
                assert_eq!(d.state.panel, Panel::Detail);
                assert_eq!(d.state.selected, Some(ChatId("weekend".into())));
                assert_eq!(
                    d.state.detail_page(),
                    Some(&DetailPage::GroupInfo(ChatId("weekend".into())))
                );
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn auto_select_keeps_settings_and_focus() {
        std::thread::spawn(|| {
            use slint::private_unstable_api::re_exports::{AccessibleStringProperty, WindowInner};
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|d| {
                d.state.panel = Panel::Settings;
                ui.global::<ShellState>().set_section(Section::Friends);
                ui.show().unwrap();
                d.render(&ui);
                ui.invoke_focus_settings_close();
                let focused = WindowInner::from_pub(ui.window())
                    .focus_item
                    .borrow()
                    .clone();
                assert_eq!(
                    focused
                        .upgrade()
                        .unwrap()
                        .accessible_string_property(AccessibleStringProperty::Label)
                        .unwrap(),
                    "Close settings"
                );
                while d.state.selected.is_none() {
                    let delta = worker
                        .deltas
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    d.apply_batch(&ui, [delta]);
                }
                assert_eq!(ui.global::<ShellState>().get_section(), Section::Friends);
                assert_eq!(d.state.panel, Panel::Settings);
                assert_eq!(ui.global::<ShellState>().get_panel(), PanelState::Settings);
                assert!(*WindowInner::from_pub(ui.window()).focus_item.borrow() == focused);
                assert!(!ui.global::<ShellState>().get_show_conversation());
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn reselect_chat_keeps_group_info_in_wide_window() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let chat = ChatId("weekend".into());
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |d, _, ui| {
                    ui.show().unwrap();
                    ui.window().set_size(slint::LogicalSize::new(1440., 900.));
                    d.state.selected = Some(chat.clone());
                    d.state.chat_order.push(chat.clone());
                    d.state.chats.insert(
                        chat.clone(),
                        crate::tests::chat_item(ChatInput {
                            id: chat.clone(),
                            is_group: true,
                            ..Default::default()
                        }),
                    );
                    d.state.push_detail(DetailPage::GroupInfo(chat.clone()));
                    d.render(ui);
                });
            let panel = runtime.update(|d| d.state.panel.clone());
            ui.global::<ChatsState>().invoke_select_chat(0);
            runtime.update(|d| {
                assert_eq!(d.state.detail_page(), Some(&DetailPage::GroupInfo(chat)));
                assert_eq!(d.state.panel, panel);
            });
            assert!(ui.global::<DetailState>().get_detail_open());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn notification_shows_hidden_window_before_focusing_editor() {
        std::thread::spawn(|| {
            use i_slint_core::{
                item_tree::ItemRc,
                window::{WindowAdapterInternal, WindowInner},
            };
            use slint::platform::{Platform, WindowAdapter, software_renderer::SoftwareRenderer};
            use slint::private_unstable_api::re_exports::AccessibleStringProperty;
            use std::cell::Cell;

            struct Window {
                window: slint::Window,
                renderer: SoftwareRenderer,
                editor_focused: Cell<bool>,
            }
            impl WindowAdapter for Window {
                fn window(&self) -> &slint::Window {
                    &self.window
                }
                fn size(&self) -> slint::PhysicalSize {
                    slint::PhysicalSize::new(980, 720)
                }
                fn renderer(&self) -> &dyn slint::platform::Renderer {
                    &self.renderer
                }
                fn internal(
                    &self,
                    _: i_slint_core::InternalToken,
                ) -> Option<&dyn WindowAdapterInternal> {
                    Some(self)
                }
            }
            impl WindowAdapterInternal for Window {
                fn handle_focus_change(&self, _: Option<ItemRc>, new: Option<ItemRc>) {
                    if new.is_some_and(|item| {
                        item.accessible_string_property(AccessibleStringProperty::Label)
                            .as_deref()
                            == Some("Message draft")
                    }) {
                        assert!(
                            self.window.is_visible(),
                            "editor focus reached a hidden window"
                        );
                        self.editor_focused.set(true);
                    }
                }
            }
            struct Backend(Rc<Window>);
            impl Platform for Backend {
                fn create_window_adapter(
                    &self,
                ) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
                    Ok(self.0.clone())
                }
            }
            let window = Rc::new_cyclic(|weak: &std::rc::Weak<Window>| Window {
                window: slint::Window::new(weak.clone()),
                renderer: SoftwareRenderer::new(),
                editor_focused: Cell::new(false),
            });
            slint::platform::set_platform(Box::new(Backend(window.clone()))).unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
                desktop.state.connection = Connection::Online;
                assert!(!ui.window().is_visible());
                notifications::handle(
                    notifications::worker::Event::OpenChat {
                        chat: "alice".into(),
                    },
                    &ui,
                    desktop,
                );
                assert!(window.editor_focused.get());
                assert!(
                    WindowInner::from_pub(ui.window())
                        .focus_item
                        .borrow()
                        .upgrade()
                        .is_some()
                );
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn notification_open_leaves_settings_and_resets_reading() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|d| {
                d.state.panel = Panel::Settings;
                d.state.push_detail(DetailPage::Profile("alice".into()));
                d.history.restore_anchor = Some((Key(99), 10.));
                d.history.jump_target = Some(ServerId::new("99").unwrap());
                ui.global::<ShellState>().set_section(Section::Friends);
                ui.global::<ShellState>().set_show_conversation(false);
                notifications::handle(
                    notifications::worker::Event::OpenChat {
                        chat: "alice".into(),
                    },
                    &ui,
                    d,
                );
                assert_eq!(ui.global::<ShellState>().get_section(), Section::Chats);
                assert!(ui.global::<ShellState>().get_show_conversation());
                assert_eq!(d.state.panel, Panel::Chats);
                assert_eq!(d.state.selected, Some(ChatId("alice".into())));
                assert!(d.state.detail.is_empty());
                assert!(d.history.opening_history);
                assert!(d.history.restore_anchor.is_none());
                assert!(d.history.jump_target.is_none());
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn own_avatar_appears_in_rail_and_profile_when_asset_arrives() {
        std::thread::spawn(|| {
            use i_slint_backend_testing::ElementHandle;
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, worker, _| {
                    while !matches!(
                        worker
                            .deltas
                            .recv_timeout(std::time::Duration::from_secs(3))
                            .unwrap(),
                        Delta::Connection(Connection::Online)
                    ) {}
                });
            runtime.update(|desktop| {
                ui.show().unwrap();
                ui.window().set_size(slint::LogicalSize::new(980., 720.));
                slint::platform::update_timers_and_animations();
                desktop.state.apply(Delta::Profile(Profile {
                    mid: "self".into(),
                    display_name: "Me".into(),
                    picture_path: Some("/own-photo".into()),
                    ..Default::default()
                }));
                desktop
                    .state
                    .push_detail(sen::controller::DetailPage::Profile("self".into()));
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
            });
        })
        .join()
        .unwrap();
    }
    #[test]
    fn asset_eviction_does_not_mark_desktop_dirty() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
                let avatar =
                    sen::client::desktop::assets::Asset::avatar("/visible-avatar").unwrap();
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
            });
        })
        .join()
        .unwrap();
    }
    #[test]
    fn unchanged_empty_window_finishes_loading() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
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
            });
        })
        .join()
        .unwrap();
    }
    #[test]
    fn profile_viewer_waits_for_original_without_showing_a_circle() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
                let avatar = sen::client::desktop::assets::Asset::avatar("/own-photo").unwrap();
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
            });
        })
        .join()
        .unwrap();
    }
    #[test]
    fn profile_viewer_without_original_keeps_an_empty_placeholder() {
        std::thread::spawn(|| {
            let _window = crate::snapshot::install().unwrap();
            let (ui, runtime, _worker) =
                crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
            runtime.update(|desktop| {
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
            });
        })
        .join()
        .unwrap();
    }
}

#[test]
fn failed_cancel_restores_sign_in_back_action() {
    std::thread::spawn(|| {
        let platform = crate::snapshot::install().unwrap();
        let (ui, runtime, _worker) =
            crate::tests::test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        runtime.update(|d| {
            ui.show().unwrap();
            for delta in [
                Delta::PairFailed {
                    reason: sen::client::error::SignInFailure::Network,
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
        });
    })
    .join()
    .unwrap();
}
