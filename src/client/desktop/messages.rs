//! Bounded decoded-message memo and authenticated, encrypted history pages.
use super::*;
use aes::cipher::{KeyIvInit, StreamCipher};
use hmac::{Hmac, Mac};
use std::io::Read;

const MAX_PAGE: usize = 4 * 1024 * 1024;
const DISK_LIMIT: u64 = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 512;
const MEMORY_LIMIT: usize = 16 * 1024 * 1024;

/// Durable UI state, isolated by pairing secret and encrypted like history.
/// Lives outside the expiring message cache so drafts are never pruned.
pub struct LocalStateStore {
    cipher: MessageCache,
    path: PathBuf,
}
impl LocalStateStore {
    pub fn open(root: &Path, session: &Path) -> Result<Self> {
        let state: Value = serde_json::from_slice(&fs::read(session)?)?;
        let secret = decode(required(&state, "login_key")?)?;
        let directory = root
            .join("local-state")
            .join(format!("{:x}", Sha256::digest(&secret)));
        fs::create_dir_all(&directory)?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(
            directory.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let mut state_key = [0u8; 32];
        hkdf::Hkdf::<Sha256>::new(None, &secret)
            .expand(b"line-native/durable-desktop-state/v1", &mut state_key)
            .map_err(|_| anyhow::anyhow!("state key derivation failed"))?;
        let cipher = MessageCache::new(&directory, &state_key)?;
        Ok(Self {
            cipher,
            path: directory.join("desktop.state"),
        })
    }
    pub fn load(&self) -> Result<Option<Vec<u8>>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(self.cipher.open(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    pub fn save(&self, bytes: &[u8]) -> Result<()> {
        private_write(&self.path, &self.cipher.seal(bytes)?)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Record {
    fingerprint: String,
    message: Message,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Page {
    version: u32,
    records: Vec<Record>,
}

pub(super) struct MessageCache {
    directory: PathBuf,
    keys: [u8; 64],
    memo: HashMap<String, (Message, u64, usize)>,
    tick: u64,
}

pub(super) fn fingerprint(raw: &Value) -> String {
    format!("{:x}", Sha256::digest(raw.to_string().as_bytes()))
}

impl MessageCache {
    pub fn new(root: &Path, secret: &[u8]) -> Result<Self> {
        let directory = root.join("messages-v1");
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let mut keys = [0; 64];
        hkdf::Hkdf::<Sha256>::new(None, secret)
            .expand(b"line-native/local-message-cache/v1", &mut keys)
            .map_err(|_| anyhow::anyhow!("cache key derivation failed"))?;
        let cache = Self {
            directory,
            keys,
            memo: HashMap::new(),
            tick: 0,
        };
        cache.prune(0)?;
        Ok(cache)
    }
    pub fn lookup(&mut self, fingerprint: &str) -> Option<Message> {
        self.tick += 1;
        let entry = self.memo.get_mut(fingerprint)?;
        entry.1 = self.tick;
        Some(entry.0.clone())
    }
    pub fn remember(&mut self, fingerprint: String, message: Message) {
        self.tick += 1;
        let size = serde_json::to_vec(&message)
            .map(|v| v.len())
            .unwrap_or(MEMORY_LIMIT + 1);
        if size > MEMORY_LIMIT {
            return;
        }
        // A message ID has one current revision, including reaction changes.
        self.memo
            .retain(|key, entry| key == &fingerprint || entry.0.id != message.id);
        self.memo.insert(fingerprint, (message, self.tick, size));
        while self.memo.len() > MAX_MESSAGES
            || self.memo.values().map(|entry| entry.2).sum::<usize>() > MEMORY_LIMIT
        {
            let oldest = self
                .memo
                .iter()
                .min_by_key(|(_, entry)| entry.1)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                self.memo.remove(&key);
            } else {
                break;
            }
        }
    }
    fn path(&self, chat: &str) -> PathBuf {
        self.directory
            .join(format!("{:x}.page", Sha256::digest(chat.as_bytes())))
    }
    pub fn page(&mut self, chat: &str) -> Result<Option<Vec<Message>>> {
        let path = self.path(chat);
        let metadata = match path.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.len() > MAX_PAGE as u64
            || metadata.modified()?.elapsed().unwrap_or_default()
                > Duration::from_secs(30 * 24 * 3600)
        {
            let _ = fs::remove_file(path);
            return Ok(None);
        }
        let result = (|| -> Result<Page> {
            let bytes = fs::read(&path)?;
            let plaintext = self.open(&bytes)?;
            let page: Page = serde_json::from_slice(&plaintext)?;
            ensure!(
                page.version == 1 && page.records.len() <= 50,
                "unsupported cached history page"
            );
            Ok(page)
        })();
        let page = match result {
            Ok(page) => page,
            Err(_) => {
                let _ = fs::remove_file(path);
                return Ok(None);
            }
        };
        let failed = page
            .records
            .iter()
            .any(|record| record.message.decryption_failed);
        for record in &page.records {
            // Legacy pages remain readable offline, but cannot satisfy a raw
            // message lookup: their decoded records predate sender identity.
            // Otherwise refresh reuses the anonymous record for the same raw
            // fingerprint and can never recover the sender from the server.
            if !record.fingerprint.is_empty()
                && !record.message.decryption_failed
                && !record.message.sender_id.is_empty()
            {
                self.remember(record.fingerprint.clone(), record.message.clone());
            }
        }
        if failed {
            return Ok(None);
        }
        Ok(Some(
            page.records
                .into_iter()
                .map(|record| record.message)
                .collect(),
        ))
    }
    pub fn store_page(&mut self, chat: &str, messages: &[Message]) -> Result<()> {
        let records = messages
            .iter()
            .take(50)
            .map(|message| {
                let key = self
                    .memo
                    .iter()
                    .find(|(_, entry)| entry.0.id == message.id)
                    .map(|(key, _)| key.clone())
                    .unwrap_or_default();
                Record {
                    fingerprint: key,
                    message: message.clone(),
                }
            })
            .collect();
        let bytes = serde_json::to_vec(&Page {
            version: 1,
            records,
        })?;
        ensure!(
            bytes.len() + 52 <= MAX_PAGE,
            "history page exceeds cache budget"
        );
        let encrypted = self.seal(&bytes)?;
        self.prune(encrypted.len() as u64)?;
        private_write(&self.path(chat), &encrypted)
    }
    fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0; 16];
        fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let mut bytes = b"LMC1".to_vec();
        bytes.extend(nonce);
        bytes.extend(plaintext);
        ctr::Ctr128BE::<aes::Aes256>::new_from_slices(&self.keys[..32], &nonce)?
            .apply_keystream(&mut bytes[20..]);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.keys[32..])?;
        mac.update(&bytes);
        bytes.extend(mac.finalize().into_bytes());
        Ok(bytes)
    }
    fn open(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            bytes.len() >= 52 && &bytes[..4] == b"LMC1",
            "invalid cache envelope"
        );
        let (body, tag) = bytes.split_at(bytes.len() - 32);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.keys[32..])?;
        mac.update(body);
        mac.verify_slice(tag)
            .map_err(|_| anyhow::anyhow!("cache authentication failed"))?;
        let mut plaintext = body[20..].to_vec();
        ctr::Ctr128BE::<aes::Aes256>::new_from_slices(&self.keys[..32], &body[4..20])?
            .apply_keystream(&mut plaintext);
        Ok(plaintext)
    }
    fn prune(&self, incoming: u64) -> Result<()> {
        let mut pages = Vec::new();
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
            pages.push((metadata.modified()?, metadata.len(), entry.path()));
        }
        pages.sort_by_key(|entry| entry.0);
        let mut total: u64 = pages.iter().map(|entry| entry.1).sum();
        let mut count = pages.len();
        for (_, size, path) in pages {
            if total + incoming <= DISK_LIMIT && count < 256 {
                break;
            }
            fs::remove_file(path)?;
            total -= size;
            count -= 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message() -> Message {
        Message {
            id: "test-message".into(),
            sender_id: "synthetic-sender".into(),
            sender_name: "葵".into(),
            sender_avatar: None,
            body: "Private synthetic 日本語".into(),
            timestamp_ms: 1,
            outgoing: false,
            image: None,
            reactions: Vec::new(),
            encrypted: true,
            decryption_failed: false,
        }
    }
    #[test]
    fn cached_history_survives_restart_without_plaintext_and_rejects_corruption() -> Result<()> {
        let root = std::env::temp_dir().join(format!("line-message-cache-{}", std::process::id()));
        let mut cache = MessageCache::new(&root, b"synthetic key")?;
        let message = message();
        cache.remember("fingerprint".into(), message.clone());
        cache.store_page("chat", std::slice::from_ref(&message))?;
        let path = cache.path("chat");
        let bytes = fs::read(&path)?;
        ensure!(
            !bytes
                .windows(message.body.len())
                .any(|v| v == message.body.as_bytes()),
            "plaintext cache leak"
        );
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        drop(cache);
        let mut cache = MessageCache::new(&root, b"synthetic key")?;
        let restored = cache.page("chat")?.unwrap();
        assert_eq!(restored[0].body, message.body);
        assert_eq!(restored[0].sender_id, message.sender_id);
        assert_eq!(restored[0].sender_name, message.sender_name);
        let mut legacy = serde_json::to_value(&message)?;
        for field in ["sender_id", "sender_name", "sender_avatar"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        let legacy: Message = serde_json::from_value(legacy)?;
        assert!(legacy.sender_id.is_empty() && legacy.sender_name.is_empty());
        assert!(cache.lookup("fingerprint").is_some());
        let mut revised = message.clone();
        revised.reactions.push(Reaction {
            label: "Like".into(),
            count: 2,
            asset: None,
        });
        cache.remember("changed-reactions".into(), revised.clone());
        assert!(cache.lookup("fingerprint").is_none());
        cache.store_page("chat", std::slice::from_ref(&revised))?;
        let mut reopened = MessageCache::new(&root, b"synthetic key")?;
        assert_eq!(reopened.page("chat")?.unwrap()[0].reactions[0].count, 2);
        assert!(reopened.lookup("changed-reactions").is_some());
        assert!(reopened.lookup("fingerprint").is_none());
        let mut wrong = MessageCache::new(&root, b"wrong key")?;
        assert!(wrong.open(&bytes).is_err());
        let mut changed = bytes.clone();
        changed[21] ^= 1;
        assert!(cache.open(&changed).is_err());
        fs::write(&path, changed)?;
        assert!(cache.page("chat")?.is_none());
        assert!(wrong.page("unknown")?.is_none());
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn legacy_page_cannot_mask_sender_identity_on_refresh() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("line-message-cache-legacy-{}", std::process::id()));
        let mut cache = MessageCache::new(&root, b"synthetic key")?;
        let raw = json!({"id":"test-message","from":"synthetic-sender","text":"hello"});
        let fingerprint = fingerprint(&raw);
        let mut legacy = serde_json::to_value(message())?;
        for field in ["sender_id", "sender_name", "sender_avatar"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        let page = json!({"version":1,"records":[{
            "fingerprint":fingerprint,"message":legacy
        }]});
        fs::write(
            cache.path("group"),
            cache.seal(&serde_json::to_vec(&page)?)?,
        )?;

        // Selection and refresh both read persisted history. Neither may seed
        // a memo hit that would bypass decoding the fresh server message.
        for _ in 0..2 {
            let messages = cache.page("group")?.unwrap();
            assert!(messages[0].sender_id.is_empty());
            assert!(!messages[0].body.is_empty());
            assert!(cache.lookup(&fingerprint).is_none());
        }
        let mut refreshed = message();
        refreshed.sender_id = raw["from"].as_str().unwrap().to_owned();
        cache.remember(fingerprint.clone(), refreshed.clone());
        cache.store_page("group", &[refreshed])?;
        let mut reopened = MessageCache::new(&root, b"synthetic key")?;
        assert_eq!(
            reopened.page("group")?.unwrap()[0].sender_id,
            "synthetic-sender"
        );
        assert_eq!(
            reopened.lookup(&fingerprint).unwrap().sender_id,
            "synthetic-sender"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn changed_reactions_or_ciphertext_invalidate_message_memo() {
        let a = json!({"id":"1","chunks":["one"],"reactions":[]});
        let b = json!({"id":"1","chunks":["one"],"reactions":[{"type":2}]});
        assert_ne!(fingerprint(&a), fingerprint(&b));
        let c = json!({"id":"1","chunks":["changed"],"reactions":[]});
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }
}
