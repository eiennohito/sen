//! RPC boundary shared by the real account and synthetic runs.
use crate::store::*;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub use crate::client::error::{Error, NetCause, RejectReason};
pub type Result<T> = std::result::Result<T, Error>;
/// Traced writes; live account verification remains pending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileAction {
    Rename { mid: String, name: String },
    AddFriend { mid: String, chat: String },
    Mute { chat: ChatId, disabled: bool },
    Leave { chat: ChatId },
}
pub use crate::store::Profile;
#[derive(Clone, Debug)]
pub struct ChatPage {
    pub chats: Vec<ChatInput>,
    pub previews: Vec<MessageInput>,
    pub more: bool,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Page {
    pub kind: PageKind,
    pub rows: Vec<MessageInput>,
    pub reaches_start: bool,
}
pub use crate::store::{LiveOperation as LiveOp, Operation as Op};
#[derive(Clone)]
pub enum PairEvent {
    Stage(String),
    Qr(Arc<crate::client::desktop::assets::Pixels>),
    Pin(String),
    Scanned,
}
pub type PairDeliver = Arc<dyn Fn(PairEvent) + Send + Sync>;
/// Asset queue handles are independent of the backend's RPC owner.
#[derive(Clone)]
pub struct AssetRequests {
    pub request: Arc<
        dyn Fn(crate::client::desktop::assets::Asset, crate::client::desktop::assets::Purpose)
            + Send
            + Sync,
    >,
    pub retry: Arc<dyn Fn(String) + Send + Sync>,
}
pub enum StreamEvent {
    AssetRequests(AssetRequests),
    Asset(crate::client::desktop::assets::Completion),
    Authorize {
        asset: crate::client::desktop::assets::Asset,
        reply: std::sync::mpsc::SyncSender<
            std::result::Result<Option<String>, crate::client::desktop::assets::Failure>,
        >,
    },
    Resync {
        revision: Option<i64>,
        complete: std::sync::mpsc::SyncSender<bool>,
    },
    Operation(LiveOp),
    Raw(Value),
    RefreshToken {
        reply: std::sync::mpsc::SyncSender<Result<()>>,
    },
    Connection(Result<()>),
    Exited,
    Connecting(u32),
    Retry {
        attempt: u32,
        error: Error,
    },
}
pub type Deliver = Arc<dyn Fn(StreamEvent) -> bool + Send + Sync>;
/// Each stream invocation runs on its own thread. Returning is supervised by the engine.
pub trait Backend: Send {
    fn sign_in(&mut self, cancel: Arc<AtomicBool>, deliver: PairDeliver) -> Result<()>;
    fn friends(&mut self) -> Result<Vec<crate::store::Friend>>;
    fn members(&mut self, chat: &ChatId) -> Result<Vec<crate::store::Member>>;
    /// Publish AssetRequests before returning. Its handles enqueue independently
    /// of RPCs, and asset completions use this delivery channel directly.
    fn start_assets(&mut self, stop: Arc<AtomicBool>, deliver: Deliver);
    fn authorize(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
    ) -> std::result::Result<Option<String>, crate::client::desktop::assets::Failure>;
    fn retry_asset(&mut self, key: &str);
    fn media(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>>;
    fn contact(&mut self, mid: &str) -> Result<crate::store::Friend>;
    fn notice(&mut self, mid: &str) -> Result<Value>;
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value>;
    fn profile_action(&mut self, action: &ProfileAction, full_chat: Value, seq: u32) -> Result<()>;
    fn profile(&mut self) -> Result<Profile>;
    fn chats(&mut self, cursor: Option<String>) -> Result<ChatPage>;
    fn details(&mut self, chat: &ChatId) -> Result<Vec<ChatInput>>;
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page>;
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput>;
    fn unsend(&mut self, id: &ServerId, seq: u32) -> Result<()>;
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>>;
    fn mark_read(&mut self, chat: &ChatId, id: &ServerId, seq: u32) -> Result<()>;
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<AtomicBool>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>>;
    fn decode(&mut self, raw: Value) -> Result<LiveOp>;
    fn refresh_token(&mut self, stop: &AtomicBool) -> Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeRpc {
    Profile,
    Action(ProfileAction),
    Contact(String),
    Chats,
    Details(ChatId),
    Messages(ChatId),
    Receipts(ChatId),
    Read(ChatId),
    Unsend(ServerId),
}
#[derive(Default)]
struct FakeState {
    calls: Vec<FakeRpc>,
    failures: Vec<(FakeRpc, Error)>,
    friends: Option<Vec<crate::store::Friend>>,
    contacts: std::collections::BTreeMap<String, Friend>,
    members: Option<Vec<crate::store::Member>>,
    offline: bool,
    fail_send: bool,
    fail_friends: bool,
    friends_calls: u64,
    receipts: std::collections::BTreeMap<ChatId, ServerId>,
    pixels: std::collections::BTreeMap<String, Arc<crate::client::desktop::assets::Pixels>>,
    uncertain_send: bool,
    echo_first: bool,
    next_id: i64,
    next_revision: i64,
    chats: Vec<ChatInput>,
    pages: std::collections::BTreeMap<ChatId, Vec<MessageInput>>,
    page_calls: std::collections::BTreeMap<ChatId, u64>,
    deliver: Option<Deliver>,
    assets: Option<Deliver>,
    failed_assets: std::collections::BTreeMap<String, crate::client::desktop::assets::Purpose>,
    asset_failures: std::collections::BTreeMap<String, crate::client::desktop::assets::Failure>,
    slow_assets: std::collections::BTreeMap<String, Duration>,
    revisions: Vec<i64>,
}
#[derive(Clone)]
pub struct FakeBackend(Arc<Mutex<FakeState>>);
impl Default for FakeBackend {
    fn default() -> Self {
        Self::canned()
    }
}
impl FakeBackend {
    pub fn canned() -> Self {
        let chats = [("alice", "葵"), ("weekend", "Weekend plans")]
            .into_iter()
            .enumerate()
            .map(|(i, (id, title))| ChatInput {
                id: ChatId(id.into()),
                title: title.into(),
                is_group: i != 0,
                member_count: (i == 1).then_some(3),
                last_message_id: Some(ServerId(if i == 0 { "10" } else { "2" }.into())),
                last_message_ms: Some(1792300000000 + if i == 0 { 10 } else { 2 }),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut pages = std::collections::BTreeMap::new();
        for (i, chat) in chats.iter().enumerate() {
            pages.insert(
                chat.id.clone(),
                vec![Self::message(
                    &chat.id.0,
                    if i == 0 { 10 } else { 2 },
                    if i == 0 {
                        "Hello! 日本語も使えます。"
                    } else {
                        "Shall we meet at the station?"
                    },
                    false,
                )],
            );
        }
        Self(Arc::new(Mutex::new(FakeState {
            chats,
            pages,
            next_id: 100,
            next_revision: 1000,
            ..Default::default()
        })))
    }
    pub fn message(chat: &str, id: i64, text: &str, outgoing: bool) -> MessageInput {
        MessageInput {
            server_id: ServerId(id.to_string()),
            chat_id: ChatId(chat.into()),
            created_ms: 1792300000000 + id,
            delivered_ms: None,
            sender_id: if outgoing { "self" } else { "alice" }.into(),
            outgoing,

            reply: None,
            mentioned: false,
            skip_badge: false,
            request_seq: None,
            body: Content::text(text),
            reactions: vec![],
            reactions_rev: id,
        }
    }
    pub fn offline(&self, offline: bool) {
        let deliver = {
            let mut s = self.0.lock().unwrap();
            s.offline = offline;
            s.deliver.clone()
        };
        if let Some(d) = deliver {
            d(StreamEvent::Connection(if offline {
                Err(Error::Offline(NetCause::Unreachable))
            } else {
                Ok(())
            }));
        }
    }
    pub fn fail_rpc(&self, rpc: FakeRpc, error: Error) {
        self.0.lock().unwrap().failures.push((rpc, error));
    }
    pub fn calls(&self) -> Vec<FakeRpc> {
        self.0.lock().unwrap().calls.clone()
    }
    fn rpc(&self, rpc: FakeRpc) -> Result<()> {
        self.online()?;
        let mut s = self.0.lock().unwrap();
        s.calls.push(rpc.clone());
        if let Some(index) = s.failures.iter().position(|(call, _)| call == &rpc) {
            return Err(s.failures.remove(index).1);
        }
        Ok(())
    }
    pub fn set_contact(&self, contact: Friend) {
        self.0
            .lock()
            .unwrap()
            .contacts
            .insert(contact.id.clone(), contact);
    }
    pub fn set_friends(&self, mut rows: Vec<crate::store::Friend>) {
        for row in &mut rows {
            if !row.group && row.profile.relation.is_none() {
                row.profile.relation = Some(0);
            }
        }
        self.0.lock().unwrap().friends = Some(rows);
    }
    pub fn set_members(&self, rows: Vec<crate::store::Member>) {
        self.0.lock().unwrap().members = Some(rows);
    }
    pub fn page(&self, chat: &ChatId) -> Vec<MessageInput> {
        self.0
            .lock()
            .unwrap()
            .pages
            .get(chat)
            .cloned()
            .unwrap_or_default()
    }
    pub fn fail_friends(&self) {
        self.0.lock().unwrap().fail_friends = true;
    }
    pub fn friends_calls(&self) -> u64 {
        self.0.lock().unwrap().friends_calls
    }
    pub fn set_asset_failure(
        &self,
        asset: &crate::client::desktop::assets::Asset,
        failure: crate::client::desktop::assets::Failure,
    ) {
        self.0
            .lock()
            .unwrap()
            .asset_failures
            .insert(asset.key(), failure);
    }
    pub fn set_pixels(
        &self,
        asset: &crate::client::desktop::assets::Asset,
        pixels: crate::client::desktop::assets::Pixels,
    ) {
        self.0
            .lock()
            .unwrap()
            .pixels
            .insert(asset.key(), Arc::new(pixels));
    }
    pub fn delay_asset(&self, asset: &crate::client::desktop::assets::Asset, delay: Duration) {
        self.0
            .lock()
            .unwrap()
            .slow_assets
            .insert(asset.key(), delay);
    }
    pub fn set_receipt(&self, chat: ChatId, id: ServerId) {
        self.0.lock().unwrap().receipts.insert(chat, id);
    }
    pub fn set_chats(&self, chats: Vec<ChatInput>) {
        let mut s = self.0.lock().unwrap();
        s.chats = chats;
        s.pages.clear();
    }
    pub fn fail_send(&self) {
        self.0.lock().unwrap().fail_send = true;
    }
    pub fn auth_expired(&self) {
        let d = self.0.lock().unwrap().deliver.clone();
        if let Some(d) = d {
            d(StreamEvent::Connection(Err(Error::Auth)));
        }
    }
    pub fn asset(photo: bool) -> crate::client::desktop::assets::Asset {
        serde_json::from_value(json!({"url":if photo {"https://obs.line-apps.com/r/talk/m/synthetic/preview"}else{"https://stickershop.line-scdn.net/stickershop/v1/sticker/1/android/sticker.png"},"private":photo,"key_material":null,"talk_meta":null,"dimension":384,"original_suffix":"","failure":null,"name":"Synthetic original"})).unwrap()
    }
    pub fn official_image(chat: &str, id: i64) -> MessageInput {
        let mut row = Self::message(chat, id, "", false);

        row.body = Content::Photo {
            asset: Some(
                crate::client::desktop::assets::Asset::photo(&json!({
                    "id":id.to_string(), "contentMetadata":{
                        "PREVIEW_URL":"https://manager.line-scdn.net/synthetic/clearpreview",
                        "DOWNLOAD_URL":"https://manager.line-scdn.net/synthetic/original"
                    }
                }))
                .unwrap(),
            ),
            size: None,
            album_count: None,
            caption: None,
        };
        row
    }
    pub fn uncertain_send(&self) {
        self.0.lock().unwrap().uncertain_send = true;
    }
    pub fn echo_first(&self, value: bool) {
        self.0.lock().unwrap().echo_first = value;
    }
    pub fn revision(&self) -> i64 {
        let mut s = self.0.lock().unwrap();
        s.next_revision = s.next_revision.max(s.next_id) + 1;
        s.next_revision
    }
    pub fn add_chat(&self, chat: ChatInput) {
        let mut s = self.0.lock().unwrap();
        s.chats.retain(|c| c.id != chat.id);
        s.chats.push(chat);
    }
    pub fn set_page(&self, chat: ChatId, rows: Vec<MessageInput>) {
        let mut s = self.0.lock().unwrap();
        for m in &rows {
            s.next_id = s.next_id.max(m.server_id.0.parse().unwrap());
        }
        s.pages.insert(chat, rows);
    }
    pub fn page_requests(&self, chat: &str) -> u64 {
        self.0
            .lock()
            .unwrap()
            .page_calls
            .get(&ChatId(chat.into()))
            .copied()
            .unwrap_or(0)
    }
    pub fn resumed_revisions(&self) -> Vec<i64> {
        self.0.lock().unwrap().revisions.clone()
    }
    pub fn inject_raw(&self, mut raw: Value) {
        raw["revision"] = self.revision().into();
        let deliver = self.0.lock().unwrap().deliver.clone();
        if let Some(d) = deliver {
            d(StreamEvent::Raw(raw));
        }
    }
    pub fn inject(&self, op: LiveOp) {
        let deliver = {
            let mut s = self.0.lock().unwrap();
            s.next_revision = s.next_revision.max(op.revision);
            if let Op::Message(m) = &op.op {
                s.next_id = s.next_id.max(m.server_id.0.parse().unwrap());
            }
            s.deliver.clone()
        };
        if let Some(d) = deliver {
            d(StreamEvent::Operation(op));
        }
    }
    fn online(&self) -> Result<()> {
        if self.0.lock().unwrap().offline {
            Err(Error::Offline(NetCause::Unreachable))
        } else {
            Ok(())
        }
    }
}
impl Backend for FakeBackend {
    fn sign_in(&mut self, _cancel: Arc<AtomicBool>, deliver: PairDeliver) -> Result<()> {
        deliver(PairEvent::Stage("Synthetic sign-in".into()));
        Err(Error::Rejected(RejectReason::SyntheticSignIn))
    }

    fn friends(&mut self) -> Result<Vec<crate::store::Friend>> {
        self.online()?;
        {
            let mut s = self.0.lock().unwrap();
            s.friends_calls += 1;
            if std::mem::take(&mut s.fail_friends) {
                return Err(Error::Rejected(RejectReason::SyntheticFriends));
            }
        }
        if let Some(rows) = &self.0.lock().unwrap().friends {
            return Ok(rows.clone());
        }
        Ok(self
            .0
            .lock()
            .unwrap()
            .chats
            .iter()
            .map(|c| crate::store::Friend {
                profile: ContactProfile {
                    relation: Some(0),
                    ..Default::default()
                },
                id: c.id.0.clone(),
                name: c.title.clone(),
                status: String::new(),
                group: c.is_group,
                avatar: c.avatar.clone(),
                member_count: c.member_count.map(|n| n as usize),
            })
            .collect())
    }
    fn members(&mut self, _chat: &ChatId) -> Result<Vec<crate::store::Member>> {
        self.online()?;
        if let Some(rows) = &self.0.lock().unwrap().members {
            return Ok(rows.clone());
        }
        Ok(self
            .friends()?
            .into_iter()
            .map(|contact| crate::store::Member {
                resolved: true,
                contact,
                is_self: false,
            })
            .collect())
    }
    fn start_assets(&mut self, _stop: Arc<AtomicBool>, deliver: Deliver) {
        self.0.lock().unwrap().assets = Some(deliver.clone());
        let fake = self.clone();
        let retry = self.clone();
        let output = deliver.clone();
        deliver(StreamEvent::AssetRequests(AssetRequests {
            request: Arc::new(move |asset, purpose| {
                let mut fake = fake.clone();
                let output = output.clone();
                std::thread::spawn(move || {
                    if let Ok(Some(payload)) = fake.media(&asset, purpose.clone()) {
                        output(StreamEvent::Asset(
                            crate::client::desktop::assets::Completion {
                                key: asset.key(),
                                purpose,
                                result: Ok(payload),
                            },
                        ));
                    }
                });
            }),
            retry: Arc::new(move |key| {
                let mut fake = retry.clone();
                fake.retry_asset(&key);
            }),
        }));
    }
    fn authorize(
        &mut self,
        _asset: &crate::client::desktop::assets::Asset,
    ) -> std::result::Result<Option<String>, crate::client::desktop::assets::Failure> {
        Ok(None)
    }
    fn retry_asset(&mut self, key: &str) {
        let (purpose, deliver) = {
            let mut s = self.0.lock().unwrap();
            (
                s.failed_assets.remove(key).unwrap(),
                s.assets.clone().unwrap(),
            )
        };
        deliver(StreamEvent::Asset(
            crate::client::desktop::assets::Completion {
                key: key.into(),
                purpose,
                result: Ok(crate::client::desktop::assets::Payload::Image(Arc::new(
                    crate::client::desktop::assets::Pixels {
                        width: 384,
                        height: 256,
                        rgba: [80, 180, 130, 255].repeat(384 * 256),
                    },
                ))),
            },
        ));
    }
    fn media(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        use crate::client::desktop::assets::{Payload, Pixels, Purpose};
        if let Some(failure) = asset.failure().or_else(|| {
            self.0
                .lock()
                .unwrap()
                .asset_failures
                .get(&asset.key())
                .copied()
        }) {
            let deliver = {
                let mut s = self.0.lock().unwrap();
                s.failed_assets.insert(asset.key(), purpose.clone());
                s.assets.clone().unwrap()
            };
            deliver(StreamEvent::Asset(
                crate::client::desktop::assets::Completion {
                    key: asset.key(),
                    purpose,
                    result: Err(failure),
                },
            ));
            return Ok(None);
        }
        let slow = self.0.lock().unwrap().slow_assets.remove(&asset.key());
        if let Some(delay) = slow {
            std::thread::sleep(delay);
        }
        if let Some(pixels) = self.0.lock().unwrap().pixels.get(&asset.key()).cloned() {
            return Ok(Some(Payload::Image(pixels)));
        }
        Ok(Some(match purpose {
            Purpose::File { .. } => Payload::File(Arc::new(b"Synthetic attachment\n".to_vec())),
            Purpose::Original { .. } => Payload::Image(Arc::new(Pixels {
                width: 1200,
                height: 800,
                rgba: [80, 180, 130, 255].repeat(1200 * 800),
            })),
            _ => Payload::Image(Arc::new(Pixels {
                width: 384,
                height: 256,
                rgba: [80, 180, 130, 255].repeat(384 * 256),
            })),
        }))
    }
    fn contact(&mut self, mid: &str) -> Result<crate::store::Friend> {
        self.rpc(FakeRpc::Contact(mid.into()))?;
        if let Some(contact) = self.0.lock().unwrap().contacts.get(mid).cloned() {
            return Ok(contact);
        }
        let state = self.0.lock().unwrap();
        state
            .friends
            .as_ref()
            .into_iter()
            .flatten()
            .find(|f| f.id == mid)
            .cloned()
            .or_else(|| {
                state
                    .members
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .find(|m| m.contact.id == mid)
                    .map(|m| m.contact.clone())
            })
            .or_else(|| {
                state.chats.iter().find(|c| c.id.0 == mid).map(|c| Friend {
                    profile: ContactProfile {
                        relation: Some(0),
                        ..Default::default()
                    },
                    id: mid.into(),
                    name: c.title.clone(),
                    avatar: c.avatar.clone(),
                    group: c.is_group,
                    member_count: c.member_count.map(|n| n as usize),
                    ..Default::default()
                })
            })
            .ok_or(Error::Decode)
    }

    fn notice(&mut self, _mid: &str) -> Result<Value> {
        self.online()?;
        Ok(json!({"notice":{"noticeAntiFraudGeneral":{}}}))
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value> {
        self.online()?;
        Ok(
            json!({"chatMid":chat.0,"chatName":self.0.lock().unwrap().chats.iter().find(|c| c.id == *chat).map(|c| &c.title),"notificationDisabled":false,"extra":{"groupExtra":{}}}),
        )
    }
    fn profile_action(&mut self, action: &ProfileAction, _full: Value, _seq: u32) -> Result<()> {
        self.rpc(FakeRpc::Action(action.clone()))?;
        match action {
            ProfileAction::Rename { mid, name } | ProfileAction::AddFriend { mid, chat: name } => {
                let mut f = self.contact(mid)?;
                if matches!(action, ProfileAction::Rename { .. }) {
                    let original = f
                        .profile
                        .original_name
                        .clone()
                        .unwrap_or_else(|| f.name.clone());
                    f.profile.relation = Some(0);
                    f.profile.original_name = Some(original);
                    f.profile.overridden_name = Some(name.clone());
                    f.name = name.clone();
                } else {
                    f.profile.relation = Some(0);
                }
                let mut rows = self.friends()?;
                rows.retain(|r| r.id != *mid);
                self.0
                    .lock()
                    .unwrap()
                    .contacts
                    .insert(f.id.clone(), f.clone());
                rows.push(f);
                self.0.lock().unwrap().friends = Some(rows);
            }
            ProfileAction::Mute { chat, disabled } => {
                if let Some(c) = self
                    .0
                    .lock()
                    .unwrap()
                    .chats
                    .iter_mut()
                    .find(|c| c.id == *chat)
                {
                    c.muted = *disabled;
                }
            }
            ProfileAction::Leave { chat } => {
                if let Some(c) = self
                    .0
                    .lock()
                    .unwrap()
                    .chats
                    .iter_mut()
                    .find(|c| c.id == *chat)
                {
                    c.left = true;
                }
            }
        }
        Ok(())
    }
    fn profile(&mut self) -> Result<Profile> {
        self.rpc(FakeRpc::Profile)?;
        Ok(Profile {
            mid: "self".into(),
            display_name: "Offline preview".into(),
            ..Default::default()
        })
    }
    fn chats(&mut self, _cursor: Option<String>) -> Result<ChatPage> {
        self.rpc(FakeRpc::Chats)?;
        let s = self.0.lock().unwrap();
        Ok(ChatPage {
            chats: s.chats.clone(),
            previews: s
                .pages
                .values()
                .filter_map(|rows| rows.last().cloned())
                .collect(),
            more: false,
            cursor: None,
        })
    }
    fn details(&mut self, chat: &ChatId) -> Result<Vec<ChatInput>> {
        self.rpc(FakeRpc::Details(chat.clone()))?;
        let mut s = self.0.lock().unwrap();
        if let Some(c) = s.chats.iter().find(|c| &c.id == chat) {
            return Ok(vec![c.clone()]);
        }
        let friend = s
            .friends
            .as_ref()
            .and_then(|rows| rows.iter().find(|f| f.id == chat.0))
            .cloned();
        if let Some(f) = friend {
            let c = ChatInput {
                id: chat.clone(),
                title: f.name,
                is_group: f.group,
                member_count: f.member_count.map(|n| n as u32),
                avatar: f.avatar,
                ..Default::default()
            };
            s.chats.push(c.clone());
            return Ok(vec![c]);
        }
        Ok(vec![])
    }
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page> {
        self.rpc(FakeRpc::Messages(chat.clone()))?;
        let mut state = self.0.lock().unwrap();
        *state.page_calls.entry(chat.clone()).or_default() += 1;
        let rows = state
            .pages
            .get(chat)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                let cursor = Cursor {
                    sort_ts: m.created_ms,
                    sort_id: format!("{:0>20}", m.server_id.0),
                };
                match kind {
                    PageKind::Recent => true,
                    PageKind::Older { before } => cursor < *before,
                    PageKind::Newer { after } => cursor > *after,
                }
            })
            .collect();
        Ok(Page {
            kind: kind.clone(),
            rows,
            reaches_start: !matches!(kind, PageKind::Newer { .. }),
        })
    }
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput> {
        self.online()?;
        let (row, echo, revision, deliver) = {
            let mut s = self.0.lock().unwrap();
            if std::mem::take(&mut s.uncertain_send) {
                return Err(Error::Offline(NetCause::Unreachable));
            }
            if std::mem::take(&mut s.fail_send) {
                return Err(Error::Rejected(RejectReason::SyntheticSend));
            }
            s.next_id += 1;
            let mut row = Self::message(&chat.0, s.next_id, text, true);
            row.request_seq = Some(seq);
            s.pages.entry(chat.clone()).or_default().push(row.clone());
            s.next_revision += 1;
            (row, s.echo_first, s.next_revision, s.deliver.clone())
        };
        if echo && let Some(d) = deliver {
            d(StreamEvent::Operation(LiveOp {
                revision,
                replay: false,
                op: Op::Message(
                    ({
                        let mut echo = row.clone();
                        echo.request_seq = None;
                        echo
                    })
                    .into(),
                ),
            }));
        }
        Ok(row)
    }
    fn unsend(&mut self, id: &ServerId, _seq: u32) -> Result<()> {
        self.rpc(FakeRpc::Unsend(id.clone()))?;
        let mut s = self.0.lock().unwrap();
        for row in s
            .pages
            .values_mut()
            .flatten()
            .filter(|m| &m.server_id == id)
        {
            row.body = Content::Withdrawn;
        }
        Ok(())
    }
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>> {
        self.rpc(FakeRpc::Receipts(chat.clone()))?;
        Ok(self
            .0
            .lock()
            .unwrap()
            .receipts
            .get(chat)
            .cloned()
            .map(|id| vec![Some(id)])
            .unwrap_or_default())
    }
    fn mark_read(&mut self, chat: &ChatId, _id: &ServerId, _seq: u32) -> Result<()> {
        self.rpc(FakeRpc::Read(chat.clone()))
    }
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<AtomicBool>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        let mut s = self.0.lock().unwrap();
        s.revisions.push(revision);
        s.deliver = Some(deliver.clone());
        drop(s);
        Ok(Box::new(move || {
            if !deliver(StreamEvent::Connection(Ok(()))) {
                return;
            }
            while !stop.load(Ordering::Relaxed) {
                std::thread::park_timeout(Duration::from_millis(20));
            }
        }))
    }
    fn decode(&mut self, raw: Value) -> Result<LiveOp> {
        Ok(LiveOp {
            revision: raw["revision"].as_i64().ok_or(Error::Decode)?,
            replay: raw["replay"].as_bool().unwrap_or(false),
            op: match raw["type"].as_u64() {
                Some(139 | 140) => reaction_operation(&raw, "self")?,
                Some(64 | 65) => Op::Withdraw(ServerId(
                    raw["param2"].as_str().ok_or(Error::Decode)?.into(),
                )),
                _ => Op::Checkpoint,
            },
        })
    }
    fn refresh_token(&mut self, _stop: &AtomicBool) -> Result<()> {
        Ok(())
    }
}

pub struct LineBackend {
    cache: std::path::PathBuf,
    fetchers: Option<Arc<crate::client::desktop::assets::Fetchers>>,
    client: crate::client::desktop::Client,
    state: std::path::PathBuf,
    wasm: std::path::PathBuf,
}
impl LineBackend {
    pub fn local_mid(&self) -> Option<String> {
        let mid = self.client.local_mid();
        (!mid.is_empty()).then(|| mid.to_owned())
    }
    pub fn open_local(
        state: &std::path::Path,
        wasm: &std::path::Path,
        cache: &std::path::Path,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            fetchers: None,
            cache: cache.into(),
            client: crate::client::desktop::Client::open_local(state, wasm, cache)?,
            state: state.into(),
            wasm: wasm.into(),
        })
    }
}
fn rpc(error: anyhow::Error) -> Error {
    crate::client::error::classify(&error)
}
fn message_input(chat: &ChatId, m: crate::client::desktop::Message, rev: i64) -> MessageInput {
    MessageInput {
        server_id: ServerId(m.id.clone()),
        chat_id: chat.clone(),
        created_ms: m.timestamp_ms,
        delivered_ms: Some(m.delivered_time),
        sender_id: m.sender_id.clone(),
        outgoing: m.outgoing,

        reply: m.reply.map(Box::new),
        mentioned: m.mentioned,
        skip_badge: m.skip_badge,
        request_seq: m.request_seq,
        reactions: m.reactions,
        body: m.content,
        reactions_rev: rev,
    }
}
fn chat_input(c: crate::client::desktop::Chat) -> ChatInput {
    ChatInput {
        id: ChatId(c.id),
        is_group: c.group,
        title: c.title,
        avatar: c.avatar,
        member_count: c.member_count.map(|n| n as u32),
        muted: c.muted.unwrap_or(false),
        left: false,
        last_message_id: c.last_message_id.map(ServerId),
        last_message_ms: Some(c.timestamp_ms),
        unread_count: c.unread.unwrap_or(0) as u32,
        unread_stale: false,
        last_read_server_id: c.last_read_message.map(ServerId),
        list_rank: 0,
    }
}
impl Backend for LineBackend {
    fn sign_in(&mut self, cancel: Arc<AtomicBool>, deliver: PairDeliver) -> Result<()> {
        use crate::client::{
            desktop::{self, assets::Pixels},
            pairing,
        };
        desktop::set_cancellation_token(cancel.clone());
        pairing::sign_in(&self.state, &self.wasm, &cancel, move |event| {
            deliver(match event {
                pairing::Event::Stage(stage) => PairEvent::Stage(stage),
                pairing::Event::Qr {
                    width,
                    height,
                    rgba,
                } => PairEvent::Qr(Arc::new(Pixels {
                    width,
                    height,
                    rgba,
                })),
                pairing::Event::Pin(pin) => PairEvent::Pin(pin),
                pairing::Event::Scanned => PairEvent::Scanned,
            })
        })
        .map_err(rpc)?;
        self.client =
            desktop::Client::open_local(&self.state, &self.wasm, &self.cache).map_err(rpc)?;
        Ok(())
    }

    fn friends(&mut self) -> Result<Vec<crate::store::Friend>> {
        self.client.friends().map_err(rpc)
    }
    fn members(&mut self, chat: &ChatId) -> Result<Vec<crate::store::Member>> {
        self.client.members(&chat.0).map(|(m, _)| m).map_err(rpc)
    }
    fn start_assets(&mut self, stop: Arc<AtomicBool>, deliver: Deliver) {
        let auth = deliver.clone();
        self.fetchers = Some(Arc::new(self.client.asset_fetchers(
            stop,
            Arc::new(Default::default()),
            Default::default(),
            move |asset, cancel| {
                let (reply, rx) = std::sync::mpsc::sync_channel(1);
                if !auth(StreamEvent::Authorize {
                    asset: asset.clone(),
                    reply,
                }) {
                    return Err(crate::client::desktop::assets::Failure::Cancelled);
                }
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(crate::client::desktop::assets::Failure::Cancelled);
                    }
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(r) => return r,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => return Err(crate::client::desktop::assets::Failure::Cancelled),
                    }
                }
            },
            {
                let deliver = deliver.clone();
                move |completion, _| {
                    deliver(StreamEvent::Asset(completion));
                }
            },
        )));
        let requests = self.fetchers.as_ref().unwrap().clone();
        let retry = requests.clone();
        deliver(StreamEvent::AssetRequests(AssetRequests {
            request: Arc::new(move |asset, purpose| {
                requests.enqueue(asset, None, purpose);
            }),
            retry: Arc::new(move |key| {
                retry.retry(&key);
            }),
        }));
    }
    fn authorize(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
    ) -> std::result::Result<Option<String>, crate::client::desktop::assets::Failure> {
        self.client
            .media_token(asset)
            .map_err(|e| crate::client::desktop::assets::Failure::from_error(&e))
    }
    fn retry_asset(&mut self, key: &str) {
        self.fetchers.as_ref().unwrap().retry(key);
    }
    fn media(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        self.fetchers
            .as_ref()
            .unwrap()
            .enqueue(asset.clone(), None, purpose);
        Ok(None)
    }
    fn contact(&mut self, mid: &str) -> Result<crate::store::Friend> {
        self.client.contact(mid).map_err(rpc)
    }
    fn notice(&mut self, mid: &str) -> Result<Value> {
        self.client.profile_notice(mid).map_err(rpc)
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value> {
        self.client.full_chat(&chat.0).map_err(rpc)
    }
    fn profile_action(&mut self, action: &ProfileAction, _full: Value, seq: u32) -> Result<()> {
        match action {
            ProfileAction::Rename { mid, name } => self.client.rename_contact(mid, name, seq),
            ProfileAction::AddFriend { mid, chat } => self.client.add_friend(mid, chat, seq),
            ProfileAction::Mute { chat, disabled } => {
                self.client.mute_chat(&chat.0, *disabled, seq)
            }
            ProfileAction::Leave { chat } => self.client.leave_chat(&chat.0, seq),
        }
        .map_err(rpc)
    }
    fn profile(&mut self) -> Result<Profile> {
        let data = self.client.online_profile().map_err(rpc)?;
        serde_json::from_value(data).map_err(|_| Error::Decode)
    }
    fn chats(&mut self, cursor: Option<String>) -> Result<ChatPage> {
        let (rows, more, cursor) = self.client.chats(cursor.as_deref()).map_err(rpc)?;
        let previews = rows
            .iter()
            .filter_map(|c| {
                c.preview_message
                    .clone()
                    .map(|m| message_input(&ChatId(c.id.clone()), m, 0))
            })
            .collect();
        Ok(ChatPage {
            chats: rows.into_iter().map(chat_input).collect(),
            previews,
            more,
            cursor,
        })
    }

    fn details(&mut self, chat: &ChatId) -> Result<Vec<ChatInput>> {
        Ok(self
            .client
            .chat_detail(&chat.0)
            .map_err(rpc)?
            .into_iter()
            .map(chat_input)
            .collect())
    }
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page> {
        let (rows, reaches_start) = match kind {
            PageKind::Older { before } => self
                .client
                .older(
                    &chat.0,
                    before.sort_id.trim_start_matches('0'),
                    before.sort_ts,
                )
                .map_err(rpc)?,
            _ => {
                let rows = self.client.refresh_messages(&chat.0).map_err(rpc)?;
                let start = rows.len() < 50;
                (rows, start)
            }
        };
        let actual_kind = if matches!(kind, PageKind::Newer { .. }) {
            PageKind::Recent
        } else {
            kind.clone()
        };
        Ok(Page {
            kind: actual_kind,
            rows: rows
                .into_iter()
                .map(|m| message_input(chat, m, 0))
                .collect(),
            reaches_start,
        })
    }
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput> {
        self.client
            .send_text_with_request(&chat.0, text, seq)
            .map_err(|e| {
                if e.downcast_ref::<Error>().is_some()
                    || e.downcast_ref::<crate::client::RpcError>()
                        .is_some_and(|e| {
                            matches!(e.http, 401 | 403) || (500..600).contains(&e.http)
                        })
                {
                    return rpc(e);
                }
                match crate::client::desktop::send_failure(&e) {
                    crate::client::desktop::SendFailure::Rejected => {
                        Error::Rejected(RejectReason::NotAccepted)
                    }
                    _ => rpc(e),
                }
            })
            .map(|m| message_input(chat, m, 0))
    }
    fn unsend(&mut self, id: &ServerId, seq: u32) -> Result<()> {
        self.client.unsend(&id.0, seq).map_err(rpc)
    }
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>> {
        Ok(self
            .client
            .read_ranges(&chat.0)
            .map_err(rpc)?
            .into_iter()
            .map(|r| (!r.open_end).then_some(ServerId(r.end)))
            .collect())
    }
    fn mark_read(&mut self, chat: &ChatId, id: &ServerId, seq: u32) -> Result<()> {
        self.client.mark_read(&chat.0, &id.0, seq).map_err(rpc)
    }
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<AtomicBool>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        let replay_through = self.client.latest_revision().map_err(rpc)?;
        let access = self.client.stream_access();
        Ok(Box::new(move || {
            use crate::client::desktop::sync::{self, Event, SyncState};
            let refresh_deliver = deliver.clone();
            let _ = sync::receive_with_auth(
                revision as u64,
                &stop,
                |renew| {
                    if renew {
                        let (reply, rx) = std::sync::mpsc::sync_channel(1);
                        if !refresh_deliver(StreamEvent::RefreshToken { reply }) {
                            return Err(Error::Auth.into());
                        }
                        loop {
                            match rx.recv_timeout(Duration::from_millis(100)) {
                                Ok(result) => {
                                    result?;
                                    break;
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                                    if !stop.load(Ordering::Relaxed) => {}
                                Err(_) => return Err(Error::Auth.into()),
                            }
                        }
                    }
                    Ok(access.read().unwrap().clone())
                },
                |e| {
                    deliver(match e {
                        Event::Operation(mut v) => {
                            let rev = v["revision"]
                                .as_i64()
                                .or_else(|| v["revision"].as_str()?.parse().ok());
                            v["replay"] = rev.is_some_and(|r| r <= replay_through).into();
                            StreamEvent::Raw(v)
                        }
                        Event::State(s) => match s {
                            SyncState::Connected => StreamEvent::Connection(Ok(())),
                            SyncState::AuthExpired => StreamEvent::Connection(Err(Error::Auth)),
                            SyncState::Reconnecting { attempt } => StreamEvent::Connecting(attempt),
                        },
                        Event::Failure { attempt, error } => StreamEvent::Retry { attempt, error },
                        Event::Resync {
                            next_revision,
                            complete,
                        } => StreamEvent::Resync {
                            revision: next_revision.and_then(|s| s.parse().ok()),
                            complete,
                        },
                    })
                },
            );
        }))
    }
    fn decode(&mut self, raw: Value) -> Result<LiveOp> {
        let revision = raw["revision"]
            .as_i64()
            .or_else(|| raw["revision"].as_str()?.parse().ok())
            .ok_or(Error::Decode)?;
        let chat = || ChatId(raw["param1"].as_str().unwrap_or_default().into());
        let id = || ServerId(raw["param2"].as_str().unwrap_or_default().into());
        let op = match raw["type"].as_u64() {
            Some(25 | 26 | 43) => {
                let (c, mut m) = self.client.operation(&raw).ok_or(Error::Decode)?;
                m.request_seq = raw["reqSeq"]
                    .as_u64()
                    .or_else(|| raw["reqSeq"].as_str()?.parse().ok())
                    .map(|n| n as u32)
                    .or(m.request_seq);
                Op::Message((message_input(&ChatId(c), m, revision)).into())
            }
            Some(55) => Op::Receipt {
                chat: chat(),
                up_to: ServerId(raw["param3"].as_str().ok_or(Error::Decode)?.into()),
            },
            Some(40) => Op::Read {
                chat: chat(),
                up_to: id(),
            },
            Some(64 | 65) => Op::Withdraw(id()),
            Some(139 | 140) => reaction_operation(&raw, self.client.local_mid())?,
            _ => Op::Checkpoint,
        };
        Ok(LiveOp {
            revision,
            replay: raw["replay"].as_bool().unwrap_or(false),
            op,
        })
    }
    fn refresh_token(&mut self, stop: &AtomicBool) -> Result<()> {
        self.client.refresh_token(stop).map_err(rpc)
    }
}

fn reaction_operation(raw: &Value, own_mid: &str) -> Result<Op> {
    let data: Value = serde_json::from_str(raw["param2"].as_str().ok_or(Error::Decode)?)
        .map_err(|_| Error::Decode)?;
    let chat = ChatId(
        data["chatMid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::Decode)?
            .into(),
    );
    let actor = if raw["type"] == 139 {
        Some(own_mid)
    } else {
        data["fromUserMid"]
            .as_str()
            .or_else(|| raw["fromUserMid"].as_str())
            .or_else(|| data["actorMid"].as_str())
            .or_else(|| raw["actorMid"].as_str())
            .or_else(|| {
                raw["param3"]
                    .as_str()
                    .filter(|s| (s.starts_with('u') && s.len() == 33) || s.len() == 44)
            })
    };
    match actor.filter(|s| !s.is_empty()) {
        Some(actor) => Ok(Op::Reaction {
            id: ServerId(raw["param1"].as_str().ok_or(Error::Decode)?.into()),
            actor: actor.into(),
            curr: crate::client::desktop::reaction_kind(&data["curr"]),
        }),
        None => Ok(Op::Recent(chat)),
    }
}
