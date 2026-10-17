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
        let mut section_rows = Vec::with_capacity(rows.len() + 2);
        for group in [false, true] {
            section_rows.push(FriendRow {
                id: if group {
                    "section-groups"
                } else {
                    "section-friends"
                }
                .into(),
                name: if group { "Groups" } else { "Friends" }.into(),
                group,
                header: true,
                ..Default::default()
            });
            section_rows.extend(rows.iter().filter(|row| row.group == group).cloned());
        }
        rows = section_rows;
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
                group: friend.group,
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
    pub(crate) fn switch_visible_chat(&mut self, ui: &AppWindow, delta: i32) {
        let rows = ui.get_chats();
        if rows.row_count() == 0 {
            return;
        }
        let current = (0..rows.row_count()).find(|&i| {
            rows.row_data(i)
                .is_some_and(|row| row.index == ui.get_selected_chat())
        });
        let index = current
            .map(|i| (i as i32 + delta).clamp(0, rows.row_count() as i32 - 1))
            .unwrap_or(if delta < 0 {
                rows.row_count() as i32 - 1
            } else {
                0
            });
        if let Some(row) = rows.row_data(index as usize) {
            self.select(ui, row.index as usize);
            ui.set_result_focus(index);
            ui.set_show_conversation(true);
            ui.invoke_focus_composer();
        }
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
            .filter(|row| {
                (ui.get_category() == 0
                    || (ui.get_category() == 1 && !row.group)
                    || (ui.get_category() == 2 && row.group))
                    && (row.title.to_lowercase().contains(&self.query)
                        || row.preview.to_lowercase().contains(&self.query))
            })
            .cloned()
            .map(|mut row| {
                let id = self.ids.get(row.index as usize);
                if let Some(draft) = id.and_then(|id| self.saved.drafts.get(id)) {
                    row.draft = !draft.trim().is_empty();
                    row.draft_preview = single_line(draft).into();
                }
                row.failed = id.is_some_and(|id| {
                    self.saved.outbox.iter().any(|item| {
                        &item.chat == id && item.server_id.is_empty() && item.status != "Pending…"
                    })
                });
                row
            })
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
        let focused = old
            .row_data(ui.get_result_focus().max(0) as usize)
            .map(|row| row.index);
        ui.set_result_focus(
            focused
                .and_then(|id| rows.iter().position(|row| row.index == id))
                .map(|i| i as i32)
                .unwrap_or(-1),
        );
        ui.set_chats(ModelRc::new(VecModel::from(rows)));
        ui.set_list_y(restored);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_metadata_and_keyboard_switch_preserve_open_draft() {
        std::thread::spawn(|| {
            let _platform = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let mut view = View {
                demo: true,
                ..Default::default()
            };
            for (index, group, title, preview) in [
                (0, false, "Friend", "outgoing text"),
                (1, true, "Group", "group text"),
            ] {
                view.ids.push(index.to_string());
                view.rows.push(ChatRow {
                    index,
                    group,
                    title: title.into(),
                    preview: preview.into(),
                    activity: order_key(2 - index as i64).into(),
                    outgoing_preview: index == 0,
                    ..Default::default()
                });
            }
            view.filter(&ui);
            view.select(&ui, 0);
            ui.set_draft("Preserve me".into());
            view.persist(&ui);
            assert!(ui.get_chats().row_data(0).unwrap().draft);
            assert_eq!(
                ui.get_chats().row_data(0).unwrap().draft_preview,
                "Preserve me"
            );
            ui.set_category(2);
            view.filter(&ui);
            assert_eq!(ui.get_chats().row_count(), 1);
            assert_eq!(view.current, "0");
            assert_eq!(ui.get_draft(), "Preserve me");
            view.switch_visible_chat(&ui, 1);
            assert_eq!(view.current, "1");
            assert_eq!(view.saved.drafts["0"], "Preserve me");
            ui.set_category(0);
            view.query = "outgoing".into();
            view.filter(&ui);
            assert_eq!(ui.get_chats().row_count(), 1);
            assert!(ui.get_chats().row_data(0).unwrap().outgoing_preview);
            assert_eq!(view.current, "1");
            ui.set_connected(true);
            view.update(
                &ui,
                account::Update::Error {
                    message: "History retry".into(),
                    generation: Some(view.generation),
                },
            );
            assert!(
                ui.get_connected() && !ui.get_connection_failed(),
                "a history failure must not override sync connection state"
            );
            assert_eq!(ui.get_history_error(), "History retry");
        })
        .join()
        .unwrap();
    }
    #[test]
    fn narrow_browser_tab_returns_to_visible_navigation() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let weak = ui.as_weak();
            ui.on_browse(move |section| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_section(section);
                }
            });
            window.set_size(slint::PhysicalSize::new(360, 520));
            ui.set_text_size(24);
            ui.show().unwrap();
            let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(360, 520);
            window.draw_if_needed(|renderer| {
                renderer.render(pixels.make_mut_slice(), 360);
            });
            ui.invoke_focus_search();
            for text in ["\t", "\t", "\u{f701}", "\n"] {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.into() });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                        text: text.into(),
                    });
            }
            assert_eq!(
                ui.get_section(),
                1,
                "Tab from the narrow list must reach visible rail navigation"
            );
            ui.set_has_chat(true);
            ui.set_show_conversation(true);
            ui.invoke_focus_composer();
            for (text, pressed) in [
                ("\u{12}", true),
                ("\u{f702}", true),
                ("\u{f702}", false),
                ("\u{12}", false),
            ] {
                ui.window().dispatch_event(if pressed {
                    slint::platform::WindowEvent::KeyPressed { text: text.into() }
                } else {
                    slint::platform::WindowEvent::KeyReleased { text: text.into() }
                });
            }
            assert!(
                !ui.get_show_conversation(),
                "Alt+Left must work while the narrow composer has focus"
            );
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }
}
