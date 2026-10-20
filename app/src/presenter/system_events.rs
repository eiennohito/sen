use sen::{controller::AppState, store::Content, system_events::Event};

pub(crate) fn text(content: &Content, state: &AppState, sender: &str) -> Option<String> {
    sen::system_events::parse(content)
        .map(|event| event_text(&event, sender, |mid| name(state, mid)))
}

pub(crate) fn name(state: &AppState, mid: &str) -> String {
    crate::presenter::format::single_line(super::names::person(state, mid))
}

fn event_text(event: &Event<'_>, sender: &str, name: impl Fn(&str) -> String) -> String {
    match event {
        Event::Rename(a, title) => format!("{} renamed the group to “{title}”", name(a)),
        Event::Photo(a) => format!("{} changed the group photo", name(a)),
        Event::Join(a) => format!("{} joined the group", name(a)),
        Event::Leave(a) => format!("{} left the group", name(a)),
        Event::Invite(a, others) => format!(
            "{} invited {}",
            name(a),
            others
                .iter()
                .map(|m| name(m))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Event::Remove(a, b) => format!("{} removed {} from the group", name(a), name(b)),
        Event::Cancel(a, b) => {
            format!("{} canceled the invitation for {}", name(a), name(b))
        }
        Event::Link(a, enabled) => format!(
            "{} {} joining via link or QR code",
            name(a),
            if *enabled { "allowed" } else { "disabled" }
        ),
        Event::AlbumRename(a, b) => format!("{} renamed album “{a}” to “{b}”", name(sender)),
        Event::AlbumDelete(a) => format!("{} deleted album “{a}”", name(sender)),
        Event::AlbumPhotos(a) => format!("{} removed photos from “{a}”", name(sender)),
        Event::Limit(others) => format!(
            "{} cannot join any more chats",
            others
                .iter()
                .map(|m| name(m))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Event::Upgrade => "This chat was upgraded to a group".into(),
        Event::LeaveChat(a) => format!("{a} left the chat"),
    }
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sen::store::Friend;

    #[test]
    fn extension_events_resolve_argument_groups_and_unknown_events_stay_hidden() {
        let mut state = AppState::default();
        for (id, name) in [("aoi", "Aoi"), ("ren", "Ren"), ("mei", "Mei")] {
            state.contacts.insert(
                id.into(),
                Friend {
                    id: id.into(),
                    name: Some(name.into()),
                    ..Default::default()
                },
            );
        }
        for (kind, params, expected) in [
            (
                "C_PN",
                "aoi\u{1e}Weekend plans",
                "Aoi renamed the group to “Weekend plans”",
            ),
            ("C_GI", "aoi\u{1e}ren\u{1f}mei", "Aoi invited Ren, Mei"),
            ("A_MJ", "ren", "Ren joined the group"),
            ("C_ML", "mei", "Mei left the group"),
            ("C_PI", "aoi", "Aoi changed the group photo"),
        ] {
            assert_eq!(
                text(
                    &Content::System {
                        kind: Some(kind.into()),
                        params: Some(params.into()),
                        text: None
                    },
                    &state,
                    "aoi"
                )
                .as_deref(),
                Some(expected)
            );
        }
        assert!(
            text(
                &Content::System {
                    kind: Some("unknown".into()),
                    params: None,
                    text: Some("System message".into())
                },
                &state,
                "aoi"
            )
            .is_none()
        );
    }
}
