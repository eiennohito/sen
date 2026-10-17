use crate::*;

impl View {
    pub(crate) fn browse(&mut self, ui: &AppWindow, section: i32) {
        if !(0..=1).contains(&section) {
            return;
        }
        ui.set_section(section);
        ui.set_show_conversation(false);
        if section == 1 && !self.friends_loaded && !ui.get_loading_friends() {
            self.load_friends(ui);
        }
    }
    pub(crate) fn load_friends(&self, ui: &AppWindow) {
        if self.demo {
            return;
        }
        ui.set_friends_error("".into());
        let queued = self.request(ui, account::Command::Friends);
        ui.set_loading_friends(queued);
        if !queued {
            ui.set_friends_error("Friends could not be loaded. Try Refresh shortly.".into());
        }
    }
    pub(crate) fn filter_friends(&mut self, ui: &AppWindow) {
        let query = ui.get_friend_query().to_lowercase();
        let y = ui.get_friends_y();
        if !query.is_empty() && self.friend_search_y.is_none() {
            self.friend_search_y = Some(y);
        }
        let old = ui.get_friends();
        let focused = old
            .row_data(ui.get_friend_result() as usize)
            .map(|row| row.id);
        let height = ui.get_friend_row_height();
        let first = (-y / height).floor().max(0.) as usize;
        let anchor = old.row_data(first).map(|row| row.id);
        let mut rows: Vec<_> = self
            .friends
            .iter()
            .filter(|row| {
                (if row.group {
                    ui.get_groups_expanded()
                } else {
                    ui.get_friends_expanded()
                }) && row.name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            a.group
                .cmp(&b.group)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        let restored = anchor
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .map(|index| -(index as f32 * height) + (y + first as f32 * height))
            .unwrap_or(0.);
        ui.set_friend_result(
            focused
                .and_then(|id| rows.iter().position(|row| row.id == id))
                .map(|i| i as i32)
                .unwrap_or(-1),
        );
        ui.set_friends(ModelRc::new(VecModel::from(rows)));
        ui.set_friends_y(if query.is_empty() {
            self.friend_search_y.take().unwrap_or(restored)
        } else {
            restored
        });
    }
    pub(crate) fn open_profile(&mut self, ui: &AppWindow, id: &str) {
        let Some(friend) = self.friends.iter().find(|friend| friend.id == id) else {
            return;
        };
        self.profile_id = id.into();
        ui.set_profile_id(id.into());
        ui.set_profile_name(friend.name.clone());
        ui.set_profile_status(friend.status.clone());
        ui.set_profile_group(friend.group);
        ui.set_profile_avatar(friend.avatar.clone());
        ui.set_profile_available(true);
        ui.invoke_show_profile();
    }
    pub(crate) fn start_friend_chat(&mut self, ui: &AppWindow) {
        let Some(friend) = self
            .friends
            .iter()
            .find(|friend| friend.id == self.profile_id)
            .cloned()
        else {
            return;
        };
        let index = if let Some(index) = self.ids.iter().position(|id| id == friend.id.as_str()) {
            index
        } else {
            let index = self.ids.len();
            self.ids.push(friend.id.to_string());
            self.rows.push(ChatRow {
                index: index as i32,
                title: friend.name.clone(),
                initial: friend.name.chars().next().unwrap_or('·').to_string().into(),
                avatar: friend.avatar,
                avatar_key: friend.avatar_key,
                ..Default::default()
            });
            self.filter(ui);
            index
        };
        ui.set_profile_open(false);
        self.select(ui, index);
        ui.set_show_conversation(true);
        ui.invoke_focus_composer();
    }
    pub(crate) fn filter(&self, ui: &AppWindow) {
        let y = ui.get_list_y();
        let old = ui.get_chats();
        let height = ui.get_chat_row_height();
        let first = (-y / height).floor().max(0.) as usize;
        let anchor = if y < -0.5 {
            old.row_data(first).map(|row| row.index)
        } else {
            None
        };
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| row.title.to_lowercase().contains(&self.query))
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.activity
                .cmp(&a.activity)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.index.cmp(&b.index))
        });
        let restored = anchor
            .and_then(|index| rows.iter().position(|row| row.index == index))
            .map(|index| -(index as f32 * height) + (y + first as f32 * height))
            .unwrap_or(0.);
        ui.set_chats(ModelRc::new(VecModel::from(rows)));
        ui.set_list_y(restored);
    }
}
