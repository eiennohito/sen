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
