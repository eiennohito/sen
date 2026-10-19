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
        presenter.present(&ui, &state, "", 0);
        let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &[0, 180, 100, 255],
            1,
            1,
        ));
        presenter.images.insert(asset.key(), image.clone());
        presenter.patch_asset(&ui, &state, &asset.key());
        assert!(state.chats.is_empty());
        assert_eq!(ui.get_chat_title(), "Friend");
        let friends = ui.get_friends();
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
        presenter.present(&ui, &state, "", 0);
        let chat = ui.get_chats().row_data(0).unwrap();
        assert_eq!(chat.avatar_key, asset.key());
        assert_eq!(chat.avatar, image);
        assert_eq!(ui.global::<State>().get_chat_avatar(), chat.avatar);
    })
    .join()
    .unwrap();
}
