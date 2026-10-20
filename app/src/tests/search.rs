use super::*;

#[test]
fn chats_find_katakana_titles_by_hiragana_and_romaji() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        let id = ChatId("ramen".into());
        state.chat_order.push(id.clone());
        state.chats.insert(
            id.clone(),
            chat_item(ChatInput {
                id,
                title: "ラーメン".into(),
                ..Default::default()
            }),
        );
        let chats = ui.global::<ChatsState>();
        for query in ["らあめん", "raamen"] {
            chats.set_chat_query(query.into());
            presenter.present(&ui, &state, ChatCategory::All);
            let rows = chats.get_chats();
            assert_eq!(rows.row_count(), 1, "query {query:?}");
            assert_eq!(rows.row_data(0).unwrap().id, "ramen");
        }
    })
    .join()
    .unwrap();
}

#[test]
fn all_lists_search_names_with_the_same_case_and_whitespace_rules() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.apply(Delta::Profile(Profile {
            mid: "bob".into(),
            display_name: "Bob".into(),
            status_message: "Available".into(),
            picture_path: None,
        }));
        for (id, name) in [("alice", "Alice\n  Smith"), ("bob", "Bob")] {
            let friend = Friend {
                id: id.into(),
                name: Some(name.into()),
                status: "Available".into(),
                ..Default::default()
            };
            state.friends.push(friend.clone());
            state.members.push(Member {
                contact: friend,
                is_self: id == "bob",
            });
            let id = ChatId(id.into());
            state.chat_order.push(id.clone());
            state.chats.insert(
                id.clone(),
                chat_item(ChatInput {
                    id,
                    title: name.into(),
                    ..Default::default()
                }),
            );
        }
        let chats = ui.global::<ChatsState>();
        let friends = ui.global::<FriendsState>();
        let members = ui.global::<DetailState>();
        let new_chat = ui.global::<NewChatState>();
        new_chat.set_new_chat_open(true);
        for (query, expected, picker_expected) in [
            ("  ALIce \t SMith\n", vec!["alice"], vec!["alice"]),
            ("sMiTh", vec!["alice"], vec!["alice"]),
            ("Available", vec![], vec![]),
            ("you", vec![], vec![]),
            (" \t ", vec!["alice", "bob"], vec!["alice"]),
        ] {
            chats.set_chat_query(query.into());
            friends.set_friend_query(query.into());
            members.set_member_query(query.into());
            new_chat.set_new_chat_query(query.into());
            presenter.present(&ui, &state, ChatCategory::All);
            let lists = [
                chats
                    .get_chats()
                    .iter()
                    .map(|r| r.id.to_string())
                    .collect::<Vec<_>>(),
                friends
                    .get_friends()
                    .iter()
                    .filter(|r| !r.header)
                    .map(|r| r.id.to_string())
                    .collect(),
                members
                    .get_members()
                    .iter()
                    .map(|r| r.id.to_string())
                    .collect(),
                new_chat
                    .get_new_chat_friends()
                    .iter()
                    .map(|r| r.id.to_string())
                    .collect(),
            ];
            for (list, mut ids) in ["chats", "friends", "members", "new chat"]
                .into_iter()
                .zip(lists)
            {
                ids.sort();
                let expected = if list == "new chat" {
                    &picker_expected
                } else {
                    &expected
                };
                assert_eq!(&ids, expected, "{list}, query {query:?}");
            }
        }
    })
    .join()
    .unwrap();
}
