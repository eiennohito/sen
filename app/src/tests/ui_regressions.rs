use super::*;

#[test]
fn keyboard_rings_follow_buttons_and_whole_friend_rows() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use i_slint_core::{
            item_tree::{ItemRc, ParentItemTraversalMode},
            items::{BasicBorderRectangle, Clip},
        };
        use slint::platform::{Key, PointerEventButton, WindowEvent};
        use slint::private_unstable_api::re_exports::WindowInner;
        i_slint_backend_testing::init_no_event_loop();
        let (ui, _runtime, _worker) =
            test_window(line::backend::FakeBackend::canned(), |_, _, _| {});
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        let weak = ui.as_weak();
        ui.global::<NewChatState>().on_open_new_chat(move || {
            weak.upgrade()
                .unwrap()
                .global::<NewChatState>()
                .set_new_chat_open(true)
        });
        ui.global::<NewChatState>().set_new_chat_friends(
            Rc::new(VecModel::from(vec![ComposeFriend {
                id: "friend".into(),
                name: "Friend".into(),
                ..Default::default()
            }]))
            .into(),
        );
        let toggled = Rc::new(std::cell::Cell::new(0));
        let recorded = toggled.clone();
        ui.global::<NewChatState>().on_new_chat_toggle(move |id| {
            assert_eq!(id, "friend");
            recorded.set(recorded.get() + 1);
        });
        let control = |label| {
            ElementHandle::find_by_accessible_label(&ui, label)
                .find(|item| {
                    item.accessible_role() != Some(i_slint_backend_testing::AccessibleRole::Text)
                })
                .unwrap()
        };
        let key = |text: slint::SharedString| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        let rings = || {
            slint::platform::update_timers_and_animations();
            let mut borders = Vec::new();
            ItemRc::new_root(WindowInner::from_pub(ui.window()).component()).visit_descendants(
                |item| {
                    if item.is_visible()
                        && let Some(rect) = item.downcast::<BasicBorderRectangle>()
                    {
                        let rect = rect.as_pin_ref();
                        if rect.border_width().get() > 0. {
                            borders.push(item.clone());
                        }
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                },
            );
            borders
        };
        let surrounds = |label, baseline: &[ItemRc]| {
            let element = control(label);
            let p = element.absolute_position();
            let s = element.size();
            rings()
                .iter()
                .filter(|item| !baseline.contains(item))
                .any(|item| {
                    let origin = item.map_to_window(Default::default());
                    let size = item.geometry().size;
                    origin.x <= p.x
                        && origin.y <= p.y
                        && origin.x + size.width >= p.x + s.width
                        && origin.y + size.height >= p.y + s.height
                })
        };
        control("New chat").mock_single_click(PointerEventButton::Left);
        control("Friend").mock_single_click(PointerEventButton::Left);
        let row_baseline = rings();
        control("Find friends for new chat").mock_single_click(PointerEventButton::Left);
        assert!(
            surrounds("Find friends for new chat", &row_baseline),
            "clicked text field has no border"
        );
        let field_baseline = rings();
        assert_eq!(
            field_baseline
                .iter()
                .filter(|item| !row_baseline.contains(item))
                .count(),
            1
        );
        key("f".into());
        assert_eq!(rings(), field_baseline, "typing added a ring after a click");
        key(Key::Backspace.into());
        key(Key::Escape.into());
        control("New chat").mock_single_click(PointerEventButton::Left);
        let dialog_baseline = rings();
        key(Key::Escape.into());
        control("Find a conversation").mock_single_click(PointerEventButton::Left);
        let main_baseline = rings();
        ui.invoke_focus_search();
        // Tab follows visual order.
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
        key(Key::Tab.into());
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
        assert_eq!(focus_label(&ui), "New chat");
        assert!(surrounds("New chat", &main_baseline));
        key(Key::Return.into());
        assert!(
            surrounds("Find friends for new chat", &row_baseline),
            "keyboard opening lost the ring"
        );
        key(Key::Escape.into());
        assert!(
            surrounds("Find a conversation", &row_baseline),
            "keyboard dismissal lost the ring"
        );
        control("New chat").mock_single_click(PointerEventButton::Left);
        assert_eq!(rings(), dialog_baseline, "pointer opening left a ring");
        key(Key::Tab.into());
        assert_eq!(focus_label(&ui), "Friend");
        let row = ElementHandle::find_by_element_type_name(&ui, "ListRow")
            .next()
            .unwrap();
        assert!(
            control("Friend").size().width >= row.size().width,
            "checkbox remained an inner stop"
        );
        assert!(
            surrounds("Friend", &dialog_baseline),
            "friend row did not ring as a whole"
        );
        for ring in rings()
            .iter()
            .filter(|item| !dialog_baseline.contains(item))
        {
            let bounds = ring.geometry().size;
            let p = ring.map_to_window(Default::default());
            let mut ancestor = ring.parent_item(ParentItemTraversalMode::StopAtPopups);
            let mut clipped = false;
            while let Some(item) = ancestor {
                if let Some(clip) = item.downcast::<Clip>()
                    && clip.as_pin_ref().clip()
                {
                    clipped = true;
                    let origin = item.map_to_window(Default::default());
                    let size = item.geometry().size;
                    assert!(
                        p.x >= origin.x
                            && p.y >= origin.y
                            && p.x + bounds.width <= origin.x + size.width
                            && p.y + bounds.height <= origin.y + size.height,
                        "row ring extends outside its clip"
                    );
                }
                ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
            }
            assert!(clipped, "row ring has no Pane clip");
        }
        control("Friend").mock_single_click(PointerEventButton::Left);
        assert_eq!(rings(), row_baseline, "pointer row selection left a ring");
        for modifier in [
            Key::Shift,
            Key::ShiftR,
            Key::Control,
            Key::ControlR,
            Key::Alt,
            Key::AltGr,
            Key::Meta,
            Key::MetaR,
            Key::CapsLock,
        ] {
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: modifier.into(),
            });
            assert_eq!(
                rings(),
                row_baseline,
                "modifier alone added a ring: {modifier:?}"
            );
            if matches!(
                modifier,
                Key::Control | Key::ControlR | Key::Meta | Key::MetaR
            ) {
                key("c".into());
                assert_eq!(rings(), row_baseline, "chord added a ring: {modifier:?}");
            }
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: modifier.into(),
            });
            assert_eq!(rings(), row_baseline);
        }
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Alt.into(),
        });
        key(Key::LeftArrow.into());
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Alt.into(),
        });
        assert!(
            surrounds("Friend", &row_baseline),
            "Alt+arrow did not enable rings"
        );
        toggled.set(0);
        key(" ".into());
        key(Key::Return.into());
        assert_eq!(toggled.get(), 2);
        key(Key::Tab.into());
        assert_eq!(focus_label(&ui), "Cancel", "row gained an inner Tab stop");
    })
    .join()
    .unwrap();
}

#[test]
fn new_contact_header_uses_friends_cached_avatar_before_send() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let (ui, _runtime, _worker) =
            test_window(line::backend::FakeBackend::canned(), |_, _, _| {});
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let asset = line::client::desktop::assets::Asset::avatar("/friend").unwrap();
        let friend = Friend {
            id: "friend".into(),
            name: Some("Friend".into()),
            avatar: Some(asset.clone()),
            ..Default::default()
        };
        let mut state = AppState::default();
        state.friends.push(friend.clone());
        state.contacts.insert(friend.id.clone(), friend);
        state.select(ChatId("friend".into()));
        presenter.present(&ui, &state, ChatCategory::All);
        let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &[0, 180, 100, 255],
            1,
            1,
        ));
        presenter.images.insert(asset.key(), image.clone());
        presenter.patch_asset(&ui, &state, &asset.key());
        assert!(state.chats.is_empty());
        assert_eq!(ui.global::<ConversationState>().get_chat_title(), "Friend");
        let friends = ui.global::<FriendsState>().get_friends();
        let friend_row = friends.iter().find(|r| r.id == "friend").unwrap();
        assert_eq!(friend_row.avatar_key, asset.key());
        assert_eq!(friend_row.avatar, image);
        assert_eq!(
            ui.global::<ConversationState>().get_chat_avatar(),
            friend_row.avatar
        );

        // The conversation arrives after the friend's picture is already cached.
        let id = ChatId("friend".into());
        state.chat_order.push(id.clone());
        state.chats.insert(
            id.clone(),
            ChatListItem {
                chat: ChatInput {
                    id,
                    avatar: Some(line::client::desktop::assets::Asset::avatar("/chat").unwrap()),
                    ..Default::default()
                },
                unread_count: 0,
                preview: None,
                activity_ms: 0,
                failed_count: 0,
                draft: None,
            },
        );
        presenter.present(&ui, &state, ChatCategory::All);
        let chat = ui.global::<ChatsState>().get_chats().row_data(0).unwrap();
        assert_eq!(chat.avatar_key, asset.key());
        assert_eq!(chat.avatar, image);
        assert_eq!(
            ui.global::<ConversationState>().get_chat_avatar(),
            chat.avatar
        );
    })
    .join()
    .unwrap();
}

// #53: this must also run in the product build, without `demo`.
#[test]
fn detail_focus_restores_the_opener_without_demo() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        for already_open in [false, true] {
            search(&ui).set_accessible_value("");
            if already_open {
                ui.global::<ShellState>().set_panel(PanelState::Detail);
                ui.global::<DetailState>().set_detail_depth(1);
                snapshot::render(&window, (980, 720)).unwrap();
            }
            ui.invoke_focus_search();
            focus::remember_detail(&ui, true);
            ui.global::<ShellState>().set_panel(PanelState::Detail);
            ui.global::<DetailState>().set_detail_depth(1);
            if !already_open {
                snapshot::render(&window, (980, 720)).unwrap();
            }
            ui.invoke_focus_profile_close();
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "x".into() });
            assert_eq!(search(&ui).accessible_value().unwrap(), "");
            ui.global::<ShellState>().set_panel(PanelState::None);
            ui.global::<DetailState>().set_detail_depth(0);
            snapshot::render(&window, (980, 720)).unwrap();
            focus::restore_detail(&ui, true);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "x".into() });
            assert_eq!(
                search(&ui).accessible_value().unwrap(),
                "x",
                "closing detail must restore its opener"
            );
        }
        ui.global::<ShellState>().set_account_mid("self".into());
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<ComposerState>()
            .set_composer_mode(ComposerMode::Idle);
        for width in [1280, 420] {
            snapshot::render(&window, (width, 720)).unwrap();
            let mut openers = vec![
                FocusOpener::None,
                FocusOpener::MemberList,
                FocusOpener::GroupMembers,
                FocusOpener::ProfilePhoto,
            ];
            if width == 420 {
                openers.extend([
                    FocusOpener::Search,
                    FocusOpener::Chats,
                    FocusOpener::FriendSearch,
                    FocusOpener::Friends,
                    FocusOpener::Navigation,
                ]);
            }
            for opener in openers {
                ui.invoke_restore_opener(opener);
                assert_eq!(
                    focus_label(&ui),
                    "Message draft",
                    "hidden opener {opener:?} accepted focus at {width}"
                );
                use slint::private_unstable_api::re_exports::WindowInner;
                assert!(
                    WindowInner::from_pub(ui.window())
                        .focus_item
                        .borrow()
                        .upgrade()
                        .unwrap()
                        .is_visible(),
                    "restored focus is hidden"
                );
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn gap_before_unread_message_keeps_its_geometry() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.profile.mid = "self".into();
        state.connection = Connection::Online;
        state.select(ChatId("chat".into()));
        let message = message("First unread message");
        state.gaps.push(Gap {
            before: None,
            after: message.key,
        });
        state.messages.insert(message.key, message);
        let geometry = Rc::new(RefCell::new(None));
        let reported = geometry.clone();
        ui.global::<ConversationState>()
            .on_row_geometry(move |kind, _, top, height| {
                if kind == HistoryRowKind::Gap {
                    *reported.borrow_mut() = Some((top, height));
                }
            });
        presenter.present(&ui, &state, ChatCategory::All);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<ConversationState>().set_history_virtual(true);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        let without_divider = geometry.borrow().expect("gap geometry was not reported");
        ui.global::<ConversationState>()
            .set_unread_divider("1".into());
        snapshot::render(&window, (980, 720)).unwrap();
        assert_eq!(
            geometry.borrow().unwrap(),
            without_divider,
            "unread divider must not shift or shrink the preceding gap"
        );
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn copy_message_preserves_the_selected_text() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        i_slint_backend_testing::init_no_event_loop();
        let (ui, _runtime, _worker) =
            test_window(line::backend::FakeBackend::canned(), |_, _, _| {});
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.select(ChatId("chat".into()));
        let message = message("keep this selection");
        state.messages.insert(message.key, message);
        presenter.present(&ui, &state, ChatCategory::All);
        ui.global::<ShellState>().set_show_conversation(true);
        let copied = Rc::new(RefCell::new(String::new()));
        let capture = copied.clone();
        ui.global::<ContentState>()
            .on_copy_text(move |text| *capture.borrow_mut() = text.to_string());
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();
        let body = ElementHandle::find_by_element_id(&ui, "Message::body")
            .next()
            .unwrap();
        body.mock_single_click(slint::platform::PointerEventButton::Left);
        body.set_accessible_selection_offsets(5, 9);
        body.mock_single_click(slint::platform::PointerEventButton::Right);
        slint::platform::update_timers_and_animations();
        ElementHandle::find_by_accessible_label(&ui, "Copy message")
            .next()
            .expect("message menu did not open")
            .mock_single_click(slint::platform::PointerEventButton::Left);
        slint::platform::update_timers_and_animations();
        assert_eq!(*copied.borrow(), "keep this selection");
        let ctrl = |text: &str| {
            use slint::platform::WindowEvent;
            let keys = [
                slint::platform::Key::Control.into(),
                slint::SharedString::from(text),
            ];
            for text in &keys {
                ui.window()
                    .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            }
            for text in keys.into_iter().rev() {
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text });
            }
        };
        ctrl("c");
        ui.invoke_focus_composer();
        ui.global::<AttachmentState>().on_paste_attachment(|| false);
        ctrl("v");
        assert_eq!(
            ui.global::<ComposerState>().get_draft(),
            "this",
            "Copy message changed the selection"
        );
    })
    .join()
    .unwrap();
}

fn focus_label(ui: &AppWindow) -> String {
    use slint::private_unstable_api::re_exports::{AccessibleStringProperty, WindowInner};
    WindowInner::from_pub(ui.window())
        .focus_item
        .borrow()
        .upgrade()
        .unwrap()
        .accessible_string_property(AccessibleStringProperty::Label)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn members_search_keeps_alt_arrows_for_editing() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use slint::platform::{Key, WindowEvent};
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(1280., 900.));
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<DetailState>()
            .set_detail_page(DetailPageMode::Members);
        slint::platform::update_timers_and_animations();
        let switches = Rc::new(std::cell::Cell::new(0));
        let recorded = switches.clone();
        ui.global::<ChatsState>()
            .on_switch_chat(move |_| recorded.set(recorded.get() + 1));
        ElementHandle::find_by_accessible_label(&ui, "Search members")
            .next()
            .unwrap()
            .mock_single_click(slint::platform::PointerEventButton::Left);
        for text in [Key::Alt.into(), Key::UpArrow.into()] {
            ui.window().dispatch_event(WindowEvent::KeyPressed { text });
        }
        for text in [Key::UpArrow.into(), Key::Alt.into()] {
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        }
        assert_eq!(switches.get(), 0, "Alt+Up in members search switched chat");
        assert_eq!(focus_label(&ui), "Search members");
    })
    .join()
    .unwrap();
}

#[test]
fn notice_banner_f6_leaves_for_history() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use slint::platform::{Key, WindowEvent};
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.global::<ShellState>().set_account_mid("self".into());
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<ShellState>()
            .set_connection(ConnectionState::SignedOut);
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        slint::platform::update_timers_and_animations();
        ElementHandle::find_by_accessible_label(&ui, "Connection needs attention")
            .next()
            .unwrap()
            .mock_single_click(slint::platform::PointerEventButton::Left);
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::F6.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::F6.into(),
        });
        assert_eq!(focus_label(&ui), "Message history");
    })
    .join()
    .unwrap();
}

#[test]
fn narrow_to_medium_detail_keeps_focus_inside_detail() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<DetailState>()
            .set_detail_page(DetailPageMode::Members);
        ui.show().unwrap();
        snapshot::render(&window, (739, 720)).unwrap();
        ui.invoke_focus_profile_close();
        snapshot::render(&window, (740, 720)).unwrap();
        assert_eq!(focus_label(&ui), "Close details");
    })
    .join()
    .unwrap();
}

#[test]
fn viewer_returns_to_current_photo_key_or_history_after_recycling() {
    std::thread::spawn(|| {
        use slint::platform::{Key, WindowEvent};
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        let photo = |key: &str| MessageRow {
            kind: MessageKind::Photo,
            id: key.into(),
            image_key: key.into(),
            has_picture: true,
            picture: slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &[0, 180, 100, 255],
                1,
                1,
            )),
            ..Default::default()
        };
        let rows = Rc::new(slint::VecModel::from(vec![photo("first"), photo("second")]));
        ui.global::<ConversationState>()
            .set_messages(rows.clone().into());
        let opened = Rc::new(RefCell::new(String::new()));
        let recorded = opened.clone();
        ui.global::<ViewerState>()
            .on_open_photo(move |key| *recorded.borrow_mut() = key.to_string());
        let weak = ui.as_weak();
        ui.global::<ShellState>().on_request_panel(move |panel| {
            let ui = weak.upgrade().unwrap();
            ui.global::<ShellState>().set_panel(panel);
            if panel == PanelState::None {
                ui.invoke_focus_history();
            }
        });
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        slint::platform::update_timers_and_animations();
        for recycled in [false, true] {
            ui.invoke_focus_history();
            ui.invoke_show_viewer("first".into());
            slint::platform::update_timers_and_animations();
            ui.global::<ViewerState>().set_viewer_key("second".into());
            if recycled {
                rows.set_row_data(1, photo("replacement"));
            }
            ui.global::<ViewerState>().invoke_close_viewer();
            slint::platform::update_timers_and_animations();
            if recycled {
                assert_eq!(
                    focus_label(&ui),
                    "Message history",
                    "recycled photo received focus for another message"
                );
            } else {
                ui.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Return.into(),
                });
                ui.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Return.into(),
                });
                assert_eq!(
                    *opened.borrow(),
                    "second",
                    "closing after Next focused the original photo"
                );
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn virtual_wheel_frames_update_latest_and_top_paging() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<ConversationState>().set_history_virtual(true);
        ui.global::<ConversationState>().set_messages(
            Rc::new(slint::VecModel::from(
                (0..100)
                    .map(|i| MessageRow {
                        id: i.to_string().into(),
                        body: format!("Wheel message {i}").into(),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        let pages = Rc::new(std::cell::Cell::new(0));
        let recorded = pages.clone();
        let weak = ui.as_weak();
        ui.global::<ConversationState>()
            .on_history_scrolled(move || {
                if weak
                    .upgrade()
                    .unwrap()
                    .global::<ConversationState>()
                    .get_history_y()
                    >= -30.
                {
                    recorded.set(recorded.get() + 1);
                }
            });
        let frame = || {
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(20));
            slint::platform::update_timers_and_animations();
        };
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        for _ in 0..10 {
            frame();
        }
        ui.invoke_history_jump(-100000.);
        for _ in 0..10 {
            frame();
        }
        let bottom = ui.global::<ConversationState>().get_history_y();
        assert!(bottom < -1000.);
        // Wheel frames update content-y after Flickable's single, pre-motion flicked callback.
        ui.global::<ConversationState>()
            .set_history_y(bottom + 120.);
        frame();
        assert!(
            !ui.global::<ConversationState>().get_following(),
            "a wheel frame away from bottom must show Latest"
        );
        ui.global::<ConversationState>().set_new_count(3);
        ui.global::<ConversationState>().set_history_y(bottom);
        frame();
        assert!(
            ui.global::<ConversationState>().get_following(),
            "the final wheel frame must resume following"
        );
        assert_eq!(ui.global::<ConversationState>().get_new_count(), 0);
        ui.global::<ConversationState>().set_history_y(0.);
        frame();
        assert!(
            pages.get() > 0,
            "the top wheel frame must request older history"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn busy_controls_refuse_focus_and_activation() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
        use slint::platform::{Key, WindowEvent};
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(360., 720.));
        slint::platform::update_timers_and_animations();
        let focused = || focus_label(&ui);
        let key = |text: slint::SharedString| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        ui.global::<ShellState>().set_panel(PanelState::SignIn);
        ui.global::<ShellState>()
            .set_connection(ConnectionState::SigningIn);
        ui.global::<AuthState>()
            .set_auth_phase(AuthPhase::Cancelling);
        ui.invoke_focus_search();
        ui.invoke_focus_auth_start();
        assert_ne!(focused(), "Cancelling…", "cancelling Back accepted focus");

        ui.global::<ShellState>().set_panel(PanelState::None);
        ui.global::<ShellState>()
            .set_connection(ConnectionState::SigningIn);
        ui.global::<ShellState>().set_account_mid("self".into());
        ui.invoke_focus_search();
        ui.invoke_focus_connection_first();
        assert_ne!(
            focused(),
            "Sign in again",
            "unavailable sign-in accepted focus"
        );

        ui.global::<ShellState>()
            .set_connection(ConnectionState::Online);
        ui.global::<ComposerState>()
            .set_composer_mode(ComposerMode::Unavailable);
        ui.invoke_focus_search();
        let opener = focused();
        ui.invoke_focus_composer();
        assert_eq!(
            focused(),
            opener,
            "disabled draft moved focus out to the rail"
        );
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<DetailState>().set_profile_friend(true);
        ui.global::<DetailState>().set_detail_busy(true);
        for (page, label) in [
            (DetailPageMode::Profile, "Rename"),
            (DetailPageMode::Group, "Leave group"),
        ] {
            ui.global::<DetailState>().set_detail_page(page);
            slint::platform::update_timers_and_animations();
            ui.invoke_focus_profile_close();
            let focusable = ElementQuery::from_root(&ui)
                .find_all()
                .into_iter()
                .filter(|item| {
                    item.accessible_enabled() != Some(false)
                        && matches!(
                            item.accessible_role(),
                            Some(
                                AccessibleRole::Button
                                    | AccessibleRole::TextInput
                                    | AccessibleRole::Checkbox
                                    | AccessibleRole::List
                                    | AccessibleRole::TabList
                                    | AccessibleRole::Slider
                            )
                        )
                })
                .count();
            for _ in 0..focusable {
                ui.global::<ShellState>().invoke_tab_focus(true);
                assert_ne!(focused(), label, "busy {label} accepted focus");
                if focused() == "Close details" {
                    break;
                }
            }
            assert_eq!(
                focused(),
                "Close details",
                "reverse Tab did not complete the cycle within the focusable controls"
            );
            let control = ElementHandle::find_by_accessible_label(&ui, label)
                .next()
                .unwrap_or_else(|| panic!("missing {label}"));
            assert_eq!(control.accessible_enabled(), Some(false));
            assert!(control.computed_opacity() < 1.);
            control.invoke_accessible_default_action();
            control.mock_single_click(slint::platform::PointerEventButton::Left);
            assert!(
                ElementHandle::find_by_element_id(&ui, "Layers::dialog-title")
                    .next()
                    .is_none()
            );
        }

        ui.global::<ShellState>().set_panel(PanelState::None);
        ui.global::<NewChatState>().set_new_chat_open(true);
        slint::platform::update_timers_and_animations();
        assert_eq!(focus_label(&ui), "Find friends for new chat");
        ui.global::<NewChatState>().set_new_chat_busy(true);
        // Keep Shift held while the dialog attempts to focus its disabled close button.
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
        key(Key::Tab.into());
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
        assert_ne!(focused(), "Close new chat");
        let close = ElementHandle::find_by_accessible_label(&ui, "Close new chat")
            .next()
            .unwrap();
        close.invoke_accessible_default_action();
        close.mock_single_click(slint::platform::PointerEventButton::Left);
        assert!(ui.global::<NewChatState>().get_new_chat_open());
    })
    .join()
    .unwrap();
}

#[test]
fn checkbox_uses_space_but_not_return() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use slint::platform::{Key, WindowEvent};
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        ui.global::<ShellState>().set_panel(PanelState::Settings);
        let checkbox = ElementHandle::find_by_accessible_label(&ui, "Enter sends")
            .next()
            .unwrap();
        checkbox.mock_single_click(slint::platform::PointerEventButton::Left);
        let checked = checkbox.accessible_checked();
        for (text, expected) in [
            (Key::Return.into(), checked),
            (" ".into(), checked.map(|v| !v)),
        ] {
            ui.window().dispatch_event(WindowEvent::KeyPressed { text });
            assert_eq!(checkbox.accessible_checked(), expected);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn dialog_scrim_blocks_wheel_scrolling() {
    std::thread::spawn(|| {
        use slint::platform::WindowEvent;
        let platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        ui.global::<ShellState>().set_account_mid("self".into());
        let chats = (0..30)
            .map(|i| ChatRow {
                title: format!("Chat {i}").into(),
                accessible_label: format!("Chat {i}, ").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        ui.global::<ChatsState>()
            .set_chats(ModelRc::from(Rc::new(VecModel::from(chats))));
        ui.show().unwrap();
        snapshot::render(&platform, (1280, 900)).unwrap();
        let scroll = || {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(200., 400.),
                delta_x: 0.,
                delta_y: -120.,
            })
        };
        let position = || {
            element(&ui, "Conversations")
                .query_descendants()
                .match_predicate(|item| {
                    item.accessible_label()
                        .is_some_and(|label| label.starts_with("Chat 5, "))
                })
                .find_first()
                .expect("Conversations has no visible fixture row")
                .absolute_position()
                .y
        };
        let initial = position();
        scroll();
        assert!(
            position() < initial,
            "wheel fixture did not scroll the underlying list"
        );
        ui.global::<ShellState>().set_panel(PanelState::Settings);
        snapshot::render(&platform, (1280, 900)).unwrap();
        let before = position();
        scroll();
        assert_eq!(
            position(),
            before,
            "wheel leaked through the settings scrim"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn keyboard_navigation_updates_only_the_current_window() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let first = window::create_app_window().unwrap();
        use i_slint_core::context::set_window_event_hook;
        let hook = set_window_event_hook(None).unwrap().unwrap();
        let installed = std::ptr::from_ref(&*hook);
        set_window_event_hook(Some(hook)).unwrap();
        let second = window::create_app_window().unwrap();
        let hook = set_window_event_hook(None).unwrap().unwrap();
        assert!(
            std::ptr::eq(installed, &*hook),
            "recreating a window installed another input hook"
        );
        set_window_event_hook(Some(hook)).unwrap();
        first.global::<ShellState>().set_keyboard_navigation(false);
        second.global::<ShellState>().set_keyboard_navigation(false);
        second
            .window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
        assert!(second.global::<ShellState>().get_keyboard_navigation());
        assert!(
            !first.global::<ShellState>().get_keyboard_navigation(),
            "an old window still receives input hooks"
        );
    })
    .join()
    .unwrap();
}
