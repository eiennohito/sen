//! App image preparation. The projection runs on the publishing worker, before
//! pixels reach the Slint queue; the UI thread only creates renderer handles.
use line_wasm_poc::{
    client::desktop::assets::{Asset, Pixels},
    engine::{Delta, Intent},
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(crate) struct Assets {
    avatars: Arc<Mutex<BTreeSet<String>>>,
    requested: BTreeSet<String>,
}
impl Assets {
    pub(crate) fn request(&mut self, asset: &Asset) -> bool {
        let key = asset.key();
        if asset.kind() == "avatar" {
            self.avatars.lock().unwrap().insert(key.clone());
        }
        self.requested.insert(key)
    }
    pub(crate) fn project(&self, delta: Delta) -> Delta {
        match delta {
            Delta::Asset { key, pixels } if self.avatars.lock().unwrap().contains(&key) => {
                Delta::Asset {
                    key,
                    pixels: Arc::new(circular_avatar(&pixels)),
                }
            }
            other => other,
        }
    }
}

/// The software renderer doesn't round image clipping. Supply a masked square
/// so old square cache entries also render as circular avatars.
pub(crate) fn circular_avatar(pixels: &Pixels) -> Pixels {
    let Some(image) = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
    else {
        return Pixels {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
    };
    let edge = image.width().min(image.height());
    let square = image::imageops::crop_imm(
        &image,
        (image.width() - edge) / 2,
        (image.height() - edge) / 2,
        edge,
        edge,
    )
    .to_image();
    let mut image = image::imageops::resize(&square, 64, 64, image::imageops::FilterType::Triangle);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let distance = ((x as f32 + 0.5 - 32.).powi(2) + (y as f32 + 0.5 - 32.).powi(2)).sqrt();
        pixel.0[3] = (pixel.0[3] as f32 * (32. - distance).clamp(0., 1.)) as u8;
    }
    Pixels {
        width: 64,
        height: 64,
        rgba: image.into_raw(),
    }
}

impl Assets {
    pub(crate) fn stickers(
        &mut self,
        state: &line_wasm_poc::controller::AppState,
        selected: usize,
        sender: &line_wasm_poc::engine::Sender,
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
        state: &line_wasm_poc::controller::AppState,
        selected: usize,
        sender: &line_wasm_poc::engine::Sender,
    ) {
        for asset in state
            .chats
            .values()
            .filter_map(|c| c.chat.avatar.as_ref())
            .cloned()
            .chain(state.friends.iter().filter_map(|f| f.avatar.clone()))
            .chain(state.contacts.values().filter_map(|f| f.avatar.clone()))
            .chain(
                state
                    .profile
                    .picture_path
                    .as_deref()
                    .and_then(line_wasm_poc::client::desktop::assets::Asset::avatar),
            )
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
        {
            use line_wasm_poc::client::desktop::assets::Asset;
            let media = m.body.image().cloned();
            let emoji = m.body.spans().iter().filter_map(Asset::inline_emoji);
            let reactions = m.reactions.iter().filter_map(|r| match &r.kind {
                line_wasm_poc::store::ReactionKind::Paid(asset) => asset.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    // Moved from the library with the presentation operation.
    #[test]
    fn avatar_is_a_centered_circle() {
        let source = Pixels {
            width: 1600,
            height: 1000,
            rgba: [25, 70, 200, 255].repeat(1600 * 1000),
        };
        let circle = circular_avatar(&source);
        assert_eq!((circle.width, circle.height), (64, 64));
        assert_eq!(circle.rgba[3], 0);
        assert_eq!(circle.rgba[(32 * 64 + 32) * 4 + 3], 255);
        assert_eq!(
            &circle.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3],
            &[25, 70, 200]
        );
    }
}
