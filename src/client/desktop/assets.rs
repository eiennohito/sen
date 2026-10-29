//! Private, account-scoped thumbnail cache. Never stores URLs or media keys.
use super::*;
use image::{ImageFormat, ImageReader};
use std::io::Cursor;

const DISK_LIMIT: u64 = 128 * 1024 * 1024;
const ORIGINAL_LIMIT: u64 = 32 * 1024 * 1024;
const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;
const MAX_MEDIA: u64 = 1024 * 1024 * 1024;

mod failure;
pub use failure::Failure;
mod fetchers;
pub use fetchers::{Completion, Failures, Fetchers, Payload, Purpose, media_name};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Played by the system player: streamed to disk, never decoded here.
    #[serde(default)]
    video: bool,
}

impl std::hash::Hash for Asset {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(
            &(
                &self.url,
                self.dimension,
                self.private,
                &self.key_material,
                self.failure,
                self.video,
            ),
            state,
        );
    }
}

impl Asset {
    pub fn dimension(&self) -> u32 {
        self.dimension
    }
    pub fn request(&self) -> Self {
        Self {
            url: self.url.clone(),
            dimension: self.dimension,
            private: self.private,
            local: None,
            key_material: self.key_material.clone(),
            talk_meta: self.talk_meta.clone(),
            original_suffix: self.original_suffix.clone(),
            original_url: self.original_url.clone(),
            failure: self.failure,
            name: self.name.clone(),
            video: self.video,
        }
    }

    fn remote(url: String, dimension: u32, private: bool) -> Self {
        Self {
            url,
            dimension,
            private,
            local: None,
            key_material: None,
            talk_meta: None,
            original_suffix: None,
            original_url: None,
            failure: None,
            name: None,
            video: false,
        }
    }

    pub fn local(item: crate::attachments::Attachment) -> Self {
        let mut asset = Self::remote(format!("local:{}", item.path.display()), 384, true);
        asset.name = Some(item.name.clone());
        asset.local = Some(Box::new(item));
        asset
    }

    pub fn local_attachment(&self) -> Option<&crate::attachments::Attachment> {
        self.local.as_deref()
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
    /// The sender's own file name, never one derived from the object URL.
    pub fn file_name(&self) -> Option<&str> {
        self.name.as_deref().filter(|name| !name.is_empty())
    }
    /// Video bytes sit at the preview's bare object, like a file's.
    pub fn video(preview: &Self) -> Option<Self> {
        let mut video = preview.original()?;
        video.video = true;
        Some(video)
    }
    pub fn is_video(&self) -> bool {
        self.video
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
        let video = if self.video { ":video" } else { "" };
        format!(
            "{:x}",
            Sha256::digest(format!(
                "v2:{}:{}:{}:{}{failure}{video}",
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
            failure,
            ..Self::remote(url, 192, false)
        })
    }

    // Incomplete sticker metadata keeps the message label visible without fetching an image.
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
        Some(Self::remote(
            format!(
                "https://stickershop.line-scdn.net/stickershop/v{version}/sticker/{id}{hash}/android{base}/sticker.png"
            ),
            256,
            false,
        ))
    }

    pub fn inline_emoji(span: &crate::store::InlineEmoji) -> Option<Self> {
        Self::paid_reaction(&json!({"productId":span.product_id,"emojiId":span.sticon_id}))
    }

    pub(super) fn paid_reaction(value: &Value) -> Option<Self> {
        let product = segment(value["productId"].as_str()?)?;
        let emoji = segment(value["emojiId"].as_str()?)?;
        Some(Self::remote(
            format!(
                "https://stickershop.line-scdn.net/sticonshop/v1/sticon/{product}/android/{emoji}.png"
            ),
            32,
            false,
        ))
    }

    // A photo without an object identity keeps its placeholder and cannot be fetched.
    pub(crate) fn photo(message: &Value) -> Option<Self> {
        let meta = &message["contentMetadata"];
        let id = message["id"].as_str()?;
        if meta["SID"].is_null()
            && meta["OID"].is_null()
            && let Some(url) = meta["PREVIEW_URL"].as_str()
        {
            let download = meta["DOWNLOAD_URL"].as_str();
            return Some(Self {
                original_url: download.map(str::to_owned),
                failure: std::iter::once(url)
                    .chain(download)
                    .any(|url| {
                        !http::Transport::production_asset_url(url, Some("manager.line-scdn.net"))
                    })
                    .then_some(Failure::UnsupportedHost),
                name: file_name(meta),
                ..Self::remote(url.into(), 384, false)
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
            key_material: key,
            talk_meta: Some(talk_meta(id)),
            failure,
            name: file_name(meta),
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
            ..Self::remote(
                format!("https://obs.line-apps.com/r/talk/{sid}/{oid}{suffix}"),
                384,
                true,
            )
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
            key_material: key,
            talk_meta: Some(talk_meta(id)),
            failure: route_failure(sid, oid).or_else(|| {
                ["DOWNLOAD_URL", "DOWNLOAD_URL_RENEWAL"]
                    .iter()
                    .any(|key| meta[*key].as_str().is_some_and(|url| !url.is_empty()))
                    .then_some(Failure::UnsupportedHost)
            }),
            ..Self::remote(
                format!("https://obs.line-apps.com/r/talk/{sid}/{oid}"),
                0,
                true,
            )
        })
    }

    /// Chrome's encrypted source uses the bare OID; __ud-preview is only a thumbnail.
    pub fn original(&self) -> Option<Self> {
        if self.dimension == 8192 {
            return None;
        }
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
            let mut original = self.clone();
            original.url = match self.kind() {
                "avatar" => self.url.strip_suffix("/preview")?.to_owned(),
                "sticker" => self.url.clone(),
                _ => return None,
            };
            original.dimension = 8192;
            return Some(original);
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
        if let Some(item) = &self.local
            && !item.preview.is_empty()
        {
            return Ok(item.preview.clone());
        }
        if let Some(path) = self.url.strip_prefix("local:") {
            return fs::read(path).map_err(Into::into);
        }
        let bytes =
            self.with_headers(http, access_token, |headers| http.asset(&self.url, headers, MAX_DOWNLOAD))?;
        if let Some(key) = &self.key_material {
            decrypt_media(&bytes, &decode(key).map_err(|_| Failure::Decode)?)
                .map_err(|_| Failure::Decode.into())
        } else {
            Ok(bytes)
        }
    }

    /// Streams plaintext into `out` without holding the whole object in memory.
    /// Encrypted bytes reach `out` before the tag is checked: callers keep the
    /// output private and discard it unless this returns `Ok`.
    pub(super) fn stream(
        &self,
        http: &http::Transport,
        access_token: Option<&str>,
        out: &mut impl std::io::Write,
    ) -> Result<()> {
        if let Some(path) = self.url.strip_prefix("local:") {
            std::io::copy(&mut fs::File::open(path)?, out)?;
            return Ok(());
        }
        let mut decryptor = self
            .key_material
            .as_ref()
            .map(|key| MediaDecryptor::new(&decode(key).map_err(|_| Failure::Decode)?))
            .transpose()?;
        self.with_headers(http, access_token, |headers| {
            http.asset_stream(&self.url, headers, MAX_MEDIA, |bytes| match &mut decryptor {
                Some(decryptor) => decryptor.push(bytes, out),
                None => out.write_all(bytes).map_err(Into::into),
            })
        })?;
        match decryptor {
            Some(decryptor) => decryptor.finish(out).map_err(|_| Failure::Decode.into()),
            None => Ok(()),
        }
    }

    fn with_headers<T>(
        &self,
        http: &http::Transport,
        access_token: Option<&str>,
        request: impl FnOnce(&[(&str, &str)]) -> Result<T>,
    ) -> Result<T> {
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
        request(&headers)
    }
}

/// Upload metadata names files; photos carry one only from some senders.
fn file_name(meta: &Value) -> Option<String> {
    meta["FILE_NAME"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Streaming form of `decrypt_media`. Chrome authenticates a video's
/// ciphertext through SHA-256 digests of its 128 KiB chunks and everything
/// else directly; both are tracked, since the video rule is read from code,
/// not traffic.
struct MediaDecryptor {
    cipher: ctr::Ctr32BE<aes::Aes256>,
    direct: hmac::Hmac<Sha256>,
    chunked: hmac::Hmac<Sha256>,
    chunk: Sha256,
    chunk_len: usize,
    held: Vec<u8>,
}

const MEDIA_CHUNK: usize = 128 * 1024;

impl MediaDecryptor {
    fn new(material: &[u8]) -> Result<Self> {
        let (cipher, mac) = media_cipher(material)?;
        Ok(Self {
            cipher,
            direct: mac.clone(),
            chunked: mac,
            chunk: Sha256::new(),
            chunk_len: 0,
            held: Vec::new(),
        })
    }
    fn push(&mut self, bytes: &[u8], out: &mut impl std::io::Write) -> Result<()> {
        use aes::cipher::StreamCipher;
        use hmac::Mac;
        // The trailing 32 bytes may be the tag, so they wait for the next push.
        self.held.extend_from_slice(bytes);
        let Some(ready) = self.held.len().checked_sub(32).filter(|n| *n > 0) else {
            return Ok(());
        };
        let mut body: Vec<u8> = self.held.drain(..ready).collect();
        self.direct.update(&body);
        let mut rest = body.as_slice();
        while !rest.is_empty() {
            let take = rest.len().min(MEDIA_CHUNK - self.chunk_len);
            self.chunk.update(&rest[..take]);
            self.chunk_len += take;
            rest = &rest[take..];
            if self.chunk_len == MEDIA_CHUNK {
                self.chunked.update(&std::mem::take(&mut self.chunk).finalize());
                self.chunk_len = 0;
            }
        }
        self.cipher.apply_keystream(&mut body);
        out.write_all(&body)?;
        Ok(())
    }
    fn finish(mut self, out: &mut impl std::io::Write) -> Result<()> {
        use hmac::Mac;
        ensure!(self.held.len() == 32, "invalid encrypted media envelope");
        if self.chunk_len > 0 {
            self.chunked.update(&self.chunk.finalize());
        }
        let direct = self.direct.verify_slice(&self.held).is_ok();
        ensure!(
            direct || self.chunked.verify_slice(&self.held).is_ok(),
            "media authentication failed"
        );
        out.flush()?;
        Ok(())
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
    use aes::cipher::StreamCipher;
    use hmac::Mac;
    let (mut cipher, mut mac) = media_cipher(material)?;
    cipher.apply_keystream(&mut ciphertext);
    mac.update(&ciphertext);
    ciphertext.extend(mac.finalize().into_bytes());
    Ok(ciphertext)
}

fn decrypt_media(bytes: &[u8], material: &[u8]) -> Result<Vec<u8>> {
    use aes::cipher::StreamCipher;
    use hmac::Mac;
    ensure!(
        material.len() == 32 && bytes.len() >= 32,
        "invalid encrypted media envelope"
    );
    let (mut cipher, mut mac) = media_cipher(material)?;
    let (ciphertext, tag) = bytes.split_at(bytes.len() - 32);
    mac.update(ciphertext);
    mac.verify_slice(tag)
        .map_err(|_| anyhow::anyhow!("media authentication failed"))?;
    let mut plaintext = ciphertext.to_vec();
    cipher.apply_keystream(&mut plaintext);
    Ok(plaintext)
}

fn media_cipher(material: &[u8]) -> Result<(ctr::Ctr32BE<aes::Aes256>, hmac::Hmac<Sha256>)> {
    use aes::cipher::KeyIvInit;
    use hmac::Mac;
    ensure!(material.len() == 32, "invalid media key material");
    let mut keys = [0_u8; 76];
    hkdf::Hkdf::<Sha256>::new(None, material)
        .expand(b"FileEncryption", &mut keys)
        .map_err(|_| anyhow::anyhow!("media key derivation failed"))?;
    let mut counter = [0_u8; 16];
    counter[..12].copy_from_slice(&keys[64..]);
    Ok((
        ctr::Ctr32BE::<aes::Aes256>::new_from_slices(&keys[..32], &counter)?,
        <hmac::Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64])?,
    ))
}

pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct ImageCache {
    directory: PathBuf,
    paths: HashMap<String, CacheFile>,
    totals: [u64; 2],
    counts: [usize; 2],
}

struct CacheFile {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

impl ImageCache {
    pub fn store_pixels(&mut self, key: &str, pixels: Pixels) -> Result<PathBuf> {
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)
            .context("invalid image pixels")?
            .write_to(&mut png, ImageFormat::Png)?;
        self.write(key, png.get_ref(), "png")
    }

    #[cfg(test)]
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn new(root: &Path, account: &str) -> Result<Self> {
        let directory = crate::locations::account_directory(root, account)?;
        super::super::private_directory(&directory)?;
        let mut cache = Self {
            directory,
            paths: HashMap::new(),
            totals: [0; 2],
            counts: [0; 2],
        };
        cache.index()?;
        cache.prune(0, false)?;
        cache.prune_media();
        Ok(cache)
    }
    /// Files handed to other applications. Kept for a day so a reopened
    /// video isn't fetched again; an open player keeps its own descriptor.
    pub fn media_directory(&self, key: &str) -> Result<PathBuf> {
        ensure!(segment(key).is_some(), Failure::RejectedPath);
        let path = self.directory.join("media").join(key);
        private_directory(&self.directory.join("media"))?;
        private_directory(&path)?;
        Ok(path)
    }
    fn prune_media(&self) {
        let day = std::time::Duration::from_secs(24 * 60 * 60);
        let Ok(entries) = fs::read_dir(self.directory.join("media")) else {
            return;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_none_or(|age| age > day);
            if stale {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    fn cache_key(asset: &Asset) -> String {
        if asset.dimension == 8192 {
            format!("original-{}", asset.key())
        } else {
            asset.key()
        }
    }
    pub fn read(&mut self, asset: &Asset) -> Result<Option<PathBuf>> {
        let path = self.read_key(&Self::cache_key(asset))?;
        if let Some(path) = &path
            && has_small(asset)
            && !small_path(path).is_file()
        {
            let bytes = small_png(&image::open(path)?.to_rgba8())?;
            self.prune(bytes.len() as u64, false)?;
            self.replace(&format!("small-{}", asset.key()), &small_path(path), &bytes)?;
        }
        Ok(path)
    }
    fn read_key(&mut self, key: &str) -> Result<Option<PathBuf>> {
        let Some(path) = self.paths.get(key).map(|file| file.path.clone()) else {
            return Ok(None);
        };
        if !path.is_file() {
            self.forget(key);
            return Ok(None);
        }
        if ImageReader::open(&path)?
            .with_guessed_format()?
            .into_dimensions()
            .is_err()
        {
            remove_file(&path)?;
            self.forget(key);
            return Ok(None);
        }
        let now = SystemTime::now();
        OpenOptions::new()
            .write(true)
            .open(&path)?
            .set_times(fs::FileTimes::new().set_modified(now))?;
        self.paths.get_mut(key).unwrap().modified = now;
        Ok(Some(path))
    }
    pub fn read_original(&mut self, asset: &Asset) -> Result<Option<(PathBuf, PathBuf)>> {
        use image::ImageDecoder;
        // Sticker originals use exactly the same file as their inline image.
        let source = match self.read(asset)? {
            None if asset.kind() == "sticker" => {
                let mut thumbnail = asset.clone();
                thumbnail.dimension = 256;
                self.read(&thumbnail)?
            }
            source => source,
        };
        let Some(source) = source else {
            return Ok(None);
        };
        let reader = ImageReader::open(&source)?.with_guessed_format()?;
        if reader.into_decoder()?.orientation()? == image::metadata::Orientation::NoTransforms {
            return Ok(Some((source.clone(), source)));
        }
        let display_key = format!("display-{}", asset.key());
        let path = match self.read_key(&display_key)? {
            Some(path) => path,
            None => self.rotated(&display_key, &fs::read(&source)?)?,
        };
        Ok(Some((path, source)))
    }
    pub fn store(&mut self, asset: &Asset, bytes: &[u8]) -> Result<PathBuf> {
        let image = prepare_image(
            bytes,
            asset.dimension,
            asset.kind() == "avatar" && asset.dimension < 8192,
        )?;
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png)?;
        let small = if has_small(asset) {
            Some(small_png(&image)?)
        } else {
            None
        };
        self.prune(
            (png.get_ref().len() + small.as_ref().map_or(0, Vec::len)) as u64,
            asset.dimension == 8192,
        )?;
        let key = Self::cache_key(asset);
        let path = self
            .directory
            .join(format!("{key}-{:x}.png", Sha256::digest(png.get_ref())));
        self.replace(&key, &path, png.get_ref())?;
        if let Some(bytes) = small {
            self.replace(&format!("small-{key}"), &small_path(&path), &bytes)?;
        }
        Ok(path)
    }
    fn write(&mut self, key: &str, bytes: &[u8], extension: &str) -> Result<PathBuf> {
        self.prune(bytes.len() as u64, original_key(key))?;
        // Content-addressed paths keep Slint's path cache valid after a rewrite.
        let path = self
            .directory
            .join(format!("{key}-{:x}.{extension}", Sha256::digest(bytes)));
        self.replace(key, &path, bytes)?;
        Ok(path)
    }
    fn replace(&mut self, key: &str, path: &Path, bytes: &[u8]) -> Result<()> {
        private_write(path, bytes)?;
        let previous = self.forget(key);
        self.insert(
            key.to_owned(),
            CacheFile {
                path: path.to_owned(),
                size: bytes.len() as u64,
                modified: SystemTime::now(),
            },
        );
        if let Some(previous) = previous
            && previous.path != path
        {
            remove_file(&previous.path)?;
        }
        Ok(())
    }
    pub fn original(&mut self, asset: &Asset, bytes: &[u8]) -> Result<(PathBuf, PathBuf)> {
        use image::ImageDecoder;
        ensure!(bytes.len() <= MAX_DOWNLOAD, Failure::TooLarge);
        let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let format = reader.format().ok_or(Failure::Decode)?;
        let mut decoder = reader.into_decoder()?;
        let orientation = decoder.orientation()?;
        let (width, height) = (decoder.dimensions().0, decoder.dimensions().1);
        ensure!(
            width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) * 4 <= 64 * 1024 * 1024,
            Failure::TooLarge
        );
        let source = self.write(&Self::cache_key(asset), bytes, format.extensions_str()[0])?;
        if orientation == image::metadata::Orientation::NoTransforms {
            return Ok((source.clone(), source));
        }
        let path = self.rotated(&format!("display-{}", asset.key()), bytes)?;
        Ok((path, source))
    }
    fn rotated(&mut self, key: &str, bytes: &[u8]) -> Result<PathBuf> {
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels(bytes, 8192)?)
            .to_rgb8()
            .write_to(&mut jpeg, ImageFormat::Jpeg)?;
        self.write(key, jpeg.get_ref(), "jpg")
    }
    fn insert(&mut self, key: String, file: CacheFile) {
        let kind = usize::from(original_key(&key));
        self.totals[kind] += file.size;
        self.counts[kind] += 1;
        self.paths.insert(key, file);
    }
    fn forget(&mut self, key: &str) -> Option<CacheFile> {
        let file = self.paths.remove(key)?;
        let kind = usize::from(original_key(key));
        self.totals[kind] -= file.size;
        self.counts[kind] -= 1;
        Some(file)
    }
    fn index(&mut self) -> Result<()> {
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some((key, _)) = name.to_str().and_then(|name| name.rsplit_once('-')) else {
                continue;
            };
            let metadata = entry.metadata()?;
            let file = CacheFile {
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified()?,
            };
            if let Some(previous) = self.paths.get(key) {
                if previous.modified > file.modified {
                    remove_file(&file.path)?;
                    continue;
                }
                let previous = self.forget(key).unwrap();
                remove_file(&previous.path)?;
            }
            self.insert(key.to_owned(), file);
        }
        Ok(())
    }
    fn prune(&mut self, incoming: u64, original: bool) -> Result<()> {
        for kind in 0..2 {
            let reserve = if original == (kind == 1) { incoming } else { 0 };
            let limit = if kind == 1 {
                ORIGINAL_LIMIT
            } else {
                DISK_LIMIT
            };
            let count_limit = if kind == 1 { 128 } else { 4096 };
            while self.totals[kind] + reserve > limit
                || self.counts[kind] + usize::from(reserve > 0) >= count_limit
            {
                let Some(key) = self
                    .paths
                    .iter()
                    .filter(|(key, _)| usize::from(original_key(key)) == kind)
                    .min_by_key(|(_, file)| file.modified)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                let file = self.forget(&key).unwrap();
                remove_file(&file.path)?;
            }
        }
        Ok(())
    }
}
fn original_key(key: &str) -> bool {
    key.starts_with("original-") || key.starts_with("display-")
}
fn remove_file(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// The software renderer doesn't round image clipping. Supply a masked square
/// to render decoded avatar pixels as a circle.
fn circular_avatar(image: image::RgbaImage, size: u32) -> image::RgbaImage {
    let mut square = image;
    for pixel in square.pixels_mut() {
        let alpha = u16::from(pixel.0[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u16::from(*channel) * alpha / 255) as u8;
        }
    }
    let mut image = image::DynamicImage::ImageRgba8(square)
        .resize_to_fill(size, size, image::imageops::FilterType::Triangle)
        .to_rgba8();
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
    image
}

fn prepare_image(bytes: &[u8], dimension: u32, avatar: bool) -> Result<image::RgbaImage> {
    let pixels = pixels(bytes, if avatar { 8192 } else { dimension })?;
    let image = if avatar {
        circular_avatar(pixels, dimension)
    } else {
        pixels
    };
    Ok(image)
}
fn has_small(asset: &Asset) -> bool {
    asset.dimension < 8192 && matches!(asset.kind(), "photo" | "sticker")
}
pub fn small_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "small-{}",
        path.file_name().unwrap().to_string_lossy()
    ))
}
fn small_png(image: &image::RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image.clone())
        .thumbnail(64.min(image.width()), 64.min(image.height()))
        .write_to(&mut bytes, ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

fn pixels(bytes: &[u8], dimension: u32) -> Result<image::RgbaImage> {
    ensure!(bytes.len() <= MAX_DOWNLOAD, Failure::TooLarge);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = crate::images::decode(reader).map_err(|error| match error {
        image::ImageError::Limits(_) => Failure::TooLarge,
        _ => Failure::Decode,
    })?;
    let image = if decoded.width() > dimension || decoded.height() > dimension {
        decoded.thumbnail(dimension, dimension).to_rgba8()
    } else {
        decoded.to_rgba8()
    };
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn avatar_crops_full_non_square_source_to_a_sharp_centred_circle() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_fn(1600, 1000, |x, _| {
            if (300..1300).contains(&x) {
                image::Rgba([0, 0, if ((x - 300) / 8) % 2 == 0 { 80 } else { 240 }, 255])
            } else {
                image::Rgba([255, 0, 0, 255])
            }
        })
        .write_to(&mut bytes, ImageFormat::Png)?;
        let path = cache.store(&Asset::avatar("/wide").unwrap(), bytes.get_ref())?;
        let circle = image::open(path)?.to_rgba8();
        assert_eq!(circle.dimensions(), (192, 192));
        assert_eq!(circle.get_pixel(0, 0).0[3], 0);
        assert_eq!(circle.get_pixel(96, 96).0[3], 255);
        assert_eq!(circle.get_pixel(1, 96).0[0], 0);
        let expected = image::load_from_memory(bytes.get_ref())?
            .resize_to_fill(192, 192, image::imageops::FilterType::Triangle)
            .to_rgba8();
        for x in 90..102 {
            assert_eq!(circle.get_pixel(x, 96), expected.get_pixel(x, 96));
        }
        Ok(())
    }
    #[test]
    fn cache_indexes_paths_and_discards_older_crash_duplicates() -> Result<()> {
        let root = root();
        let asset = Asset::avatar("/crash").unwrap();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let newest = cache.store(&asset, &png([25, 70, 200, 255]))?;
        let older = cache.directory.join(format!("{}-older.png", asset.key()));
        fs::copy(&newest, &older)?;
        OpenOptions::new().write(true).open(&older)?.set_times(
            fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(60)),
        )?;
        let mut restored = ImageCache::new(root.path(), "synthetic")?;
        assert!(!older.exists());
        restored.directory = root.path().join("does-not-exist");
        assert_eq!(restored.read(&asset)?, Some(newest));
        Ok(())
    }
    #[test]
    fn cache_bounds_thumbnail_files_during_repeated_writes() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let bytes = png([25, 70, 200, 255]);
        let mut newest = None;
        for id in 0..4100 {
            let asset =
                Asset::paid_reaction(&json!({"productId":"test", "emojiId":id.to_string()}))
                    .unwrap();
            newest = Some((asset.clone(), cache.store(&asset, &bytes)?));
        }
        let count = fs::read_dir(cache.directory())?.count();
        assert!(count <= 4096, "thumbnail file budget exceeded: {count}");
        let (asset, path) = newest.unwrap();
        assert_eq!(cache.read(&asset)?, Some(path.clone()));
        let mut reopened = ImageCache::new(root.path(), "synthetic")?;
        assert_eq!(reopened.read(&asset)?, Some(path));
        Ok(())
    }
    #[test]
    fn cache_keeps_storing_after_oldest_file_disappears() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let bytes = png([25, 70, 200, 255]);
        let oldest = Asset::avatar("/oldest").unwrap().original().unwrap();
        let path = cache.store(&oldest, &bytes)?;
        cache
            .paths
            .get_mut(&ImageCache::cache_key(&oldest))
            .unwrap()
            .modified = UNIX_EPOCH;
        fs::remove_file(path)?;
        for id in 0..132 {
            let asset = Asset::avatar(&format!("/next-{id}"))
                .unwrap()
                .original()
                .unwrap();
            let path = cache.store(&asset, &bytes)?;
            assert_eq!(cache.read_original(&asset)?, Some((path.clone(), path)));
            assert!(cache.counts[1] <= 128);
            assert!(cache.totals[1] <= ORIGINAL_LIMIT);
        }
        let files = fs::read_dir(cache.directory())?.collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(files.len(), cache.counts[1]);
        let size = files
            .iter()
            .map(|file| file.metadata().unwrap().len())
            .sum::<u64>();
        assert_eq!(size, cache.totals[1]);
        Ok(())
    }
    #[test]
    fn originals_have_a_separate_smaller_disk_budget() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let asset = Asset::avatar("/budget").unwrap();
        let thumbnail = cache.store(&asset, &png([25, 70, 200, 255]))?;
        let (original, _) = cache.original(&asset.original().unwrap(), &png([25, 70, 200, 255]))?;
        cache.prune(DISK_LIMIT, false)?;
        assert!(!thumbnail.exists());
        assert!(original.exists());
        let thumbnail = cache.store(&asset, &png([25, 70, 200, 255]))?;
        cache.prune(ORIGINAL_LIMIT, true)?;
        assert!(!original.exists());
        assert!(thumbnail.exists());
        Ok(())
    }
    #[test]
    fn rewrite_cleanup_tolerates_a_concurrently_removed_previous_file() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let first = cache.write("key", &png([25, 70, 200, 255]), "png")?;
        fs::remove_file(&first)?;
        let next = cache.directory.join("key-next.png");
        cache.replace("key", &next, &png([200, 70, 25, 255]))?;
        assert!(next.exists());
        assert_eq!(cache.read_key("key")?, Some(next));
        assert_eq!(cache.counts, [1, 0]);
        Ok(())
    }
    #[test]
    fn replacement_remains_indexed_if_previous_file_cannot_be_removed() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let first = cache.write("key", &png([25, 70, 200, 255]), "png")?;
        fs::remove_file(&first)?;
        fs::create_dir(&first)?;
        let next = cache.directory.join("key-next.png");
        let bytes = png([200, 70, 25, 255]);
        assert!(cache.replace("key", &next, &bytes).is_err());
        assert_eq!(cache.read_key("key")?, Some(next));
        assert_eq!(cache.counts, [1, 0]);
        assert_eq!(cache.totals, [bytes.len() as u64, 0]);
        Ok(())
    }
    #[test]
    fn cached_sticker_opens_as_original_without_downloading() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let sticker = Asset::sticker(&json!({"STKID":"42"})).unwrap();
        let path = cache.store(&sticker, &png([25, 70, 200, 255]))?;
        let original = sticker.original().unwrap();
        assert_ne!(sticker.key(), original.key());
        assert_eq!(cache.read_original(&original)?, Some((path.clone(), path)));
        Ok(())
    }
    #[test]
    fn small_previews_are_stored_and_regenerated_after_pruning() -> Result<()> {
        let root = root();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let asset = Asset::photo(&json!({"id":"42"})).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        image::RgbImage::new(1600, 1000).write_to(&mut bytes, ImageFormat::Jpeg)?;
        let path = cache.store(&asset, bytes.get_ref())?;
        assert_eq!(image::open(&path)?.width(), 384);
        let small = small_path(&path);
        assert_eq!(image::open(&small)?.to_rgba8().dimensions(), (64, 40));
        fs::remove_file(&small)?;
        assert_eq!(cache.read(&asset)?, Some(path));
        assert_eq!(image::open(&small)?.width(), 64);
        assert_eq!(fs::metadata(small)?.permissions().mode() & 0o777, 0o600);
        Ok(())
    }
    fn root() -> tempfile::TempDir {
        fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tmp")).unwrap();
        tempfile::Builder::new()
            .prefix("image-cache-")
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tmp"))
            .unwrap()
    }
    fn png(color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 3, image::Rgba(color))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    #[test]
    fn cache_prunes_least_recently_used_files() -> Result<()> {
        let root = root();
        let asset = Asset::avatar("/profile-picture").unwrap();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let unread = Asset::avatar("/unread").unwrap();
        let first = cache.store(&asset, &png([25, 70, 200, 255]))?;
        let unused = cache.store(&unread, &png([25, 70, 200, 255]))?;
        for path in [&first, &unused] {
            OpenOptions::new().write(true).open(path)?.set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_secs(60 * 24 * 3600)),
            )?;
        }
        cache = ImageCache::new(root.path(), "synthetic")?;
        assert!(unused.exists());
        assert_eq!(cache.read(&asset)?, Some(first.clone()));
        cache.prune(DISK_LIMIT - fs::metadata(&unused)?.len(), false)?;
        assert!(first.exists());
        assert!(!unused.exists());

        Ok(())
    }
    #[test]
    fn cache_restores_private_files_across_instances_and_isolates_accounts() -> Result<()> {
        let root = root();
        let asset = Asset::avatar("/profile-picture").unwrap();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        assert_eq!(cache.directory(), root.path().join("synthetic"));
        assert!(ImageCache::new(root.path(), "../bad").is_err());
        let first = cache.store(&asset, &png([25, 70, 200, 255]))?;
        cache = ImageCache::new(root.path(), "synthetic")?;
        assert_eq!(cache.read(&asset)?, Some(first.clone()));
        assert_eq!(fs::metadata(&first)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&cache.directory)?.permissions().mode() & 0o777,
            0o700
        );
        assert!(
            ImageCache::new(root.path(), "other-account")?
                .read(&asset)?
                .is_none()
        );

        Ok(())
    }
    #[test]
    fn rewritten_assets_get_a_new_path_and_missing_files_are_misses() -> Result<()> {
        let root = root();
        let asset = Asset::avatar("/profile-picture").unwrap();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let first = cache.store(&asset, &png([25, 70, 200, 255]))?;
        let second = cache.store(&asset, &png([200, 70, 25, 255]))?;
        assert_ne!(first, second);
        assert!(!first.exists());
        assert_eq!(cache.read(&asset)?, Some(second.clone()));
        fs::remove_file(second)?;
        assert!(cache.read(&asset)?.is_none());

        Ok(())
    }
    #[test]
    fn originals_preserve_source_rotation_and_encrypted_media_identity() -> Result<()> {
        let root = root();
        let asset = Asset::avatar("/profile-picture").unwrap();
        let mut cache = ImageCache::new(root.path(), "synthetic")?;
        let original = asset.original().unwrap();
        assert_eq!(
            asset.url,
            "https://profile.line-scdn.net/profile-picture/preview"
        );
        assert_eq!(
            original.url,
            "https://profile.line-scdn.net/profile-picture"
        );
        assert_eq!(original.dimension, 8192);
        assert_ne!(original.key(), asset.key());
        let jpeg = crate::test_support::oriented_jpeg();
        assert_eq!(pixels(&jpeg, 256)?.dimensions(), (2, 3));
        let (path, source) = cache.original(&original, &jpeg)?;
        assert_eq!(fs::read(&source)?, jpeg);
        assert_eq!(fs::metadata(source)?.permissions().mode() & 0o777, 0o600);
        let display = image::open(&path)?.to_rgb8();
        assert_eq!(display.dimensions(), (2, 3));
        assert!(display.get_pixel(0, 0)[0] > display.get_pixel(0, 0)[2]);
        assert_eq!(
            ImageReader::open(&path)?.with_guessed_format()?.format(),
            Some(ImageFormat::Jpeg)
        );
        fs::remove_file(&path)?;
        assert_eq!(cache.read_original(&original)?.unwrap().0, path);
        assert!(path.exists());
        let mut plain = Cursor::new(Vec::new());
        image::RgbImage::new(1600, 1000).write_to(&mut plain, ImageFormat::Jpeg)?;
        let (path, source) = cache.original(&original, plain.get_ref())?;
        assert_eq!(path, source);
        assert_eq!(fs::read(&path)?, plain.into_inner());
        assert_eq!(image::open(&path)?.into_rgb8().dimensions(), (1600, 1000));
        assert_eq!(pixels(&fs::read(path)?, 384)?.width(), 384);
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

        Ok(())
    }
    #[test]
    fn decrypts_independent_go_media_fixture() {
        // Regenerate fixtures/media.json with: go run tools/generate-media-fixture.go
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
