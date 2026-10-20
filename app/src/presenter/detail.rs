use super::*;

impl Presenter {
    pub(crate) fn present_members(&mut self, ui: &AppWindow, state: &AppState) {
        let g = ui.global::<DetailState>();
        let mut members: Vec<_> = state.members.iter().collect();
        members.sort_by_key(|m| (m.contact.profile.invited, !m.is_self));
        let query = Query::new(&g.get_member_query());
        self.members.replace(
            members
                .into_iter()
                .filter_map(|m| {
                    let contact = if m.is_self {
                        state.contact(&m.contact.id).unwrap_or(&m.contact)
                    } else {
                        &m.contact
                    };
                    if !query.matches([names::member_name(contact.name.as_deref())]) {
                        return None;
                    }
                    Some((
                        contact.id.clone(),
                        (
                            contact.clone(),
                            m.is_self,
                            state.friends.iter().any(|f| f.id == contact.id && !f.group)
                                || contact.profile.relation == Some(0),
                        ),
                        contact.avatar.iter().map(|a| a.key()).collect(),
                    ))
                })
                .collect(),
        );
    }
    pub(super) fn present_detail(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        detail_error: String,
    ) {
        let g = ui.global::<DetailState>();
        g.set_detail_depth(state.detail.len() as i32);
        g.set_invitee_count(
            state
                .members
                .iter()
                .filter(|m| m.contact.profile.invited)
                .count() as i32,
        );
        g.set_detail_busy(state.action.is_some());
        g.set_detail_running(
            state
                .action
                .as_ref()
                .map(|a| match a {
                    line::backend::ProfileAction::Rename { .. } => DetailAction::Rename,
                    line::backend::ProfileAction::AddFriend { .. } => DetailAction::Add,
                    line::backend::ProfileAction::Mute { .. } => DetailAction::Mute,
                    line::backend::ProfileAction::Leave { .. } => DetailAction::Leave,
                })
                .unwrap_or(DetailAction::None),
        );
        g.set_detail_error(
            state
                .action_error
                .as_ref()
                .map(text::profile_failure)
                .map(str::to_owned)
                .unwrap_or(detail_error)
                .into(),
        );
        if let Some(page) = state.detail_page() {
            use line::controller::DetailPage;
            let (id, kind) = match page {
                DetailPage::Profile(id) => (id.as_str(), DetailPageMode::Profile),
                DetailPage::GroupInfo(id) => (id.0.as_str(), DetailPageMode::Group),
                DetailPage::Members(id) => (id.0.as_str(), DetailPageMode::Members),
            };
            g.set_detail_page(kind);
            let room = ChatId(id.into()).is_room();
            let f = state.contact(id);
            let c = state.chats.get(&ChatId(id.into())).map(|c| &c.chat);
            let name = if kind != DetailPageMode::Profile {
                names::chat(state, &ChatId(id.into()), "Group")
            } else if let Some(friend) = f.filter(|_| state.profile_friend(id)) {
                names::friend_name(friend)
            } else {
                names::member_name(f.and_then(|f| f.name.as_deref()))
            };
            let room_title = state
                .members
                .iter()
                .filter(|m| !m.is_self && !m.contact.profile.invited)
                .map(|m| names::member_name(m.contact.name.as_deref()))
                .collect::<Vec<_>>()
                .join(", ");
            let name = if kind != DetailPageMode::Profile && room && !room_title.is_empty() {
                room_title.as_str()
            } else {
                name
            };
            let status = f.map(|f| f.status.as_str()).unwrap_or("");
            let friend = kind == DetailPageMode::Profile && state.profile_friend(id);
            g.set_profile_id(id.into());
            g.set_profile_name(name.into());
            g.set_profile_status(status.into());
            g.set_profile_initial(name_initial(name));
            g.set_profile_friend(friend);
            g.set_profile_addable(kind == DetailPageMode::Profile && state.profile_addable(id));
            g.set_profile_available(kind == DetailPageMode::Profile && state.profile_available(id));
            g.set_profile_original(
                f.and_then(|f| {
                    let original = f.profile.original_name.as_deref()?;
                    (original != name).then_some(original)
                })
                .unwrap_or("")
                .into(),
            );
            g.set_profile_favourite(f.is_some_and(|f| f.profile.favorite_time.unwrap_or(0) > 0));
            g.set_profile_notice(
                if !friend
                    && state
                        .notices
                        .get(id)
                        .is_some_and(|v| v["notice"].as_object().is_some_and(|o| !o.is_empty()))
                {
                    "Be careful when sharing personal information or sending money."
                } else {
                    ""
                }
                .into(),
            );
            g.set_profile_name_runs(rich_content::runs(name, &[]));
            g.set_profile_status_runs(rich_content::runs(status, &[]));
            let avatar = profile_avatar(state, id);
            g.set_profile_avatar(self.images.picture(avatar.as_ref()));
            g.set_group_count(
                c.and_then(|c| c.member_count)
                    .map(|n| n as i32)
                    .unwrap_or(-1),
            );
            g.set_group_muted(c.is_some_and(|c| c.muted));
            g.set_group_left(c.is_some_and(|c| c.left));
            g.set_group_room(room);
            g.set_group_writable(
                state
                    .group_info
                    .get(&ChatId(id.into()))
                    .is_some_and(|v| v.is_object()),
            );
        }
    }
}

pub(crate) fn profile_avatar(state: &AppState, id: &str) -> Option<Asset> {
    state
        .contact(id)
        .and_then(|f| f.avatar.clone())
        .or_else(|| {
            state
                .chats
                .get(&ChatId(id.into()))
                .and_then(|c| c.chat.avatar.clone())
        })
}
