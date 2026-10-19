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
    Stage(crate::client::pairing::Stage),
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
pub type FinishUpload = Box<dyn FnOnce(&mut dyn Backend) -> Result<MessageInput> + Send>;
pub type UploadJob = Box<dyn FnOnce(&mut dyn FnMut(f32)) -> Result<FinishUpload> + Send>;
/// Each stream invocation runs on its own thread. Returning is supervised by the engine.
pub trait Backend: Send {
    fn has_session(&self) -> bool {
        true
    }
    fn session(&self) -> Result<bool> {
        Ok(self.has_session())
    }
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
    /// Cache-only access; must never authorize or download an asset.
    fn cached_media(
        &mut self,
        _mid: &str,
        _asset: &crate::client::desktop::assets::Asset,
        _purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        Ok(None)
    }
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
    fn prepare_attachment(
        &mut self,
        _chat: ChatId,
        _item: crate::attachments::Attachment,
        _seq: u32,
    ) -> Result<UploadJob> {
        Err(Error::Protocol)
    }
    fn finish_attachment(
        &mut self,
        _chat: &ChatId,
        _uploaded: Value,
        _seq: u32,
    ) -> Result<MessageInput> {
        Err(Error::Protocol)
    }
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput>;
    fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()>;
    fn sticker_packages(&mut self) -> Result<Vec<StickerPackage>>;
    fn send_sticker(&mut self, chat: &ChatId, sticker: &Content, seq: u32) -> Result<MessageInput>;
    fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput>;
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
    React(ServerId, Option<u32>),
    StickerPackages,
    CreateChat,
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
    signed_out: bool,
    open_failure: Option<crate::client::error::SignInFailure>,
    sign_in_failure: Option<crate::client::error::SignInFailure>,
    fail_send: bool,
    upload_delay: Duration,
    send_gate: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    upload_gate: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    upload_failures: std::collections::BTreeSet<String>,
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
    /// An account with no saved session or cached account data.
    pub fn signed_out() -> Self {
        Self(Arc::new(Mutex::new(FakeState {
            signed_out: true,
            ..Default::default()
        })))
    }
    /// Retryable local initialization failure, before a backend exists.
    pub fn fail_open(&self, reason: Option<crate::client::error::SignInFailure>) {
        self.0.lock().unwrap().open_failure = reason;
    }
    pub fn open(&self) -> anyhow::Result<Self> {
        if let Some(reason) = self.0.lock().unwrap().open_failure.clone()
            && !matches!(reason, crate::client::error::SignInFailure::SavedSession(_))
        {
            return Err(Error::SignIn(reason).into());
        }
        Ok(self.clone())
    }
    pub fn block_send(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, wait) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        self.0.lock().unwrap().send_gate = Some((entered, blocked));
        (wait, release)
    }
    pub fn block_upload(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, wait) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        self.0.lock().unwrap().upload_gate = Some((entered, blocked));
        (wait, release)
    }
    pub fn upload_delay(&self, delay: Duration) {
        self.0.lock().unwrap().upload_delay = delay;
    }
    pub fn fail_upload(&self, name: &str) {
        self.0.lock().unwrap().upload_failures.insert(name.into());
    }

    fn fake_send(&mut self, chat: &ChatId, body: Content, seq: u32) -> Result<MessageInput> {
        let gate = self.0.lock().unwrap().send_gate.take();
        if let Some((entered, blocked)) = gate {
            let _ = entered.send(());
            let _ = blocked.recv();
        }
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
            let mut row = Self::message(&chat.0, s.next_id, "", true);
            row.body = body;
            row.request_seq = Some(seq);
            s.pages.entry(chat.clone()).or_default().push(row.clone());
            s.next_revision += 1;
            (row, s.echo_first, s.next_revision, s.deliver.clone())
        };
        if echo
            && !matches!(row.body, Content::Photo { .. } | Content::File { .. })
            && let Some(d) = deliver
        {
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
    pub fn demo_sticker_packages() -> Vec<StickerPackage> {
        (0..2)
            .map(|p| StickerPackage {
                id: format!("demo-{p}"),
                name: ["Little shapes", "Happy shapes"][p].into(),
                version: "1".into(),
                stickers: (0..12)
                    .map(|n| Content::Sticker {
                        pkg: format!("demo-{p}"),
                        id: format!("9900{p}{n:02}"),
                        ver: Some("1".into()),
                        asset: crate::client::desktop::assets::Asset::sticker(
                            &json!({"STKID":format!("9900{p}{n:02}")}),
                        ),
                        text: Some("Sticker".into()),
                    })
                    .collect(),
            })
            .collect()
    }
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
    pub fn fail_sign_in(&self, reason: crate::client::error::SignInFailure) {
        self.0.lock().unwrap().sign_in_failure = Some(reason);
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
    fn has_session(&self) -> bool {
        !self.0.lock().unwrap().signed_out
    }
    fn session(&self) -> Result<bool> {
        if let Some(reason) = self.0.lock().unwrap().open_failure.clone() {
            return Err(Error::SignIn(reason));
        }
        Ok(self.has_session())
    }
    fn sign_in(&mut self, cancel: Arc<AtomicBool>, deliver: PairDeliver) -> Result<()> {
        use crate::client::{desktop::assets::Pixels, pairing::Stage};
        if let Some(reason) = self.0.lock().unwrap().sign_in_failure.take() {
            return Err(Error::SignIn(reason));
        }
        deliver(PairEvent::Stage(Stage::Synthetic));
        if self.0.lock().unwrap().signed_out {
            let qr =
                qrcodegen::QrCode::encode_text("offline-demo-sign-in", qrcodegen::QrCodeEcc::Low)
                    .expect("synthetic QR");
            let width = (qr.size() + 8) as u32;
            let mut rgba = Vec::with_capacity((width * width * 4) as usize);
            for y in 0..width as i32 {
                for x in 0..width as i32 {
                    let shade = if qr.get_module(x - 4, y - 4) { 0 } else { 255 };
                    rgba.extend_from_slice(&[shade, shade, shade, 255]);
                }
            }
            deliver(PairEvent::Qr(Arc::new(Pixels {
                width,
                height: width,
                rgba,
            })));
            deliver(PairEvent::Stage(Stage::AwaitScan));
            while !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Err(Error::SignIn(crate::client::error::SignInFailure::Server))
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
    fn cached_media(
        &mut self,
        _mid: &str,
        asset: &crate::client::desktop::assets::Asset,
        _purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .pixels
            .get(&asset.key())
            .cloned()
            .map(crate::client::desktop::assets::Payload::Image))
    }
    fn media(
        &mut self,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        use crate::client::desktop::assets::{Payload, Pixels, Purpose};
        if let Some(pixels) = asset.local_pixels().map_err(|_| Error::Decode)? {
            return Ok(Some(Payload::Image(Arc::new(pixels))));
        }

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
        if self.0.lock().unwrap().signed_out {
            return Err(Error::Auth);
        }
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
    fn prepare_attachment(
        &mut self,
        chat: ChatId,
        item: crate::attachments::Attachment,
        seq: u32,
    ) -> Result<UploadJob> {
        let mut fake = self.clone();
        Ok(Box::new(move |progress| {
            item.read(crate::attachments::FILE_LIMIT).map_err(|_| {
                Error::Rejected(if item.path.is_file() {
                    RejectReason::NotAccepted
                } else {
                    RejectReason::FileMissing
                })
            })?;
            fake.online()?;
            let gate = fake.0.lock().unwrap().upload_gate.take();
            if let Some((entered, blocked)) = gate {
                let _ = entered.send(());
                let _ = blocked.recv();
            }
            let delay = fake.0.lock().unwrap().upload_delay;
            for tick in 0..=10 {
                progress(tick as f32 / 10.);
                std::thread::sleep(delay / 11);
            }
            if fake.0.lock().unwrap().upload_failures.remove(&item.name) {
                return Err(Error::Rejected(RejectReason::SyntheticSend));
            }
            Ok(Box::new(move |_backend: &mut dyn Backend| {
                let row = fake.fake_send(&chat, item.content(), seq)?;
                // SEND_MESSAGE carries the persisted request identity; SEND_CONTENT repeats
                // the same server identity after storage, without a matching request seq.
                let (deliver, revision) = {
                    let mut state = fake.0.lock().unwrap();
                    state.next_revision += 2;
                    (state.deliver.clone(), state.next_revision)
                };
                if let Some(deliver) = deliver {
                    deliver(StreamEvent::Operation(LiveOp {
                        revision: revision - 1,
                        replay: false,
                        op: Op::Message(row.clone().into()),
                    }));
                    let mut content_echo = row.clone();
                    content_echo.request_seq = Some(0);
                    deliver(StreamEvent::Operation(LiveOp {
                        revision,
                        replay: false,
                        op: Op::Message(content_echo.into()),
                    }));
                }
                Ok(row)
            }) as FinishUpload)
        }))
    }
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput> {
        self.fake_send(chat, Content::text(text), seq)
    }
    fn send_sticker(&mut self, chat: &ChatId, sticker: &Content, seq: u32) -> Result<MessageInput> {
        self.fake_send(chat, sticker.clone(), seq)
    }
    fn sticker_packages(&mut self) -> Result<Vec<StickerPackage>> {
        self.rpc(FakeRpc::StickerPackages)?;
        let packages = Self::demo_sticker_packages();
        for (p, package) in packages.iter().enumerate() {
            for (i, sticker) in package.stickers.iter().enumerate() {
                let mut rgba = vec![0; 96 * 96 * 4];
                for y in 0..96i32 {
                    for x in 0..96i32 {
                        let dx = x - 48;
                        let dy = y - 48;
                        if dx * dx + dy * dy < (32 + (i % 3) as i32 * 3).pow(2) {
                            let color = if ((x - 36).abs() < 3 || (x - 60).abs() < 3)
                                && (y - 38).abs() < 5
                                || (y - 57).abs() < 2 && (x - 48).abs() < 13
                            {
                                [45, 55, 65, 255]
                            } else if p == 0 {
                                [245, (170 + i * 5) as u8, 92, 255]
                            } else {
                                [115, (180 + i * 3) as u8, 220, 255]
                            };
                            let offset = ((y * 96 + x) * 4) as usize;
                            rgba[offset..offset + 4].copy_from_slice(&color);
                        }
                    }
                }
                self.set_pixels(
                    sticker.image().unwrap(),
                    crate::client::desktop::assets::Pixels {
                        width: 96,
                        height: 96,
                        rgba,
                    },
                );
            }
        }
        Ok(packages)
    }
    fn react(&mut self, id: &ServerId, kind: Option<u32>, _seq: u32) -> Result<()> {
        self.rpc(FakeRpc::React(id.clone(), kind))?;
        let mut s = self.0.lock().unwrap();
        for row in s
            .pages
            .values_mut()
            .flatten()
            .filter(|m| &m.server_id == id)
        {
            row.reactions.retain(|r| r.actor != "self");
            if let Some(kind) = kind {
                row.reactions.push(Reaction {
                    actor: "self".into(),
                    kind: ReactionKind::Predefined(kind),
                });
            }
        }
        s.next_revision += 1;
        if let Some(deliver) = &s.deliver {
            deliver(StreamEvent::Operation(LiveOp {
                revision: s.next_revision,
                replay: false,
                op: Op::Reaction {
                    id: id.clone(),
                    actor: "self".into(),
                    curr: kind.map(ReactionKind::Predefined),
                },
            }));
        }
        Ok(())
    }
    fn create_chat(&mut self, name: &str, targets: &[String], _seq: u32) -> Result<ChatInput> {
        self.rpc(FakeRpc::CreateChat)?;
        let mut s = self.0.lock().unwrap();
        s.next_id += 1;
        let row = ChatInput {
            id: ChatId(format!("group-{}", s.next_id)),
            title: name.into(),
            is_group: true,
            member_count: Some(targets.len() as u32 + 1),
            ..Default::default()
        };
        s.chats.push(row.clone());
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
    session_failure: Option<crate::client::error::SignInFailure>,
    fetchers: Option<Arc<crate::client::desktop::assets::Fetchers>>,
    client: Option<crate::client::desktop::Client>,
    state: std::path::PathBuf,
    wasm_override: Option<std::path::PathBuf>,
    version_override: Option<String>,
}
impl LineBackend {
    pub fn local_mid(&self) -> Option<String> {
        self.client
            .as_ref()
            .map(|client| client.local_mid().to_owned())
    }
    pub fn open_local(
        state: &std::path::Path,
        wasm_override: Option<std::path::PathBuf>,
        cache: &std::path::Path,
        version_override: Option<String>,
    ) -> anyhow::Result<Self> {
        let open = || -> anyhow::Result<Option<crate::client::desktop::Client>> {
            let module =
                crate::crypto_module::resolve(wasm_override.clone(), version_override.clone())?;
            if state.try_exists()? {
                crate::client::desktop::Client::open_local(state, module, cache).map(Some)
            } else {
                drop(module.runtime(&module.read()?)?);
                Ok(None)
            }
        };
        let (client, session_failure) = match open() {
            Ok(client) => (client, None),
            Err(error) => {
                crate::reporting::log("saved-session", "open", error.as_ref());
                let reason = match crate::client::error::classify(&error) {
                    Error::SignIn(reason) => reason,
                    _ => crate::client::error::SignInFailure::SavedSession(
                        crate::client::error::SavedSessionFailure::Corrupt,
                    ),
                };
                (None, Some(reason))
            }
        };
        Ok(Self {
            fetchers: None,
            cache: cache.into(),
            client,
            session_failure,
            state: state.into(),
            wasm_override,
            version_override,
        })
    }
}
fn rpc(error: anyhow::Error) -> Error {
    crate::reporting::log("backend-rpc", "source", error.as_ref());
    crate::client::error::classify(&error)
}
fn send_rpc(e: anyhow::Error) -> Error {
    crate::reporting::log("send", "source", e.as_ref());
    if e.downcast_ref::<Error>().is_some()
        || e.downcast_ref::<crate::client::RpcError>()
            .is_some_and(|e| matches!(e.http, 401 | 403) || (500..600).contains(&e.http))
    {
        return rpc(e);
    }
    match crate::client::desktop::send_failure(&e) {
        crate::client::desktop::SendFailure::Rejected => Error::Rejected(RejectReason::NotAccepted),
        _ => rpc(e),
    }
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
    fn has_session(&self) -> bool {
        self.client.is_some()
    }
    fn session(&self) -> Result<bool> {
        match &self.session_failure {
            Some(reason) => Err(Error::SignIn(reason.clone())),
            None => Ok(self.has_session()),
        }
    }
    fn sign_in(&mut self, cancel: Arc<AtomicBool>, deliver: PairDeliver) -> Result<()> {
        use crate::client::{
            desktop::{self, assets::Pixels},
            pairing,
        };
        desktop::set_cancellation_token(cancel.clone());
        let module = crate::crypto_module::resolve(
            self.wasm_override.clone(),
            self.version_override.clone(),
        )
        .map_err(rpc)?;
        self.client = None;
        let mut client = pairing::sign_in_validated(
            &self.state,
            module,
            &cancel,
            move |event| {
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
            },
            |staged, api, wasm| desktop::Client::from_api(staged, &self.cache, wasm, api),
        )
        .map_err(|error| {
            crate::reporting::log("sign-in", "pairing", error.as_ref());
            Error::SignIn(pairing::failure(&error, cancel.load(Ordering::Relaxed)))
        })?;
        client.installed_at(&self.state);
        self.client = Some(client);
        self.session_failure = None;
        Ok(())
    }

    fn friends(&mut self) -> Result<Vec<crate::store::Friend>> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .friends()
            .map_err(rpc)
    }
    fn members(&mut self, chat: &ChatId) -> Result<Vec<crate::store::Member>> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .members(&chat.0)
            .map(|(m, _)| m)
            .map_err(rpc)
    }
    fn start_assets(&mut self, stop: Arc<AtomicBool>, deliver: Deliver) {
        let auth = deliver.clone();
        let Some(client) = self.client.as_mut() else {
            return;
        };
        self.fetchers = Some(Arc::new(client.asset_fetchers(
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
            .as_mut()
            .ok_or(crate::client::desktop::assets::Failure::Cancelled)?
            .media_token(asset)
            .map_err(|e| crate::client::desktop::assets::Failure::from_error(&e))
    }
    fn retry_asset(&mut self, key: &str) {
        self.fetchers.as_ref().unwrap().retry(key);
    }
    fn cached_media(
        &mut self,
        mid: &str,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>> {
        use crate::client::desktop::assets::{ImageCache, Payload, Purpose};
        if mid.is_empty() {
            return Ok(None);
        }
        let asset = match purpose {
            Purpose::Thumbnail => asset.clone(),
            Purpose::Original { .. } => match asset.original() {
                Some(asset) => asset,
                None => return Ok(None),
            },
            Purpose::File { .. } => return Ok(None),
        };
        ImageCache::new(&self.cache, mid)
            .and_then(|mut cache| cache.read(&asset))
            .map(|pixels| pixels.map(Payload::Image))
            .map_err(rpc)
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
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .contact(mid)
            .map_err(rpc)
    }
    fn notice(&mut self, mid: &str) -> Result<Value> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .profile_notice(mid)
            .map_err(rpc)
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .full_chat(&chat.0)
            .map_err(rpc)
    }
    fn profile_action(&mut self, action: &ProfileAction, _full: Value, seq: u32) -> Result<()> {
        match action {
            ProfileAction::Rename { mid, name } => self
                .client
                .as_mut()
                .ok_or(Error::Auth)?
                .rename_contact(mid, name, seq),
            ProfileAction::AddFriend { mid, chat } => self
                .client
                .as_mut()
                .ok_or(Error::Auth)?
                .add_friend(mid, chat, seq),
            ProfileAction::Mute { chat, disabled } => self
                .client
                .as_mut()
                .ok_or(Error::Auth)?
                .mute_chat(&chat.0, *disabled, seq),
            ProfileAction::Leave { chat } => self
                .client
                .as_mut()
                .ok_or(Error::Auth)?
                .leave_chat(&chat.0, seq),
        }
        .map_err(rpc)
    }
    fn profile(&mut self) -> Result<Profile> {
        let data = self
            .client
            .as_mut()
            .ok_or(Error::Auth)?
            .online_profile()
            .map_err(rpc)?;
        serde_json::from_value(data).map_err(|_| Error::Decode)
    }
    fn chats(&mut self, cursor: Option<String>) -> Result<ChatPage> {
        let (rows, more, cursor) = self
            .client
            .as_mut()
            .ok_or(Error::Auth)?
            .chats(cursor.as_deref())
            .map_err(rpc)?;
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
            .as_mut()
            .ok_or(Error::Auth)?
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
                .as_mut()
                .ok_or(Error::Auth)?
                .older(
                    &chat.0,
                    before.sort_id.trim_start_matches('0'),
                    before.sort_ts,
                )
                .map_err(rpc)?,
            _ => {
                let rows = self
                    .client
                    .as_mut()
                    .ok_or(Error::Auth)?
                    .refresh_messages(&chat.0)
                    .map_err(rpc)?;
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
    fn prepare_attachment(
        &mut self,
        chat: ChatId,
        item: crate::attachments::Attachment,
        seq: u32,
    ) -> Result<UploadJob> {
        if !item.path.is_file() {
            return Err(Error::Rejected(RejectReason::FileMissing));
        }
        let upload = self
            .client
            .as_mut()
            .ok_or(Error::Auth)?
            .prepare_attachment(&chat.0, item, seq)
            .map_err(|error| match rpc(error) {
                e @ (Error::Auth | Error::SessionInvalid | Error::Rejected(_)) => e,
                _ => Error::Rejected(RejectReason::UploadFailed),
            })?;
        Ok(Box::new(move |progress| {
            let uploaded = upload(progress).map_err(|e| match rpc(e) {
                e @ (Error::Auth | Error::SessionInvalid | Error::Rejected(_)) => e,
                _ => Error::Rejected(RejectReason::UploadFailed),
            })?;
            Ok(Box::new(move |backend: &mut dyn Backend| {
                backend.finish_attachment(&chat, uploaded, seq)
            }) as FinishUpload)
        }))
    }
    fn finish_attachment(
        &mut self,
        chat: &ChatId,
        uploaded: Value,
        seq: u32,
    ) -> Result<MessageInput> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .finish_attachment(&chat.0, uploaded, seq)
            .map_err(send_rpc)
            .map(|m| message_input(chat, m, 0))
    }
    fn send(&mut self, chat: &ChatId, text: &str, seq: u32) -> Result<MessageInput> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .send_text_with_request(&chat.0, text, seq)
            .map_err(send_rpc)
            .map(|m| message_input(chat, m, 0))
    }
    fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .react(&id.0, kind, seq)
            .map_err(rpc)
    }
    fn sticker_packages(&mut self) -> Result<Vec<StickerPackage>> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .sticker_packages()
            .map_err(rpc)
    }
    fn send_sticker(&mut self, chat: &ChatId, sticker: &Content, seq: u32) -> Result<MessageInput> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .send_sticker(&chat.0, sticker, seq)
            .map(|m| message_input(chat, m, 0))
            .map_err(send_rpc)
    }
    fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .create_chat(name, targets, seq)
            .map_err(rpc)
    }
    fn unsend(&mut self, id: &ServerId, seq: u32) -> Result<()> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .unsend(&id.0, seq)
            .map_err(rpc)
    }
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>> {
        Ok(self
            .client
            .as_mut()
            .ok_or(Error::Auth)?
            .read_ranges(&chat.0)
            .map_err(rpc)?
            .into_iter()
            .map(|r| (!r.open_end).then_some(ServerId(r.end)))
            .collect())
    }
    fn mark_read(&mut self, chat: &ChatId, id: &ServerId, seq: u32) -> Result<()> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .mark_read(&chat.0, &id.0, seq)
            .map_err(rpc)
    }
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<AtomicBool>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        let replay_through = self
            .client
            .as_mut()
            .ok_or(Error::Auth)?
            .latest_revision()
            .map_err(rpc)?;
        let client_version = self
            .client
            .as_ref()
            .ok_or(Error::Auth)?
            .client_version()
            .to_owned();
        let access = self.client.as_mut().ok_or(Error::Auth)?.stream_access();
        Ok(Box::new(move || {
            use crate::client::desktop::sync::{self, Event, SyncState};
            let refresh_deliver = deliver.clone();
            let _ = sync::receive_with_auth(
                &client_version,
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
                let (c, mut m) = self
                    .client
                    .as_mut()
                    .ok_or(Error::Auth)?
                    .operation(&raw)
                    .map_err(rpc)?
                    .ok_or(Error::Decode)?;
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
            Some(139 | 140) => {
                reaction_operation(&raw, self.client.as_mut().ok_or(Error::Auth)?.local_mid())?
            }
            _ => Op::Checkpoint,
        };
        Ok(LiveOp {
            revision,
            replay: raw["replay"].as_bool().unwrap_or(false),
            op,
        })
    }
    fn refresh_token(&mut self, stop: &AtomicBool) -> Result<()> {
        self.client
            .as_mut()
            .ok_or(Error::Auth)?
            .refresh_token(stop)
            .map_err(rpc)
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
