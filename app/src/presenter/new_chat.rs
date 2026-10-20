use super::*;

impl Presenter {
    pub fn clear_compose_selection(&mut self) {
        self.compose_ids.clear();
    }
    pub fn toggle_compose_friend(&mut self, id: &str) {
        if !self.compose_ids.remove(id) {
            self.compose_ids.insert(id.into());
        }
    }
    pub fn present_new_chat(&mut self, ui: &AppWindow, state: &AppState) {
        let new_chat = ui.global::<NewChatState>();
        if new_chat.get_new_chat_open() {
            let query = Query::new(&new_chat.get_new_chat_query());
            let (mut friends, mut chosen, mut names) = (Vec::new(), Vec::new(), Vec::new());
            for f in state
                .friends
                .iter()
                .filter(|f| !f.group && f.id != state.profile.mid)
            {
                let name = single_line(names::friend_name(f));
                let selected = self.compose_ids.contains(&f.id);
                let row = (
                    f.id.clone(),
                    (f.clone(), selected),
                    f.avatar.iter().map(|a| a.key()).collect(),
                );
                if selected {
                    names.push(name.clone());
                    chosen.push(row.clone());
                }
                if query.matches([names::friend_name(f)]) {
                    friends.push(row);
                }
            }
            new_chat.set_new_chat_count(chosen.len() as i32);
            new_chat.set_new_chat_placeholder(names.join(", ").into());
            self.compose_selected.replace(chosen);
            self.compose_friends.replace(friends);
        }
    }
}
