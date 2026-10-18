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
        let state = crate::credentials::load(session)?;
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
    #[serde(default)]
    reached_beginning: Option<bool>,
    records: Vec<Record>,
}

pub(super) struct MessageCache {
    directory: PathBuf,
    keys: [u8; 64],
    memo: HashMap<String, (Message, u64, usize)>,
    tick: u64,
    requests: HashMap<u32, String>,
    withdrawals: std::collections::VecDeque<String>,
}

pub(super) fn fingerprint(raw: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("rich-v1:{}", raw).as_bytes())
    )
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
        let mut cache = Self {
            directory,
            keys,
            memo: HashMap::new(),
            tick: 0,
            requests: HashMap::new(),
            withdrawals: Default::default(),
        };
        cache.load_metadata()?;
        cache.prune(0)?;
        Ok(cache)
    }
    fn load_metadata(&mut self) -> Result<()> {
        let path = self.directory.join("protocol.state");
        if !path.exists() {
            return Ok(());
        }
        ensure!(
            path.metadata()?.len() <= 2 * 1024 * 1024,
            "protocol metadata too large"
        );
        let bytes = self.open(&fs::read(path)?)?;
        let data: ProtocolMetadata = serde_json::from_slice(&bytes)?;
        ensure!(
            data.requests.len() <= 256 && data.withdrawals.len() <= 16384,
            "protocol metadata exceeds budget"
        );
        self.requests = data.requests;
        self.withdrawals = data.withdrawals;
        Ok(())
    }
    fn save_metadata(&self) -> Result<()> {
        let data = ProtocolMetadata {
            requests: self.requests.clone(),
            withdrawals: self.withdrawals.clone(),
        };
        private_write(
            &self.directory.join("protocol.state"),
            &self.seal(&serde_json::to_vec(&data)?)?,
        )
    }
    pub fn record_request(&mut self, sequence: u32, raw: &Value) -> Result<()> {
        ensure!(
            self.requests.len() < 256 || self.requests.contains_key(&sequence),
            "too many unresolved send bindings"
        );
        let fingerprint = request_fingerprint(raw);
        if let Some(previous) = self.requests.get(&sequence) {
            ensure!(previous == &fingerprint, "send sequence already bound");
        }
        self.requests.insert(sequence, fingerprint);
        self.save_metadata()
    }
    pub fn retain_requests(&mut self, sequences: &[u32]) -> Result<()> {
        self.requests.retain(|seq, _| sequences.contains(seq));
        self.save_metadata()
    }
    pub fn matching_request(&self, raw: &Value) -> Option<u32> {
        let fingerprint = request_fingerprint(raw);
        let mut matches = self
            .requests
            .iter()
            .filter(|(_, value)| **value == fingerprint)
            .map(|(sequence, _)| *sequence);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    pub fn request_matches(&self, sequence: u32, raw: &Value) -> bool {
        self.requests
            .get(&sequence)
            .is_some_and(|fingerprint| *fingerprint == request_fingerprint(raw))
    }
    pub fn is_withdrawn(&self, message: &str) -> bool {
        self.withdrawals.iter().any(|id| id == message)
    }
    pub fn withdraw(&mut self, message: &str) -> Result<()> {
        if !self.is_withdrawn(message) {
            self.withdrawals.push_back(message.into());
        }
        if self.withdrawals.len() > 16384 {
            // Eviction cannot resurrect a withdrawn item from an old disk page.
            for entry in fs::read_dir(&self.directory)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|e| e == "page") {
                    fs::remove_file(entry.path())?;
                }
            }
            self.withdrawals.pop_front();
        }
        for entry in self.memo.values_mut().filter(|entry| entry.0.id == message) {
            withdrawn(&mut entry.0);
        }
        self.save_metadata()
    }
    pub fn cached_message(&self, id: &str) -> Result<Option<Message>> {
        if let Some(entry) = self.memo.values().find(|entry| entry.0.id == id) {
            return Ok(Some(entry.0.clone()));
        }
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "page")
                || entry.metadata()?.len() > MAX_PAGE as u64
            {
                continue;
            }
            let page = (|| -> Result<Page> {
                Ok(serde_json::from_slice(
                    &self.open(&fs::read(entry.path())?)?,
                )?)
            })();
            if let Ok(page) = page
                && page.version == 1
                && page.records.len() <= 50
                && let Some(mut message) = page
                    .records
                    .into_iter()
                    .map(|record| record.message)
                    .find(|message| message.id == id)
            {
                if self.is_withdrawn(id) {
                    withdrawn(&mut message);
                }
                return Ok(Some(message));
            }
        }
        Ok(None)
    }
    pub fn lookup(&mut self, fingerprint: &str) -> Option<Message> {
        self.tick += 1;
        let entry = self.memo.get_mut(fingerprint)?;
        entry.1 = self.tick;
        Some(entry.0.clone())
    }
    pub fn remember(&mut self, fingerprint: String, mut message: Message) {
        if self.is_withdrawn(&message.id) {
            withdrawn(&mut message);
        }
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
        Ok(self.page_with_end(chat)?.map(|(messages, _)| messages))
    }
    pub fn range(
        &mut self,
        chat: &str,
        before: &str,
        delivered: i64,
    ) -> Result<Option<(Vec<Message>, bool)>> {
        Ok(self
            .page_with_end(&range_key(chat, before, delivered))?
            .and_then(|(messages, end)| end.map(|end| (messages, end))))
    }
    pub fn store_range(
        &mut self,
        chat: &str,
        before: &str,
        delivered: i64,
        messages: &[Message],
        end: bool,
    ) -> Result<()> {
        self.store_page_with_end(&range_key(chat, before, delivered), messages, Some(end))
    }
    fn page_with_end(&mut self, chat: &str) -> Result<Option<(Vec<Message>, Option<bool>)>> {
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
        let mut page = match result {
            Ok(page) => page,
            Err(_) => {
                let _ = fs::remove_file(path);
                return Ok(None);
            }
        };
        for record in &mut page.records {
            if self.is_withdrawn(&record.message.id) {
                withdrawn(&mut record.message);
            }
        }
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
                && record.message.metadata_version >= 2
            {
                self.remember(record.fingerprint.clone(), record.message.clone());
            }
        }
        if failed {
            return Ok(None);
        }
        Ok(Some((
            page.records
                .into_iter()
                .map(|record| record.message)
                .collect(),
            page.reached_beginning,
        )))
    }
    pub fn store_page(&mut self, chat: &str, messages: &[Message]) -> Result<()> {
        self.store_page_with_end(chat, messages, None)
    }
    fn store_page_with_end(
        &mut self,
        chat: &str,
        messages: &[Message],
        reached_beginning: Option<bool>,
    ) -> Result<()> {
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
            reached_beginning,
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
            if !entry.file_type()?.is_file()
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "page")
            {
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

#[derive(serde::Serialize, serde::Deserialize)]
struct ProtocolMetadata {
    requests: HashMap<u32, String>,
    withdrawals: std::collections::VecDeque<String>,
}
fn request_fingerprint(raw: &Value) -> String {
    fingerprint(&json!([
        raw["to"],
        raw["from"],
        raw["contentType"],
        raw["chunks"]
    ]))
}
fn withdrawn(message: &mut Message) {
    message.body.clear();
    message.image = None;
    message.attachment = None;
    message.reactions.clear();
    message.rich.withdrawn = true;
    message.rich.content_type = 18;
}

fn range_key(chat: &str, before: &str, delivered: i64) -> String {
    json!(["older-v1", chat, before, delivered]).to_string()
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
            delivered_time: 1,
            outgoing: false,
            image: None,
            reactions: Vec::new(),
            encrypted: true,
            decryption_failed: false,
            rich: Default::default(),
            request_seq: None,
            attachment: None,
            mentioned: false,
            skip_badge: false,
            metadata_version: 2,
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
    #[test]
    fn send_bindings_survive_restart_and_reject_same_sequence_other_ciphertext() -> Result<()> {
        let root = std::env::temp_dir().join(format!("line-send-bindings-{}", std::process::id()));
        let raw =
            json!({"from":"self","to":"chat","contentType":0,"chunks":["synthetic ciphertext"]});
        let mut cache = MessageCache::new(&root, b"synthetic")?;
        cache.record_request(12, &raw)?;
        drop(cache);
        let mut cache = MessageCache::new(&root, b"synthetic")?;
        assert!(cache.request_matches(12, &raw));
        // Recent-page catch-up has no operation reqSeq; the exact encrypted
        // envelope still recovers the persisted local submission identity.
        assert_eq!(cache.matching_request(&raw), Some(12));
        let mut unrelated = raw.clone();
        unrelated["chunks"] = json!(["different ciphertext"]);
        assert!(!cache.request_matches(12, &unrelated));
        assert!(cache.record_request(12, &unrelated).is_err());
        cache.retain_requests(&[])?;
        drop(cache);
        assert!(!MessageCache::new(&root, b"synthetic")?.request_matches(12, &raw));
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn range_pages_retry_end_and_withdrawal_overlay_survive_restart() -> Result<()> {
        let root = std::env::temp_dir().join(format!("line-range-pages-{}", std::process::id()));
        let mut cache = MessageCache::new(&root, b"synthetic")?;
        // A failed fetch stores nothing: retry sees precisely the same miss.
        assert!(cache.range("chat", "150", 150)?.is_none());
        assert!(cache.range("chat", "150", 150)?.is_none());
        for (before, start, end) in [("150", 100, false), ("100", 50, false), ("50", 1, true)] {
            let page: Vec<_> = (start..start + 49)
                .map(|id| {
                    let mut m = message();
                    m.id = id.to_string();
                    m.timestamp_ms = id;
                    m.delivered_time = id;
                    m
                })
                .collect();
            cache.store_range("chat", before, before.parse().unwrap(), &page, end)?;
        }
        cache.withdraw("7")?;
        drop(cache);
        let mut cache = MessageCache::new(&root, b"synthetic")?;
        assert!(cache.cached_message("7")?.unwrap().rich.withdrawn);
        let mut ids = std::collections::HashSet::new();
        for before in ["150", "100", "50"] {
            let (page, end) = cache
                .range("chat", before, before.parse().unwrap())?
                .unwrap();
            assert_eq!(end, before == "50");
            for m in page {
                assert!(ids.insert(m.id.clone()));
                if m.id == "7" {
                    assert!(m.rich.withdrawn);
                    assert!(m.body.is_empty());
                }
            }
        }
        assert_eq!(ids.len(), 147);
        assert!(cache.memo.len() <= MAX_MESSAGES);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn synthetic_transport_three_inclusive_pages_failure_retry_and_beginning() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("line-older-transport-{}", std::process::id()));
        let mut cache = MessageCache::new(&root, b"synthetic")?;
        let mut transport: std::collections::VecDeque<Result<Vec<Message>>> = Default::default();
        let page = |start: i64, end: i64| {
            (start..=end)
                .map(|n| {
                    let mut m = message();
                    m.id = n.to_string();
                    m.timestamp_ms = n;
                    m
                })
                .collect()
        };
        transport.push_back(Ok(page(71, 120)));
        transport.push_back(Err(anyhow::anyhow!("synthetic failure")));
        transport.push_back(Ok(page(22, 71)));
        transport.push_back(Ok(page(1, 22)));
        let mut before = "120".to_owned();
        let mut all = std::collections::HashSet::new();
        let mut failed = false;
        for index in 0..4 {
            assert!(cache.range("chat", &before, before.parse()?)?.is_none());
            let raw = match transport.pop_front().unwrap() {
                Ok(page) => page,
                Err(_) => {
                    failed = true;
                    continue;
                }
            };
            let reached = index == 3;
            let normalized = normalize_older(&before, raw);
            assert!(normalized.iter().all(|m| m.id != before));
            for m in &normalized {
                assert!(all.insert(m.id.clone()));
            }
            cache.store_range("chat", &before, before.parse()?, &normalized, reached)?;
            let (restored, end) = cache.range("chat", &before, before.parse()?)?.unwrap();
            assert_eq!(end, reached);
            assert_eq!(restored.len(), normalized.len());
            before = normalized[0].id.clone();
        }
        assert!(failed);
        assert!(transport.is_empty());
        assert_eq!(before, "1");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
