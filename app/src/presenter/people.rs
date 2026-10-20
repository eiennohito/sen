use super::*;

#[derive(Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Keep Friend inline instead of allocating it every present.
pub(super) enum FriendEntry {
    Header(&'static str, usize, bool),
    Contact(Friend),
}

impl Presenter {
    pub(crate) fn present_people(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<FriendsState>();
        let query = Query::new(&g.get_friend_query());
        let friends: Vec<_> = state
            .friends
            .iter()
            .filter(|f| query.matches([names::friend_name(f)], &mut self.search_fields))
            .collect();
        self.status_assets = friends
            .iter()
            .flat_map(|f| emoji_spans(&f.status, &f.profile.status_metadata))
            .filter_map(|span| Asset::inline_emoji(&span))
            .collect();
        let mut grouped = Vec::new();
        for (group, label, expanded) in [
            (true, "Groups", g.get_groups_expanded()),
            (false, "Friends", g.get_friends_expanded()),
        ] {
            let rows: Vec<_> = friends.iter().filter(|f| f.group == group).collect();
            if state.friends.is_empty() || (!query.is_empty() && rows.is_empty()) {
                continue;
            }
            grouped.push((
                format!("header-{label}"),
                FriendEntry::Header(label, rows.len(), group),
                Vec::new(),
            ));
            if expanded {
                grouped.extend(rows.into_iter().map(|f| {
                    (
                        f.id.clone(),
                        FriendEntry::Contact((*f).clone()),
                        friend_assets(f),
                    )
                }));
            }
        }
        self.friends.replace(grouped);
    }
}

fn friend_assets(friend: &Friend) -> Vec<String> {
    friend
        .avatar
        .iter()
        .map(|a| a.key())
        .chain(
            emoji_spans(&friend.status, &friend.profile.status_metadata)
                .iter()
                .filter_map(Asset::inline_emoji)
                .map(|a| a.key()),
        )
        .collect()
}

pub(super) fn friend_row(entry: FriendEntry, images: &Images) -> FriendRow {
    match entry {
        FriendEntry::Header(label, count, group) => FriendRow {
            id: format!("header-{label}").into(),
            name: label.into(),
            count: count as i32,
            header: true,
            group,
            ..Default::default()
        },
        FriendEntry::Contact(f) => {
            let metadata = emoji_spans(&f.status, &f.profile.status_metadata)
                .into_iter()
                .map(EmojiView::new)
                .collect::<Vec<_>>();
            FriendRow {
                id: f.id.clone().into(),
                name: single_line(names::friend_name(&f)).into(),
                initial: name_initial(names::friend_name(&f)),
                status: single_line(&f.status).into(),
                status_runs: rich_content::preview::preview_runs(&f.status, &metadata, images),
                group: f.group,
                avatar: images.picture(f.avatar.as_ref()),
                ..Default::default()
            }
        }
    }
}
pub(super) fn member_row(
    (contact, is_self, friend): (Friend, bool, bool),
    images: &Images,
) -> MemberRow {
    MemberRow {
        id: contact.id.into(),
        name: single_line(names::member_name(contact.name.as_deref())).into(),
        initial: name_initial(names::member_name(contact.name.as_deref())),
        status: single_line(&contact.status).into(),
        is_self,
        friend,
        invited: contact.profile.invited,
        avatar: images.picture(contact.avatar.as_ref()),
    }
}
