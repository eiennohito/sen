//! Avatar rasterization and visible asset requests.
use line::{
    client::desktop::assets::{Asset, Pixels},
    engine::Intent,
};
use std::collections::BTreeSet;

#[derive(Clone, Default)]
pub(crate) struct Assets {
    requested: BTreeSet<String>,
}
impl Assets {
    pub(crate) fn request(&mut self, asset: &Asset) -> bool {
        let key = asset.key();
        self.requested.insert(key)
    }
}

/// The software renderer doesn't round image clipping. Supply a masked square
/// to render decoded avatar pixels as a circle.
pub(crate) fn circular_avatar(pixels: &Pixels, size: u32) -> Pixels {
    let Some(image) = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
    else {
        return Pixels {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
    };
    let edge = image.width().min(image.height());
    let mut square = image::imageops::crop_imm(
        &image,
        (image.width() - edge) / 2,
        (image.height() - edge) / 2,
        edge,
        edge,
    )
    .to_image();
    for pixel in square.pixels_mut() {
        let alpha = u16::from(pixel.0[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u16::from(*channel) * alpha / 255) as u8;
        }
    }
    let mut image =
        image::imageops::resize(&square, size, size, image::imageops::FilterType::Triangle);
    let radius = size as f32 / 2.;
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let distance =
            ((x as f32 + 0.5 - radius).powi(2) + (y as f32 + 0.5 - radius).powi(2)).sqrt();
        let alpha = u16::from(pixel.0[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u16::from(*channel) * 255)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
        pixel.0[3] = (alpha as f32 * (radius + 0.5 - distance).clamp(0., 1.)).round() as u8;
    }
    Pixels {
        width: size,
        height: size,
        rgba: image.into_raw(),
    }
}

impl Assets {
    pub(crate) fn stickers(
        &mut self,
        state: &line::controller::AppState,
        selected: usize,
        sender: &line::engine::Sender,
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
        state: &line::controller::AppState,
        selected: usize,
        sender: &line::engine::Sender,
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
                    .and_then(line::client::desktop::assets::Asset::avatar),
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
            .chain(state.reply_targets.values())
        {
            use line::client::desktop::assets::Asset;
            let media = m.body.image().cloned();
            let emoji = m.body.spans().iter().filter_map(Asset::inline_emoji);
            let reactions = m.reactions.iter().filter_map(|r| match &r.kind {
                line::store::ReactionKind::Paid(asset) => asset.clone(),
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
    #[test]
    fn avatar_is_a_centered_circle() {
        let source = Pixels {
            width: 1600,
            height: 1000,
            rgba: [25, 70, 200, 255].repeat(1600 * 1000),
        };
        let circle = circular_avatar(&source, 44);
        assert_eq!((circle.width, circle.height), (44, 44));
        assert_eq!(circle.rgba[3], 0);
        assert_eq!(circle.rgba[(22 * 44 + 22) * 4 + 3], 255);
        assert_eq!(
            &circle.rgba[(22 * 44 + 22) * 4..(22 * 44 + 22) * 4 + 3],
            &[25, 70, 200]
        );
    }
}
