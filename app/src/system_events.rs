use line::{controller::AppState, store::Content};

pub(crate) fn text(content: &Content, state: &AppState, sender: &str) -> Option<String> {
    line::system_events::parse(content).map(|event| event.text(sender, |mid| name(state, mid)))
}

pub(crate) fn name(state: &AppState, mid: &str) -> String {
    if mid == state.profile.mid {
        return "You".into();
    }
    state
        .contacts
        .get(mid)
        .or_else(|| state.friends.iter().find(|f| f.id == mid))
        .or_else(|| {
            state
                .members
                .iter()
                .find(|m| m.contact.id == mid)
                .map(|m| &m.contact)
        })
        .map(|f| crate::present::single_line(&f.name))
        .unwrap_or_else(|| "Unknown member".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use line::store::Friend;

    #[test]
    fn extension_events_resolve_argument_groups_and_unknown_events_stay_hidden() {
        let mut state = AppState::default();
        for (id, name) in [("aoi", "Aoi"), ("ren", "Ren"), ("mei", "Mei")] {
            state.contacts.insert(
                id.into(),
                Friend {
                    id: id.into(),
                    name: name.into(),
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
