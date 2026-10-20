use super::*;

#[test]
fn new_contact_header_uses_friends_cached_avatar_before_send() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = test_window();
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
        presenter.present(&ui, &state, "", ChatCategory::All);
        let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &[0, 180, 100, 255],
            1,
            1,
        ));
        presenter.images.insert(asset.key(), image.clone());
        presenter.patch_asset(&ui, &state, &asset.key());
        assert!(state.chats.is_empty());
        assert_eq!(ui.global::<State>().get_chat_title(), "Friend");
        let friends = ui.global::<State>().get_friends();
        let friend_row = friends.iter().find(|r| r.id == "friend").unwrap();
        assert_eq!(friend_row.avatar_key, asset.key());
        assert_eq!(friend_row.avatar, image);
        assert_eq!(ui.global::<State>().get_chat_avatar(), friend_row.avatar);

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
        presenter.present(&ui, &state, "", ChatCategory::All);
        let chat = ui.global::<State>().get_chats().row_data(0).unwrap();
        assert_eq!(chat.avatar_key, asset.key());
        assert_eq!(chat.avatar, image);
        assert_eq!(ui.global::<State>().get_chat_avatar(), chat.avatar);
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
        ui.invoke_focus_search();
        ui.global::<State>().invoke_remember_detail_focus(true);
        ui.global::<State>().set_panel(PanelState::Detail);
        ui.global::<State>().set_detail_depth(1);
        snapshot::render(&window, (980, 720)).unwrap();
        ui.invoke_focus_profile_close();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "x".into() });
        assert_eq!(ui.global::<State>().get_query(), "");
        ui.global::<State>().set_panel(PanelState::None);
        ui.global::<State>().set_detail_depth(0);
        snapshot::render(&window, (980, 720)).unwrap();
        ui.global::<State>().invoke_restore_detail_focus(true);
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "x".into() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "x".into() });
        assert_eq!(
            ui.global::<State>().get_query(),
            "x",
            "closing detail must restore its opener"
        );
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
        ui.global::<State>()
            .on_row_geometry(move |kind, _, top, height| {
                if kind == HistoryRowKind::Gap {
                    *reported.borrow_mut() = Some((top, height));
                }
            });
        presenter.present(&ui, &state, "", ChatCategory::All);
        ui.global::<State>().set_show_conversation(true);
        ui.global::<State>().set_history_virtual(true);
        ui.show().unwrap();
        snapshot::render(&window, (980, 720)).unwrap();
        let without_divider = geometry.borrow().expect("gap geometry was not reported");
        ui.global::<State>().set_unread_divider("1".into());
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
        let ui = test_window();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.select(ChatId("chat".into()));
        let message = message("keep this selection");
        state.messages.insert(message.key, message);
        presenter.present(&ui, &state, "", ChatCategory::All);
        ui.global::<State>().set_show_conversation(true);
        let copied = Rc::new(RefCell::new(String::new()));
        let capture = copied.clone();
        ui.global::<State>()
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
        ui.global::<State>().on_paste_attachment(|| false);
        ctrl("v");
        assert_eq!(
            ui.global::<State>().get_draft(),
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
        let g = ui.global::<State>();
        g.set_panel(PanelState::Detail);
        g.set_detail_depth(1);
        g.set_detail_page(DetailPageMode::Members);
        slint::platform::update_timers_and_animations();
        let switches = Rc::new(std::cell::Cell::new(0));
        let recorded = switches.clone();
        g.on_switch_chat(move |_| recorded.set(recorded.get() + 1));
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
        let g = ui.global::<State>();
        g.set_has_account(true);
        g.set_has_chat(true);
        g.set_show_conversation(true);
        g.set_connection(ConnectionState::SignedOut);
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
        ui.global::<State>().set_panel(PanelState::Detail);
        ui.global::<State>().set_detail_depth(1);
        ui.global::<State>()
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
        let g = ui.global::<State>();
        g.set_has_chat(true);
        g.set_show_conversation(true);
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
        g.set_messages(rows.clone().into());
        let opened = Rc::new(RefCell::new(String::new()));
        let recorded = opened.clone();
        g.on_open_photo(move |key| *recorded.borrow_mut() = key.to_string());
        let weak = ui.as_weak();
        g.on_request_panel(move |panel| {
            let ui = weak.upgrade().unwrap();
            ui.global::<State>().set_panel(panel);
            if panel == PanelState::None {
                ui.invoke_focus_history();
            }
        });
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(980., 720.));
        slint::platform::update_timers_and_animations();
        for recycled in [false, true] {
            ui.invoke_focus_history();
            g.invoke_show_viewer("first".into());
            slint::platform::update_timers_and_animations();
            g.set_viewer_key("second".into());
            if recycled {
                rows.set_row_data(1, photo("replacement"));
            }
            g.invoke_close_viewer();
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
        let g = ui.global::<State>();
        g.set_has_chat(true);
        g.set_show_conversation(true);
        g.set_history_virtual(true);
        g.set_messages(
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
        g.on_history_scrolled(move || {
            if weak.upgrade().unwrap().global::<State>().get_history_y() >= -30. {
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
        let bottom = g.get_history_y();
        assert!(bottom < -1000.);
        // Wheel frames update content-y after Flickable's single, pre-motion flicked callback.
        g.set_history_y(bottom + 120.);
        frame();
        assert!(
            !g.get_following(),
            "a wheel frame away from bottom must show Latest"
        );
        g.set_new_count(3);
        g.set_history_y(bottom);
        frame();
        assert!(
            g.get_following(),
            "the final wheel frame must resume following"
        );
        assert_eq!(g.get_new_count(), 0);
        g.set_history_y(0.);
        frame();
        assert!(
            pages.get() > 0,
            "the top wheel frame must request older history"
        );
    })
    .join()
    .unwrap();
}
