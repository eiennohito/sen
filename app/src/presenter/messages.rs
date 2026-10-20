use super::system_events;
use super::*;

impl Presenter {
    pub(super) fn row(&mut self, m: &StoredMessage, state: &AppState) -> MessageRow {
        let sender = state.contact(&m.sender_id).cloned();
        let system_text = system_events::text(&m.body, state, &m.sender_id);
        let reply = m
            .reply
            .as_ref()
            .and_then(|r| state.reply_target(&r.message_id))
            .map(|target| {
                (
                    target.clone(),
                    system_events::name(state, &target.sender_id),
                )
            });
        let inputs = MessageInputs {
            message: m.clone(),
            reactors: m
                .reactions
                .iter()
                .map(|r| {
                    (
                        system_events::name(state, &r.actor),
                        state.contact(&r.actor).cloned(),
                    )
                })
                .collect(),
            own_mid: state.profile.mid.clone(),
            sender: sender.clone(),
            // Only an unnamed sender needs the friend lookup for its fallback.
            sender_friend: sender.as_ref().is_some_and(|s| s.name.is_none())
                && state.profile_friend(&m.sender_id),
            system_text: system_text.clone(),
            reply,
            upload: state.uploads.get(&m.key).copied().unwrap_or(0.),
        };
        if let Some((old, row)) = self.rendered.get(&m.key)
            && *old == inputs
        {
            return row.clone();
        }
        let mut row = message_row(m, system_text.as_deref());
        if !m.body.is_withdrawn() {
            row.reactions = ModelRc::new(VecModel::from(reaction_views(
                &m.reactions,
                |id| {
                    let key = state
                        .contact(id)
                        .and_then(|c| c.avatar.as_ref())
                        .map(|a| a.key())
                        .unwrap_or_default();
                    let name = system_events::name(state, id);
                    line_desktop_ui::ReactorRow {
                        id: id.into(),
                        initial: name_initial(if id == state.profile.mid {
                            text::member_name(state.contact(id).and_then(|c| c.name.as_deref()))
                        } else {
                            &name
                        }),
                        name: name.into(),
                        avatar_key: key.clone().into(),
                        avatar: self.images.get(&key).cloned().unwrap_or_default(),
                    }
                },
                &state.profile.mid,
            )));
        }
        row.sender_name = sender
            .as_ref()
            .map(|s| {
                if inputs.sender_friend {
                    text::friend_name(s)
                } else {
                    text::member_name(s.name.as_deref())
                }
                .to_owned()
            })
            .unwrap_or_else(|| {
                if m.sender_id.is_empty() {
                    "Unknown sender".into()
                } else {
                    let start = m
                        .sender_id
                        .char_indices()
                        .rev()
                        .nth(5)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    format!("Member · {}", &m.sender_id[start..])
                }
            })
            .into();
        row.sender_key = sender
            .as_ref()
            .and_then(|s| s.avatar.as_ref())
            .map(|a| a.key())
            .unwrap_or_default()
            .into();
        row.delivery = match &m.delivery {
            Delivery::Pending => DeliveryState::Pending,
            Delivery::Sent => DeliveryState::Sent,
            Delivery::Read => DeliveryState::Read,
            Delivery::Rejected { .. } => DeliveryState::Rejected,
        };
        if let Some(_item) = m
            .body
            .image()
            .or_else(|| m.body.attachment())
            .and_then(|asset| asset.local_attachment())
        {
            row.local_media = true;
            row.upload_progress = state.uploads.get(&m.key).copied().unwrap_or(0.);
            row.uploading = m.delivery == Delivery::Pending && row.upload_progress < 1.;
            row.file_missing = matches!(
                m.delivery,
                Delivery::Rejected {
                    reason: line::backend::Refusal::FileMissing
                }
            );
        }
        row.can_react = m.can_react();
        row.reaction_count = m.reactions.len() as i32;
        row.own_reaction = m
            .reactions
            .iter()
            .find(|r| r.actor == state.profile.mid)
            .and_then(|r| {
                if let ReactionKind::Predefined(n) = r.kind {
                    Some(n as i32)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        row.can_unsend = m.can_unsend();
        row.recoverable = matches!(m.delivery, Delivery::Rejected { .. });
        row.delivery_detail = match &m.delivery {
            Delivery::Rejected { reason } => format!("Not sent: {}", text::rejection(reason)),
            _ => String::new(),
        }
        .into();
        if let Some(image) = self.images.get(row.image_key.as_str()) {
            row.picture = image.clone();
            row.has_picture = true;
        }
        if let Some(image) = self.images.get(row.sender_key.as_str()) {
            row.sender_avatar = image.clone();
            row.has_sender_avatar = true;
        }
        if let Some(failure) = self.failures.get(row.image_key.as_str()) {
            row.image_failed = true;
            row.image_error = text::image_text(*failure).into();
            row.image_retryable = failure.retry_read();
        }
        if let Some(runs) = row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>() {
            for i in 0..runs.row_count() {
                let mut run = runs.row_data(i).unwrap();
                if let Some(picture) = self.images.get(run.image_key.as_str()) {
                    run.picture = picture.clone();
                    run.has_picture = true;
                    runs.set_row_data(i, run);
                }
            }
        }
        if let Some(reactions) = row
            .reactions
            .as_any()
            .downcast_ref::<VecModel<ReactionRow>>()
        {
            for i in 0..reactions.row_count() {
                let mut reaction = reactions.row_data(i).unwrap();
                if let Some(image) = self.images.get(reaction.image_key.as_str()) {
                    reaction.picture = image.clone();
                    reaction.has_picture = true;
                    reactions.set_row_data(i, reaction);
                }
            }
        }
        if let Some((target, _)) = &inputs.reply {
            let quoted = self.quote(target, state);
            row.reply_sender = quoted.sender_name;
            row.reply_sender_id = quoted.sender_id;
            row.reply_preview = single_line(
                &system_events::text(&target.body, state, &target.sender_id)
                    .unwrap_or_else(|| message_text(&target.body).1),
            )
            .into();
            row.reply_image_key = quoted.image_key;
            row.reply_picture = quoted.picture;
            row.reply_has_picture = quoted.has_picture && (quoted.photo || quoted.sticker);
        }
        rich_content::links::decorate_message(&mut row);
        self.rendered.insert(m.key, (inputs, row.clone()));
        row
    }
    pub(super) fn present_messages(&mut self, ui: &AppWindow, state: &AppState) {
        let rows = state
            .rows()
            .into_iter()
            .filter(|m| line::system_events::displayable(&m.body))
            .collect::<Vec<_>>();
        // Choose on first population, then retain the mode for this conversation.
        // Switching delegate types during a live arrival would lose selection
        // and the current reading anchor at the size boundary.
        if self.history_chat != state.selected
            && ui.global::<State>().get_link_feedback() == text::LINK_FAILURE
        {
            ui.global::<State>().set_link_feedback("".into());
        }
        if self.messages.row_count() == 0 || self.history_chat != state.selected {
            ui.global::<State>().set_history_virtual(rows.len() > 60);
            self.history_chat = state.selected.clone();
        }
        let mut messages = rows.iter().map(|m| self.row(m, state)).collect::<Vec<_>>();

        let group = state
            .selected
            .as_ref()
            .and_then(|id| state.chats.get(id))
            .is_some_and(|c| c.chat.is_group);
        if !group {
            for row in &mut messages {
                row.sender_name = "".into();
                row.has_sender_avatar = false;
                row.sender_avatar = Default::default();
            }
        }
        let mut projected = Vec::new();
        let mut source = Vec::new();
        for (m, row) in rows.iter().zip(messages) {
            if state.gaps.iter().any(|g| g.after == m.key) {
                source.push(None);
                projected.push((
                    RowKey::Gap(m.key),
                    MessageRow {
                        id: format!("gap-{}", m.key.0).into(),
                        row_kind: HistoryRowKind::Gap,
                        body: "Loading messages…".into(),
                        kind: MessageKind::System,
                        ..Default::default()
                    },
                ));
            }
            source.push(Some(*m));
            projected.push((RowKey::Message(m.key), row));
        }
        let (keys, mut messages): (Vec<_>, Vec<_>) = projected.into_iter().unzip();
        group_messages(&mut messages, &source);
        patch(
            &self.messages,
            &mut self.message_keys,
            keys.into_iter().zip(messages).collect(),
        );
        self.rendered.retain(|k, _| {
            state.messages.contains_key(k)
                || state.reply_targets.values().any(|m| m.key == *k)
                || state
                    .chats
                    .values()
                    .any(|c| c.preview.as_ref().is_some_and(|m| m.key == *k))
        });
    }
    pub(super) fn present_history(&mut self, ui: &AppWindow, state: &AppState) {
        ui.global::<State>().set_unread_divider(
            state
                .unread_divider
                .map(|k| k.0.to_string())
                .unwrap_or_default()
                .into(),
        );
        ui.global::<State>().set_older_status(
            if state.reaches_start {
                "Beginning of conversation"
            } else {
                ""
            }
            .into(),
        );
        ui.global::<State>()
            .set_history_can_follow(state.reaches_end);
        ui.global::<State>().set_unread_conversations(
            state.chats.values().filter(|c| c.unread_count > 0).count() as i32,
        );
        ui.global::<State>().set_unread_stale(
            state.connection != Connection::Online
                || state.chats.values().any(|c| c.chat.unread_stale),
        );
    }
}

pub(crate) fn compact_text(content: &Content, state: &AppState, sender: &str) -> String {
    system_events::text(content, state, sender).unwrap_or_else(|| kind_label(content).1)
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn reaction_chips_group_assets_counts_and_reactors() {
        let mut state = AppState::default();
        state.profile.mid = "self".into();
        state.contacts.insert(
            "friend".into(),
            Friend {
                id: "friend".into(),
                name: Some("Aoi".into()),
                ..Default::default()
            },
        );
        let asset = line::client::desktop::assets::Asset::avatar("/reaction").unwrap();
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
        let mut presenter = presenter::Presenter::default();
        let row = presenter.row(&m, &state);
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
            presenter
                .row(&m, &state)
                .reactions
                .row_data(0)
                .unwrap()
                .reactors
                .row_data(0)
                .unwrap()
                .name,
            "Renamed"
        );
    }

    #[test]
    fn own_reactor_avatar_follows_profile_contact_and_profile_changes() {
        let mut state = AppState::default();
        let mut presenter = presenter::Presenter::default();
        let mut message = crate::tests::message("Own reaction");
        message.reactions = vec![Reaction {
            actor: "self".into(),
            kind: ReactionKind::Predefined(3),
        }];
        for (name, picture) in [("Offline preview", None), ("Renamed", Some("/own-photo"))] {
            state.apply(Delta::Profile(line::store::Profile {
                mid: "self".into(),
                display_name: name.into(),
                picture_path: picture.map(str::to_owned),
                ..Default::default()
            }));
            let contact = state.contact("self").unwrap();
            let key = contact.avatar.as_ref().map(|a| a.key()).unwrap_or_default();
            let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
            if picture.is_some() {
                presenter.images.insert(key.clone(), image.clone());
            }
            let row = presenter.row(&message, &state);
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
                name_initial(text::member_name(contact.name.as_deref()))
            );
            assert_eq!(actor.avatar_key, key);
            assert_eq!(
                actor.avatar.size(),
                if picture.is_some() {
                    image.size()
                } else {
                    slint::Image::default().size()
                }
            );
        }
    }

    #[test]
    fn reply_rows_keep_full_text_while_composer_quotes_are_compact() {
        for (body, compact, full) in crate::tests::previews::label_cases() {
            let mut target = crate::tests::message("");
            target.body = body;
            target.server_id = Some(line::store::ServerId::new("10").unwrap());
            let mut reply = crate::tests::message("reply");
            reply.key = Key(2);
            reply.reply = Some(Box::new(line::store::ReplyReference {
                message_id: "10".into(),
                service_code: None,
            }));
            let mut state = AppState::default();
            state.messages.insert(target.key, target.clone());
            let mut presenter = Presenter::default();
            assert_eq!(presenter.quote(&target, &state).body, compact);
            assert_eq!(presenter.row(&reply, &state).reply_preview, full);
        }
    }

    #[test]
    fn sender_fallback_tracks_friend_relation_even_when_contact_is_unchanged() {
        let mut state = AppState::default();
        let contact = Friend {
            id: "sender".into(),
            ..Default::default()
        };
        state.contacts.insert(contact.id.clone(), contact.clone());
        let mut message = crate::tests::message("hello");
        message.sender_id = contact.id.clone();
        let mut presenter = Presenter::default();
        assert_eq!(
            presenter.row(&message, &state).sender_name,
            "Unknown member"
        );
        assert_eq!(system_events::name(&state, "sender"), "Unknown member");
        state.friends.push(contact);
        assert_eq!(
            presenter.row(&message, &state).sender_name,
            "Unnamed friend"
        );
        assert_eq!(system_events::name(&state, "sender"), "Unnamed friend");
    }
}
