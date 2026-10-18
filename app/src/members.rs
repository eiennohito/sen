use crate::*;

pub(crate) fn member_initial(name: &str) -> slint::SharedString {
    use unicode_segmentation::UnicodeSegmentation;
    name.graphemes(true).next().unwrap_or("?").into()
}

pub(crate) fn filtered(rows: &[MemberRow], query: &str) -> Vec<MemberRow> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<_> = rows
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&query) || m.is_self && "you".contains(&query))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

pub(crate) fn bridge(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<State>().on_back_to_members(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_back_to_members();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_chat_header(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_chat_header();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_members(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_members();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_filter_members(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_filter_members();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_member(move |id| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_member(id);
        }
    });
}

pub(crate) fn install(ui: &AppWindow, state: &Rc<RefCell<View>>) {
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_members(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_members(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_filter_members(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().filter_members(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_member(move |id| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_member(&ui, &id);
        }
    });
}

impl View {
    pub(crate) fn member_chat_available(&self, id: &str) -> bool {
        self.friends.iter().any(|f| !f.group && f.id == id)
            || self.ids.iter().any(|chat| chat == id)
    }
    pub(crate) fn open_members(&mut self, ui: &AppWindow) {
        if !ui.get_chat_group() {
            return;
        }
        let reopening = ui.get_members_open();
        if !reopening {
            self.members.clear();
            self.members_loaded = false;
            self.members_pending = false;
            ui.set_member_query("".into());
            self.filter_members(ui);
        }
        ui.set_members_open(true);
        ui.set_member_profile(false);
        ui.invoke_show_profile();
        if self.demo {
            self.members = demo::members();
            ui.set_member_count(self.members.len() as i32);
            self.filter_members(ui);
        } else {
            self.load_members(ui);
        }
    }
    pub(crate) fn load_members(&mut self, ui: &AppWindow) {
        if self.members_pending {
            return;
        }
        self.member_request = self.member_request.wrapping_add(1);
        let queued = self.request(
            ui,
            account::Command::Members {
                chat: self.current.clone(),
                request: self.member_request,
                avatars: self
                    .members
                    .iter()
                    .filter(|m| m.avatar.size().width > 0)
                    .map(|m| m.avatar_key.to_string())
                    .collect(),
            },
        );
        self.members_pending = queued;
        ui.set_members_failed(!queued);
        ui.set_members_loading(queued && !self.members_loaded);
    }
    pub(crate) fn filter_members(&self, ui: &AppWindow) {
        let state = ui.global::<State>();
        let old = ui.get_members();
        let focused = old
            .row_data(state.get_member_result().max(0) as usize)
            .map(|m| m.id);
        let rows = filtered(&self.members, &ui.get_member_query());
        state.set_member_result(
            focused
                .and_then(|id| rows.iter().position(|m| m.id == id))
                .unwrap_or(0) as i32,
        );
        if let Some(model) = old.as_any().downcast_ref::<VecModel<MemberRow>>() {
            for (i, row) in rows.iter().enumerate() {
                if i >= model.row_count() {
                    model.push(row.clone());
                } else if model.row_data(i).as_ref() != Some(row) {
                    model.set_row_data(i, row.clone());
                }
            }
            while model.row_count() > rows.len() {
                model.remove(model.row_count() - 1);
            }
        } else {
            ui.set_members(ModelRc::new(VecModel::from(rows)));
        }
    }
    pub(crate) fn open_member(&mut self, ui: &AppWindow, id: &str) {
        let Some(member) = self.members.iter().find(|m| m.id == id) else {
            return;
        };
        self.profile_id = id.into();
        ui.global::<State>()
            .set_profile_initial(member.initial.clone());
        ui.set_profile_id(id.into());
        ui.set_profile_name(member.name.clone());
        ui.set_profile_status(if member.is_self {
            "You".into()
        } else {
            member.status.clone()
        });
        ui.set_profile_avatar(member.avatar.clone());
        ui.set_profile_group(false);
        ui.set_profile_available(!member.is_self && self.member_chat_available(id));
        ui.set_member_profile(true);
        ui.invoke_focus_profile_close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_self_first_and_searches_names_and_you() {
        let rows = vec![
            MemberRow {
                id: "z".into(),
                name: "Zoe".into(),
                ..Default::default()
            },
            MemberRow {
                id: "self".into(),
                name: "Owner".into(),
                is_self: true,
                ..Default::default()
            },
            MemberRow {
                id: "a".into(),
                name: "alice".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            filtered(&rows, "")
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["self", "a", "z"]
        );
        assert_eq!(filtered(&rows, " ALI ")[0].id, "a");
        assert_eq!(filtered(&rows, "You")[0].id, "self");
        assert!(filtered(&rows, "nobody").is_empty());
        assert_eq!(member_initial("👩‍💻 Person"), "👩‍💻");
    }

    #[test]
    fn panel_sheet_profile_back_and_header_focus_at_all_widths() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let view = Rc::new(RefCell::new(View {
                demo: true,
                ..Default::default()
            }));
            install(&ui, &view);
            view.borrow_mut().ids = vec!["person".into(), "group".into()];
            view.borrow_mut().rows = vec![
                ChatRow::default(),
                ChatRow {
                    index: 1,
                    group: true,
                    member_count: 25,
                    title: "Group".into(),
                    ..Default::default()
                },
            ];
            ui.show().unwrap();
            for (width, sheet, x) in [(1400, false, 1080.), (980, true, 342.), (360, true, 0.)] {
                window.set_size(slint::PhysicalSize::new(width, 720));
                view.borrow_mut().select(&ui, 1);
                ui.set_show_conversation(true);
                ui.invoke_open_members();
                snapshot::render(&window, (width, 720)).unwrap();
                assert!(ui.get_profile_open() && ui.get_members_open());
                assert_eq!(ui.global::<State>().get_profile_sheet(), sheet);
                let bounds = ui.invoke_connection_obstacles().row_data(4).unwrap();
                assert_eq!(bounds.x, x);
                assert_eq!(bounds.width, if sheet { width as f32 - x } else { 320. });
                assert_eq!(ui.get_members().row_count(), 25);
                assert!(ui.get_members().row_data(0).unwrap().is_self);
                ui.set_member_query("蓮".into());
                ui.invoke_filter_members();
                assert_eq!(ui.get_members().row_count(), 1);
                ui.invoke_open_member("member-2".into());
                assert!(ui.get_member_profile());
                assert_eq!(ui.get_profile_name(), "蓮");
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "\n".into() });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                        text: "\n".into(),
                    });
                slint::platform::update_timers_and_animations();
                assert!(
                    !ui.get_member_profile(),
                    "the focused Back button must return to members"
                );
                assert_eq!(ui.get_member_query(), "蓮");
                ui.invoke_close_profile();
                slint::platform::update_timers_and_animations();
                assert!(!ui.get_profile_open());
                let key = |text: &str| {
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                            text: text.into(),
                        });
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                            text: text.into(),
                        });
                    slint::platform::update_timers_and_animations();
                };
                key("\n");
                assert!(
                    ui.get_members_open(),
                    "closing must return focus to header at {width}"
                );
                key("\u{1b}");
                assert!(!ui.get_profile_open());
                view.borrow_mut().select(&ui, 0);
                ui.invoke_open_members();
                assert!(!ui.get_members_open(), "1:1 must have no members");
            }
            view.borrow_mut().select(&ui, 1);
            view.borrow_mut().demo = false;
            ui.invoke_open_members();
            assert!(ui.get_members_failed());
            let request = view.borrow().member_request;
            view.borrow_mut().update(
                &ui,
                account::Update::Members {
                    chat: "group".into(),
                    request: request.wrapping_sub(1),
                    members: Some(vec![]),
                    contacts_failed: false,
                },
            );
            assert!(
                ui.get_members_failed(),
                "stale member completion must be ignored"
            );
            view.borrow_mut().update(
                &ui,
                account::Update::Members {
                    chat: "group".into(),
                    request,
                    members: Some(vec![]),
                    contacts_failed: false,
                },
            );
            assert!(!ui.get_members_failed());
            assert_eq!(ui.get_member_count(), 0);
            assert_eq!(ui.get_members().row_count(), 0);
            ui.invoke_close_profile();
        })
        .join()
        .unwrap();
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn member_chat_requires_friend_or_existing_conversation() {
        std::thread::spawn(|| {
            let _window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let mut view = View {
                demo: true,
                current: "group".into(),
                ..Default::default()
            };
            view.ids = vec!["group".into(), "existing".into()];
            view.friends = vec![FriendRow {
                id: "friend".into(),
                ..Default::default()
            }];
            view.members = ["friend", "existing", "stranger", "unknown", "self"]
                .into_iter()
                .map(|id| MemberRow {
                    id: id.into(),
                    name: id.into(),
                    is_self: id == "self",
                    ..Default::default()
                })
                .collect();
            ui.set_members_open(true);
            ui.set_profile_open(true);
            ui.set_draft("Keep this".into());
            for id in ["friend", "existing"] {
                view.open_member(&ui, id);
                assert!(ui.get_profile_available());
            }
            for id in ["stranger", "unknown", "self"] {
                view.open_member(&ui, id);
                assert!(!ui.get_profile_available());
                ui.set_profile_available(true); // The controller also enforces the restriction.
                view.start_friend_chat(&ui);
                assert_eq!(view.current, "group");
                assert_eq!(view.ids.len(), 2);
                assert_eq!(ui.get_draft(), "Keep this");
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn refresh_keeps_model_focus_selection_and_loaded_avatars() {
        std::thread::spawn(|| {
            let window = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            let (tx, rx) = std::sync::mpsc::sync_channel(8);
            let mut view = View { current: "group".into(), members_loaded: true, commands: Some(tx.into()), ..Default::default() };
            view.members = vec![
                MemberRow { id: "a".into(), name: "Alpha".into(), avatar: demo::avatar(0), avatar_key: "kept".into(), ..Default::default() },
                MemberRow { id: "b".into(), name: "Beta".into(), avatar_key: "missing".into(), ..Default::default() },
            ];
            ui.set_members_open(true); ui.set_profile_open(true);
            ui.show().unwrap();
            snapshot::render(&window, (980, 720)).unwrap();
            view.filter_members(&ui);
            ui.global::<State>().set_member_result(1);
            ui.invoke_back_to_members();
            let model = ui.get_members();
            view.load_members(&ui);
            assert!(!ui.get_members_loading());
            assert!(matches!(rx.try_recv().unwrap(), account::Command::Members { avatars, .. } if avatars == ["kept"]));
            view.load_members(&ui);
            assert!(rx.try_recv().is_err(), "coalesce in-flight requests");
            view.update(&ui, account::Update::Members { chat: "group".into(), request: view.member_request,
                contacts_failed: true, members: Some([("a", "Alpha", "kept"), ("b", "Beta", "missing")].into_iter().map(|(id, name, key)| account::MemberView {
                    is_self: false, contact: account::FriendView { id: id.into(), name: name.into(), status: String::new(), group: false, avatar_key: key.into(), member_count: None }
                }).collect()) });
            assert!(std::ptr::eq(model.as_any(), ui.get_members().as_any()));
            assert_eq!(ui.global::<State>().get_member_result(), 1);
            assert!(ui.get_members().row_data(0).unwrap().avatar.size().width > 0);
            assert!(ui.get_members_failed());
            assert!(!ui.get_members_loading());
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "\u{f700}".into() });
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "\u{f700}".into() });
            assert_eq!(ui.global::<State>().get_member_result(), 0, "refresh must retain list focus even on partial lookup failure");
            view.load_members(&ui);
            assert!(rx.try_recv().is_ok(), "Retry must queue another contact lookup");
        }).join().unwrap();
    }
}
