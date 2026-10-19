use line::{controller::AppState, store::Content};

pub(crate) fn text(content: &Content, state: &AppState, sender: &str) -> Option<String> {
    let Content::System { kind, params, .. } = content else {
        return None;
    };
    let groups: Vec<Vec<&str>> = params
        .as_deref()
        .unwrap_or_default()
        .split('\u{1e}')
        .map(|group| group.split('\u{1f}').collect())
        .collect();
    let arg = |index: usize| {
        groups
            .get(index)
            .and_then(|g| g.first())
            .copied()
            .filter(|s| !s.is_empty())
    };
    let name = |mid: &str| {
        if mid == state.profile.mid {
            return "You".to_owned();
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
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "Unknown member".into())
    };
    let actor = || arg(0).map(name);
    let result = match kind.as_deref()? {
        "C_PN" => format!("{} renamed the group to “{}”", actor()?, arg(1)?),
        "C_PI" => format!("{} changed the group photo", actor()?),
        "C_MJ" | "A_MJ" => format!("{} joined the group", actor()?),
        "C_ML" | "A_ML" => format!("{} left the group", actor()?),
        "C_GI" | "C_MI" | "A_MC" | "A_MI" => {
            let invitees = groups
                .get(1)?
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| name(s))
                .collect::<Vec<_>>();
            if invitees.is_empty() {
                return None;
            }
            format!("{} invited {}", actor()?, invitees.join(", "))
        }
        "C_MR" | "A_MR" => format!("{} removed {} from the group", actor()?, name(arg(1)?)),
        "C_IC" | "A_IC" => format!("{} canceled the invitation for {}", actor()?, name(arg(1)?)),
        "C_SP" => format!("{} allowed joining via link or QR code", actor()?),
        "C_SN" => format!("{} disabled joining via link or QR code", actor()?),
        "BB" => format!(
            "{} renamed album “{}” to “{}”",
            name(sender),
            arg(0)?,
            arg(1)?
        ),
        "BD" => format!("{} deleted album “{}”", name(sender), arg(0)?),
        "BO" => format!("{} removed photos from “{}”", name(sender), arg(0)?),
        "C_OL" => {
            let invitees = groups
                .get(1)?
                .iter()
                .map(|mid| name(mid))
                .collect::<Vec<_>>();
            format!("{} cannot join any more chats", invitees.join(", "))
        }
        "C_BG" => "This chat was upgraded to a group".into(),
        "pseudo_leave_chat" => format!("{} left the chat", arg(0)?),
        _ => return None,
    };
    Some(result.replace(['\n', '\r'], " "))
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
