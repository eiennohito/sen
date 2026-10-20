use super::*;
use presenter::{Presenter, system_events};
use sen::client::desktop::assets::Asset;

fn fixture() -> (AppWindow, Presenter, AppState) {
    let ui = crate::window::create_app_window().unwrap();
    let presenter = Presenter::default();
    presenter.install(&ui);
    let mut state = AppState {
        selected: Some(ChatId("chat".into())),
        ..Default::default()
    };
    state.chats.insert(
        ChatId("chat".into()),
        chat_item(ChatInput {
            is_group: true,
            ..Default::default()
        }),
    );
    (ui, presenter, state)
}
fn present(
    ui: &AppWindow,
    presenter: &mut Presenter,
    state: &mut AppState,
    message: &StoredMessage,
) -> MessageRow {
    state.messages.insert(message.key, message.clone());
    presenter.present(ui, state, ChatCategory::All);
    ui.global::<ConversationState>()
        .get_messages()
        .iter()
        .find(|r| r.id == message.key.0.to_string())
        .unwrap()
}
#[test]
fn reaction_chips_group_assets_counts_and_reactors() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();

        let (ui, mut presenter, mut state) = fixture();
        state.profile.mid = "self".into();
        state.contacts.insert(
            "friend".into(),
            Friend {
                id: "friend".into(),
                name: Some("Aoi".into()),
                ..Default::default()
            },
        );
        let asset = Asset::avatar("/reaction").unwrap();
        let mut m = crate::tests::message("Reactions");
        m.reactions = vec![
            Reaction {
                actor: "friend".into(),
                kind: ReactionKind::Paid(Some(asset.clone())),
            },
            Reaction {
                actor: "self".into(),
                kind: ReactionKind::Paid(Some(asset.clone())),
            },
            Reaction {
                actor: "unknown".into(),
                kind: ReactionKind::Predefined(2),
            },
        ];
        let row = present(&ui, &mut presenter, &mut state, &m);
        assert_eq!(row.reactions.row_count(), 2);
        let paid = row.reactions.row_data(0).unwrap();
        assert_eq!(paid.image_key, asset.key());
        assert_eq!(paid.glyph, "");
        assert_eq!(paid.count, 2);
        assert!(paid.own);
        assert_eq!(
            paid.reactors
                .iter()
                .map(|r| r.name.to_string())
                .collect::<Vec<_>>(),
            ["Aoi", "You"]
        );
        let predefined = row.reactions.row_data(1).unwrap();
        assert_eq!(predefined.glyph, "👍");
        assert_eq!(predefined.count, 1);
        assert!(!predefined.own);
        assert_eq!(
            predefined.reactors.row_data(0).unwrap().name,
            "Unknown member"
        );
        state.contacts.get_mut("friend").unwrap().name = Some("Renamed".into());
        assert_eq!(
            present(&ui, &mut presenter, &mut state, &m)
                .reactions
                .row_data(0)
                .unwrap()
                .reactors
                .row_data(0)
                .unwrap()
                .name,
            "Renamed"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn own_reactor_avatar_follows_profile_contact_and_profile_changes() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();

        let (ui, mut presenter, mut state) = fixture();
        let mut message = crate::tests::message("Own reaction");
        message.reactions = vec![Reaction {
            actor: "self".into(),
            kind: ReactionKind::Predefined(3),
        }];
        for (name, picture) in [("Offline preview", None), ("Renamed", Some("/own-photo"))] {
            state.apply(Delta::Profile(sen::store::Profile {
                mid: "self".into(),
                display_name: name.into(),
                picture_path: picture.map(str::to_owned),
                ..Default::default()
            }));
            let contact = state.contact("self").unwrap().clone();
            let key = contact.avatar.as_ref().map(|a| a.key()).unwrap_or_default();
            let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
            if picture.is_some() {
                presenter
                    .images
                    .insert(key.clone(), crate::tests::image_file(&image));
            }
            let row = present(&ui, &mut presenter, &mut state, &message);
            let actor = row
                .reactions
                .row_data(0)
                .unwrap()
                .reactors
                .row_data(0)
                .unwrap();
            assert_eq!(actor.name, "You");
            assert_eq!(
                actor.initial,
                name_initial(crate::presenter::names::member_name(
                    contact.name.as_deref()
                ))
            );
            assert_eq!(
                actor.avatar.size(),
                if picture.is_some() {
                    image.size()
                } else {
                    slint::Image::default().size()
                }
            );
        }
    })
    .join()
    .unwrap();
}

#[test]
fn reply_rows_keep_full_text_while_composer_quotes_are_compact() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();

        for (body, compact, full) in crate::tests::previews::label_cases() {
            let mut target = crate::tests::message("");
            target.body = body;
            target.server_id = Some(sen::store::ServerId::new("10").unwrap());
            let mut reply = crate::tests::message("reply");
            reply.key = Key(2);
            reply.reply = Some(Box::new(sen::store::ReplyReference {
                message_id: sen::store::ServerId::new("10").unwrap(),
                service_code: None,
            }));
            let (ui, mut presenter, mut state) = fixture();
            state.messages.insert(target.key, target.clone());
            state.composer_reply = reply.reply.as_deref().cloned();
            let row = present(&ui, &mut presenter, &mut state, &reply);
            assert_eq!(
                ui.global::<ComposerState>().get_composer_reply().body,
                compact
            );
            assert_eq!(row.reply_preview, full);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn sender_fallback_tracks_friend_relation_even_when_contact_is_unchanged() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();

        let (ui, mut presenter, mut state) = fixture();
        let contact = Friend {
            id: "sender".into(),
            ..Default::default()
        };
        state.contacts.insert(contact.id.clone(), contact.clone());
        let mut message = crate::tests::message("hello");
        message.sender_id = contact.id.clone();
        for (friend, expected) in [(false, "Unknown member"), (true, "Unnamed friend")] {
            if friend {
                state.friends.push(contact.clone());
            }
            for name in [None, Some(""), Some(" \t\n")] {
                state.contacts.get_mut("sender").unwrap().name = name.map(str::to_owned);
                assert_eq!(
                    present(&ui, &mut presenter, &mut state, &message).sender_name,
                    expected
                );
                assert_eq!(system_events::name(&state, "sender"), expected);
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn blank_contact_card_names_omit_the_separator() {
    for name in [None, Some(""), Some(" \t\n")] {
        let content = Content::Contact {
            mid: "contact".into(),
            name: name.map(str::to_owned),
        };
        assert_eq!(presenter::format::kind_label(&content).1, "Contact");
    }
}

#[test]
fn switching_chats_restores_both_editor_drafts_and_reply() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();
        let fake = sen::backend::FakeBackend::canned();
        let mut app = sen::engine::harness::App::new(fake.clone(), StoreLocation::Memory);
        app.online();
        app.select("alice");
        let key = app.body_key("Hello! 日本語も使えます。");
        let reply = app.state.messages[&key].server_id.clone().unwrap();
        let (ui, runtime, _worker) = test_window(fake, |d, _, ui| {
            d.state = std::mem::take(&mut app.state);
            d.render(ui);
        });
        let chats = ui.global::<ChatsState>();
        let select = |id: &str| {
            let index = chats
                .get_chats()
                .iter()
                .find(|row| row.id == id)
                .unwrap()
                .index;
            chats.invoke_select_chat(index);
        };
        let composer = ui.global::<ComposerState>();
        composer.set_draft("Alice draft".into());
        composer.invoke_reply(key.0.to_string().into());
        select("weekend");
        assert_eq!(composer.get_draft(), "");
        composer.set_draft("Weekend draft".into());
        select("alice");
        assert_eq!(composer.get_draft(), "Alice draft");
        runtime.update(|d| assert_eq!(d.state.composer_reply.as_ref().unwrap().message_id, reply));
        assert_eq!(composer.get_composer_reply().reply_id, reply.as_str());
        select("weekend");
        assert_eq!(composer.get_draft(), "Weekend draft");
        runtime.update(|d| assert!(d.state.composer_reply.is_none()));
    })
    .join()
    .unwrap();
}
