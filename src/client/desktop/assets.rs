//! Private, account-scoped thumbnail cache. Never stores URLs or media keys.
use super::*;
use image::{ImageFormat, ImageReader};
use std::{collections::HashMap, io::Cursor};

const DISK_LIMIT: u64 = 128 * 1024 * 1024;
const MEMORY_LIMIT: usize = 24 * 1024 * 1024;
const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;

mod fetchers;
pub use fetchers::{Completion, Failures, Fetchers, Payload, Priority, Purpose};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    url: String,
    pub(super) private: bool,
    key_material: Option<String>,
    talk_meta: Option<String>,
    dimension: u32,
    #[serde(default)]
    original_suffix: Option<String>,
}

impl Asset {
    pub fn kind(&self) -> &'static str {
        if self.private {
            "photo"
        } else if self.url.contains("/sticonshop/") {
            "reaction"
        } else if self.url.contains("/stickershop/") {
            "sticker"
        } else {
            "avatar"
        }
    }
    pub fn key(&self) -> String {
        let material = self.key_material.as_deref().unwrap_or("");
        format!(
            "{:x}",
            Sha256::digest(format!(
                "v1:{}:{}:{}:{}",
                self.url, self.dimension, self.private, material
            ))
        )
    }

    pub(super) fn avatar(path: &str) -> Option<Self> {
        if path.is_empty() {
            return None;
        }
        let url = if path.starts_with('/') {
            format!("https://profile.line-scdn.net{path}/preview")
        } else if path.starts_with("https://profile.line-scdn.net/") {
            format!("{path}/preview")
        } else {
            return None;
        };
        Some(Self {
            url,
            private: false,
            key_material: None,
            talk_meta: None,
            dimension: 64,
            original_suffix: None,
        })
    }

    pub(super) fn sticker(meta: &Value) -> Option<Self> {
        let id = segment(meta["STKID"].as_str()?)?;
        let hash = meta["STKHASH"].as_str().and_then(segment);
        let version = if hash.is_some() { 2 } else { 1 };
        let hash = hash.map(|h| format!("/{h}")).unwrap_or_default();
        let base = if meta["STKOPT"].as_str() == Some("NAME_TEXT") {
            "/base"
        } else {
            ""
        };
        Some(Self {
            url: format!(
                "https://stickershop.line-scdn.net/stickershop/v{version}/sticker/{id}{hash}/android{base}/sticker.png"
            ),
            private: false,
            key_material: None,
            talk_meta: None,
            dimension: 256,
            original_suffix: None,
        })
    }

    pub fn inline_emoji(span: &rich::InlineEmoji) -> Option<Self> {
        Self::paid_reaction(&json!({"productId":span.product_id,"emojiId":span.sticon_id}))
    }

    pub(super) fn paid_reaction(value: &Value) -> Option<Self> {
        let product = segment(value["productId"].as_str()?)?;
        let emoji = segment(value["emojiId"].as_str()?)?;
        Some(Self {
            url: format!(
                "https://stickershop.line-scdn.net/sticonshop/v1/sticon/{product}/android/{emoji}.png"
            ),
            private: false,
            key_material: None,
            talk_meta: None,
            dimension: 32,
            original_suffix: None,
        })
    }

    pub(super) fn photo(message: &Value) -> Option<Self> {
        let meta = &message["contentMetadata"];
        let id = message["id"].as_str()?;
        let sid = segment(meta["SID"].as_str().unwrap_or("m"))?;
        let oid = segment(meta["OID"].as_str().unwrap_or(id))?;
        let key = meta["ENC_KM"].as_str().map(str::to_owned);
        let suffix = if key.is_some() {
            "__ud-preview"
        } else {
            "/preview"
        };
        Some(Self {
            url: format!("https://obs.line-apps.com/r/talk/{sid}/{oid}{suffix}"),
            private: true,
            key_material: key,
            talk_meta: Some(talk_meta(id)),
            dimension: 384,
            original_suffix: Some(
                if meta["MEDIA_CONTENT_INFO"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .is_some_and(|v| v["category"] == "original")
                {
                    "/original"
                } else {
                    ""
                }
                .into(),
            ),
        })
    }

    /// Source QO/zD: files use the bare OBS object, never a preview URL.
    pub fn attachment(raw: &Value) -> Option<Self> {
        if raw["contentType"].as_u64() != Some(14) {
            return None;
        }
        let meta = &raw["contentMetadata"];
        let id = segment(raw["id"].as_str()?)?;
        let key = meta["ENC_KM"].as_str().map(str::to_owned);
        let (sid, oid) = if key.is_some() {
            (
                segment(meta["SID"].as_str().unwrap_or("m"))?,
                segment(meta["OID"].as_str().unwrap_or(id))?,
            )
        } else {
            ("m", id)
        };
        Some(Self {
            url: format!("https://obs.line-apps.com/r/talk/{sid}/{oid}"),
            private: true,
            key_material: key,
            talk_meta: Some(talk_meta(id)),
            dimension: 0,
            original_suffix: None,
        })
    }

    /// Chrome's encrypted source uses the bare OID; __ud-preview is only a thumbnail.
    pub fn original(&self) -> Option<Self> {
        if !self.private {
            return None;
        }
        let base = self
            .url
            .strip_suffix("__ud-preview")
            .or_else(|| self.url.strip_suffix("/preview"))?;
        let mut original = self.clone();
        original.url = format!(
            "{base}{}",
            if self.key_material.is_some() {
                ""
            } else {
                self.original_suffix.as_deref().unwrap_or("")
            }
        );
        original.dimension = 8192;
        Some(original)
    }

    pub(super) fn fetch(
        &self,
        http: &http::Transport,
        access_token: Option<&str>,
    ) -> Result<Vec<u8>> {
        ensure!(allowed_url(&self.url), "unsupported asset host");
        let application = format!("CHROMEOS\t{VERSION}\tChrome_OS\t");
        let mut headers = Vec::new();
        if self.private {
            let token = access_token.context("missing media token")?;
            ensure!(!token.contains(['\r', '\n']), "invalid media token");
            headers.push(("X-Line-Access", token));
            headers.push(("X-Line-Application", application.as_str()));
            if let Some(meta) = &self.talk_meta {
                headers.push(("X-Talk-Meta", meta.as_str()));
            }
        }
        let bytes = http.asset(&self.url, &headers, MAX_DOWNLOAD)?;
        if let Some(key) = &self.key_material {
            decrypt_media(&bytes, &decode(key)?)
        } else {
            Ok(bytes)
        }
    }
}

fn allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    matches!(
        host,
        "profile.line-scdn.net" | "stickershop.line-scdn.net" | "obs.line-apps.com"
    ) && !url.contains(['\r', '\n', '\\'])
}

fn segment(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)))
    .then_some(value)
}

fn talk_meta(id: &str) -> String {
    // Same minimal binary-Thrift message envelope as Chrome's lB helper.
    let mut bytes = vec![11, 0, 4];
    bytes.extend((id.len() as u32).to_be_bytes());
    bytes.extend(id.as_bytes());
    bytes.extend([15, 0, 27, 12, 0, 0, 0, 0, 0]);
    STANDARD.encode(
        serde_json::to_vec(&json!({"message":STANDARD.encode(bytes)})).expect("JSON string"),
    )
}

fn decrypt_media(bytes: &[u8], material: &[u8]) -> Result<Vec<u8>> {
    use aes::cipher::{KeyIvInit, StreamCipher};
    use hmac::{Hmac, Mac};
    ensure!(
        material.len() == 32 && bytes.len() >= 32,
        "invalid encrypted media envelope"
    );
    let mut keys = [0_u8; 76];
    hkdf::Hkdf::<Sha256>::new(None, material)
        .expand(b"FileEncryption", &mut keys)
        .map_err(|_| anyhow::anyhow!("media key derivation failed"))?;
    let (ciphertext, tag) = bytes.split_at(bytes.len() - 32);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64])?;
    mac.update(ciphertext);
    mac.verify_slice(tag)
        .map_err(|_| anyhow::anyhow!("media authentication failed"))?;
    let mut counter = [0_u8; 16];
    counter[..12].copy_from_slice(&keys[64..]);
    let mut plaintext = ciphertext.to_vec();
    ctr::Ctr32BE::<aes::Aes256>::new_from_slices(&keys[..32], &counter)?
        .apply_keystream(&mut plaintext);
    Ok(plaintext)
}

pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    pub downloads: usize,
}

pub struct ImageCache {
    directory: PathBuf,
    memory: HashMap<String, (Arc<Pixels>, u64)>,
    tick: u64,
    stats: CacheStats,
}

impl ImageCache {
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn new(root: &Path, account: &str) -> Result<Self> {
        let directory = root.join(format!("{:x}", Sha256::digest(account.as_bytes())));
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let mut cache = Self {
            directory,
            memory: HashMap::new(),
            tick: 0,
            stats: CacheStats::default(),
        };
        cache.prune(0)?;
        Ok(cache)
    }
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    fn memory(&mut self, key: &str) -> Option<Arc<Pixels>> {
        self.tick += 1;
        let (pixels, used) = self.memory.get_mut(key)?;
        *used = self.tick;
        self.stats.hits += 1;
        Some(pixels.clone())
    }

    fn finish_read(&mut self, key: String, disk: DiskImage) -> Result<Option<Arc<Pixels>>> {
        let path = self.directory.join(format!("{key}.png"));
        match disk {
            DiskImage::Pixels(pixels) => {
                if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                    let _ = file.set_times(fs::FileTimes::new().set_accessed(SystemTime::now()));
                }
                self.stats.hits += 1;
                Ok(Some(self.remember(key, pixels)))
            }
            disk => {
                if matches!(disk, DiskImage::Invalid) {
                    let _ = fs::remove_file(path);
                }
                self.stats.misses += 1;
                Ok(None)
            }
        }
    }

    pub fn read(&mut self, asset: &Asset) -> Result<Option<Arc<Pixels>>> {
        let key = asset.key();
        if let Some(pixels) = self.memory(&key) {
            return Ok(Some(pixels));
        }
        let disk = disk_image(&self.directory.join(format!("{key}.png")), asset.dimension)?;
        self.finish_read(key, disk)
    }

    pub(super) fn read_shared(
        cache: &std::sync::Mutex<Self>,
        asset: &Asset,
    ) -> Result<Option<Arc<Pixels>>> {
        let key = asset.key();
        let path = {
            let mut cache = cache.lock().unwrap();
            if let Some(pixels) = cache.memory(&key) {
                return Ok(Some(pixels));
            }
            cache.directory.join(format!("{key}.png"))
        };
        let disk = disk_image(&path, asset.dimension)?;
        cache.lock().unwrap().finish_read(key, disk)
    }

    pub fn store(&mut self, asset: &Asset, bytes: &[u8]) -> Result<Arc<Pixels>> {
        self.store_prepared(asset, prepare_image(bytes, asset.dimension)?)
    }

    pub(super) fn store_shared(
        cache: &std::sync::Mutex<Self>,
        asset: &Asset,
        bytes: &[u8],
    ) -> Result<Arc<Pixels>> {
        let prepared = prepare_image(bytes, asset.dimension)?;
        cache.lock().unwrap().store_prepared(asset, prepared)
    }

    fn store_prepared(
        &mut self,
        asset: &Asset,
        (pixels, png): (Pixels, Vec<u8>),
    ) -> Result<Arc<Pixels>> {
        self.prune(png.len() as u64)?;
        let key = asset.key();
        private_write(&self.directory.join(format!("{key}.png")), &png)?;
        self.stats.downloads += 1;
        Ok(self.remember(key, pixels))
    }

    fn remember(&mut self, key: String, pixels: Pixels) -> Arc<Pixels> {
        self.tick += 1;
        let pixels = Arc::new(pixels);
        while self
            .memory
            .values()
            .map(|(p, _)| p.rgba.len())
            .sum::<usize>()
            + pixels.rgba.len()
            > MEMORY_LIMIT
        {
            let Some(oldest) = self
                .memory
                .iter()
                .min_by_key(|(_, (_, tick))| tick)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.memory.remove(&oldest);
        }
        self.memory.insert(key, (pixels.clone(), self.tick));
        pixels
    }

    fn prune(&mut self, incoming: u64) -> Result<()> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let metadata = entry.metadata()?;
            if metadata.modified()?.elapsed().unwrap_or_default()
                > Duration::from_secs(30 * 24 * 3600)
            {
                fs::remove_file(entry.path())?;
                continue;
            }
            entries.push((
                metadata.accessed().or_else(|_| metadata.modified())?,
                metadata.len(),
                entry.path(),
            ));
        }
        entries.sort_by_key(|entry| entry.0);
        let mut total: u64 = entries.iter().map(|entry| entry.1).sum();
        let mut count = entries.len();
        for (_, size, path) in entries {
            if total + incoming <= DISK_LIMIT && count < 4096 {
                break;
            }
            fs::remove_file(path)?;
            total -= size;
            count -= 1;
        }
        Ok(())
    }
}

enum DiskImage {
    Missing,
    Invalid,
    Pixels(Pixels),
}
fn disk_image(path: &Path, dimension: u32) -> Result<DiskImage> {
    if let Ok(metadata) = path.metadata()
        && metadata.len() <= MAX_DOWNLOAD as u64
        && metadata.modified()?.elapsed().unwrap_or_default() < Duration::from_secs(30 * 24 * 3600)
    {
        return Ok(
            match fs::read(path).ok().and_then(|v| pixels(&v, dimension).ok()) {
                Some(pixels) => DiskImage::Pixels(pixels),
                None => DiskImage::Invalid,
            },
        );
    }
    Ok(DiskImage::Missing)
}

fn prepare_image(bytes: &[u8], dimension: u32) -> Result<(Pixels, Vec<u8>)> {
    let pixels = pixels(bytes, dimension)?;
    let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
        .context("invalid thumbnail pixels")?;
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok((pixels, png.into_inner()))
}

fn pixels(bytes: &[u8], dimension: u32) -> Result<Pixels> {
    ensure!(bytes.len() <= MAX_DOWNLOAD, "encoded image too large");
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().context("unsupported or damaged image")?;
    let image = if decoded.width() > dimension || decoded.height() > dimension {
        decoded.thumbnail(dimension, dimension).to_rgba8()
    } else {
        decoded.to_rgba8()
    };
    Ok(Pixels {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

/// Source images are decoded on demand and never put in the thumbnail cache.
pub(super) fn original_pixels(bytes: &[u8]) -> Result<Pixels> {
    pixels(bytes, 8192)
}

/// The software renderer doesn't round image clipping. Supply a masked square
/// so old square cache entries also render as circular avatars.
pub fn circular_avatar(pixels: &Pixels) -> Pixels {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_photo_uses_source_object_and_avatar_is_a_centered_circle() -> Result<()> {
        let encrypted = Asset::photo(
            &json!({"id":"42", "contentMetadata":{"SID":"m","OID":"asset42","ENC_KM":"synthetic"}}),
        )
        .unwrap();
        assert!(encrypted.url.ends_with("asset42__ud-preview"));
        let original = encrypted.original().unwrap();
        assert_eq!(original.url, "https://obs.line-apps.com/r/talk/m/asset42");
        assert_eq!(original.key_material, encrypted.key_material);
        assert_ne!(original.key(), encrypted.key());
        let plain = Asset::photo(&json!({"id":"43","contentMetadata":{"MEDIA_CONTENT_INFO":"{\"category\":\"original\"}"}})).unwrap();
        assert!(plain.original().unwrap().url.ends_with("/43/original"));
        let image = image::RgbaImage::from_pixel(1600, 1000, image::Rgba([25, 70, 200, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png)?;
        let source = original_pixels(bytes.get_ref())?;
        assert_eq!((source.width, source.height), (1600, 1000));
        assert_eq!(pixels(bytes.get_ref(), 384)?.width, 384);
        let circle = circular_avatar(&source);
        assert_eq!((circle.width, circle.height), (64, 64));
        assert_eq!(circle.rgba[3], 0);
        assert_eq!(circle.rgba[(32 * 64 + 32) * 4 + 3], 255);
        assert_eq!(
            &circle.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3],
            &[25, 70, 200]
        );
        Ok(())
    }
    #[test]
    fn decrypts_independent_go_media_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/media.json")).unwrap();
        let bytes = decode(fixture["ciphertext"].as_str().unwrap()).unwrap();
        let material = decode(fixture["material"].as_str().unwrap()).unwrap();
        assert_eq!(
            decrypt_media(&bytes, &material).unwrap(),
            fixture["plaintext"].as_str().unwrap().as_bytes()
        );
    }
    #[test]
    fn authenticated_media_rejects_modified_ciphertext() {
        use aes::cipher::{KeyIvInit, StreamCipher};
        use hmac::{Hmac, Mac};
        let material = [7; 32];
        let mut keys = [0; 76];
        hkdf::Hkdf::<Sha256>::new(None, &material)
            .expand(b"FileEncryption", &mut keys)
            .unwrap();
        let mut iv = [0; 16];
        iv[..12].copy_from_slice(&keys[64..]);
        let mut bytes = "synthetic media\0日本語".as_bytes().to_vec();
        ctr::Ctr32BE::<aes::Aes256>::new_from_slices(&keys[..32], &iv)
            .unwrap()
            .apply_keystream(&mut bytes);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64]).unwrap();
        mac.update(&bytes);
        bytes.extend(mac.finalize().into_bytes());
        assert_eq!(
            decrypt_media(&bytes, &material).unwrap(),
            "synthetic media\0日本語".as_bytes()
        );
        bytes[0] ^= 1;
        assert!(decrypt_media(&bytes, &material).is_err());
    }
    #[test]
    fn thumbnail_cache_restores_across_instances_and_is_private() {
        let root = std::env::temp_dir().join(format!("line-cache-test-{}", std::process::id()));
        let asset = Asset::avatar("/test").unwrap();
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let mut cache = ImageCache::new(&root, "synthetic-account").unwrap();
        assert!(cache.read(&asset).unwrap().is_none());
        cache.store(&asset, bytes.get_ref()).unwrap();
        let directory = cache.directory.clone();
        drop(cache);
        let mut cache = ImageCache::new(&root, "synthetic-account").unwrap();
        let pixels = cache.read(&asset).unwrap().unwrap();
        assert_eq!((pixels.width, pixels.height), (2, 3));
        assert_eq!(
            fs::metadata(directory.join(format!("{}.png", asset.key())))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(
            ImageCache::new(&root, "other-account")
                .unwrap()
                .read(&asset)
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn asset_paths_cannot_change_origins_or_escape_segments() {
        assert!(Asset::avatar("https://evil.invalid/image").is_none());
        assert!(Asset::sticker(&json!({"STKID":"../../escape"})).is_none());
        assert!(!allowed_url(
            "https://obs.line-apps.com@evil.invalid/r/talk/m/1"
        ));
    }
}
