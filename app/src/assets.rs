//! Visible asset requests.
use sen::{client::desktop::assets::Asset, engine::Intent};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Default)]
pub(crate) struct Assets {
    requested: Rc<RefCell<HashSet<u64>>>,
    reloads: Rc<RefCell<HashMap<String, Asset>>>,
}
impl Assets {
    pub(crate) fn evicted(&mut self, key: &str) {
        if let Some(asset) = self.reloads.borrow_mut().remove(key) {
            self.requested.borrow_mut().remove(&identity(&asset));
        }
    }
    pub(crate) fn request(&mut self, asset: &Asset) -> bool {
        if !self.requested.borrow_mut().insert(identity(asset)) {
            return false;
        }
        self.reloads
            .borrow_mut()
            .insert(asset.key(), asset.request());
        true
    }
    pub(crate) fn reload(&self, key: &str, sender: &sen::engine::Sender) {
        if let Some(asset) = self.reloads.borrow().get(key) {
            sender.send(Intent::Asset(asset.clone()));
        }
    }
    pub(crate) fn stickers(
        &mut self,
        state: &sen::controller::AppState,
        selected: usize,
        sender: &sen::engine::Sender,
    ) {
        for asset in state
            .sticker_packages
            .iter()
            .enumerate()
            .flat_map(|(index, package)| {
                package
                    .stickers
                    .iter()
                    .take(if index == selected {
                        package.stickers.len()
                    } else {
                        1
                    })
                    .filter_map(|sticker| sticker.image())
            })
        {
            if self.request(asset) {
                sender.send(Intent::Asset(asset.clone()));
            }
        }
    }
    pub(crate) fn visible(
        &mut self,
        state: &sen::controller::AppState,
        selected: usize,
        sender: &sen::engine::Sender,
    ) {
        for asset in state
            .chats
            .values()
            .filter_map(|c| c.chat.avatar.as_ref())
            .cloned()
            .chain(state.friends.iter().filter_map(|f| f.avatar.clone()))
            .chain(state.contacts.values().filter_map(|f| f.avatar.clone()))
            .chain(state.self_contact.avatar.clone())
            .chain(
                state
                    .members
                    .iter()
                    .filter_map(|m| m.contact.avatar.clone()),
            )
        {
            if self.request(&asset) {
                sender.send(Intent::Asset(asset));
            }
        }

        self.stickers(state, selected, sender);

        for m in state
            .messages
            .values()
            .chain(state.chats.values().filter_map(|c| c.preview.as_ref()))
            .chain(state.reply_targets.values())
        {
            use sen::client::desktop::assets::Asset;
            let media = m.body.image().cloned();
            let emoji = m.body.spans().iter().filter_map(Asset::inline_emoji);
            let reactions = m.reactions.iter().filter_map(|r| match &r.kind {
                sen::store::ReactionKind::Paid(asset) => asset.clone(),
                _ => None,
            });
            for asset in media.into_iter().chain(emoji).chain(reactions) {
                if self.request(&asset) {
                    sender.send(Intent::Asset(asset));
                }
            }
        }
    }
}

fn identity(asset: &Asset) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    asset.hash(&mut hash);
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_distinguish_failures_for_the_same_url() {
        use sen::client::desktop::assets::Failure;
        let mut value = serde_json::to_value(sen::backend::FakeBackend::asset(true)).unwrap();
        let mut assets = Assets::default();
        for failure in [Failure::Timeout, Failure::Http(404)] {
            value["failure"] = serde_json::to_value(failure).unwrap();
            let asset: Asset = serde_json::from_value(value.clone()).unwrap();
            assert!(assets.request(&asset));
            assert!(!assets.request(&asset));
        }
    }
    #[test]
    fn local_requests_deduplicate_by_identity_without_retaining_preview_bytes() {
        let asset = Asset::local(sen::attachments::Attachment {
            path: "synthetic.png".into(),
            name: "synthetic.png".into(),
            size: 4,
            photo: None,
            preview: vec![42; 1024],
            clipboard: false,
        });
        let mut assets = Assets::default();
        assert!(assets.request(&asset));
        assert!(!assets.request(&asset.request()));
        let reloads = assets.reloads.borrow();
        let reload = &reloads[&asset.key()];
        assert!(reload.local_attachment().is_none());
        assert_eq!(reload.key(), asset.key());
        drop(reloads);
        assets.evicted(&asset.key());
        assert!(assets.request(&asset));
    }
}
