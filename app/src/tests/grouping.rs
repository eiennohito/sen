use super::*;

#[test]
fn grouping_uses_identity_time_and_delivery() {
    std::thread::spawn(|| {
        use chrono::TimeZone;
        use line_desktop_ui::RunPosition::{First, Last, Single};
        let midnight = chrono::Local
            .with_ymd_and_hms(2026, 10, 16, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let message = |id: u64, sender: &str, time| {
            let mut m = super::message("Hello");
            m.key = Key(id as i64);
            m.server_id = Some(ServerId::new(id.to_string()).unwrap());
            m.sort.sort_ts = time;
            m.sender_id = sender.into();
            m
        };
        let mut source = vec![
            message(1, "a", midnight - 600_001),
            message(2, "a", midnight - 300_002),
            message(3, "a", midnight - 1),
            message(4, "b", midnight - 1),
            message(5, "b", midnight),
        ];
        let _platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        let mut group = |source: &[StoredMessage], gap: Option<Key>| {
            state.messages.clear();
            for row in source {
                state.messages.insert(row.key, row.clone());
            }
            state.gaps = gap
                .into_iter()
                .map(|after| line::store::Gap {
                    after,
                    before: None,
                })
                .collect();
            presenter.present(&ui, &state, ChatCategory::All);
            ui.global::<ConversationState>()
                .get_messages()
                .iter()
                .collect::<Vec<_>>()
        };
        let mut rows = group(&source, None);
        assert_eq!(
            rows.iter().map(|m| m.run_position).collect::<Vec<_>>(),
            [First, Last, Single, Single, Single]
        );
        assert!(!rows[0].day_label.is_empty());
        assert!(rows[1..4].iter().all(|row| row.day_label.is_empty()));
        assert!(!rows[4].day_label.is_empty());
        assert_eq!(rows[4].clock, "00:00");
        source[3].outgoing = true;
        source[4].outgoing = true;
        source[4].sender_id.clear();
        source[4].sort.sort_ts = source[3].sort.sort_ts;
        rows = group(&source, None);
        assert_eq!((rows[3].run_position, rows[4].run_position), (First, Last));
        for delivery in [
            Delivery::Pending,
            Delivery::Rejected {
                reason: line::backend::Refusal::FileMissing,
            },
        ] {
            source[4].delivery = delivery;
            rows = group(&source, None);
            assert_eq!(
                (rows[3].run_position, rows[4].run_position),
                (Single, Single)
            );
        }
        for message in &mut source[3..] {
            message.outgoing = false;
            message.sender_id.clear();
            message.delivery = Delivery::Sent;
        }
        rows = group(&source, None);
        assert_eq!(
            (rows[3].run_position, rows[4].run_position),
            (Single, Single)
        );
        rows = group(&source[..2], Some(source[1].key));
        assert!(rows[..3].iter().all(|row| row.run_position == Single));
    })
    .join()
    .unwrap();
}

#[test]
fn own_member_uses_account_profile_and_keeps_you_note() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let (ui, _runtime, _worker) =
            test_window(line::backend::FakeBackend::canned(), |_, _, _| {});
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.apply(Delta::Profile(Profile {
            mid: "self".into(),
            display_name: "Account name".into(),
            status_message: "Account status".into(),
            picture_path: Some("/own".into()),
        }));
        state.members.push(Member {
            contact: Friend {
                id: "self".into(),
                ..Default::default()
            },
            is_self: true,
        });
        let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(1, 1));
        let avatar = state.contact("self").unwrap().avatar.clone();
        presenter.images.insert(
            avatar.as_ref().unwrap().key(),
            crate::tests::image_file(&image),
        );
        presenter.present(&ui, &state, ChatCategory::All);
        let member = ui
            .global::<DetailState>()
            .get_members()
            .row_data(0)
            .unwrap();
        assert_eq!(member.name, "Account name");
        assert_eq!(member.status, "Account status");
        assert!(member.is_self, "member row must keep its You note");
        assert_eq!(member.avatar, presenter.images.picture(avatar.as_ref()));
        assert_eq!(member.avatar.size(), image.size());
    })
    .join()
    .unwrap();
}

#[test]
fn room_detail_uses_member_names_for_uppercase_wire_mid() {
    std::thread::spawn(|| {
        let _platform = snapshot::install().unwrap();
        let ui = window::create_app_window().unwrap();
        let mut presenter = presenter::Presenter::default();
        presenter.install(&ui);
        let mut state = AppState::default();
        state.push_detail(line::controller::DetailPage::GroupInfo(ChatId(
            "R00000000000000000000000000000001".into(),
        )));
        for (name, is_self, invited) in [
            ("Self", true, false),
            ("Alice", false, false),
            ("Bob", false, false),
            ("Invitee", false, true),
        ] {
            state.members.push(Member {
                is_self,
                contact: Friend {
                    id: name.into(),
                    name: Some(name.into()),
                    profile: line::store::ContactProfile {
                        invited,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            });
        }
        presenter.present(&ui, &state, ChatCategory::All);
        assert_eq!(ui.global::<DetailState>().get_profile_name(), "Alice, Bob");
        assert!(ui.global::<DetailState>().get_group_room());
    })
    .join()
    .unwrap();
}
