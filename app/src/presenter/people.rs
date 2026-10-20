use super::*;

impl Presenter {
    pub(super) fn present_people(&mut self, ui: &AppWindow, state: &AppState) {
        let friends = state
            .friends
            .iter()
            .filter(|f| {
                text::friend_name(f)
                    .to_lowercase()
                    .contains(&ui.global::<State>().get_friend_query().to_lowercase())
            })
            .map(|f| {
                if let Some(cached) = self.rendered_friends.get(&f.id)
                    && cached.friend == *f
                    && cached.size == ui.global::<State>().get_text_size()
                {
                    return cached.row.clone();
                }
                let spans =
                    line::client::desktop::rich::emoji_spans(&f.status, &f.profile.status_metadata);
                let assets = spans
                    .iter()
                    .filter_map(line::client::desktop::assets::Asset::inline_emoji)
                    .collect::<Vec<_>>();
                let metadata = spans.into_iter().map(EmojiView::new).collect::<Vec<_>>();
                let row = FriendRow {
                    id: f.id.clone().into(),
                    name: single_line(text::friend_name(f)).into(),
                    initial: name_initial(text::friend_name(f)),
                    status: single_line(&f.status).into(),
                    status_runs: rich_content::preview::preview_runs(
                        &f.status,
                        &metadata,
                        &self.images,
                        ui.global::<State>().get_text_size(),
                    ),
                    group: f.group,
                    avatar_key: f
                        .avatar
                        .as_ref()
                        .map(|a| a.key())
                        .unwrap_or_default()
                        .into(),
                    avatar: f
                        .avatar
                        .as_ref()
                        .and_then(|a| self.images.get(&a.key()))
                        .cloned()
                        .unwrap_or_default(),
                    ..Default::default()
                };
                self.rendered_friends.insert(
                    f.id.clone(),
                    CachedFriend {
                        friend: f.clone(),
                        size: ui.global::<State>().get_text_size(),
                        row: row.clone(),
                        assets,
                    },
                );
                row
            })
            .collect::<Vec<_>>();
        let mut grouped = Vec::new();
        for (group, label, expanded) in [
            (true, "Groups", ui.global::<State>().get_groups_expanded()),
            (
                false,
                "Friends",
                ui.global::<State>().get_friends_expanded(),
            ),
        ] {
            let rows = friends
                .iter()
                .filter(|row| row.group == group)
                .cloned()
                .collect::<Vec<_>>();
            if state.friends.is_empty()
                || (!ui.global::<State>().get_friend_query().is_empty() && rows.is_empty())
            {
                continue;
            }
            grouped.push(FriendRow {
                id: format!("header-{label}").into(),
                name: label.into(),
                count: rows.len() as i32,
                header: true,
                group,
                ..Default::default()
            });
            if expanded {
                grouped.extend(rows);
            }
        }
        patch(
            &self.friends,
            &mut self.friend_keys,
            grouped.into_iter().map(|r| (r.id.to_string(), r)).collect(),
        );
        let mut ordered_members = state.members.iter().collect::<Vec<_>>();
        ordered_members.sort_by_key(|m| (m.contact.profile.invited, !m.is_self));
        let members = ordered_members
            .into_iter()
            .map(|m| {
                let contact = if m.is_self {
                    state.contact(&m.contact.id).unwrap_or(&m.contact)
                } else {
                    &m.contact
                };
                (m, contact)
            })
            .filter(|(m, contact)| {
                let query = ui.global::<State>().get_member_query().to_lowercase();
                text::member_name(contact.name.as_deref())
                    .to_lowercase()
                    .contains(&query)
                    || contact.status.to_lowercase().contains(&query)
                    || (m.is_self && "you".contains(&query))
            })
            .map(|(m, contact)| MemberRow {
                id: contact.id.clone().into(),
                name: single_line(text::member_name(contact.name.as_deref())).into(),
                initial: name_initial(text::member_name(contact.name.as_deref())),
                status: single_line(&contact.status).into(),
                is_self: m.is_self,
                friend: state.friends.iter().any(|f| f.id == contact.id && !f.group)
                    || contact.profile.relation == Some(0),
                invited: contact.profile.invited,
                avatar_key: contact
                    .avatar
                    .as_ref()
                    .map(|a| a.key())
                    .unwrap_or_default()
                    .into(),
                avatar: contact
                    .avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        patch(
            &self.members,
            &mut self.member_keys,
            members.into_iter().map(|r| (r.id.to_string(), r)).collect(),
        );
    }
}
