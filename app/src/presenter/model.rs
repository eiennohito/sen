use slint::{Model, ModelNotify, ModelTracker};
use std::cell::RefCell;

/// Keyed application data; Slint's MapModel performs the data-to-view mapping.
pub(crate) struct KeyedModel<K, T> {
    rows: RefCell<Vec<(K, T, Vec<String>)>>,
    notify: ModelNotify,
}
impl<K, T> Default for KeyedModel<K, T> {
    fn default() -> Self {
        Self {
            rows: RefCell::new(Vec::new()),
            notify: ModelNotify::default(),
        }
    }
}
impl<K: PartialEq + Clone + 'static, T: Clone + PartialEq + 'static> KeyedModel<K, T> {
    pub fn update(&self, key: &K, update: impl FnOnce(&mut T)) {
        let changed = {
            let mut rows = self.rows.borrow_mut();
            let Some(i) = rows.iter().position(|r| &r.0 == key) else {
                return;
            };
            let old = rows[i].1.clone();
            update(&mut rows[i].1);
            (old != rows[i].1).then_some(i)
        };
        if let Some(i) = changed {
            self.notify.row_changed(i);
        }
    }
    pub fn asset_changed(&self, asset: &str) {
        for (i, row) in self.rows.borrow().iter().enumerate() {
            if row.2.iter().any(|a| a == asset) {
                // Asset arrival rebuilds nested lists, resetting their focus/hover.
                self.notify.row_changed(i);
            }
        }
    }
    pub fn replace(&self, next: Vec<(K, T, Vec<String>)>) {
        let old = self.rows.borrow();
        let prefix = old
            .iter()
            .zip(&next)
            .take_while(|(a, b)| a.0 == b.0)
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(next[prefix..].iter().rev())
            .take_while(|(a, b)| a.0 == b.0)
            .count();
        let removed = old.len() - prefix - suffix;
        let added = next.len() - prefix - suffix;
        let end = prefix + removed;
        let same_keys = |a: &[(K, T, Vec<String>)], b: &[(K, T, Vec<String>)]| {
            a.iter().map(|r| &r.0).eq(b.iter().map(|r| &r.0))
        };
        // Chats move to the top; keep the other delegates alive for a single move.
        let moved = if removed == added && removed > 1 {
            if same_keys(&old[prefix..end - 1], &next[prefix + 1..end]) {
                Some((end - 1, prefix))
            } else if same_keys(&old[prefix + 1..end], &next[prefix..end - 1]) {
                Some((prefix, end - 1))
            } else {
                None
            }
        } else {
            None
        };
        let changed: Vec<_> = next
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                old.iter()
                    .any(|r| r.0 == row.0 && r.1 != row.1)
                    .then_some(i)
            })
            .collect();
        drop(old);
        if let Some((from, to)) = moved {
            let row = self.rows.borrow_mut().remove(from);
            self.notify.row_removed(from, 1);
            self.rows.borrow_mut().insert(to, row);
            self.notify.row_added(to, 1);
        } else {
            if removed > 0 {
                self.rows.borrow_mut().drain(prefix..prefix + removed);
                self.notify.row_removed(prefix, removed);
            }
            if added > 0 {
                self.rows
                    .borrow_mut()
                    .splice(prefix..prefix, next[prefix..prefix + added].iter().cloned());
                self.notify.row_added(prefix, added);
            }
        }
        *self.rows.borrow_mut() = next;
        for i in changed {
            self.notify.row_changed(i);
        }
    }
}
impl<K: 'static, T: Clone + 'static> Model for KeyedModel<K, T> {
    type Data = T;
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<T> {
        self.rows.borrow().get(row).map(|r| r.1.clone())
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presenter::Asset;
    use i_slint_core::model::{ModelChangeListener, ModelChangeListenerContainer};
    use slint::ComponentHandle;
    use std::{pin::Pin, rc::Rc};

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Added(usize, usize),
        Removed(usize, usize),
        Changed(usize),
    }
    #[derive(Default)]
    struct Peer(RefCell<Vec<Event>>);
    impl ModelChangeListener for Peer {
        fn row_added(self: Pin<&Self>, row: usize, count: usize) {
            self.0.borrow_mut().push(Event::Added(row, count));
        }
        fn row_removed(self: Pin<&Self>, row: usize, count: usize) {
            self.0.borrow_mut().push(Event::Removed(row, count));
        }
        fn row_changed(self: Pin<&Self>, row: usize) {
            self.0.borrow_mut().push(Event::Changed(row));
        }
        fn reset(self: Pin<&Self>) {
            panic!("keyed updates must preserve delegates");
        }
    }
    #[test]
    fn inserts_removals_moves_and_data_changes_reach_the_view() {
        use Event::*;
        let model = Rc::new(KeyedModel::<u8, String>::default());
        let view = slint::MapModel::new(model.clone(), |s: String| s.to_uppercase());
        let peer = Box::pin(ModelChangeListenerContainer::new(Peer::default()));
        view.model_tracker().attach_peer(peer.as_ref().model_peer());
        let rows = |keys: &[u8]| keys.iter().map(|&k| (k, k.to_string(), vec![])).collect();
        model.replace(rows(&[1, 2, 3]));
        assert_eq!(peer.0.take(), [Added(0, 3)]);
        model.replace(rows(&[1, 4, 2, 3]));
        assert_eq!(peer.0.take(), [Added(1, 1)]);
        model.replace(rows(&[1, 2, 3]));
        assert_eq!(peer.0.take(), [Removed(1, 1)]);
        model.replace(rows(&[3, 1, 2]));
        assert_eq!(peer.0.take(), [Removed(2, 1), Added(0, 1)]);
        model.update(&1, |s| *s = "updated".into());
        assert_eq!(peer.0.take(), [Changed(1)]);
        assert_eq!(view.iter().collect::<Vec<_>>(), ["3", "UPDATED", "2"]);
        model.update(&1, |s| *s = "updated".into());
        assert!(peer.0.take().is_empty());
    }
    #[test]
    fn asset_arrival_notifies_only_consumers_after_moves_and_removal() {
        use Event::*;
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let ui = crate::window::create_app_window().unwrap();
            let mut presenter = crate::presenter::Presenter::default();
            presenter.install(&ui);
            let asset = Asset::avatar("/shared").unwrap();
            let other = Asset::avatar("/other").unwrap();
            let mut state = crate::AppState {
                friends: [("a", asset.clone()), ("b", other), ("c", asset.clone())]
                    .into_iter()
                    .map(|(id, avatar)| crate::Friend {
                        id: id.into(),
                        name: Some(id.into()),
                        avatar: Some(avatar),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            presenter.present(&ui, &state, "", crate::ChatCategory::All);
            let view = ui.global::<crate::FriendsState>().get_friends();
            let peer = Box::pin(ModelChangeListenerContainer::new(Peer::default()));
            view.model_tracker().attach_peer(peer.as_ref().model_peer());
            let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
            presenter.images.insert(asset.key(), image.clone());
            presenter.patch_asset(&ui, &state, &asset.key());
            // Both section headers precede the three contacts.
            assert_eq!(peer.0.take(), [Changed(2), Changed(4)]);
            assert_eq!(view.row_data(2).unwrap().avatar, image);
            assert_eq!(view.row_data(4).unwrap().avatar, image);
            assert_ne!(view.row_data(3).unwrap().avatar, image);
            state.friends.rotate_right(1);
            presenter.present(&ui, &state, "", crate::ChatCategory::All);
            peer.0.take();
            presenter.patch_asset(&ui, &state, &asset.key());
            assert_eq!(peer.0.take(), [Changed(2), Changed(3)]);
            state.friends.retain(|f| f.id != "a");
            presenter.present(&ui, &state, "", crate::ChatCategory::All);
            peer.0.take();
            presenter.patch_asset(&ui, &state, &asset.key());
            assert_eq!(peer.0.take(), [Changed(2)]);
        })
        .join()
        .unwrap();
    }
    #[test]
    fn midnight_and_insertion_before_a_run_refresh_the_view() {
        std::thread::spawn(|| {
            use crate::*;
            use Event::*;
            use chrono::TimeZone;
            let _platform = snapshot::install().unwrap();
            let ui = window::create_app_window().unwrap();
            let mut presenter = presenter::Presenter::default();
            presenter.install(&ui);
            let mut state = AppState::default();
            presenter::format::set_clock(|| {
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 16, 23, 59, 0)
                    .single()
                    .unwrap()
                    .timestamp_millis()
            });
            let mut message = tests::message("run start");
            message.sender_id = "sender".into();
            message.sort.sort_ts = now_millis();
            state.messages.insert(message.key, message.clone());
            presenter.present(&ui, &state, "", ChatCategory::All);
            let view = ui.global::<ConversationState>().get_messages();
            let peer = Box::pin(ModelChangeListenerContainer::new(Peer::default()));
            view.model_tracker().attach_peer(peer.as_ref().model_peer());
            assert_eq!(view.row_data(0).unwrap().day_label, "Today");
            presenter::format::set_clock(|| {
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 17, 0, 1, 0)
                    .single()
                    .unwrap()
                    .timestamp_millis()
            });
            presenter.present(&ui, &state, "", ChatCategory::All);
            assert_eq!(peer.0.take(), [Changed(0)]);
            assert_eq!(view.row_data(0).unwrap().day_label, "Yesterday");
            message.key = Key(2);
            message.sort.sort_ts -= 1;
            state.messages.insert(message.key, message);
            presenter.present(&ui, &state, "", ChatCategory::All);
            assert_eq!(peer.0.take(), [Added(0, 1), Changed(1)]);
            assert_eq!(view.row_data(0).unwrap().run_position, RunPosition::First);
            assert_eq!(view.row_data(1).unwrap().run_position, RunPosition::Last);
            assert!(view.row_data(1).unwrap().day_label.is_empty());
        })
        .join()
        .unwrap();
    }
}
