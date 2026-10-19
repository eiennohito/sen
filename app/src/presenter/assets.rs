use super::*;

impl Presenter {
    /// Patch only consumers of this key. Nested models retain their identity.
    pub fn patch_asset(&mut self, ui: &AppWindow, state: &AppState, key: &str) {
        let image = self.images.get(key);
        let failure = self.failures.get(key);
        if let Some(image) = image {
            let global = ui.global::<State>();
            for model in [
                global.get_new_chat_friends(),
                global.get_new_chat_selected(),
            ] {
                if let Some(rows) = model.as_any().downcast_ref::<VecModel<ComposeFriend>>() {
                    for i in 0..rows.row_count() {
                        let mut row = rows.row_data(i).unwrap();
                        if state.friends.iter().any(|friend| {
                            friend.id == row.id.as_str()
                                && friend
                                    .avatar
                                    .as_ref()
                                    .is_some_and(|asset| asset.key() == key)
                        }) && row.avatar != *image
                        {
                            row.avatar = image.clone();
                            rows.set_row_data(i, row);
                        }
                    }
                }
            }
            let tabs = global.get_sticker_tabs();
            if let Some(rows) = tabs.as_any().downcast_ref::<VecModel<StickerTab>>() {
                for (i, package) in state.sticker_packages.iter().enumerate() {
                    if package
                        .stickers
                        .first()
                        .and_then(Content::image)
                        .is_some_and(|asset| asset.key() == key)
                        && let Some(mut row) = rows.row_data(i)
                        && row.picture != *image
                    {
                        row.picture = image.clone();
                        rows.set_row_data(i, row);
                    }
                }
            }
            let items = global.get_sticker_items();
            if let Some(rows) = items.as_any().downcast_ref::<VecModel<StickerItem>>()
                && let Some(package) = state
                    .sticker_packages
                    .get(global.get_sticker_package() as usize)
            {
                for (i, sticker) in package.stickers.iter().enumerate() {
                    if sticker.image().is_some_and(|asset| asset.key() == key)
                        && let Some(mut row) = rows.row_data(i)
                        && row.picture != *image
                    {
                        row.picture = image.clone();
                        rows.set_row_data(i, row);
                    }
                }
            }
        }
        for (_, row) in self.rendered.values_mut() {
            patch_message_asset(row, key, image, failure);
        }
        for i in 0..self.messages.row_count() {
            let mut row = self.messages.row_data(i).unwrap();
            if !uses_asset(&row, key) {
                continue;
            }
            if patch_message_asset(&mut row, key, image, failure) {
                self.messages.set_row_data(i, row);
            }
        }
        for (_, row) in self.rendered_chats.values_mut() {
            patch_chat_asset(row, key, image, failure.is_some(), ui.get_text_size());
        }
        for (i, id) in self.chat_keys.iter().enumerate() {
            if let Some((_, cached)) = self.rendered_chats.get(id)
                && cached.avatar_key == key
                && self.chats.row_data(i).as_ref() != Some(cached)
            {
                self.chats.set_row_data(i, cached.clone());
            }
        }
        for cached in self.rendered_friends.values_mut() {
            let row = &mut cached.row;
            patch_runs(
                &row.status_runs,
                key,
                image,
                failure.is_some(),
                Some(ui.get_text_size()),
            );
            if row.avatar_key == key
                && let Some(image) = image
            {
                row.avatar = image.clone();
            }
        }
        for i in 0..self.friends.row_count() {
            let mut row = self.friends.row_data(i).unwrap();
            if row.avatar_key == key
                && let Some(image) = image
                && row.avatar != *image
            {
                row.avatar = image.clone();
                self.friends.set_row_data(i, row);
            }
        }
        for i in 0..self.members.row_count() {
            let mut row = self.members.row_data(i).unwrap();
            if row.avatar_key == key
                && let Some(image) = image
                && row.avatar != *image
            {
                row.avatar = image.clone();
                self.members.set_row_data(i, row);
            }
        }
        if let Some(image) = image {
            if state
                .selected
                .as_ref()
                .and_then(|id| chats::chat_avatar(state, id))
                .is_some_and(|asset| asset.key() == key)
            {
                ui.global::<State>().set_chat_avatar(image.clone());
            }
            let avatar = match state.detail_page() {
                Some(line::controller::DetailPage::Profile(id)) => {
                    super::detail::profile_avatar(state, id)
                }
                Some(line::controller::DetailPage::GroupInfo(id)) => {
                    super::detail::profile_avatar(state, &id.0)
                }
                _ => None,
            };
            if avatar.is_some_and(|a| a.key() == key) {
                ui.set_profile_avatar(image.clone());
            }
        }
        let mut quote = ui.global::<State>().get_composer_reply();
        if uses_asset(&quote, key) && patch_message_asset(&mut quote, key, image, failure) {
            ui.global::<State>().set_composer_reply(quote);
        }
    }
    pub fn request_status_assets(&self, assets: &mut crate::assets::Assets, sender: &Sender) {
        for cached in self.rendered_friends.values() {
            for asset in &cached.assets {
                if assets.request(asset) {
                    sender.send(Intent::Asset(asset.clone()));
                }
            }
        }
    }
}

fn uses_asset(row: &MessageRow, key: &str) -> bool {
    row.reply_image_key == key
        || row.image_key == key
        || row.sender_key == key
        || row.rich_runs.iter().any(|run| run.image_key == key)
        || row.reactions.iter().any(|run| {
            run.image_key == key || run.reactors.iter().any(|actor| actor.avatar_key == key)
        })
}
fn patch_message_asset(
    row: &mut MessageRow,
    key: &str,
    image: Option<&slint::Image>,
    failure: Option<&line::client::desktop::assets::Failure>,
) -> bool {
    if !uses_asset(row, key) {
        return false;
    }
    let old = row.clone();
    if row.reply_image_key == key
        && let Some(image) = image
    {
        row.reply_picture = image.clone();
        row.reply_has_picture = failure.is_none();
    }
    if row.image_key == key {
        if let Some(image) = image {
            row.picture = image.clone();
            row.has_picture = true;
        }
        row.image_failed = failure.is_some();
        row.image_error = failure
            .map(|f| text::image_text(*f))
            .unwrap_or_default()
            .into();
        row.image_retryable = failure.is_some_and(|f| f.retry_read());
    }
    if row.sender_key == key
        && let Some(image) = image
    {
        row.sender_avatar = image.clone();
        row.has_sender_avatar = true;
    }
    patch_runs(&row.rich_runs, key, image, failure.is_some(), None);
    if let Some(model) = row
        .reactions
        .as_any()
        .downcast_ref::<VecModel<ReactionRow>>()
    {
        for i in 0..model.row_count() {
            let mut run = model.row_data(i).unwrap();
            if let Some(actors) = run.reactors.as_any().downcast_ref::<VecModel<ReactorRow>>() {
                for j in 0..actors.row_count() {
                    let mut actor = actors.row_data(j).unwrap();
                    if actor.avatar_key == key
                        && let Some(image) = image
                    {
                        actor.avatar = image.clone();
                        actors.set_row_data(j, actor);
                    }
                }
            }
            if run.image_key == key
                && let Some(image) = image
                && run.picture != *image
            {
                run.picture = image.clone();
                run.has_picture = true;
                model.set_row_data(i, run);
            }
        }
    }
    *row != old
}
fn patch_chat_asset(
    row: &mut ChatRow,
    key: &str,
    image: Option<&slint::Image>,
    failed: bool,
    size: i32,
) -> bool {
    patch_runs(&row.preview_runs, key, image, failed, Some(size));
    if row.avatar_key == key
        && let Some(image) = image
        && row.avatar != *image
    {
        row.avatar = image.clone();
        return true;
    }
    false
}
fn patch_runs(
    runs: &ModelRc<RichRun>,
    key: &str,
    image: Option<&slint::Image>,
    failed: bool,
    size: Option<i32>,
) {
    if let Some(model) = runs.as_any().downcast_ref::<VecModel<RichRun>>() {
        // Do the thumbnail conversion once, even when an emoji repeats.
        let mut picture = None;
        for i in 0..model.row_count() {
            let mut run = model.row_data(i).unwrap();
            if run.image_key == key {
                if failed {
                    if run.has_picture {
                        run.has_picture = false;
                        model.set_row_data(i, run);
                    }
                    continue;
                }
                let Some(image) = image else {
                    continue;
                };
                let image = picture.get_or_insert_with(|| {
                    size.map(|size| preview::small(image, size))
                        .unwrap_or_else(|| image.clone())
                });
                if !run.has_picture || run.picture != *image {
                    run.picture = image.clone();
                    run.has_picture = true;
                    model.set_row_data(i, run);
                }
            }
        }
    }
}
