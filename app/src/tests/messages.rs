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

#[test]
fn unread_flex_and_unsupported_messages_have_history_rows() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();
        let (ui, mut presenter, mut state) = fixture();
        let mut m = message("");
        m.body = sen::client::desktop::flex::parse(&serde_json::json!({
            "contentMetadata":{"ALT_TEXT":"Login notice","FLEX_JSON":{
                "type":"bubble","body":{"type":"box","contents":[
                    {"type":"text","text":"New login"},
                    {"type":"button","action":{"type":"uri","label":"Review","uri":"https://example.org/login"}},
                    {"type":"button","action":{"type":"uri","uri":"line://app/example.org"}}
                ]}}
            }}));
        state.chat_order.push(m.chat_id.clone());
        let item = state.chats.get_mut(&m.chat_id).unwrap();
        item.chat.id = m.chat_id.clone();
        item.unread_count = 1;
        item.preview = Some(m.clone());
        state.unread_divider = Some(m.key);
        let row = present(&ui, &mut presenter, &mut state, &m);
        assert_eq!(row.kind, MessageKind::Flex);
        assert_eq!(row.body, "New login\nReview\nhttps://example.org/login\nline://app/example.org");
        assert!(row.detail.is_empty());
        assert!(row.accessible_label.contains("Login notice\nNew login"));
        assert_eq!(row.links.row_count(), 1);
        assert_eq!(row.links.row_data(0).unwrap().url, "https://example.org/login");
        assert!(!row.rich_runs.iter().any(|r| r.url.starts_with("line:")));
        assert_eq!(
            presenter::messages::compact_text(&m.body, &state, &m.sender_id),
            "Login notice"
        );
        let chat = ui.global::<ChatsState>().get_chats().row_data(0).unwrap();
        assert_eq!(chat.preview, "Login notice");
        assert_eq!(chat.unread, 1);
        m.body = Content::Flex { alt_text: "Alt only".into(), lines: vec![], links: vec![] };
        let row = present(&ui, &mut presenter, &mut state, &m);
        assert_eq!(row.body, "Alt only");
        assert_eq!(row.detail, "Open on your phone");
        for content in [
            Content::Unknown { content_type: 22 },
            Content::System { kind: Some("future".into()), params: None, text: None },
            Content::System { kind: Some("C_MJ".into()), params: None, text: None },
        ] {
            m.body = content;
            let row = present(&ui, &mut presenter, &mut state, &m);
            assert_eq!(row.kind, MessageKind::Unknown);
            assert_eq!(row.body, "This message type isn't supported yet");
        }
    }).join().unwrap();
}

#[test]
fn reaction_artwork_is_shared_by_picker_and_chips_with_emoji_fallback() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();
        let directory = sen::test_support::TempDir::new("reaction-artwork");
        let media = directory.0.join("static/media");
        std::fs::create_dir_all(&media).unwrap();
        // Distinct generated rectangles pin the protocol-to-artwork mapping.
        for number in 1001..=1006 {
            let width = number - 990;
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="16">
                    <path fill="#0050e6" d="M0 0h{width}v16H0z"/>
                </svg>"##
            );
            std::fs::write(
                media.join(format!("icon_{number}_32x32.synthetic.svg")),
                svg,
            )
            .unwrap();
        }
        let ui = crate::window::create_app_window().unwrap();
        let mut presenter = Presenter::default();
        presenter.images.load_reactions(&directory.0);
        presenter.install(&ui);
        let mut state = AppState::default();
        state.select(ChatId("chat".into()));
        let mut message = crate::tests::message("Reactions");
        for (id, width) in [(2, 13), (3, 11), (4, 12), (5, 14), (6, 16), (7, 15)] {
            message.reactions = vec![Reaction {
                actor: "self".into(),
                kind: ReactionKind::Predefined(id),
            }];
            let chip = present(&ui, &mut presenter, &mut state, &message)
                .reactions
                .row_data(0)
                .unwrap();
            let picker = ui
                .global::<ContentState>()
                .get_reaction_kinds()
                .iter()
                .find(|r| r.id == id as i32)
                .unwrap();
            for row in [chip, picker] {
                assert!(row.has_picture, "reaction {id} did not resolve artwork");
                assert_eq!(
                    (row.picture.size().width, row.picture.size().height),
                    (width, 16)
                );
            }
        }
        for contents in [None, Some("invalid svg")] {
            let unavailable = sen::test_support::TempDir::new("reaction-fallback");
            if let Some(contents) = contents {
                let media = unavailable.0.join("static/media");
                std::fs::create_dir_all(&media).unwrap();
                std::fs::write(media.join("icon_1003_32x32.synthetic.svg"), contents).unwrap();
            }
            let mut presenter = Presenter::default();
            presenter.images.load_reactions(&unavailable.0);
            presenter.install(&ui);
            message.reactions = vec![Reaction {
                actor: "self".into(),
                kind: ReactionKind::Predefined(2),
            }];
            let chip = present(&ui, &mut presenter, &mut state, &message)
                .reactions
                .row_data(0)
                .unwrap();
            let picker = ui
                .global::<ContentState>()
                .get_reaction_kinds()
                .row_data(0)
                .unwrap();
            for row in [chip, picker] {
                assert!(!row.has_picture);
                assert_eq!(row.glyph, "👍");
            }
        }
    })
    .join()
    .unwrap();
}
