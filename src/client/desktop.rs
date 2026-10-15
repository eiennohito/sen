//! Account API shared by the desktop worker and read-only diagnostics.
use super::{chat, *};
use std::{collections::HashMap, sync::Arc};

pub mod assets;
mod messages;
use assets::{Asset, ImageCache};

#[derive(Clone)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub avatar: Option<Asset>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Reaction {
    pub label: String,
    pub count: usize,
    pub asset: Option<Asset>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: String,
    pub body: String,
    pub timestamp_ms: i64,
    pub outgoing: bool,
    pub image: Option<Asset>,
    pub reactions: Vec<Reaction>,
    pub encrypted: bool,
    pub decryption_failed: bool,
}

pub struct Client {
    api: Api,
    wasm: Vec<u8>,
    state: Value,
    my_mid: String,
    pub account_name: String,
    pub more_chats: bool,
    contacts: HashMap<String, Value>,
    chat_cursor: Option<String>,
    obs_access: Option<String>,
    sequence: u64,
    request_seq: u32,
    cache: ImageCache,
    messages: messages::MessageCache,
}

impl Client {
    pub fn open(state_path: &Path, wasm_path: &Path, cache_root: &Path) -> Result<Self> {
        let state: Value =
            serde_json::from_slice(&fs::read(state_path).context("read saved session")?)?;
        ensure!(
            state["wasm_sha256"] == HASH,
            "saved session WASM version mismatch"
        );
        let wasm = fs::read(wasm_path).context("read crypto WASM")?;
        ensure!(
            format!("{:x}", Sha256::digest(&wasm)) == HASH,
            "unsupported WASM release"
        );
        let mut api = Api::new(&wasm)?;
        validate_keys(&mut api.rt, &state)?;
        let profile = api.profile(access(&state)?)?;
        let my_mid = required(&profile, "mid")?.to_owned();
        let cache = ImageCache::new(cache_root, &my_mid)?;
        let messages = messages::MessageCache::new(
            cache.directory(),
            &decode(required(&state, "login_key")?)?,
        )?;
        let sequence = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        Ok(Self {
            api,
            wasm,
            state,
            my_mid,
            account_name: profile["displayName"]
                .as_str()
                .unwrap_or("Signed in")
                .into(),
            more_chats: false,
            contacts: HashMap::new(),
            chat_cursor: None,
            obs_access: None,
            sequence,
            request_seq: (sequence % 1_000_000_000) as u32,
            cache,
            messages,
        })
    }

    fn talk(&mut self, method: &str, args: Value) -> Result<Value> {
        chat::talk(&mut self.api, access(&self.state)?, method, args)
    }

    /// Reset only the poisoned WASM store, preserving validated public-key data.
    fn reset_runtime(&mut self) -> Result<()> {
        let keys = std::mem::take(&mut self.api.key_cache);
        self.api = Api::new(&self.wasm)?;
        self.api.key_cache = keys;
        Ok(())
    }

    pub fn chats(&mut self, next_page: bool) -> Result<Vec<Chat>> {
        if !next_page {
            self.chat_cursor = None;
        }
        let mut request = json!({"activeOnly":true,"unreadOnly":false,
            "messageBoxCountLimit":50,"withUnreadCount":true,"lastMessagesPerMessageBoxCount":1});
        if let Some(cursor) = &self.chat_cursor {
            request["minChatId"] = json!(cursor);
        }
        let response = self.talk("getMessageBoxes", json!([request, 0]))?;
        let boxes = response["messageBoxes"]
            .as_array()
            .context("missing message boxes")?;
        self.more_chats = response["hasNext"].as_bool().unwrap_or(false);
        self.chat_cursor = boxes
            .last()
            .and_then(|c| c["id"].as_str())
            .map(str::to_owned);
        let users: Vec<_> = boxes
            .iter()
            .filter_map(|c| c["id"].as_str())
            .filter(|id| is_user(id))
            .collect();
        if !users.is_empty() {
            let response = self.talk(
                "getContactsV2",
                json!([{"targetUserMids":users,"neededContactCalendarEvents":[]}, 0]),
            )?;
            if let Some(contacts) = response["contacts"].as_object() {
                for (mid, contact) in contacts {
                    self.contacts
                        .insert(mid.clone(), contact["contact"].clone());
                }
            }
        }
        let groups: Vec<_> = boxes
            .iter()
            .filter_map(|c| c["id"].as_str())
            .filter(|id| !is_user(id))
            .collect();
        let mut group_info = HashMap::new();
        if !groups.is_empty() {
            let response = self.talk(
                "getChats",
                json!([{"chatMids":groups,"withMembers":false,"withInvitees":false},0]),
            )?;
            if let Some(chats) = response["chats"].as_array() {
                for chat in chats {
                    if let Some(id) = chat["chatMid"].as_str() {
                        group_info.insert(id.to_owned(), chat.clone());
                    }
                }
            }
        }
        let mut chats = Vec::new();
        for entry in boxes.iter().take(50) {
            let id = required(entry, "id")?.to_owned();
            let info = self.contacts.get(&id).or_else(|| group_info.get(&id));
            let title = info
                .and_then(|v| {
                    v["displayNameOverridden"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| v["displayName"].as_str())
                        .or_else(|| v["chatName"].as_str())
                })
                .unwrap_or("Conversation")
                .to_owned();
            let avatar = info
                .and_then(|v| v["picturePath"].as_str())
                .and_then(Asset::avatar);
            let preview = entry["lastMessages"]
                .as_array()
                .and_then(|v| v.first())
                .map(|m| self.message(m).body)
                .unwrap_or_default();
            chats.push(Chat {
                id,
                title,
                preview,
                avatar,
            });
        }
        Ok(chats)
    }

    pub fn messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        if let Ok(Some(messages)) = self.messages.page(chat_mid) {
            return Ok(messages);
        }
        self.refresh_messages(chat_mid)
    }

    pub fn refresh_messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        // Prime fingerprints from valid persisted history before fetching updates.
        let _ = self.messages.page(chat_mid);
        let data = self.talk("getRecentMessagesV2", json!([chat_mid, 50]))?;
        let messages = data.as_array().context("missing recent messages")?;
        let mut messages: Vec<_> = messages.iter().take(50).map(|m| self.message(m)).collect();
        messages.sort_by(|a, b| {
            a.timestamp_ms
                .cmp(&b.timestamp_ms)
                .then_with(|| a.id.len().cmp(&b.id.len()))
                .then_with(|| a.id.cmp(&b.id))
        });
        // A cache write failure must not hide successfully fetched messages.
        let _ = self.messages.store_page(chat_mid, &messages);
        Ok(messages)
    }

    fn message(&mut self, raw: &Value) -> Message {
        let fingerprint = messages::fingerprint(raw);
        if let Some(message) = self.messages.lookup(&fingerprint) {
            return message;
        }
        let mut data = raw.clone();
        let encrypted = raw["chunks"].as_array().is_some_and(|c| !c.is_empty());
        let mut failed = false;
        if encrypted {
            let result = (|| -> Result<Value> {
                let envelope = chat::Envelope::parse(raw)?;
                let bytes =
                    chat::decrypt(&mut self.api, &self.state, &self.my_mid, raw, &envelope)?;
                let body: Value = serde_json::from_slice(&bytes)?;
                ensure!(body.is_object(), "invalid decrypted message");
                Ok(body)
            })();
            match result {
                Ok(body) => {
                    for field in ["text", "location"] {
                        if !body[field].is_null() {
                            data[field] = body[field].clone();
                        }
                    }
                    if let Some(key) = body["keyMaterial"].as_str() {
                        data["contentMetadata"]["ENC_KM"] = json!(key);
                    }
                    if let Some(name) = body["fileName"].as_str() {
                        data["contentMetadata"]["FILE_NAME"] = json!(name);
                    }
                }
                Err(_) => {
                    failed = true;
                    let _ = self.reset_runtime();
                }
            }
        }
        let content = data["contentType"].as_u64().unwrap_or(0);
        let meta = &data["contentMetadata"];
        let body = if failed {
            "Unable to decrypt this message with the saved keys".into()
        } else {
            match content {
                0 => data["text"].as_str().unwrap_or("").to_owned(),
                1 => "Photo".into(),
                2 => "Video".into(),
                3 => "Voice message".into(),
                7 => meta["STKTXT"].as_str().unwrap_or("Sticker").to_owned(),
                13 => meta["displayName"].as_str().unwrap_or("Contact").to_owned(),
                14 => meta["FILE_NAME"].as_str().unwrap_or("File").to_owned(),
                15 => "Location".into(),
                _ => "Message".into(),
            }
        };
        let image = if failed {
            None
        } else {
            match content {
                1 => Asset::photo(&data),
                7 => Asset::sticker(meta),
                _ => None,
            }
        };
        let message = Message {
            id: data["id"].as_str().unwrap_or_default().into(),
            body,
            timestamp_ms: data["createdTime"]
                .as_str()
                .and_then(|v| v.parse().ok())
                .or_else(|| data["createdTime"].as_i64())
                .unwrap_or(0),
            outgoing: data["from"].as_str() == Some(&self.my_mid),
            image,
            reactions: reactions(&data["reactions"]),
            encrypted,
            decryption_failed: failed,
        };
        if !message.decryption_failed {
            self.messages.remember(fingerprint, message.clone());
        }
        message
    }

    pub fn image(&mut self, asset: &Asset) -> Result<Arc<assets::Pixels>> {
        if let Some(pixels) = self.cache.read(asset)? {
            return Ok(pixels);
        }
        if asset.private && self.obs_access.is_none() {
            let result = self.talk("acquireEncryptedAccessToken", json!([2]))?;
            let text = result.as_str().context("invalid media access response")?;
            let token = text
                .split('\x1e')
                .nth(1)
                .and_then(|s| s.split('\x1f').next())
                .filter(|s| !s.is_empty())
                .context("missing media access token")?;
            self.obs_access = Some(token.into());
        }
        let data = asset.fetch(self.obs_access.as_deref())?;
        self.cache.store(asset, &data)
    }

    pub fn cache_stats(&self) -> assets::CacheStats {
        self.cache.stats()
    }

    /// Called only after the user presses Send; diagnostics never call this.
    pub fn send_text(&mut self, to: &str, text: &str) -> Result<Message> {
        ensure!(
            !text.trim().is_empty() && text.len() <= 10_000,
            "message must contain 1–10000 UTF-8 bytes"
        );
        let result = self.encrypted_text(to, text);
        let message = match result {
            Ok(message) => message,
            Err(error) => {
                self.reset_runtime()?;
                return Err(error);
            }
        };
        self.request_seq = self.request_seq.wrapping_add(1) & 0x7fff_ffff;
        // Never automatically retry this RPC: a timeout can mean it was sent.
        let result = self.talk("sendMessage", json!([self.request_seq, message]))?;
        Ok(self.message(&result))
    }

    fn encrypted_text(&mut self, to: &str, text: &str) -> Result<Value> {
        let key = self.state["e2ee_keys"]
            .as_array()
            .context("missing own keys")?
            .iter()
            .max_by_key(|k| k["id"].as_u64().unwrap_or(0))
            .context("missing own key")?
            .clone();
        let sender_id = key["id"].as_u64().context("missing own key ID")?;
        let own = chat::own_key(&mut self.api.rt, &self.state, sender_id)?;
        let (receiver_id, version, ch) = if is_user(to) {
            let negotiation = self.talk("negotiateE2EEPublicKey", json!([to]))?;
            ensure!(
                negotiation["allowedTypes"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(0))),
                "recipient does not support encrypted text"
            );
            let public = &negotiation["publicKey"];
            let id = public["keyId"].as_u64().context("missing peer key ID")?;
            let bytes = decode(required(public, "keyData")?)?;
            let ch = chat::channel(&mut self.api.rt, own, &bytes)?;
            (id, negotiation["specVersion"].as_u64().unwrap_or(2), ch)
        } else {
            // Existing shared keys only; never register/rekey a group silently.
            let shared = self.talk("getLastE2EEGroupSharedKey", json!([1, to]))?;
            let id = shared["groupKeyId"]
                .as_u64()
                .context("missing group key ID")?;
            let private_id = shared["receiverKeyId"]
                .as_u64()
                .context("missing group receiver key")?;
            let creator_id = shared["creatorKeyId"]
                .as_u64()
                .context("missing group creator key")?;
            let public = chat::public_key(
                &mut self.api,
                access(&self.state)?,
                required(&shared, "creator")?,
                creator_id,
            )?;
            let transfer_key = chat::own_key(&mut self.api.rt, &self.state, private_id)?;
            let transfer = chat::channel(&mut self.api.rt, transfer_key, &public)?;
            let h = self
                .api
                .rt
                .bytes(&decode(required(&shared, "encryptedSharedKey")?)?);
            let group_key =
                self.api
                    .rt
                    .method("E2EEChannel", "unwrapGroupSharedKey", transfer, &[h])?;
            self.api.rt.release(h)?;
            let public = decode(required(&key, "public_key")?)?;
            let ch = chat::channel(&mut self.api.rt, group_key, &public)?;
            self.api.rt.destroy("E2EEKey", group_key)?;
            self.api.rt.destroy("E2EEChannel", transfer)?;
            self.api.rt.destroy("E2EEKey", transfer_key)?;
            (id, shared["specVersion"].as_u64().unwrap_or(2), ch)
        };
        let h = self
            .api
            .rt
            .bytes(&serde_json::to_vec(&json!({"text":text}))?);
        let encrypted = if version == 1 {
            bytes_method(&mut self.api.rt, "E2EEChannel", "encryptV1", ch, &[h])?
        } else {
            ensure!(version == 2, "unsupported encryption version");
            let dest = self.api.rt.string(to)?;
            let from = self.api.rt.string(&self.my_mid)?;
            self.sequence = self.sequence.checked_add(1).context("sequence exhausted")?;
            let bytes = bytes_method(
                &mut self.api.rt,
                "E2EEChannel",
                "encryptV2",
                ch,
                &[dest, from, sender_id, receiver_id, 0, self.sequence, h],
            )?;
            self.api.rt.free_string(dest)?;
            self.api.rt.free_string(from)?;
            bytes
        };
        self.api.rt.release(h)?;
        self.api.rt.destroy("E2EEChannel", ch)?;
        self.api.rt.destroy("E2EEKey", own)?;
        let chunks = pack_chunks(&encrypted, version, sender_id, receiver_id)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .to_string();
        Ok(
            json!({"to":to,"from":self.my_mid,"toType":if is_user(to){0}else if to.starts_with(['C','c']){2}else{1},
            "createdTime":now,"sessionId":0,"contentType":0,"hasContent":false,
            "contentMetadata":{"e2eeVersion":version.to_string()},"chunks":chunks}),
        )
    }
}

fn is_user(mid: &str) -> bool {
    mid.starts_with(['u', 'U'])
}

fn pack_chunks(bytes: &[u8], version: u64, sender: u64, receiver: u64) -> Result<Vec<String>> {
    let chunks = if version == 1 {
        ensure!(bytes.len() >= 40, "short V1 ciphertext");
        [
            &bytes[..8],
            &bytes[8..bytes.len() - 16],
            &bytes[bytes.len() - 16..],
        ]
    } else {
        ensure!(bytes.len() >= 44, "short V2 ciphertext");
        [&bytes[..16], &bytes[28..], &bytes[16..28]]
    };
    let mut result: Vec<_> = chunks.iter().map(|v| STANDARD.encode(v)).collect();
    result.push(STANDARD.encode(u32::try_from(sender)?.to_be_bytes()));
    result.push(STANDARD.encode(u32::try_from(receiver)?.to_be_bytes()));
    Ok(result)
}

fn reactions(value: &Value) -> Vec<Reaction> {
    let mut groups: Vec<Reaction> = Vec::new();
    for item in value.as_array().into_iter().flatten() {
        let kind = &item["reactionType"];
        let (label, asset) = if let Some(id) = kind["predefinedReactionType"].as_u64() {
            (
                match id {
                    2 => "👍",
                    3 => "❤️",
                    4 => "😆",
                    5 => "😮",
                    6 => "😢",
                    7 => "😰",
                    _ => "Reaction",
                }
                .to_owned(),
                None,
            )
        } else {
            (
                "Emoji".into(),
                Asset::paid_reaction(&kind["paidReactionType"]),
            )
        };
        if let Some(group) = groups.iter_mut().find(|g| {
            g.label == label && g.asset.as_ref().map(Asset::key) == asset.as_ref().map(Asset::key)
        }) {
            group.count += 1;
        } else {
            groups.push(Reaction {
                label,
                count: 1,
                asset,
            });
        }
    }
    groups.truncate(6);
    groups
}
