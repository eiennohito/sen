//! Private, account-scoped thumbnail cache. Never stores URLs or media keys.
use super::*;
use image::{ImageFormat, ImageReader};
use std::{collections::HashMap, io::Cursor};

const DISK_LIMIT: u64 = 128 * 1024 * 1024;
const MEMORY_LIMIT: usize = 24 * 1024 * 1024;
const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;

mod failure;
pub use failure::Failure;
mod fetchers;
pub use fetchers::{Completion, Failures, Fetchers, Payload, Priority, Purpose};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    url: String,
    #[serde(default)]
    local: Option<Box<crate::attachments::Attachment>>,
    pub(super) private: bool,
    key_material: Option<String>,
    talk_meta: Option<String>,
    dimension: u32,
    #[serde(default)]
    original_suffix: Option<String>,
    #[serde(default)]
    original_url: Option<String>,
    #[serde(default)]
    failure: Option<Failure>,
    #[serde(default)]
    name: Option<String>,
}

impl Asset {
    pub fn local(item: crate::attachments::Attachment) -> Self {
        Self {
            url: format!("local:{}", item.path.display()),
            private: true,
            key_material: None,
            talk_meta: None,
            dimension: 384,
            original_suffix: None,
            original_url: None,
            failure: None,
            name: Some(item.name.clone()),
            local: Some(Box::new(item)),
        }
    }
    pub fn local_attachment(&self) -> Option<&crate::attachments::Attachment> {
        self.local.as_deref()
    }
    pub fn local_pixels(&self) -> Result<Option<Pixels>> {
        self.local
            .as_ref()
            .filter(|item| !item.preview.is_empty())
            .map(|item| pixels(&item.preview, 384))
            .transpose()
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or_else(|| {
            self.url
                .trim_end_matches("/preview")
                .trim_end_matches("/original")
                .rsplit('/')
                .next()
                .unwrap_or("Photo")
        })
    }
    pub fn failure(&self) -> Option<Failure> {
        self.failure
    }
    pub fn kind(&self) -> &'static str {
        if self.private || self.url.starts_with("https://manager.line-scdn.net/") {
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
        let failure = self.failure.map(|f| format!(":{f:?}")).unwrap_or_default();
        format!(
            "{:x}",
            Sha256::digest(format!(
                "v1:{}:{}:{}:{}{failure}",
                self.url, self.dimension, self.private, material
            ))
        )
    }

    pub fn avatar(path: &str) -> Option<Self> {
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
        let failure = (!http::Transport::production_asset_url(&url, None))
            .then_some(Failure::UnsupportedHost);
        Some(Self {
            url,
            private: false,
            key_material: None,
            talk_meta: None,
            dimension: 256,
            local: None,
            original_url: None,
            original_suffix: None,
            name: None,
            failure,
        })
    }

    pub fn sticker(meta: &Value) -> Option<Self> {
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
            local: None,
            original_url: None,
            original_suffix: None,
            name: None,
            failure: None,
        })
    }

    pub fn inline_emoji(span: &crate::store::InlineEmoji) -> Option<Self> {
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
            local: None,
            original_url: None,
            original_suffix: None,
            name: None,
            failure: None,
        })
    }

    pub(crate) fn photo(message: &Value) -> Option<Self> {
        let meta = &message["contentMetadata"];
        let id = message["id"].as_str()?;
        if meta["SID"].is_null()
            && meta["OID"].is_null()
            && let Some(url) = meta["PREVIEW_URL"].as_str()
        {
            let download = meta["DOWNLOAD_URL"].as_str();
            return Some(Self {
                url: url.into(),
                private: false,
                key_material: None,
                talk_meta: None,
                dimension: 384,
                original_suffix: None,
                local: None,
                original_url: download.map(str::to_owned),
                name: None,
                failure: std::iter::once(url)
                    .chain(download)
                    .any(|url| {
                        !http::Transport::production_asset_url(url, Some("manager.line-scdn.net"))
                    })
                    .then_some(Failure::UnsupportedHost),
            });
        }
        let sid = meta["SID"].as_str().unwrap_or("m");
        let oid = meta["OID"].as_str().unwrap_or(id);
        let failure = route_failure(sid, oid);
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
            name: None,
            failure,
            local: None,
            original_url: None,
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
        let id = raw["id"].as_str()?;
        let key = meta["ENC_KM"].as_str().map(str::to_owned);
        let (sid, oid) = if key.is_some() {
            (
                meta["SID"].as_str().unwrap_or("m"),
                meta["OID"].as_str().unwrap_or(id),
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
            local: None,
            original_url: None,
            original_suffix: None,
            name: None,
            failure: route_failure(sid, oid).or_else(|| {
                ["DOWNLOAD_URL", "DOWNLOAD_URL_RENEWAL"]
                    .iter()
                    .any(|key| meta[*key].as_str().is_some_and(|url| !url.is_empty()))
                    .then_some(Failure::UnsupportedHost)
            }),
        })
    }

    /// Chrome's encrypted source uses the bare OID; __ud-preview is only a thumbnail.
    pub fn original(&self) -> Option<Self> {
        if let Some(url) = &self.original_url {
            let mut original = self.clone();
            original.url = url.clone();
            original.failure = self.failure.or_else(|| {
                (!http::Transport::production_asset_url(url, Some("manager.line-scdn.net")))
                    .then_some(Failure::UnsupportedHost)
            });
            original.original_url = None;
            original.dimension = 8192;
            return Some(original);
        }
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
        if let Some(item) = &self.local {
            return Ok(item.preview.clone());
        }
        if let Some(failure) = self.failure {
            return Err(failure.into());
        }
        ensure!(http.allowed_asset_url(&self.url), Failure::UnsupportedHost);
        let application = format!("CHROMEOS\t{}\tChrome_OS\t", http.client_version);
        let mut headers = Vec::new();
        if self.private {
            let token = access_token.ok_or(Failure::Network)?;
            ensure!(!token.contains(['\r', '\n']), "invalid media token");
            headers.push(("X-Line-Access", token));
            headers.push(("X-Line-Application", application.as_str()));
            if let Some(meta) = &self.talk_meta {
                headers.push(("X-Talk-Meta", meta.as_str()));
            }
        }
        let bytes = http.asset(&self.url, &headers, MAX_DOWNLOAD)?;
        if let Some(key) = &self.key_material {
            decrypt_media(&bytes, &decode(key).map_err(|_| Failure::Decode)?)
                .map_err(|_| Failure::Decode.into())
        } else {
            Ok(bytes)
        }
    }
}

fn route_failure(sid: &str, oid: &str) -> Option<Failure> {
    (segment(sid).is_none() || segment(oid).is_none()).then_some(Failure::RejectedPath)
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

pub(super) fn encrypt_media(mut ciphertext: Vec<u8>, material: &[u8]) -> Result<Vec<u8>> {
    use aes::cipher::{KeyIvInit, StreamCipher};
    use hmac::{Hmac, Mac};
    ensure!(material.len() == 32, "invalid media key material");
    let mut keys = [0_u8; 76];
    hkdf::Hkdf::<Sha256>::new(None, material)
        .expand(b"FileEncryption", &mut keys)
        .map_err(|_| anyhow::anyhow!("media key derivation failed"))?;
    let mut counter = [0_u8; 16];
    counter[..12].copy_from_slice(&keys[64..]);
    ctr::Ctr32BE::<aes::Aes256>::new_from_slices(&keys[..32], &counter)?
        .apply_keystream(&mut ciphertext);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64])?;
    mac.update(&ciphertext);
    ciphertext.extend(mac.finalize().into_bytes());
    Ok(ciphertext)
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

pub struct ImageCache {
    directory: PathBuf,
    memory: HashMap<String, (Arc<Pixels>, u64)>,
    tick: u64,
}

impl ImageCache {
    #[cfg(test)]
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn new(root: &Path, account: &str) -> Result<Self> {
        let directory = root.join(format!("{:x}", Sha256::digest(account.as_bytes())));
        super::super::private_directory(&directory)?;
        let mut cache = Self {
            directory,
            memory: HashMap::new(),
            tick: 0,
        };
        cache.prune(0)?;
        Ok(cache)
    }
    fn memory(&mut self, key: &str) -> Option<Arc<Pixels>> {
        self.tick += 1;
        let (pixels, used) = self.memory.get_mut(key)?;
        *used = self.tick;
        Some(pixels.clone())
    }

    fn finish_read(&mut self, key: String, disk: DiskImage) -> Result<Option<Arc<Pixels>>> {
        let path = self.directory.join(format!("{key}.png"));
        match disk {
            DiskImage::Pixels(pixels) => {
                if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                    let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
                }
                Ok(Some(self.remember(key, pixels)))
            }
            disk => {
                if matches!(disk, DiskImage::Invalid) {
                    let _ = fs::remove_file(path);
                }
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
            entries.push((metadata.modified()?, metadata.len(), entry.path()));
        }
        entries.sort_by_key(|entry| entry.0);
        let mut total: u64 = entries.iter().map(|entry| entry.1).sum();
        let mut count = entries.len();
        for (_, size, path) in entries {
            if total + incoming <= DISK_LIMIT && count + usize::from(incoming > 0) <= 4096 {
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
    ensure!(bytes.len() <= MAX_DOWNLOAD, Failure::TooLarge);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| match error {
        image::ImageError::Limits(_) => Failure::TooLarge,
        _ => Failure::Decode,
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn recently_read_old_media_survives_prune() -> Result<()> {
        let root = std::env::temp_dir().join(format!("line-cache-lru-{}", std::process::id()));
        let mut cache = ImageCache::new(&root, "synthetic-account")?;
        let read = Asset::avatar("/read").unwrap();
        let unread = Asset::avatar("/unread").unwrap();
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, ImageFormat::Png)?;
        let old = SystemTime::now() - Duration::from_secs(60 * 24 * 3600);
        for asset in [&read, &unread] {
            let path = cache.directory.join(format!("{}.png", asset.key()));
            private_write(&path, png.get_ref())?;
            OpenOptions::new()
                .write(true)
                .open(path)?
                .set_times(fs::FileTimes::new().set_modified(old))?;
        }
        // Reopening must not expire old files, and a disk hit must refresh mtime.
        cache = ImageCache::new(&root, "synthetic-account")?;
        assert!(cache.read(&read)?.is_some());
        assert!(
            cache
                .directory
                .join(format!("{}.png", unread.key()))
                .exists()
        );
        cache.prune(DISK_LIMIT - png.get_ref().len() as u64)?;
        assert!(cache.directory.join(format!("{}.png", read.key())).exists());
        assert!(
            !cache
                .directory
                .join(format!("{}.png", unread.key()))
                .exists()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn original_photo_uses_source_object() -> Result<()> {
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
        Ok(())
    }
    #[test]
    fn decrypts_independent_go_media_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/media.json")).unwrap();
        let bytes = decode(fixture["ciphertext"].as_str().unwrap()).unwrap();
        let material = decode(fixture["material"].as_str().unwrap()).unwrap();
        assert_eq!(
            encrypt_media(
                fixture["plaintext"].as_str().unwrap().as_bytes().to_vec(),
                &material
            )
            .unwrap(),
            bytes
        );
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
        let oa = Asset::photo(&json!({"id":"99","contentMetadata":{
            "PREVIEW_URL":"https://manager.line-scdn.net/id/clearpreview",
            "DOWNLOAD_URL":"https://manager.line-scdn.net/id/original"}}))
        .unwrap();
        assert_eq!(oa.kind(), "photo");
        assert_eq!(oa.url, "https://manager.line-scdn.net/id/clearpreview");
        assert_eq!(
            oa.original().unwrap().url,
            "https://manager.line-scdn.net/id/original"
        );
        assert!(!oa.private && oa.key_material.is_none() && oa.talk_meta.is_none());
        assert!(oa.failure.is_none());
        for url in [
            "https://manager.line-scdn.net.evil.invalid/id",
            "https://manager.line-scdn.net@evil.invalid/id",
            "https://manager.line-scdn.net:443/id",
            "https://evil.invalid/id",
            "http://manager.line-scdn.net/id",
            "https://profile.line-scdn.net/id",
            "https://obs.line-apps.com/id",
            "https://stickershop.line-scdn.net/id",
        ] {
            let bad =
                Asset::photo(&json!({"id":"99","contentMetadata":{"PREVIEW_URL":url}})).unwrap();
            assert_eq!(bad.failure(), Some(Failure::UnsupportedHost));
            let bad_original = Asset::photo(
                &json!({"id":"99","contentMetadata":{"PREVIEW_URL":oa.url,"DOWNLOAD_URL":url}}),
            )
            .unwrap()
            .original()
            .unwrap();
            assert_eq!(bad_original.failure(), Some(Failure::UnsupportedHost));
        }
        assert!(Asset::avatar("https://evil.invalid/image").is_none());
        assert!(Asset::sticker(&json!({"STKID":"../../escape"})).is_none());
        assert!(
            !http::Transport::local("test", String::new())
                .allowed_asset_url("https://obs.line-apps.com@evil.invalid/r/talk/m/1")
        );
    }
}
