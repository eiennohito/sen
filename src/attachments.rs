//! Locally prepared selections. Preparation never authorizes or uploads media.
use anyhow::{Context, Result, ensure};
use image::{AnimationDecoder, ImageReader};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const FILE_LIMIT: u64 = 1_073_741_824;
pub const PHOTO_LIMIT: u64 = 10_000_000;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    pub extension: String,
    pub preview_width: u32,
    pub preview_height: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub photo: Option<Photo>,
    pub preview: Vec<u8>,
    pub clipboard: bool,
}
impl Attachment {
    pub fn prepare(path: &Path) -> Result<Self> {
        let metadata = path.metadata().context("File missing")?;
        ensure!(metadata.is_file(), "Not a file");
        let path = path.canonicalize()?;
        let mut item = Self {
            name: path
                .file_name()
                .context("Missing file name")?
                .to_string_lossy()
                .into_owned(),
            path,
            size: metadata.len(),
            photo: None,
            preview: vec![],
            clipboard: false,
        };
        if item.size > PHOTO_LIMIT {
            return Ok(item);
        }
        let reader = ImageReader::open(&item.path)?.with_guessed_format()?;
        let format = reader.format();
        if let Some(format) = format {
            let extension = match format {
                image::ImageFormat::Jpeg => "jpg",
                image::ImageFormat::Png => "png",
                image::ImageFormat::Gif => "gif",
                image::ImageFormat::WebP => "webp",
                _ => return Ok(item),
            }
            .to_string();
            let (width, height) = reader.into_dimensions()?;
            if u64::from(width) * u64::from(height) <= 100_000_000 {
                let bytes = item.read()?;
                let animated = match format {
                    image::ImageFormat::Gif => {
                        image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))?
                            .into_frames()
                            .take(2)
                            .count()
                            > 1
                    }
                    image::ImageFormat::Png => {
                        image::codecs::png::PngDecoder::new(Cursor::new(&bytes))?.is_apng()?
                    }
                    image::ImageFormat::WebP => {
                        image::codecs::webp::WebPDecoder::new(Cursor::new(&bytes))?.has_animation()
                    }
                    _ => false,
                };
                let mut reader = ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(500_000_000);
                reader.limits(limits);
                let decoded = crate::images::decode(reader)?;
                let preview = decoded.thumbnail(384, 384);
                let photo = Photo {
                    width: decoded.width(),
                    height: decoded.height(),
                    extension,
                    animated,
                    preview_width: preview.width(),
                    preview_height: preview.height(),
                };
                let mut png = Cursor::new(Vec::new());
                preview.write_to(&mut png, image::ImageFormat::Png)?;
                item.preview = png.into_inner();
                item.photo = Some(photo);
            }
        }
        Ok(item)
    }
    pub fn delete_clipboard(&self) {
        if self.clipboard {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    pub fn too_large(&self) -> bool {
        self.size
            > if self.photo.is_some() {
                PHOTO_LIMIT
            } else {
                FILE_LIMIT
            }
    }
    pub fn read(&self) -> Result<Vec<u8>> {
        let file = std::fs::File::open(&self.path).context("File missing")?;
        ensure!(!self.too_large(), "Too large");
        ensure!(
            file.metadata()?.len() == self.size,
            "File changed; select it again"
        );
        let limit = if self.photo.is_some() {
            PHOTO_LIMIT
        } else {
            FILE_LIMIT
        };
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == self.size,
            "File changed; select it again"
        );
        Ok(bytes)
    }
    pub fn content(&self) -> crate::store::Content {
        let asset = Some(crate::client::desktop::assets::Asset::local(self.clone()));
        if let Some(photo) = &self.photo {
            crate::store::Content::Photo {
                asset,
                size: Some((photo.width, photo.height)),
                album_count: None,
                caption: None,
            }
        } else {
            crate::store::Content::File {
                asset,
                name: Some(self.name.clone()),
                size: Some(self.size),
            }
        }
    }
}
