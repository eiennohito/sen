use super::*;

type Reload = Rc<dyn Fn(&str)>;

#[derive(Clone, Default)]
pub(crate) struct Images {
    paths: Rc<RefCell<HashMap<String, std::path::PathBuf>>>,
    requests: Rc<RefCell<Option<Reload>>>,
    failures: Rc<RefCell<HashMap<String, Failure>>>,
}
impl Images {
    pub fn load_reactions(&self, root: &std::path::Path) {
        let Ok(entries) = std::fs::read_dir(root.join("static/media")) else {
            return;
        };
        let paths: Vec<_> = entries
            .filter_map(|entry| entry.ok())
            .map(|e| e.path())
            .collect();
        // The extension's LX table maps protocol types to these artwork numbers.
        for (id, artwork) in [
            (2, 1003),
            (3, 1001),
            (4, 1002),
            (5, 1004),
            (6, 1006),
            (7, 1005),
        ] {
            let prefix = format!("icon_{artwork}_32x32.");
            if let Some(path) = paths.iter().find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
                    && path.extension().is_some_and(|ext| ext == "svg")
            }) {
                self.insert(format!("reaction:{id}"), path.clone());
            }
        }
    }
    pub fn reaction(&self, reaction: &mut ReactionRow) {
        reaction.picture = if reaction.id > 0 {
            self.paths
                .borrow()
                .get(&format!("reaction:{}", reaction.id))
                .and_then(|path| slint::Image::load_from_path(path).ok())
        } else {
            self.get(reaction.image_key.as_str())
        }
        .unwrap_or_default();
        reaction.has_picture = reaction.picture.size().width > 0;
    }
    pub fn on_missing(&self, reload: impl Fn(&str) + 'static) {
        *self.requests.borrow_mut() = Some(Rc::new(reload));
    }
    pub fn insert(&self, key: String, path: std::path::PathBuf) {
        self.failures.borrow_mut().remove(&key);
        self.paths.borrow_mut().insert(key, path);
    }
    pub fn small(&self, key: &str) -> Option<slint::Image> {
        let path = self
            .paths
            .borrow()
            .get(key)
            .map(|path| sen::client::desktop::assets::small_path(path))?;
        if path.is_file() {
            return slint::Image::load_from_path(&path).ok();
        }
        if let Some(reload) = self.requests.borrow().as_ref() {
            reload(key);
        }
        self.get(key)
    }
    pub fn get(&self, key: &str) -> Option<slint::Image> {
        let path = self.paths.borrow().get(key)?.clone();
        if path.is_file() {
            return slint::Image::load_from_path(&path).ok();
        }
        self.paths.borrow_mut().remove(key);
        if let Some(reload) = self.requests.borrow().as_ref() {
            reload(key);
        }
        None
    }
    pub fn picture(&self, asset: Option<&Asset>) -> slint::Image {
        asset.and_then(|a| self.get(&a.key())).unwrap_or_default()
    }
    pub fn failed(&self, key: String, failure: Failure) {
        self.paths.borrow_mut().remove(&key);
        self.failures.borrow_mut().insert(key, failure);
    }
    pub fn failure(&self, key: &str) -> Option<Failure> {
        self.failures.borrow().get(key).copied()
    }
    pub fn runs(&self, runs: Vec<RichRun>) -> ModelRc<RichRun> {
        ModelRc::new(VecModel::from(
            runs.into_iter()
                .map(|mut run| {
                    run.picture = self.get(run.image_key.as_str()).unwrap_or_default();
                    run.has_picture = run.picture.size().width > 0
                        && self.failure(run.image_key.as_str()).is_none();
                    run
                })
                .collect::<Vec<_>>(),
        ))
    }
    pub fn map<K: Ord + Clone + 'static, T: Clone + 'static, R: Clone + 'static>(
        &self,
        model: Rc<KeyedModel<K, T>>,
        map: fn(T, &Images) -> R,
    ) -> ModelRc<R> {
        let images = self.clone();
        ModelRc::new(slint::MapModel::new(model, move |data| map(data, &images)))
    }
}

impl Presenter {
    pub fn patch_asset(&mut self, ui: &AppWindow, state: &AppState, key: &str) {
        let conversation = ui.global::<ConversationState>();
        let detail = ui.global::<DetailState>();
        self.messages.asset_changed(key);
        self.chats.asset_changed(key);
        self.friends.asset_changed(key);
        self.members.asset_changed(key);
        self.compose_friends.asset_changed(key);
        self.compose_selected.asset_changed(key);
        self.sticker_tabs.asset_changed(key);
        self.sticker_items.asset_changed(key);
        if let Some(image) = self.images.get(key) {
            if state
                .self_contact
                .avatar
                .as_ref()
                .is_some_and(|a| a.key() == key)
            {
                ui.global::<ShellState>().set_account_avatar(image.clone());
            }
            if state
                .selected
                .as_ref()
                .and_then(|id| chats::chat_avatar(state, id))
                .is_some_and(|a| a.key() == key)
            {
                conversation.set_chat_avatar(image.clone());
            }
            let avatar = match state.detail_page() {
                Some(DetailPage::Profile(id)) => detail::profile_avatar(state, id),
                Some(DetailPage::GroupInfo(id)) => detail::profile_avatar(state, &id.0),
                _ => None,
            };
            if avatar.is_some_and(|a| a.key() == key) {
                detail.set_profile_avatar(image);
            }
        }
        if state
            .composer_reply
            .as_ref()
            .and_then(|r| state.reply_target(&r.message_id))
            .and_then(|m| m.body.image())
            .is_some_and(|a| a.key() == key)
        {
            self.present_quote(ui, state);
        }
    }
    pub fn request_status_assets(&self, assets: &mut crate::assets::Assets, sender: &Sender) {
        for asset in &self.status_assets {
            if assets.request(asset) {
                sender.send(Intent::Asset(asset.clone()));
            }
        }
    }
}

pub(super) fn content_assets(content: &Content) -> Vec<String> {
    content
        .image()
        .into_iter()
        .map(|a| a.key())
        .chain(
            content
                .spans()
                .iter()
                .filter_map(Asset::inline_emoji)
                .map(|a| a.key()),
        )
        .collect()
}

pub(super) fn asset_key(asset: Option<&Asset>) -> slint::SharedString {
    asset.map(|a| a.key()).unwrap_or_default().into()
}
