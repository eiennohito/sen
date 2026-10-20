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
impl<K: Eq + std::hash::Hash + Clone + 'static, T: Clone + PartialEq + 'static> KeyedModel<K, T> {
    pub fn update(&self, key: &K, update: impl FnOnce(&mut T) -> bool) {
        let changed = {
            let mut rows = self.rows.borrow_mut();
            let Some(i) = rows.iter().position(|r| &r.0 == key) else {
                return;
            };
            update(&mut rows[i].1).then_some(i)
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
        let middle: std::collections::HashMap<_, _> =
            old[prefix..end].iter().map(|r| (&r.0, &r.1)).collect();
        let changed: Vec<_> = next
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let previous = if i < prefix {
                    Some(&old[i].1)
                } else if i >= next.len() - suffix {
                    Some(&old[old.len() - (next.len() - i)].1)
                } else {
                    middle.get(&row.0).copied()
                };
                previous.is_some_and(|value| value != &row.1).then_some(i)
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
        model.update(&1, |s| std::mem::replace(s, "updated".into()) != "updated");
        assert_eq!(peer.0.take(), [Changed(1)]);
        assert_eq!(view.iter().collect::<Vec<_>>(), ["3", "UPDATED", "2"]);
        model.update(&1, |s| std::mem::replace(s, "updated".into()) != "updated");
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
            presenter.present(&ui, &state, crate::ChatCategory::All);
            let view = ui.global::<crate::FriendsState>().get_friends();
            let peer = Box::pin(ModelChangeListenerContainer::new(Peer::default()));
            view.model_tracker().attach_peer(peer.as_ref().model_peer());
            let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
            let path = crate::tests::image_file(&image);
            let image = slint::Image::load_from_path(&path).unwrap();
            presenter.images.insert(asset.key(), path.clone());
            presenter.patch_asset(&ui, &state, &asset.key());
            // Both section headers precede the three contacts.
            assert_eq!(peer.0.take(), [Changed(2), Changed(4)]);
            assert_eq!(view.row_data(2).unwrap().avatar, image);
            assert_eq!(view.row_data(4).unwrap().avatar, image);
            assert_ne!(view.row_data(3).unwrap().avatar, image);
            state.friends.rotate_right(1);
            presenter.present(&ui, &state, crate::ChatCategory::All);
            peer.0.take();
            presenter.patch_asset(&ui, &state, &asset.key());
            assert_eq!(peer.0.take(), [Changed(2), Changed(3)]);
            state.friends.retain(|f| f.id != "a");
            presenter.present(&ui, &state, crate::ChatCategory::All);
            peer.0.take();
            presenter.patch_asset(&ui, &state, &asset.key());
            assert_eq!(peer.0.take(), [Changed(2)]);
        })
        .join()
        .unwrap();
    }
    #[test]
    fn pruned_row_shows_placeholder_then_reloads_on_first_read() {
        std::thread::spawn(|| {
            let _platform = crate::snapshot::install().unwrap();
            let images = crate::presenter::Images::default();
            let requests = Rc::new(RefCell::new(Vec::new()));
            let received = requests.clone();
            images.on_missing(move |key| received.borrow_mut().push(key.to_owned()));
            let model = Rc::new(KeyedModel::default());
            model.replace(vec![(1, "avatar".to_string(), vec!["avatar".into()])]);
            let view = images.map(model, |key, images| images.get(&key).unwrap_or_default());
            let replacement = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &[220u8, 30, 40, 255],
                1,
                1,
            ));
            let path = crate::tests::image_file(&replacement);
            images.insert("avatar".into(), path.clone());
            assert_eq!(view.row_data(0).unwrap().size().width, 1);
            std::fs::remove_file(path).unwrap();
            assert_eq!(view.row_data(0).unwrap().size().width, 0);
            assert_eq!(*requests.borrow(), ["avatar"]);
            images.insert("avatar".into(), crate::tests::image_file(&replacement));
            assert_eq!(
                view.row_data(0).unwrap().to_rgba8().unwrap().as_bytes(),
                &[220, 30, 40, 255]
            );
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
            presenter.present(&ui, &state, ChatCategory::All);
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
            presenter.present(&ui, &state, ChatCategory::All);
            assert_eq!(peer.0.take(), [Changed(0)]);
            assert_eq!(view.row_data(0).unwrap().day_label, "Yesterday");
            message.key = Key(2);
            message.sort.sort_ts -= 1;
            state.messages.insert(message.key, message);
            presenter.present(&ui, &state, ChatCategory::All);
            assert_eq!(peer.0.take(), [Added(0, 1), Changed(1)]);
            assert_eq!(view.row_data(0).unwrap().run_position, RunPosition::First);
            assert_eq!(view.row_data(1).unwrap().run_position, RunPosition::Last);
            assert!(view.row_data(1).unwrap().day_label.is_empty());
        })
        .join()
        .unwrap();
    }
}

#[cfg(test)]
mod complexity_tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    #[derive(Clone)]
    struct Key(usize, Rc<Cell<usize>>);
    impl PartialEq for Key {
        fn eq(&self, other: &Self) -> bool {
            self.1.set(self.1.get() + 1);
            self.0 == other.0
        }
    }
    impl Eq for Key {}
    impl std::hash::Hash for Key {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            std::hash::Hash::hash(&self.0, state);
        }
    }
    #[test]
    fn unchanged_windows_compare_keys_linearly() {
        let comparisons = Rc::new(Cell::new(0));
        let rows = || {
            (0..500)
                .map(|i| (Key(i, comparisons.clone()), i, vec![]))
                .collect()
        };
        let model = KeyedModel::default();
        model.replace(rows());
        comparisons.set(0);
        model.replace(rows());
        assert!(
            comparisons.get() < 2000,
            "{} key comparisons for 500 cached rows",
            comparisons.get()
        );
    }
}
