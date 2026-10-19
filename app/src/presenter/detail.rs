use super::*;

impl Presenter {
    pub(super) fn present_detail(
        &mut self,
        ui: &AppWindow,
        state: &AppState,
        detail_error: String,
    ) {
        let g = ui.global::<State>();
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
                    line::backend::ProfileAction::Rename { .. } => "rename",
                    line::backend::ProfileAction::AddFriend { .. } => "add",
                    line::backend::ProfileAction::Mute { .. } => "mute",
                    line::backend::ProfileAction::Leave { .. } => "leave",
                })
                .unwrap_or("")
                .into(),
        );
        g.set_detail_error(
            state
                .action_error
                .map(text::profile_failure)
                .map(str::to_owned)
                .unwrap_or(detail_error)
                .into(),
        );
        g.set_chat_left(
            state
                .selected
                .as_ref()
                .is_some_and(|id| state.chats.get(id).is_some_and(|c| c.chat.left)),
        );
        if let Some(page) = state.detail_page() {
            use line::controller::DetailPage;
            let (id, kind) = match page {
                DetailPage::Profile(id) => (id.as_str(), 0),
                DetailPage::GroupInfo(id) => (id.0.as_str(), 1),
                DetailPage::Members(id) => (id.0.as_str(), 2),
            };
            g.set_detail_page(kind);
            let room = ChatId(id.into()).is_room();
            let own = state.profile_own(id);
            let f = state.contact(id);
            let c = state.chats.get(&ChatId(id.into())).map(|c| &c.chat);
            let name = if kind != 0 {
                c.map(|c| c.title.as_str())
                    .filter(|name| !name.is_empty())
                    .or_else(|| f.and_then(|f| f.name.as_deref()))
                    .unwrap_or("Group")
            } else if let Some(friend) = f.filter(|_| state.profile_friend(id)) {
                text::friend_name(friend)
            } else {
                text::member_name(f.and_then(|f| f.name.as_deref()))
            };
            let room_title = state
                .members
                .iter()
                .filter(|m| !m.is_self && !m.contact.profile.invited)
                .map(|m| text::member_name(m.contact.name.as_deref()))
                .collect::<Vec<_>>()
                .join(", ");
            let name = if kind != 0 && room && !room_title.is_empty() {
                room_title.as_str()
            } else {
                name
            };
            let status = f.map(|f| f.status.as_str()).unwrap_or("");
            let friend = kind == 0 && state.profile_friend(id);
            ui.set_profile_id(id.into());
            ui.set_profile_name(name.into());
            ui.set_profile_status(status.into());
            g.set_profile_initial(name_initial(name));
            g.set_profile_own(own);
            g.set_profile_friend(friend);
            g.set_profile_addable(kind == 0 && state.profile_addable(id));
            ui.set_profile_available(kind == 0 && state.profile_available(id));
            ui.set_profile_group(kind != 0);
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
            ui.set_profile_avatar(
                avatar
                    .as_ref()
                    .and_then(|a| self.images.get(&a.key()))
                    .cloned()
                    .unwrap_or_default(),
            );
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

pub(crate) fn profile_avatar(
    state: &AppState,
    id: &str,
) -> Option<line::client::desktop::assets::Asset> {
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
