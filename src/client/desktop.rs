//! Account API shared by the desktop worker and read-only diagnostics.
use super::{chat, *};
use std::{collections::HashMap, sync::Arc};

pub mod assets;
mod messages;
pub mod read;
pub mod rich;
pub mod sync;
use assets::{Asset, ImageCache};
pub use messages::LocalStateStore;

#[derive(Clone)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub timestamp_ms: i64,
    pub avatar: Option<Asset>,
    pub unread: Option<u64>,
    pub muted: Option<bool>,
    pub last_read_message: Option<String>,
    pub last_message_id: Option<String>,
    pub member_count: Option<usize>,
    pub outgoing_preview: bool,
    pub group: bool,
}

/// Read-only friend/group identity, independent of the loaded conversation list.
#[derive(Clone)]
pub struct Friend {
    pub id: String,
    pub name: String,
    pub status: String,
    pub group: bool,
    pub avatar: Option<Asset>,
    pub member_count: Option<usize>,
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
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub sender_avatar: Option<Asset>,
    pub body: String,
    pub timestamp_ms: i64,
    #[serde(default)]
    pub delivered_time: i64,
    pub outgoing: bool,
    pub image: Option<Asset>,
    pub reactions: Vec<Reaction>,
    pub encrypted: bool,
    pub decryption_failed: bool,
    #[serde(default)]
    pub rich: rich::RichContent,
    #[serde(default)]
    pub request_seq: Option<u32>,
    #[serde(default)]
    pub attachment: Option<Asset>,
    #[serde(default)]
    pub mentioned: bool,
    #[serde(default)]
    pub skip_badge: bool,
    #[serde(default)]
    pub metadata_version: u8,
}

#[derive(Debug)]
pub enum SendFailure {
    Rejected,
    Uncertain,
}
impl std::fmt::Display for SendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rejected => "Message was not accepted",
            Self::Uncertain => "Acceptance could not be confirmed",
        })
    }
}
impl std::error::Error for SendFailure {}
pub fn send_failure(error: &anyhow::Error) -> SendFailure {
    if error
        .downcast_ref::<SendFailure>()
        .is_some_and(|v| matches!(v, SendFailure::Rejected))
        || error
            .downcast_ref::<RpcError>()
            .is_some_and(|e| (400..500).contains(&e.http) || e.code != 0)
    {
        SendFailure::Rejected
    } else {
        SendFailure::Uncertain
    }
}

/// Install the owning worker's cancellation token for all synchronous requests.
pub fn set_cancellation_token(cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    set_request_cancel(cancel);
}

type ResyncPages = (Vec<Chat>, Vec<(String, Vec<Message>)>);

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
    cache: Arc<std::sync::Mutex<ImageCache>>,
    messages: messages::MessageCache,
    detail_boxes: Vec<Value>,
    loaded_chats: Vec<String>,
    live_ids: std::collections::VecDeque<String>,
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
            cache: Arc::new(std::sync::Mutex::new(cache)),
            messages,
            detail_boxes: Vec::new(),
            loaded_chats: Vec::new(),
            live_ids: std::collections::VecDeque::new(),
        })
    }

    fn talk(&mut self, method: &str, args: Value) -> Result<Value> {
        chat::talk(&mut self.api, access(&self.state)?, method, args)
    }

    pub fn retain_send_requests(&mut self, sequences: &[u32]) -> Result<()> {
        self.messages.retain_requests(sequences)
    }
    pub fn withdraw(&mut self, message: &str) -> Result<()> {
        self.messages.withdraw(message)
    }
    pub fn mark_read(&mut self, chat: &str, message: &str) -> Result<()> {
        self.request_seq = self.request_seq.wrapping_add(1) & 0x7fff_ffff;
        self.talk(
            "sendChatChecked",
            json!([self.request_seq, chat, message, 0]),
        )?;
        Ok(())
    }
    pub fn read_ranges(&mut self, chat: &str) -> Result<Vec<ReadRange>> {
        let data = self.talk("getMessageReadRange", json!([[chat], 0]))?;
        Ok(parse_read_ranges(&data)
            .into_iter()
            .filter(|range| range.reader != self.my_mid)
            .collect())
    }
    pub fn own_echo(&self, operation: &Value) -> Option<(String, u32, String)> {
        if operation["type"].as_u64() != Some(25)
            || operation["message"]["from"].as_str() != Some(self.my_mid.as_str())
        {
            return None;
        }
        let request = operation["reqSeq"]
            .as_u64()
            .or_else(|| operation["reqSeq"].as_str()?.parse().ok())?;
        let request = u32::try_from(request).ok()?;
        if !self
            .messages
            .request_matches(request, &operation["message"])
        {
            return None;
        }
        Some((
            operation["message"]["to"].as_str()?.into(),
            request,
            operation["message"]["id"].as_str()?.into(),
        ))
    }
    pub fn operation(&mut self, operation: &Value) -> Option<(String, Message)> {
        if !matches!(operation["type"].as_u64(), Some(25 | 26 | 43)) {
            return None;
        }
        let raw = &operation["message"];
        let message_id = raw["id"].as_str()?;
        if self.live_ids.iter().any(|id| id == message_id) {
            return None;
        }
        self.live_ids.push_back(message_id.into());
        if self.live_ids.len() > 2048 {
            self.live_ids.pop_front();
        }
        let from = raw["from"].as_str()?;
        let to = raw["to"].as_str()?;
        let chat = if is_user(to) && from != self.my_mid {
            from
        } else {
            to
        }
        .to_owned();
        Some((chat, self.message(raw)))
    }
    pub fn replace_token(&mut self, token: Value) {
        self.state["token_v3_issue_result"] = token;
        self.obs_access = None;
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
        let more_chats = response["hasNext"].as_bool().unwrap_or(false);
        let chat_cursor = boxes
            .last()
            .and_then(|c| c["id"].as_str())
            .map(str::to_owned);
        self.detail_boxes = boxes.clone();
        let chats = self.build_chats(boxes, &HashMap::new())?;
        self.more_chats = more_chats;
        self.chat_cursor = chat_cursor;
        Ok(chats)
    }
    /// Follow-up metadata work yields to queued Send/Messages commands first.
    pub fn chat_details(&mut self) -> Result<Vec<Chat>> {
        let boxes = self.detail_boxes.clone();
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
                json!([{"chatMids":groups,"withMembers":true,"withInvitees":false},0]),
            )?;
            if let Some(chats) = response["chats"].as_array() {
                for chat in chats {
                    if let Some(id) = chat["chatMid"].as_str() {
                        group_info.insert(id.to_owned(), chat.clone());
                    }
                }
            }
        }
        self.build_chats(&boxes, &group_info)
    }
    fn build_chats(
        &mut self,
        boxes: &[Value],
        group_info: &HashMap<String, Value>,
    ) -> Result<Vec<Chat>> {
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
            let last = entry["lastMessages"].as_array().and_then(|messages| {
                messages.iter().max_by(|a, b| {
                    message_timestamp(a)
                        .cmp(&message_timestamp(b))
                        .then_with(|| {
                            message_id_cmp(
                                a["id"].as_str().unwrap_or_default(),
                                b["id"].as_str().unwrap_or_default(),
                            )
                        })
                })
            });
            let timestamp_ms = last.map(message_timestamp).unwrap_or(0);
            let member_count = info.and_then(|chat| rich::group_member_count(chat, &self.my_mid));
            let preview = last.map(|m| self.message(m).body).unwrap_or_default();
            chats.push(Chat {
                group: !is_user(&id),
                member_count,
                unread: entry["unreadCount"]
                    .as_u64()
                    .or_else(|| entry["unreadCount"].as_str()?.parse().ok()),
                muted: None,
                last_read_message: entry["lastSeenMessageId"].as_str().map(str::to_owned),
                last_message_id: last.and_then(|m| m["id"].as_str().map(str::to_owned)),
                id,
                title,
                preview,
                timestamp_ms,
                avatar,
                outgoing_preview: last
                    .is_some_and(|m| m["from"].as_str() == Some(self.my_mid.as_str())),
            });
        }
        Ok(chats)
    }

    pub fn friends(&mut self) -> Result<Vec<Friend>> {
        let ids = self.talk("getAllContactIds", json!([0]))?;
        let ids = ids.as_array().context("missing contact IDs")?;
        let mut friends = Vec::new();
        // Bound each metadata request; publish only a complete snapshot.
        for chunk in ids.chunks(100) {
            let response = self.talk(
                "getContactsV2",
                json!([{
                "targetUserMids": chunk, "neededContactCalendarEvents": []
            }, 0]),
            )?;
            let contacts = response["contacts"]
                .as_object()
                .context("missing contacts")?;
            for id in chunk {
                let id = id.as_str().context("invalid contact ID")?;
                let contact = contacts.get(id).context("missing contact")?;
                let contact = &contact["contact"];
                friends.push(Friend {
                    id: id.into(),
                    name: contact_name(contact).unwrap_or("Unnamed friend").into(),
                    status: contact["statusMessage"].as_str().unwrap_or_default().into(),
                    group: false,
                    member_count: None,
                    avatar: contact["picturePath"].as_str().and_then(Asset::avatar),
                });
                self.contacts.insert(id.into(), contact.clone());
            }
        }
        let response = self.talk(
            "getAllChatMids",
            json!([{
            "withMemberChats": true, "withInvitedChats": false
        }, 0]),
        )?;
        let groups = response["memberChatMids"]
            .as_array()
            .context("missing group IDs")?;
        for chunk in groups.chunks(100) {
            let response = self.talk(
                "getChats",
                json!([{
                "chatMids": chunk, "withMembers": true, "withInvitees": false
            }, 0]),
            )?;
            let chats = response["chats"].as_array().context("missing groups")?;
            for chat in chats {
                friends.push(Friend {
                    id: required(chat, "chatMid")?.into(),
                    name: chat["chatName"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Unnamed group")
                        .into(),
                    status: String::new(),
                    group: true,
                    member_count: rich::group_member_count(chat, &self.my_mid),
                    avatar: chat["picturePath"].as_str().and_then(Asset::avatar),
                });
            }
        }
        Ok(friends)
    }

    pub fn resync(&mut self) -> Result<ResyncPages> {
        self.chats(false)?;
        let chats = self.chat_details()?;
        let mut pages = Vec::new();
        for chat in self.loaded_chats.clone() {
            pages.push((chat.clone(), self.refresh_messages(&chat)?));
        }
        Ok((chats, pages))
    }
    pub fn messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        self.loaded_chats.retain(|id| id != chat_mid);
        self.loaded_chats.push(chat_mid.into());
        if self.loaded_chats.len() > 8 {
            self.loaded_chats.remove(0);
        }
        if let Ok(Some(mut messages)) = self.messages.page(chat_mid) {
            // Old cache pages predate sender identity. Refresh these rather
            // than silently showing anonymous group messages indefinitely.
            let needs_refresh = messages.iter().any(|m| m.metadata_version < 2)
                || (!is_user(chat_mid) && messages.iter().any(|m| m.sender_id.is_empty()));
            if needs_refresh {
                messages = cached_or_refreshed(messages, || self.refresh_messages(chat_mid));
            }
            self.resolve_senders(chat_mid, &mut messages);
            sort_messages(&mut messages);
            return Ok(messages);
        }
        self.refresh_messages(chat_mid)
    }

    pub fn older(
        &mut self,
        chat: &str,
        before: &str,
        delivered_time: i64,
    ) -> Result<(Vec<Message>, bool)> {
        ensure!(
            !before.is_empty() && delivered_time > 0,
            "invalid history cursor"
        );
        if let Ok(Some(page)) = self.messages.range(chat, before, delivered_time) {
            return Ok(page);
        }
        let data = self.talk("getPreviousMessagesV2WithRequest", json!([{
            "messageBoxId":chat,"endMessageId":{"deliveredTime":delivered_time.to_string(),"messageId":before},"messagesCount":50
        },0]))?;
        let raw = data.as_array().context("missing older messages")?;
        let reached_beginning = raw.len() < 50;
        let messages: Vec<_> = raw.iter().take(50).map(|m| self.message(m)).collect();
        let mut messages = normalize_older(before, messages);
        self.resolve_senders(chat, &mut messages);
        let _ =
            self.messages
                .store_range(chat, before, delivered_time, &messages, reached_beginning);
        Ok((messages, reached_beginning))
    }
    pub fn refresh_messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        // Prime fingerprints from valid persisted history before fetching updates.
        let _ = self.messages.page(chat_mid);
        let data = self.talk("getRecentMessagesV2", json!([chat_mid, 50]))?;
        let messages = data.as_array().context("missing recent messages")?;
        let mut messages: Vec<_> = messages.iter().take(50).map(|m| self.message(m)).collect();
        self.resolve_senders(chat_mid, &mut messages);
        sort_messages(&mut messages);
        // A cache write failure must not hide successfully fetched messages.
        let _ = self.messages.store_page(chat_mid, &messages);
        Ok(messages)
    }

    fn resolve_senders(&mut self, chat_mid: &str, messages: &mut [Message]) {
        if is_user(chat_mid) {
            return;
        }
        let mut users: Vec<_> = messages
            .iter()
            .filter(|m| {
                !m.outgoing && is_user(&m.sender_id) && !self.contacts.contains_key(&m.sender_id)
            })
            .map(|m| m.sender_id.clone())
            .collect();
        users.sort();
        users.dedup();
        // Sender lookup is bounded to this page and must never hide otherwise
        // readable history on a contact/transport failure.
        if !users.is_empty()
            && let Ok(response) = self.talk(
                "getContactsV2",
                json!([{"targetUserMids":users,"neededContactCalendarEvents":[]},0]),
            )
            && let Some(contacts) = response["contacts"].as_object()
        {
            for (mid, contact) in contacts {
                self.contacts
                    .insert(mid.clone(), contact["contact"].clone());
            }
        }
        for message in messages.iter_mut().filter(|m| !m.outgoing) {
            if let Some(contact) = self.contacts.get(&message.sender_id) {
                message.sender_name = contact_name(contact).unwrap_or("Unknown member").to_owned();
                message.sender_avatar = contact["picturePath"].as_str().and_then(Asset::avatar);
            } else if message.sender_name.is_empty() {
                message.sender_name = if message.sender_id.is_empty() {
                    "Unknown sender".into()
                } else {
                    format!(
                        "Member · {}",
                        message
                            .sender_id
                            .chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    )
                };
            }
        }
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
                Ok(body) => merge_decrypted(&mut data, &body),
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
                _ => "Unsupported message · this content cannot be displayed yet".into(),
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
        let mut message = Message {
            sender_id: data["from"].as_str().unwrap_or_default().to_owned(),
            sender_name: String::new(),
            sender_avatar: None,
            id: data["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("unavailable-{fingerprint}")),
            body,
            timestamp_ms: message_timestamp(&data),
            delivered_time: data["deliveredTime"]
                .as_i64()
                .or_else(|| data["deliveredTime"].as_str()?.parse().ok())
                .unwrap_or_else(|| message_timestamp(&data)),
            outgoing: data["from"].as_str() == Some(&self.my_mid),
            image,
            reactions: reactions(&data["reactions"]),
            encrypted,
            decryption_failed: failed,
            request_seq: self.messages.matching_request(raw),
            mentioned: mentioned(&data["contentMetadata"], &self.my_mid),
            skip_badge: data["contentMetadata"]["SKIP_BADGE_COUNT"].as_str() == Some("true"),
            metadata_version: 2,
            attachment: if failed {
                None
            } else {
                Asset::attachment(&data)
            },
            rich: {
                let mut rich = rich::parse(&data);
                rich.withdrawn |= self
                    .messages
                    .is_withdrawn(data["id"].as_str().unwrap_or_default());
                rich
            },
        };
        if message.rich.withdrawn {
            message.body.clear();
            message.image = None;
            message.reactions.clear();
            message.rich.content_type = 18;
        }
        if !message.decryption_failed {
            self.messages.remember(fingerprint, message.clone());
        }
        message
    }

    pub fn image(&mut self, asset: &Asset) -> Result<Arc<assets::Pixels>> {
        if let Some(pixels) = ImageCache::read_shared(&self.cache, asset)? {
            return Ok(pixels);
        }
        self.authorize_media(asset)?;
        let data = asset.fetch(&self.api.http, self.obs_access.as_deref())?;
        ImageCache::store_shared(&self.cache, asset, &data)
    }

    fn authorize_media(&mut self, asset: &Asset) -> Result<()> {
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
        Ok(())
    }
    pub fn media_token(&mut self, asset: &Asset) -> Result<Option<String>> {
        self.authorize_media(asset)?;
        Ok(self.obs_access.clone())
    }

    pub fn asset_fetchers(
        &self,
        stop: Arc<std::sync::atomic::AtomicBool>,
        priority: Arc<assets::Priority>,
        failures: assets::Failures,
        authorize: impl Fn(&Asset, &std::sync::atomic::AtomicBool) -> Option<String>
        + Send
        + Sync
        + 'static,
        publish: impl Fn(assets::Completion, &std::sync::atomic::AtomicBool) + Send + Sync + 'static,
    ) -> assets::Fetchers {
        assets::Fetchers::start(
            self.api.http.assets(),
            self.cache.clone(),
            stop,
            priority,
            failures,
            authorize,
            publish,
        )
    }

    pub fn file_asset(&mut self, chat: &str, message: &str) -> Result<(String, Asset)> {
        let item = match self.messages.cached_message(message)? {
            Some(message) => message,
            None => self
                .messages(chat)?
                .into_iter()
                .find(|m| m.id == message)
                .context("file is outside loaded history")?,
        };
        ensure!(
            !item.rich.withdrawn
                && item
                    .rich
                    .file_size
                    .is_none_or(|size| size <= 16 * 1024 * 1024),
            "file unavailable or exceeds 16 MiB download limit"
        );
        let asset = item.attachment.context("file download unavailable")?;
        Ok((
            item.rich.file_name.unwrap_or_else(|| "attachment".into()),
            asset,
        ))
    }

    pub fn cache_stats(&self) -> assets::CacheStats {
        self.cache.lock().unwrap().stats()
    }

    /// Called only after the user presses Send; diagnostics never call this.
    pub fn send_text(&mut self, to: &str, text: &str) -> Result<Message> {
        self.request_seq = self.request_seq.wrapping_add(1) & 0x7fff_ffff;
        self.send_text_with_request(to, text, self.request_seq)
    }
    /// The desktop persists this request identity before calling the network.
    pub fn send_text_with_request(
        &mut self,
        to: &str,
        text: &str,
        request_seq: u32,
    ) -> Result<Message> {
        ensure!(
            request_seq > 0 && request_seq <= 0x7fff_ffff,
            "invalid persisted send sequence"
        );
        if text.trim().is_empty() || text.len() > 10_000 {
            return Err(SendFailure::Rejected.into());
        }
        let result = self.encrypted_text(to, text);
        let message = match result {
            Ok(message) => message,
            Err(error) => {
                let _ = self.reset_runtime();
                return Err(error.context(SendFailure::Rejected));
            }
        };
        self.messages
            .record_request(request_seq, &message)
            .map_err(|error| error.context(SendFailure::Rejected))?;
        // Never automatically retry this RPC: a timeout can mean it was sent.
        let result = self.talk("sendMessage", json!([request_seq, message]))?;
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

fn mentioned(metadata: &Value, my_mid: &str) -> bool {
    let value = if metadata["MENTION"].is_object() {
        Some(metadata["MENTION"].clone())
    } else {
        metadata["MENTION"]
            .as_str()
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
    };
    value
        .as_ref()
        .and_then(|v| v["MENTIONEES"].as_array())
        .is_some_and(|mentions| {
            mentions.iter().any(|mention| {
                mention["M"].as_str() == Some(my_mid) || mention["A"].as_str() == Some("1")
            })
        })
}

fn merge_decrypted(data: &mut Value, body: &Value) {
    if !data["contentMetadata"].is_object() {
        data["contentMetadata"] = json!({});
    }
    for field in ["text", "location"] {
        if !body[field].is_null() {
            data[field] = body[field].clone();
        }
    }
    if !body["REPLACE"].is_null() {
        data["contentMetadata"]["REPLACE"] = body["REPLACE"].clone();
    }
    for (source, target) in [("keyMaterial", "ENC_KM"), ("fileName", "FILE_NAME")] {
        if let Some(value) = body[source].as_str() {
            data["contentMetadata"][target] = json!(value);
        }
    }
}

fn normalize_older(before: &str, mut messages: Vec<Message>) -> Vec<Message> {
    let mut ids = std::collections::HashSet::new();
    messages.retain(|m| m.id != before && ids.insert(m.id.clone()));
    sort_messages(&mut messages);
    messages
}

fn contact_name(contact: &Value) -> Option<&str> {
    contact["displayNameOverridden"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            contact["displayName"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
        })
}

fn message_timestamp(raw: &Value) -> i64 {
    ["createdTime", "deliveredTime"]
        .iter()
        .find_map(|field| {
            raw[*field]
                .as_i64()
                .or_else(|| raw[*field].as_str().and_then(|value| value.parse().ok()))
                .filter(|value| *value > 0)
        })
        .unwrap_or(0)
}
fn message_id_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
fn sort_messages(messages: &mut [Message]) {
    messages.sort_by(|a, b| {
        a.timestamp_ms
            .cmp(&b.timestamp_ms)
            .then_with(|| message_id_cmp(&a.id, &b.id))
    });
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

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn send_rejected_response_shapes_and_unanswered_transport() {
        for (http, code) in [(400, 0), (403, 10052), (200, 10102)] {
            let error = anyhow::Error::new(RpcError {
                method: "sendMessage".into(),
                http,
                code,
                status: 0,
            });
            assert!(matches!(send_failure(&error), SendFailure::Rejected));
        }
        assert!(matches!(
            send_failure(&anyhow::anyhow!("transport timeout")),
            SendFailure::Uncertain
        ));
        assert!(matches!(
            send_failure(&anyhow::Error::new(SendFailure::Rejected)),
            SendFailure::Rejected
        ));
    }
}

#[derive(Clone, Debug)]
pub struct ReadRange {
    pub chat: String,
    pub reader: String,
    pub start: String,
    pub end: String,
    pub open_end: bool,
}
fn parse_read_ranges(data: &Value) -> Vec<ReadRange> {
    let mut ranges = Vec::new();
    for chat in data.as_array().into_iter().flatten() {
        let Some(id) = chat["chatId"].as_str() else {
            continue;
        };
        for (reader, values) in chat["ranges"].as_object().into_iter().flatten() {
            for range in values.as_array().into_iter().flatten() {
                if let (Some(start), Some(end)) = (
                    range["startMessageId"].as_str(),
                    range["endMessageId"].as_str(),
                ) && (end == "1" || message_id_cmp(start, end).is_le())
                {
                    ranges.push(ReadRange {
                        chat: id.into(),
                        reader: reader.clone(),
                        start: start.into(),
                        end: end.into(),
                        open_end: end == "1",
                    });
                }
            }
        }
    }
    ranges
}

#[cfg(test)]
mod read_tests {
    use super::*;
    #[test]
    fn preserves_member_ranges_and_open_sentinel() {
        let ranges = parse_read_ranges(
            &json!([{"chatId":"group","ranges":{"member":[{"startMessageId":"4","endMessageId":"9"},{"startMessageId":"10","endMessageId":"1"}]}}]),
        );
        assert_eq!(ranges.len(), 2);
        assert!(ranges[1].open_end);
        assert!(!ranges[0].open_end);
        assert_eq!(ranges[0].reader, "member");
        assert_eq!(ranges[0].start, "4");
        assert_eq!(ranges[0].end, "9");
    }
}

#[cfg(test)]
mod decrypted_metadata_tests {
    use super::*;
    #[test]
    fn decrypted_replace_and_attachment_details_reach_rich_parser() {
        let mut raw = json!({"contentType":0,"contentMetadata":null});
        let body = json!({"text":"日😀[笑]","REPLACE":{"sticon":{"resources":[{"S":3,"E":6,"productId":"synthetic","sticonId":"1"}]}},"keyMaterial":"synthetic-key","fileName":"example.txt"});
        merge_decrypted(&mut raw, &body);
        let rich = rich::parse(&raw);
        assert_eq!(rich.emoji.len(), 1);
        assert_eq!(rich.emoji[0].fallback, "[笑]");
        assert_eq!(raw["contentMetadata"]["ENC_KM"], "synthetic-key");
        assert_eq!(raw["contentMetadata"]["FILE_NAME"], "example.txt");
    }
}

// A metadata upgrade must not turn a transient outage into lost offline history.
fn cached_or_refreshed(
    cached: Vec<Message>,
    refresh: impl FnOnce() -> Result<Vec<Message>>,
) -> Vec<Message> {
    refresh().unwrap_or(cached)
}
#[cfg(test)]
mod offline_metadata_tests {
    use super::*;
    #[test]
    fn failed_metadata_refresh_preserves_cached_history() {
        let old: Message = serde_json::from_value(serde_json::json!({
            "id":"123","body":"offline cached history","timestamp_ms":1,
            "outgoing":false,"image":null,"reactions":[],"encrypted":false,
            "decryption_failed":false
        }))
        .unwrap();
        let rows = cached_or_refreshed(vec![old], || {
            Err(anyhow::anyhow!("synthetic transport outage"))
        });
        assert_eq!(rows[0].body, "offline cached history");
        assert_eq!(rows[0].metadata_version, 0);
        assert!(!rows[0].skip_badge);
    }
}
