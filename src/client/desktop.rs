//! Account API shared by the desktop worker and read-only diagnostics.
use super::error::SignInFailure;
use super::*;
use std::{collections::HashMap, sync::Arc};

pub mod assets;
#[cfg(test)]
mod members_tests;
#[cfg(test)]
mod sticker_tests;

pub mod read;
pub mod rich;
pub mod sync;
use assets::{Asset, ImageCache};

#[derive(Clone)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub preview_message: Option<Message>,
    pub timestamp_ms: i64,
    pub avatar: Option<Asset>,
    pub unread: Option<u64>,
    pub muted: Option<bool>,
    pub last_read_message: Option<String>,
    pub last_message_id: Option<String>,
    pub member_count: Option<usize>,
    pub group: bool,
}

use crate::store::{Content, Friend, Member, Reaction, ReactionKind};

fn member_identity(id: &str, contact: &Value, my_mid: &str) -> Member {
    Member {
        is_self: id == my_mid,
        contact: Friend {
            profile: serde_json::from_value(contact.clone()).unwrap_or_default(),
            id: id.into(),
            name: contact_name(contact).map(str::to_owned),
            status: contact["statusMessage"].as_str().unwrap_or_default().into(),
            group: false,
            avatar: contact["picturePath"].as_str().and_then(Asset::avatar),
            member_count: None,
        },
    }
}

#[derive(Clone)]
pub struct Message {
    pub id: String,
    pub sender_id: String,
    pub content: Content,
    pub timestamp_ms: i64,
    pub delivered_time: i64,
    pub outgoing: bool,
    pub reactions: Vec<Reaction>,
    pub request_seq: Option<u32>,
    pub mentioned: bool,
    pub reply: Option<crate::store::ReplyReference>,
    pub skip_badge: bool,
}

/// Install the owning worker's cancellation token for all synchronous requests.
pub fn set_cancellation_token(cancel: std::sync::Arc<crate::client::http::CancelToken>) {
    set_request_cancel(cancel);
}

pub struct Client {
    api: Api,
    cache_root: PathBuf,
    my_mid: String,
    obs_access: Option<String>,
    sequence: u64,
    cache: Arc<std::sync::Mutex<ImageCache>>,
}

fn sticker_language(locale: Option<&str>) -> String {
    // Match extension aw(navigator.language), then settings.locale.slice(0, 2).
    const SUPPORTED: &[&str] = &[
        "en-US", "ja-JP", "ko-KR", "zh-CN", "zh-TW", "th-TH", "tr-TR", "ru-RU", "id-ID", "es-419",
        "es-ES",
    ];
    let normalized = locale
        .unwrap_or("")
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .replace('_', "-")
        .to_ascii_lowercase();
    let mapped = SUPPORTED
        .iter()
        .find(|s| s.eq_ignore_ascii_case(&normalized))
        .or_else(|| {
            SUPPORTED
                .iter()
                .find(|s| s[..2] == *normalized.split('-').next().unwrap_or(""))
        })
        .copied()
        .unwrap_or("en-US");
    mapped[..2].to_owned()
}

fn load_sticker_packages(
    language: &str,
    country: &str,
    mut request: impl FnMut(Value) -> Result<Value>,
) -> Result<Vec<crate::store::StickerPackage>> {
    let mut packages = Vec::new();
    let mut offset = 0u64;
    loop {
        let value =
            request(json!(["stickershop", offset, 1000, {"language":language,"country":country}]))?;
        let products = value["productList"]
            .as_array()
            .context("missing sticker packages")?;
        for p in products {
            let valid_until = p["validUntil"]
                .as_i64()
                .or_else(|| p["validUntil"].as_str()?.parse().ok());
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
            if valid_until.is_some_and(|expiry| expiry != -1 && expiry < now) {
                continue;
            }
            let id = required(p, "id")?.to_owned();
            let version = p["latestVersion"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| p["latestVersion"].to_string());
            let mut stickers = Vec::new();
            if let Some(ranges) =
                p["productTypeSummary"]["stickerSummary"]["stickerIdRanges"].as_array()
            {
                for range in ranges {
                    let number = |key: &str| {
                        range[key]
                            .as_u64()
                            .or_else(|| range[key].as_str()?.parse().ok())
                            .unwrap_or(0)
                    };
                    let start = number("start");
                    for n in start..start.saturating_add(number("size").min(1000)) {
                        let sticker_id = n.to_string();
                        stickers.push(Content::Sticker {
                            pkg: id.clone(),
                            id: sticker_id.clone(),
                            ver: Some(version.clone()),
                            asset: Asset::sticker(&json!({"STKID":sticker_id})),
                            text: None,
                        });
                    }
                }
                packages.push(crate::store::StickerPackage {
                    id,
                    name: p["name"].as_str().unwrap_or("Stickers").into(),
                    version,
                    stickers,
                });
            }
        }
        let page_offset = value["offset"].as_u64().context("missing sticker offset")?;
        let total = value["totalSize"]
            .as_u64()
            .context("missing sticker total")?;
        if page_offset + products.len() as u64 >= total {
            break;
        }
        ensure!(!products.is_empty(), "empty sticker page before total");
        offset += 1000;
    }
    Ok(packages)
}

impl Client {
    pub fn open_local(
        state_path: &Path,
        module: crate::crypto_module::CryptoModule,
        cache_root: &Path,
    ) -> Result<Self> {
        let wasm = module.read()?;
        let mut api = Api::with_runtime(module.runtime(&wasm)?, &wasm, &module.version)?;
        let state: Session = serde_json::from_value(crate::credentials::load(state_path)?)
            .context(SignInFailure::SavedSession)?;
        validate_keys(&mut api, &state).context(SignInFailure::SavedSession)?;
        Self::from_api(state_path, cache_root, state, api)
    }
    pub(crate) fn from_api(
        state_path: &Path,
        cache_root: &Path,
        state: Session,
        mut api: Api,
    ) -> Result<Self> {
        access(&state).context(SignInFailure::SavedSession)?;
        ensure!(!state.profile.mid.is_empty(), SignInFailure::SavedSession);
        let my_mid = state.profile.mid.clone();
        let cache = ImageCache::new(cache_root, &my_mid)
            .context(SignInFailure::StateDirectory(cache_root.into()))?;
        api.stream_access = Arc::new(std::sync::RwLock::new(access(&state)?.into()));
        api.state = state;
        api.session_path = Some(state_path.into());
        let sequence = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        Ok(Self {
            api,
            cache_root: cache_root.into(),
            my_mid: my_mid.clone(),
            obs_access: None,
            sequence,
            cache: Arc::new(std::sync::Mutex::new(cache)),
        })
    }

    pub(crate) fn installed_at(&mut self, path: &Path) {
        self.api.session_path = Some(path.into());
    }

    fn talk(&mut self, method: &str, args: Value) -> Result<Value> {
        e2ee::talk(&mut self.api, method, args)
    }

    pub fn local_mid(&self) -> &str {
        &self.my_mid
    }
    pub fn online_profile(&mut self) -> Result<Value> {
        let profile = self.api.profile()?;
        let mid = required(&profile, "mid")?;
        if !self.my_mid.is_empty() && mid != self.my_mid {
            return Err(crate::client::error::Failure::SignedOut(
                crate::client::error::SignOutReason::IdentityChanged,
            )
            .into());
        }
        if self.my_mid.is_empty() {
            self.my_mid = mid.into();
            self.cache = Arc::new(std::sync::Mutex::new(ImageCache::new(
                &self.cache_root,
                mid,
            )?));
        }
        *self.api.stream_access.write().unwrap() = access(&self.api.state)?.to_owned();
        self.api.state.profile = serde_json::from_value(profile.clone())?;
        save_state(
            self.api.session_path.as_ref().context("session path")?,
            &self.api.state,
        )?;
        Ok(profile)
    }
    pub fn latest_revision(&mut self) -> Result<i64> {
        let v = self.talk("getLastOpRevision", json!([]))?;
        v.as_i64()
            .or_else(|| v.as_str()?.parse().ok())
            .context("missing operation revision")
    }
    pub fn mark_read(&mut self, chat: &str, message: &str, seq: u32) -> Result<()> {
        self.talk("sendChatChecked", json!([seq, chat, message, 0]))?;
        Ok(())
    }
    pub fn read_ranges(&mut self, chat: &str) -> Result<Vec<ReadRange>> {
        let data = self.talk("getMessageReadRange", json!([[chat], 0]))?;
        Ok(parse_read_ranges(&data)
            .into_iter()
            .filter(|range| range.reader != self.my_mid)
            .collect())
    }
    pub fn operation(&mut self, operation: &Value) -> Result<Option<(String, Message)>> {
        if !matches!(operation["type"].as_u64(), Some(25 | 26 | 43)) {
            return Ok(None);
        }
        let raw = &operation["message"];
        let from = required(raw, "from")?;
        let to = required(raw, "to")?;
        let chat = if is_user(to) && from != self.my_mid {
            from
        } else {
            to
        }
        .to_owned();
        Ok(Some((chat, self.message(raw)?)))
    }
    pub fn client_version(&self) -> &str {
        &self.api.http.client_version
    }
    pub fn stream_access(&self) -> Arc<std::sync::RwLock<String>> {
        self.api.stream_access.clone()
    }
    pub fn refresh_token(&mut self) -> Result<()> {
        self.api.refresh_session()?;
        *self.api.stream_access.write().unwrap() = access(&self.api.state)?.to_owned();
        self.obs_access = None;
        Ok(())
    }

    pub fn chats(&mut self, cursor: Option<&str>) -> Result<(Vec<Chat>, bool, Option<String>)> {
        let mut request = json!({"activeOnly":true,"unreadOnly":false,
            "messageBoxCountLimit":50,"withUnreadCount":true,"lastMessagesPerMessageBoxCount":1});
        if let Some(cursor) = cursor {
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
        let chats = self.build_chats(boxes, &HashMap::new())?;
        Ok((chats, more_chats, chat_cursor))
    }
    pub fn chat_detail(&mut self, id: &str) -> Result<Vec<Chat>> {
        let mut group_info = HashMap::new();
        if is_user(id) {
            let response = self.talk(
                "getContactsV2",
                json!([{"targetUserMids":[id],"neededContactCalendarEvents":[]},0]),
            )?;
            group_info.insert(id.into(), response["contacts"][id]["contact"].clone());
        } else {
            let response = self.talk(
                "getChats",
                json!([{"chatMids":[id],"withMembers":true,"withInvitees":false},0]),
            )?;
            for chat in response["chats"].as_array().context("missing chats")? {
                group_info.insert(id.into(), chat.clone());
            }
        }
        self.build_chats(&[json!({"id":id})], &group_info)
    }
    fn build_chats(
        &mut self,
        boxes: &[Value],
        group_info: &HashMap<String, Value>,
    ) -> Result<Vec<Chat>> {
        let mut chats = Vec::new();
        for entry in boxes.iter().take(50) {
            let id = required(entry, "id")?.to_owned();
            let info = group_info.get(&id);
            let title = info
                .and_then(|v| {
                    v["displayNameOverridden"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| v["displayName"].as_str())
                        .or_else(|| v["chatName"].as_str())
                })
                .unwrap_or_default()
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
            let preview_message = last.map(|m| self.message(m)).transpose()?;
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
                preview_message,
                timestamp_ms,
                avatar,
            });
        }
        Ok(chats)
    }

    pub fn contact(&mut self, mid: &str) -> Result<Option<Friend>> {
        let response = self.talk(
            "getContactsV2",
            json!([{"targetUserMids":[mid],"neededContactCalendarEvents":[]}, 2]),
        )?;
        let contact = &response["contacts"][mid]["contact"];
        let member = member_identity(mid, contact, &self.my_mid);
        Ok(member.contact.name.is_some().then_some(member.contact))
    }
    pub fn profile_notice(&mut self, mid: &str) -> Result<Value> {
        self.api.call(
            "/api/talk/thrift/Relation/RelationService/getTargetProfileNotice",
            &json!([{"targetUserMid":mid}]),
            "",
            0,
        )
    }
    pub fn rename_contact(&mut self, mid: &str, name: &str, seq: u32) -> Result<()> {
        self.talk("updateContactSetting", json!([seq, mid, 2, name]))?;
        Ok(())
    }
    pub fn add_friend(&mut self, mid: &str, chat_mid: &str, seq: u32) -> Result<()> {
        self.api.call("/api/talk/thrift/Relation/RelationService/addFriendByMid", &json!([{"reqSeq":seq,"userMid":mid,"tracking":{"reference":"","trackingMetaV2":{"chat":{"chatMid":chat_mid}}}}]), "", 0)?;
        Ok(())
    }
    pub fn full_chat(&mut self, mid: &str) -> Result<Value> {
        let response = self.talk(
            "getChats",
            json!([{"chatMids":[mid],"withMembers":true,"withInvitees":true},1]),
        )?;
        let full = response["chats"]
            .as_array()
            .and_then(|c| c.first())
            .cloned()
            .context("missing chat")?;
        Ok(full)
    }
    pub fn mute_chat(&mut self, mid: &str, disabled: bool, seq: u32) -> Result<()> {
        let mut chat = self.full_chat(mid)?;
        ensure!(chat.is_object(), "missing full chat");
        chat["notificationDisabled"] = disabled.into();
        self.talk(
            "updateChat",
            json!([{"reqSeq":seq,"chat":chat,"updatedAttribute":8}]),
        )?;
        Ok(())
    }
    pub fn leave_chat(&mut self, mid: &str, seq: u32) -> Result<()> {
        self.talk("deleteSelfFromChat", json!([{"reqSeq":seq,"chatMid":mid}]))?;
        Ok(())
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
                    profile: serde_json::from_value(contact.clone()).unwrap_or_default(),
                    id: id.into(),
                    name: contact_name(contact).map(str::to_owned),
                    status: contact["statusMessage"].as_str().unwrap_or_default().into(),
                    group: false,
                    member_count: None,
                    avatar: contact["picturePath"].as_str().and_then(Asset::avatar),
                });
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
                    profile: Default::default(),
                    id: required(chat, "chatMid")?.into(),
                    name: chat["chatName"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    status: String::new(),
                    group: true,
                    member_count: rich::group_member_count(chat, &self.my_mid),
                    avatar: chat["picturePath"].as_str().and_then(Asset::avatar),
                });
            }
        }
        Ok(friends)
    }

    pub fn members(&mut self, chat: &str) -> Result<Vec<Member>> {
        let my_mid = self.my_mid.clone();
        load_members(chat, &my_mid, |method, args| self.talk(method, args))
    }

    pub fn messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        self.refresh_messages(chat_mid)
    }

    pub fn around(&mut self, chat: &str, id: &str) -> Result<(Vec<Message>, bool)> {
        let data = self.talk("getMessagesByIds", json!([[id]]))?;
        let raw = data
            .as_array()
            .and_then(|rows| rows.iter().find(|m| m["id"] == id))
            .context("original message unavailable")?;
        let original = self.message(raw)?;
        let (mut rows, start) = self.older(chat, id, original.delivered_time)?;
        rows.push(original);
        sort_messages(&mut rows);
        Ok((rows, start))
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
        let data = self.talk("getPreviousMessagesV2WithRequest", json!([{
            "messageBoxId":chat,"endMessageId":{"deliveredTime":delivered_time.to_string(),"messageId":before},"messagesCount":50
        },0]))?;
        let raw = data.as_array().context("missing older messages")?;
        let reached_beginning = raw.len() < 50;
        let messages: Vec<_> = raw
            .iter()
            .take(50)
            .map(|m| self.message(m))
            .collect::<Result<_>>()?;
        let messages = normalize_older(before, messages);
        Ok((messages, reached_beginning))
    }
    pub fn refresh_messages(&mut self, chat_mid: &str) -> Result<Vec<Message>> {
        let data = self.talk("getRecentMessagesV2", json!([chat_mid, 50]))?;
        let messages = data.as_array().context("missing recent messages")?;
        let mut messages: Vec<_> = messages
            .iter()
            .take(50)
            .map(|m| self.message(m))
            .collect::<Result<_>>()?;
        sort_messages(&mut messages);
        Ok(messages)
    }

    fn confirmation(&mut self, raw: &Value) -> Result<Message> {
        required(raw, "id")?;
        let mut confirmation = raw.clone();
        confirmation["chunks"] = Value::Null;
        self.message(&confirmation)
    }
    fn message(&mut self, raw: &Value) -> Result<Message> {
        let fingerprint = format!("{:x}", Sha256::digest(raw.to_string()));
        let mut data = raw.clone();
        let encrypted = raw["chunks"].as_array().is_some_and(|c| !c.is_empty());
        let mut undecryptable = false;
        if encrypted {
            let state = self.api.state.clone();
            match e2ee::decrypt_body(&mut self.api, &state, &self.my_mid, raw) {
                Ok(body) => merge_decrypted(&mut data, &body),
                Err(error) if crate::client::error::transport_loss(&error) => {
                    return Err(error);
                }
                Err(_) => undecryptable = true,
            }
        }
        let meta = &data["contentMetadata"];
        let details = rich::parse(&data);
        let content = if details.withdrawn {
            Content::Withdrawn
        } else if undecryptable {
            Content::Undecryptable
        } else {
            match data["contentType"].as_i64().unwrap_or(0) {
                0 => Content::Text {
                    text: data["text"].as_str().unwrap_or_default().into(),
                    spans: details.emoji,
                },
                1 => Content::Photo {
                    asset: Asset::photo(&data),
                    size: None,
                    album_count: details.album_count,
                    caption: details.caption,
                },
                2 => Content::Video {
                    duration: details.duration_ms,
                    asset: Asset::attachment(&data),
                    preview: Asset::photo(&data),
                    album_count: details.album_count,
                    caption: details.caption,
                },
                3 => Content::Voice {
                    duration: details.duration_ms,
                    asset: Asset::attachment(&data),
                },
                7 => Content::Sticker {
                    pkg: meta["STKPKGID"].as_str().unwrap_or_default().into(),
                    id: meta["STKID"].as_str().unwrap_or_default().into(),
                    ver: meta["STKVER"].as_str().map(str::to_owned),
                    asset: Asset::sticker(meta),
                    text: meta["STKTXT"].as_str().map(str::to_owned),
                },
                13 => Content::Contact {
                    mid: meta["mid"].as_str().unwrap_or_default().into(),
                    name: details.contact_name,
                },
                14 => Content::File {
                    name: details.file_name,
                    size: details.file_size,
                    asset: Asset::attachment(&data),
                },
                15 => Content::Location {
                    title: data["location"]["title"].as_str().map(str::to_owned),
                    lat: data["location"]["latitude"].as_f64().unwrap_or_default(),
                    lon: data["location"]["longitude"].as_f64().unwrap_or_default(),
                },
                18 => Content::System {
                    kind: details.system_key,
                    params: details.system_args,
                    text: details.system_text,
                },
                content_type => Content::Unknown {
                    content_type: content_type as i32,
                },
            }
        };
        Ok(Message {
            id: data["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("unavailable-{fingerprint}")),
            sender_id: data["from"].as_str().unwrap_or_default().into(),
            timestamp_ms: message_timestamp(&data),
            delivered_time: data["deliveredTime"]
                .as_i64()
                .or_else(|| data["deliveredTime"].as_str()?.parse().ok())
                .unwrap_or_else(|| message_timestamp(&data)),
            outgoing: data["from"].as_str() == Some(&self.my_mid),
            content,
            reactions: if details.withdrawn {
                vec![]
            } else {
                reactions(&data["reactions"])
            },
            request_seq: data["reqSeq"]
                .as_u64()
                .or_else(|| data["requestSeq"].as_u64())
                .map(|n| n as u32),
            mentioned: mentioned(meta, &self.my_mid),
            reply: details.reply,
            skip_badge: meta["SKIP_BADGE_COUNT"].as_str() == Some("true"),
        })
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
        if asset.local_attachment().is_none() && asset.private && self.obs_access.is_none() {
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
        if asset.local_attachment().is_some() {
            return Ok(None);
        }
        self.authorize_media(asset)?;
        Ok(self.obs_access.clone())
    }

    pub fn asset_fetchers(
        &self,
        stop: Arc<crate::client::http::CancelToken>,
        failures: assets::Failures,
        authorize: impl Fn(
            &Asset,
            &crate::client::http::CancelToken,
        ) -> std::result::Result<Option<String>, assets::Failure>
        + Send
        + Sync
        + 'static,
        publish: impl Fn(assets::Completion, &crate::client::http::CancelToken) + Send + Sync + 'static,
    ) -> assets::Fetchers {
        failures.scope(&self.my_mid);
        assets::Fetchers::start(
            self.api.http.assets(),
            self.cache.clone(),
            stop,
            failures,
            authorize,
            publish,
        )
    }

    pub fn file_asset(&mut self, chat: &str, message: &str) -> Result<(String, Asset)> {
        let item = self
            .messages(chat)?
            .into_iter()
            .find(|m| m.id == message)
            .ok_or(assets::Failure::Http(404))?;
        ensure!(
            !matches!(item.content, Content::Withdrawn),
            assets::Failure::Http(404)
        );
        ensure!(
            match &item.content {
                Content::File { size, .. } => *size,
                _ => None,
            }
            .is_none_or(|size| size <= 16 * 1024 * 1024),
            assets::Failure::TooLarge
        );
        let asset = item
            .content
            .attachment()
            .cloned()
            .ok_or(assets::Failure::UnsupportedHost)?;
        Ok((
            match item.content {
                Content::File { name, .. } => name.unwrap_or_else(|| "attachment".into()),
                _ => "attachment".into(),
            },
            asset,
        ))
    }

    /// Source-backed shapes; live unverified.
    pub fn react(&mut self, id: &str, kind: Option<u32>, seq: u32) -> Result<()> {
        if let Some(kind) = kind {
            ensure!((2..=7).contains(&kind), "invalid reaction");
            self.talk("react", json!([{"reqSeq":seq,"messageId":id,"reactionType":{"predefinedReactionType":kind}}]))?;
        } else {
            self.talk("cancelReaction", json!([{"reqSeq":seq,"messageId":id}]))?;
        }
        Ok(())
    }
    pub fn send_sticker(
        &mut self,
        to: &str,
        sticker: &Content,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<Message> {
        let Content::Sticker { pkg, id, ver, .. } = sticker else {
            return Err(crate::client::error::Failure::Bug("invalid send payload".into()).into());
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let mut args = json!([seq, {"from":self.my_mid,"to":to,"toType":if is_user(to){0}else if to.starts_with(['C','c']){2}else{1},"id":format!("{now:015}"),"createdTime":now,"sessionId":0,"contentType":7,"contentMetadata":{"STKPKGID":pkg,"STKID":id,"STKVER":ver.as_deref().unwrap_or("1")},"hasContent":true}]);
        attach_reply(&mut args[1], reply);
        let result = self.talk("sendMessage", args)?;
        self.confirmation(&result)
    }
    pub fn create_chat(
        &mut self,
        name: &str,
        targets: &[String],
        seq: u32,
    ) -> Result<crate::store::ChatInput> {
        let targets = targets
            .iter()
            .filter(|id| *id != &self.my_mid)
            .collect::<Vec<_>>();
        let result = self.talk(
            "createChat",
            json!([{"reqSeq":seq,"type":0,"name":name.trim(),"targetUserMids":targets}]),
        )?;
        let chat = &result["chat"];
        Ok(crate::store::ChatInput {
            id: crate::store::ChatId(required(chat, "chatMid")?.into()),
            title: chat["chatName"].as_str().unwrap_or(name).into(),
            is_group: true,
            member_count: Some(targets.len() as u32 + 1),
            ..Default::default()
        })
    }
    pub fn sticker_packages(&mut self) -> Result<Vec<crate::store::StickerPackage>> {
        let country = self
            .api
            .state
            .profile
            .rest
            .get("regionCode")
            .and_then(Value::as_str)
            .context("missing profile regionCode")?
            .to_owned();
        let language = sticker_language(sys_locale::get_locale().as_deref());
        load_sticker_packages(&language, &country, |args| {
            self.api.call(
                "/api/shop/thrift/ShopService/ShopService/getOwnedProductSummaries",
                &args,
                "",
                0,
            )
        })
    }
    pub fn unsend(&mut self, id: &str, seq: u32) -> Result<()> {
        self.talk("unsendMessage", json!([seq, id]))?;
        Ok(())
    }
    /// The desktop persists this request identity before calling the network.
    pub fn send_text_with_request(
        &mut self,
        to: &str,
        text: &str,
        request_seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<Message> {
        ensure!(
            request_seq > 0 && request_seq <= 0x7fff_ffff,
            "invalid persisted send sequence"
        );
        if text.trim().is_empty() || text.len() > 10_000 {
            return Err(crate::client::error::Failure::Bug("invalid send payload".into()).into());
        }
        self.send_encrypted(to, 0, &json!({"text":text}), &json!({}), request_seq, reply)
    }
    fn send_encrypted(
        &mut self,
        to: &str,
        content_type: u64,
        payload: &Value,
        metadata: &Value,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<Message> {
        let send = |client: &mut Self| -> Result<Value> {
            let mut message = client.encrypted_payload(to, content_type, payload)?;
            if content_type != 0 {
                ensure!(
                    message["contentMetadata"]["e2eeVersion"] == "2",
                    "media requires E2EE V2"
                );
            }
            for (key, value) in metadata.as_object().context("message metadata")? {
                message["contentMetadata"][key] = value.clone();
            }
            attach_reply(&mut message, reply);
            client.talk("sendMessage", json!([seq, message]))
        };
        let result = match send(self) {
            Err(error) if super::error::classify(&error).key_rotated() => send(self),
            result => result,
        }?;
        self.confirmation(&result)
    }

    fn encrypted_payload(&mut self, to: &str, content_type: u64, payload: &Value) -> Result<Value> {
        self.api.prepare_runtime()?;
        let sender_id = self
            .api
            .state
            .e2ee_keys
            .iter()
            .map(|key| key.id)
            .max()
            .context("missing own key")?;
        let (receiver_id, version) = if is_user(to) {
            let negotiation = self.talk("negotiateE2EEPublicKey", json!([to]))?;
            ensure!(
                negotiation["allowedTypes"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(content_type))),
                "recipient does not support this encrypted content"
            );
            let public = &negotiation["publicKey"];
            let id = public["keyId"].as_u64().context("missing peer key ID")?;
            (id, negotiation["specVersion"].as_u64().unwrap_or(2))
        } else {
            let shared = self.talk("getLastE2EEGroupSharedKey", json!([1, to]))?;
            let id = shared["groupKeyId"]
                .as_u64()
                .context("missing group key ID")?;
            let version = shared["specVersion"].as_u64().unwrap_or(2);
            (id, version)
        };
        let state = self.api.state.clone();
        let ch = e2ee::message_channel(
            &mut self.api,
            &state,
            &self.my_mid,
            &if is_user(to) {
                e2ee::Channel::Direct {
                    mid: to.into(),
                    own_key_id: sender_id,
                    peer_key_id: receiver_id,
                }
            } else {
                e2ee::Channel::Group {
                    mid: to.into(),
                    group_key_id: receiver_id,
                    sender: self.my_mid.clone(),
                    sender_key_id: sender_id,
                }
            },
        )?;
        let h = self.api.rt.bytes(&serde_json::to_vec(payload)?);
        let encrypted = if version == 1 {
            bytes_method(&mut self.api.rt, Call::E2EEChannelEncryptV1, ch, &[h])?
        } else {
            ensure!(version == 2, "unsupported encryption version");
            let dest = self.api.rt.string(to)?;
            let from = self.api.rt.string(&self.my_mid)?;
            self.sequence = self.sequence.checked_add(1).context("sequence exhausted")?;
            let bytes = bytes_method(
                &mut self.api.rt,
                Call::E2EEChannelEncryptV2,
                ch,
                &[
                    dest,
                    from,
                    sender_id,
                    receiver_id,
                    content_type,
                    self.sequence,
                    h,
                ],
            )?;
            self.api.rt.free_string(dest)?;
            self.api.rt.free_string(from)?;
            bytes
        };
        self.api.rt.release(h)?;
        let chunks = pack_chunks(&encrypted, version, sender_id, receiver_id)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .to_string();
        Ok(
            json!({"to":to,"from":self.my_mid,"toType":if is_user(to){0}else if to.starts_with(['C','c']){2}else{1},
            "createdTime":now,"sessionId":0,"contentType":content_type,"hasContent":content_type != 0,
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

pub fn reaction_kind(value: &Value) -> Option<ReactionKind> {
    if let Some(id) = value["predefinedReactionType"].as_u64() {
        Some(ReactionKind::Predefined(id as u32))
    } else if value["paidReactionType"].is_object() {
        Some(ReactionKind::Paid(Asset::paid_reaction(
            &value["paidReactionType"],
        )))
    } else if value.as_object().is_some_and(|v| !v.is_empty()) {
        Some(ReactionKind::Emoji)
    } else {
        None
    }
}
pub fn reactions(value: &Value) -> Vec<Reaction> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| Reaction {
            actor: v["fromUserMid"].as_str().unwrap_or_default().into(),
            kind: reaction_kind(&v["reactionType"]).unwrap_or(ReactionKind::Emoji),
        })
        .collect()
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn member_identity_uses_existing_contact_names_and_unknown_fallback() {
        let known = member_identity(
            "other",
            &json!({"displayNameOverridden":"Alias","displayName":"Name","statusMessage":"Hello","picturePath":"/avatar"}),
            "self",
        );
        assert_eq!(known.contact.name.as_deref(), Some("Alias"));
        assert_eq!(known.contact.status, "Hello");
        assert!(!known.is_self);
        for contact in [Value::Null, json!({"displayName":"  "})] {
            let member = member_identity("self", &contact, "self");
            assert_eq!(member.contact.name, None);
            assert!(member.is_self);
            assert!(member.contact.avatar.is_none());
        }
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

mod upload;

#[cfg(test)]
mod send_tests;
#[cfg(test)]
mod test_client;

fn load_members(
    chat: &str,
    my_mid: &str,
    mut talk: impl FnMut(&str, Value) -> Result<Value>,
) -> Result<Vec<Member>> {
    ensure!(!is_user(chat), "not a group or room");
    let response = talk(
        "getChats",
        json!([{"chatMids":[chat],"withMembers":true,"withInvitees":true},1]),
    )?;
    let full = response["chats"]
        .as_array()
        .and_then(|c| c.first())
        .context("missing chat")?;
    let mut ids = rich::member_mids(full, my_mid).context("missing members")?;
    let invited: Vec<String> = full
        .pointer("/extra/groupExtra/inviteeMids")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for id in &invited {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let mut contacts = HashMap::new();
    for chunk in ids.chunks(100) {
        let response = talk(
            "getContactsV2",
            json!([{"targetUserMids":chunk,"neededContactCalendarEvents":[]},0]),
        )?;
        ensure!(response["contacts"].is_object(), "missing contacts");
        for id in chunk {
            contacts.insert(id.clone(), response["contacts"][id]["contact"].clone());
        }
    }
    Ok(ids
        .iter()
        .map(|id| {
            let mut member = member_identity(id, contacts.get(id).unwrap_or(&Value::Null), my_mid);
            member.contact.profile.invited = invited.contains(id);
            member
        })
        .collect())
}

fn attach_reply(message: &mut Value, reply: Option<&crate::store::ReplyReference>) {
    if let Some(reply) = reply {
        message["relatedMessageId"] = json!(reply.message_id);
        message["messageRelationType"] = json!(3);
        message["relatedMessageServiceCode"] = json!(reply.service_code.unwrap_or(1));
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::message_id_cmp;
    #[test]
    fn message_ids_have_a_total_order_even_for_invalid_ids() {
        let ids = [
            "",
            "0",
            "00",
            "2",
            "10",
            "1a",
            "abc",
            "999999999999999999999999999",
        ];
        for a in ids {
            for b in ids {
                assert_eq!(message_id_cmp(a, b), message_id_cmp(b, a).reverse());
                for c in ids {
                    if message_id_cmp(a, b).is_le() && message_id_cmp(b, c).is_le() {
                        assert!(message_id_cmp(a, c).is_le());
                    }
                }
            }
        }
        assert!(message_id_cmp("2", "10").is_lt());
    }
}
