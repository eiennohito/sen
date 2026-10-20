use super::*;
use slint::platform::WindowEvent;

#[test]
fn keyboard_rings_follow_input_and_cover_friend_rows() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.global::<NewChatState>()
            .set_new_chat_friends(ModelRc::new(VecModel::from(vec![ComposeFriend {
                id: "friend".into(),
                name: "Friend".into(),
                ..Default::default()
            }])));
        let weak = ui.as_weak();
        ui.global::<NewChatState>().on_open_new_chat(move || {
            weak.unwrap()
                .global::<NewChatState>()
                .set_new_chat_open(true);
        });
        let toggled = Rc::new(std::cell::Cell::new(0));
        let count = toggled.clone();
        ui.global::<NewChatState>().on_new_chat_toggle(move |id| {
            assert_eq!(id, "friend");
            count.set(count.get() + 1);
        });
        ui.global::<NewChatState>().set_new_chat_open(true);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        use slint::platform::Key;
        let pointer = || click_at(&ui, slint::LogicalPosition::new(1., 1.));
        let modifiers = [
            Key::Shift,
            Key::ShiftR,
            Key::Control,
            Key::ControlR,
            Key::Alt,
            Key::AltGr,
            Key::Meta,
            Key::MetaR,
            Key::CapsLock,
        ];
        let events = modifiers.into_iter().map(|key| (key, None, false)).chain([
            (Key::Control, Some("c".into()), false),
            (Key::ControlR, Some("c".into()), false),
            (Key::Meta, Some("c".into()), false),
            (Key::MetaR, Some("c".into()), false),
            (Key::Alt, Some(Key::LeftArrow.into()), true),
        ]);
        for (modifier, key, expected) in events {
            pointer();
            if let Some(key) = key {
                chord(&ui, modifier, key);
            } else {
                press(&ui, modifier);
            }
            assert_eq!(
                ui.global::<ShellState>().get_keyboard_navigation(),
                expected,
                "navigation mode after {modifier:?}"
            );
        }
        press(&ui, Key::Tab);
        pointer();
        assert!(!ui.global::<ShellState>().get_keyboard_navigation());
        for keyboard in [false, true] {
            press(&ui, Key::Escape);
            let opener = element(&ui, "New chat");
            if keyboard {
                ui.invoke_focus_search();
                chord(&ui, Key::Shift, Key::Tab.into());
                assert_eq!(focus_label(&ui), "New chat");
                press(&ui, Key::Return);
            } else {
                let p = opener.absolute_position();
                click_at(&ui, slint::LogicalPosition::new(p.x + 8., p.y + 8.));
            }
            assert!(ui.global::<NewChatState>().get_new_chat_open());
            assert_eq!(
                ui.global::<ShellState>().get_keyboard_navigation(),
                keyboard
            );
            if keyboard {
                press(&ui, Key::Escape);
            } else {
                let close = element(&ui, "Close new chat");
                let p = close.absolute_position();
                click_at(&ui, slint::LogicalPosition::new(p.x + 8., p.y + 8.));
            }
            assert!(!ui.global::<NewChatState>().get_new_chat_open());
            assert_eq!(
                ui.global::<ShellState>().get_keyboard_navigation(),
                keyboard
            );
        }
        ui.global::<NewChatState>().set_new_chat_query("".into());
        ui.global::<NewChatState>().set_new_chat_open(true);
        snapshot::render(&window, (980, 720)).unwrap();
        let row = i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, "Friend")
            .find(|e| {
                e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Checkbox)
            })
            .unwrap();
        let list_row =
            i_slint_backend_testing::ElementHandle::find_by_element_type_name(&ui, "ListRow")
                .next()
                .unwrap();
        assert!(
            row.size().width >= list_row.size().width,
            "checkbox is narrower than its row"
        );
        let p = row.absolute_position();
        let size = row.size();
        let edges = |pixels: slint::SharedPixelBuffer<slint::Rgb8Pixel>| {
            [p.x, p.x + size.width - 1.].map(|x| {
                let pixel = pixels.as_slice()[(p.y + size.height / 2.) as usize * 980 + x as usize];
                (pixel.r, pixel.g, pixel.b)
            })
        };
        let unfocused = edges(snapshot::render(&window, (980, 720)).unwrap());
        click_at(
            &ui,
            slint::LogicalPosition::new(p.x + size.width / 2., p.y + size.height / 2.),
        );
        assert_eq!(
            edges(snapshot::render(&window, (980, 720)).unwrap()),
            unfocused
        );
        let search = element(&ui, "Find friends for new chat");
        let search_position = search.absolute_position();
        click_at(
            &ui,
            slint::LogicalPosition::new(search_position.x + 8., search_position.y + 8.),
        );
        press(&ui, slint::platform::Key::Tab);
        assert_eq!(focus_label(&ui), "Friend");
        let focused = edges(snapshot::render(&window, (980, 720)).unwrap());
        assert!(
            focused
                .iter()
                .zip(unfocused)
                .all(|(after, before)| *after != before)
        );
        assert_eq!(focused[0], focused[1], "ring must surround both row edges");
        use i_slint_core::{
            item_tree::ParentItemTraversalMode,
            items::{BasicBorderRectangle, Clip},
        };
        let mut rings = Vec::new();
        focus_item(&ui).visit_descendants(|item| {
            if let Some(rect) = item.downcast::<BasicBorderRectangle>()
                && rect.as_pin_ref().border_width().get() > 0.
            {
                rings.push(item.clone());
            }
            std::ops::ControlFlow::<()>::Continue(())
        });
        let ring = rings.first().expect("focused row has no ring");
        let p = ring.map_to_window(Default::default());
        let size = ring.geometry().size;
        let mut ancestor = ring.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut clipped = false;
        while let Some(item) = ancestor {
            if let Some(clip) = item.downcast::<Clip>()
                && clip.as_pin_ref().clip()
            {
                clipped = true;
                let origin = item.map_to_window(Default::default());
                let bounds = item.geometry().size;
                assert!(
                    p.x >= origin.x
                        && p.y >= origin.y
                        && p.x + size.width <= origin.x + bounds.width
                        && p.y + size.height <= origin.y + bounds.height,
                    "row ring escapes Pane clip"
                );
            }
            ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        assert!(clipped, "row ring has no Pane clip");
        toggled.set(0);
        for (key, expected) in [(slint::SharedString::from(" "), 1), (Key::Return.into(), 2)] {
            press(&ui, key);
            assert_eq!(toggled.get(), expected);
        }
        press(&ui, Key::Tab);
        assert_eq!(focus_label(&ui), "Cancel");
    })
    .join()
    .unwrap();
}

#[test]
fn new_contact_header_uses_friends_cached_avatar_before_send() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let asset = sen::client::desktop::assets::Asset::avatar("/friend").unwrap();
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
        let image = green_image();
        let path = crate::tests::image_file(&image);
        let image = slint::Image::load_from_path(&path).unwrap();
        presenter.images.insert(asset.key(), path);
        presenter.patch_asset(&ui, &state, &asset.key());
        assert!(state.chats.is_empty());
        assert_eq!(ui.global::<ConversationState>().get_chat_title(), "Friend");
        let friends = ui.global::<FriendsState>().get_friends();
        let friend_row = friends.iter().find(|r| r.id == "friend").unwrap();
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
            chat_item(ChatInput {
                id,
                avatar: Some(sen::client::desktop::assets::Asset::avatar("/chat").unwrap()),
                ..Default::default()
            }),
        );
        presenter.present(&ui, &state, ChatCategory::All);
        let chat = ui.global::<ChatsState>().get_chats().row_data(0).unwrap();
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
            press(&ui, "x");
            assert_eq!(search(&ui).accessible_value().unwrap(), "");
            ui.global::<ShellState>().set_panel(PanelState::None);
            ui.global::<DetailState>().set_detail_depth(0);
            snapshot::render(&window, (980, 720)).unwrap();
            focus::restore_detail(&ui, true);
            press(&ui, "x");
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
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
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

#[test]
fn members_search_keeps_alt_arrows_for_editing() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use slint::platform::Key;
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
        use slint::platform::Key;
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
fn viewer_returns_to_current_photo_key_or_history_after_recycling() {
    std::thread::spawn(|| {
        use slint::platform::Key;
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        let photo = |key: &str| MessageRow {
            kind: MessageKind::Photo,
            id: key.into(),
            image_key: key.into(),
            has_picture: true,
            picture: green_image(),
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
            advance(20);
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
        use slint::platform::Key;
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(360., 720.));
        ui.global::<ShellState>().set_panel(PanelState::SignIn);
        ui.global::<ShellState>()
            .set_connection(ConnectionState::SigningIn);
        ui.global::<AuthState>()
            .set_auth_phase(AuthPhase::Cancelling);
        ui.invoke_focus_search();
        ui.invoke_focus_auth_start();
        assert_ne!(focus_label(&ui), "Cancelling…");
        ui.global::<ShellState>().set_panel(PanelState::None);
        ui.global::<ShellState>().set_account_mid("self".into());
        ui.invoke_focus_search();
        ui.invoke_focus_connection_first();
        assert!(!ui.global::<ShellState>().get_sign_in_available());
        assert!(!visible(&ui, "Sign in again"));
        assert_ne!(focus_label(&ui), "Sign in again");
        ui.global::<ShellState>()
            .set_connection(ConnectionState::Online);
        ui.global::<ComposerState>()
            .set_composer_mode(ComposerMode::Unavailable);
        ui.invoke_focus_search();
        let opener = focus_label(&ui);
        ui.invoke_focus_composer();
        assert_eq!(
            focus_label(&ui),
            opener,
            "unavailable composer pulled focus to the rail"
        );
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<DetailState>()
            .set_detail_page(DetailPageMode::Group);
        ui.global::<DetailState>().set_detail_busy(true);
        slint::platform::update_timers_and_animations();
        ui.invoke_focus_profile_close();
        press(&ui, Key::Tab);
        assert_eq!(focus_label(&ui), "Open profile photo");
        press(&ui, Key::Tab);
        assert_eq!(focus_label(&ui), "Members (0)");
        press(&ui, Key::Tab);
        assert_eq!(focus_label(&ui), "Close details");
        let leave = element(&ui, "Leave group");
        assert_eq!(leave.accessible_enabled(), Some(false));
        leave.invoke_accessible_default_action();
        leave.mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::None
        );
        ui.global::<ShellState>().set_panel(PanelState::None);
        ui.global::<NewChatState>().set_new_chat_open(true);
        slint::platform::update_timers_and_animations();
        assert_eq!(focus_label(&ui), "Find friends for new chat");
        ui.global::<NewChatState>().set_new_chat_busy(true);
        chord(&ui, Key::Shift, Key::Tab.into());
        assert_ne!(focus_label(&ui), "Close new chat");
        let close = element(&ui, "Close new chat");
        assert_eq!(close.accessible_enabled(), Some(false));
        close.invoke_accessible_default_action();
        assert!(ui.global::<NewChatState>().get_new_chat_open());
        let p = close.absolute_position();
        let size = close.size();
        click_at(
            &ui,
            slint::LogicalPosition::new(p.x + size.width / 2., p.y + size.height / 2.),
        );
        assert!(ui.global::<NewChatState>().get_new_chat_open());
    })
    .join()
    .unwrap();
}

#[test]
fn busy_profile_action_ignores_click_enter_and_tab() {
    std::thread::spawn(|| {
        i_slint_backend_testing::init_no_event_loop();
        let (ui, _runtime, _worker) =
            test_window(sen::backend::FakeBackend::canned(), |_, _, _| {});
        ui.window().set_size(slint::LogicalSize::new(360., 720.));
        ui.global::<ShellState>().set_panel(PanelState::Detail);
        ui.global::<DetailState>().set_detail_depth(1);
        ui.global::<DetailState>().set_profile_friend(true);
        ui.global::<DetailState>().set_profile_available(true);
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();
        ui.invoke_focus_profile_close();
        for expected in ["Close details", "Open profile photo", "Chat", "Rename"] {
            press(&ui, slint::platform::Key::Tab);
            assert_eq!(focus_label(&ui), expected);
        }
        ui.global::<DetailState>().set_detail_busy(true);
        press(&ui, slint::platform::Key::Return);
        let rename = element(&ui, "Rename");
        assert_eq!(rename.accessible_enabled(), Some(false));
        rename.mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::None
        );
        ui.invoke_focus_profile_close();
        for expected in [
            "Close details",
            "Open profile photo",
            "Chat",
            "Profile more",
        ] {
            press(&ui, slint::platform::Key::Tab);
            assert_eq!(focus_label(&ui), expected);
        }
        ui.global::<DetailState>().set_detail_busy(false);
        rename.mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(focus_label(&ui), "Name");
        assert_eq!(
            ui.global::<DetailState>().get_detail_dialog(),
            DetailAction::Rename
        );
    })
    .join()
    .unwrap();
}

#[test]
fn checkbox_uses_space_but_not_return() {
    std::thread::spawn(|| {
        use i_slint_backend_testing::ElementHandle;
        use slint::platform::Key;
        i_slint_backend_testing::init_no_event_loop();
        let ui = window::create_app_window().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        ui.global::<ShellState>().set_panel(PanelState::Settings);
        let checkbox = ElementHandle::find_by_accessible_label(&ui, "Enter sends")
            .next()
            .unwrap();
        checkbox.mock_single_click(slint::platform::PointerEventButton::Left);
        let checked = checkbox.accessible_checked().unwrap();
        for (key, expected) in [
            (slint::SharedString::from(" "), !checked),
            (Key::Return.into(), !checked),
            (slint::SharedString::from(" "), checked),
        ] {
            press(&ui, key);
            assert_eq!(checkbox.accessible_checked(), Some(expected));
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

slint::slint! {
    import { Button } from "../../ui/button.slint";
    import { IconButton } from "../../ui/icon-button.slint";
    export component DisabledControlProbe inherits Window {
        background: white;
        Button { x: 16px; y: 16px; width: 64px; background: black; enabled: false; }
        Button { x: 112px; y: 16px; width: 64px; background: black; }
        IconButton {
            x: 16px; y: 80px; background: black; icon-color: white;
            icon: @image-url("../../assets/icons/rename.svg"); enabled: false;
        }
        IconButton {
            x: 112px; y: 80px; background: black; icon-color: white;
            icon: @image-url("../../assets/icons/rename.svg");
        }
    }
}

#[test]
fn disabled_controls_keep_their_dimmed_colours() {
    std::thread::spawn(|| {
        let platform = snapshot::install().unwrap();
        let ui = DisabledControlProbe::new().unwrap();
        ui.show().unwrap();
        let pixels = snapshot::render(&platform, (200, 128)).unwrap();
        let pixel = |x: usize, y: usize| {
            let p = pixels.as_slice()[y * 200 + x];
            (p.r, p.g, p.b)
        };
        assert_eq!(pixel(32, 24), (153, 153, 153));
        assert_eq!(pixel(128, 24), (0, 0, 0));
        assert_eq!(pixel(20, 84), (153, 153, 153));
        assert_eq!(pixel(116, 84), (0, 0, 0));
        let mut icon_ink = 0;
        for y in 86..106 {
            for x in 118..138 {
                if pixel(x, y) == (255, 255, 255) {
                    icon_ink += 1;
                    let dimmed = pixel(x - 96, y);
                    assert!(dimmed.0 > 153 && dimmed.0 < 255);
                    assert_eq!(dimmed.0, dimmed.1);
                    assert_eq!(dimmed.0, dimmed.2);
                }
            }
        }
        assert!(icon_ink > 5, "idle icon has no fully coloured ink");
    })
    .join()
    .unwrap();
}
