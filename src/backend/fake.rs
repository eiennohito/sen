use super::*;
use serde_json::json;
use std::{sync::Mutex, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeRpc {
    Send(ChatId),
    Friends,
    SignIn,
    Upload(String),
    Stream(i64),
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
pub struct FakeConfig {
    pub members: Option<Vec<crate::store::Member>>,
    pub chats: Vec<ChatInput>,
    pub upload_delay: Duration,
    pub revision: i64,
}
#[derive(Default)]
struct FakeState {
    config: FakeConfig,
    calls: Vec<FakeRpc>,
    failures: Vec<(FakeRpc, Failure)>,
    friends: Option<Vec<crate::store::Friend>>,
    contacts: std::collections::BTreeMap<String, Friend>,
    offline: bool,
    signed_out: bool,
    sign_in_failure: Option<crate::client::error::SignInFailure>,
    open_failure: Option<crate::client::error::SignInFailure>,
    send_gate: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    upload_gate: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    receipts: std::collections::BTreeMap<ChatId, ServerId>,
    pixels: std::collections::BTreeMap<String, Arc<crate::client::desktop::assets::Pixels>>,
    echo_first: bool,
    next_id: i64,
    pages: std::collections::BTreeMap<ChatId, Vec<MessageInput>>,
    deliver: Option<Deliver>,
    assets: Option<Deliver>,
    failed_assets: std::collections::BTreeMap<String, crate::client::desktop::assets::Purpose>,
    asset_failures: std::collections::BTreeMap<String, crate::client::desktop::assets::Failure>,
    slow_assets: std::collections::BTreeMap<String, Duration>,
}
#[derive(Clone)]
pub struct FakeBackend(Arc<Mutex<FakeState>>);
impl Default for FakeBackend {
    fn default() -> Self {
        Self::canned()
    }
}
impl FakeBackend {
    pub fn configure<T>(&self, configure: impl FnOnce(&mut FakeConfig) -> T) -> T {
        configure(&mut self.0.lock().unwrap().config)
    }

    pub fn revision(&self) -> i64 {
        self.configure(|s| {
            s.revision += 1;
            s.revision
        })
    }
    pub fn fail_sign_in(&self, reason: Option<crate::client::error::SignInFailure>) {
        self.0.lock().unwrap().sign_in_failure = reason;
    }

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
            return Err(Failure::SignedOut(SignOutReason::Unavailable(reason)).into());
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
    fn fake_send(
        &mut self,
        chat: &ChatId,
        body: Content,
        seq: u32,
        reply: Option<&ReplyReference>,
    ) -> Result<MessageInput> {
        let gate = self.0.lock().unwrap().send_gate.take();
        if let Some((entered, blocked)) = gate {
            let _ = entered.send(());
            let _ = blocked.recv();
        }
        self.rpc(FakeRpc::Send(chat.clone()))?;
        let (row, echo, revision, deliver) = {
            let mut s = self.0.lock().unwrap();
            s.next_id += 1;
            let mut row = Self::message(&chat.0, s.next_id, "", true);
            row.body = body;
            row.reply = reply.cloned().map(Box::new);
            row.request_seq = Some(seq);
            s.pages.entry(chat.clone()).or_default().push(row.clone());
            s.config.revision += 1;
            (row, s.echo_first, s.config.revision, s.deliver.clone())
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
                last_message_id: Some(ServerId::new(if i == 0 { "10" } else { "2" }).unwrap()),
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
            config: FakeConfig {
                chats,
                revision: 1000,
                ..Default::default()
            },
            pages,
            next_id: 100,
            ..Default::default()
        })))
    }
    pub fn message(chat: &str, id: i64, text: &str, outgoing: bool) -> MessageInput {
        MessageInput {
            server_id: ServerId::new(id.to_string()).unwrap(),
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
            rev: id,
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
                Err(Failure::NotDelivered)
            } else {
                Ok(())
            }));
        }
    }
    pub fn fail_rpc(&self, rpc: FakeRpc, error: Failure) {
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
    pub fn page(&self, chat: &ChatId) -> Vec<MessageInput> {
        self.0
            .lock()
            .unwrap()
            .pages
            .get(chat)
            .cloned()
            .unwrap_or_default()
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
    pub fn auth_expired(&self) {
        let d = self.0.lock().unwrap().deliver.clone();
        if let Some(d) = d {
            d(StreamEvent::Connection(Err(Failure::SignedOut(
                SignOutReason::AuthenticationFailed,
            ))));
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
    pub fn echo_first(&self, value: bool) {
        self.0.lock().unwrap().echo_first = value;
    }
    pub fn add_chat(&self, chat: ChatInput) {
        let mut s = self.0.lock().unwrap();
        s.config.chats.retain(|c| c.id != chat.id);
        s.config.chats.push(chat);
    }
    pub fn set_page(&self, chat: ChatId, rows: Vec<MessageInput>) {
        let mut s = self.0.lock().unwrap();
        for m in &rows {
            s.next_id = s.next_id.max(m.server_id.as_str().parse().unwrap());
            s.config.revision = s.config.revision.max(s.next_id);
        }
        s.pages.insert(chat, rows);
    }
    pub fn inject_raw(&self, mut raw: Value) {
        raw["revision"] = self.revision().into();
        let deliver = self.0.lock().unwrap().deliver.clone();
        if let Some(d) = deliver {
            d(StreamEvent::Connection(Ok(())));
            d(StreamEvent::Raw(raw));
        }
    }
    pub fn inject(&self, op: LiveOp) {
        let deliver = {
            let mut s = self.0.lock().unwrap();
            s.config.revision = s.config.revision.max(op.revision);
            if let Op::Message(m) = &op.op {
                s.next_id = s.next_id.max(m.server_id.as_str().parse().unwrap());
                s.config.revision = s.config.revision.max(s.next_id);
            }
            s.deliver.clone()
        };
        if let Some(d) = deliver {
            d(StreamEvent::Connection(Ok(())));
            d(StreamEvent::Operation(op));
        }
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
                network: true,
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
        if let Some(pixels) = asset
            .local_pixels()
            .map_err(|_| Failure::Bug("decode".into()))?
        {
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
            crate::client::network_touched();
            let deliver = {
                let mut s = self.0.lock().unwrap();
                s.failed_assets.insert(asset.key(), purpose.clone());
                s.assets.clone().unwrap()
            };
            deliver(StreamEvent::Asset(
                crate::client::desktop::assets::Completion {
                    network: true,
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
        crate::client::network_touched();
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
    fn online(&self) -> Result<()> {
        crate::client::network_touched();
        if self.0.lock().unwrap().offline {
            Err(Failure::NotDelivered)
        } else {
            Ok(())
        }
    }
}
impl Backend for FakeBackend {
    fn session(&self) -> Result<bool> {
        if let Some(reason) = self.0.lock().unwrap().open_failure.clone() {
            return Err(Failure::SignedOut(SignOutReason::Unavailable(reason)));
        }
        Ok(!self.0.lock().unwrap().signed_out)
    }
    fn sign_in(&mut self, cancel: Arc<CancelToken>, deliver: PairDeliver) -> Result<()> {
        use crate::client::{desktop::assets::Pixels, pairing::Stage};
        {
            let mut state = self.0.lock().unwrap();
            state.calls.push(FakeRpc::SignIn);
            if let Some(reason) = state.sign_in_failure.take() {
                return Err(Failure::SignedOut(SignOutReason::Unavailable(reason)));
            }
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
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            return Err(Failure::SignedOut(SignOutReason::Unavailable(
                crate::client::error::SignInFailure::Cancelled,
            )));
        }
        Err(Failure::SignedOut(SignOutReason::Unavailable(
            crate::client::error::SignInFailure::Server,
        )))
    }

    fn friends(&mut self) -> Result<Vec<crate::store::Friend>> {
        self.rpc(FakeRpc::Friends)?;
        if let Some(rows) = &self.0.lock().unwrap().friends {
            return Ok(rows.clone());
        }
        Ok(self
            .0
            .lock()
            .unwrap()
            .config
            .chats
            .iter()
            .map(|c| crate::store::Friend {
                profile: ContactProfile {
                    relation: Some(0),
                    ..Default::default()
                },
                id: c.id.0.clone(),
                name: Some(c.title.clone()),
                status: String::new(),
                group: c.is_group,
                avatar: c.avatar.clone(),
                member_count: c.member_count.map(|n| n as usize),
            })
            .collect())
    }
    fn members(&mut self, _chat: &ChatId) -> Result<Vec<crate::store::Member>> {
        self.online()?;
        if let Some(rows) = &self.0.lock().unwrap().config.members {
            return Ok(rows.clone());
        }
        Ok(self
            .friends()?
            .into_iter()
            .map(|contact| crate::store::Member {
                contact,
                is_self: false,
            })
            .collect())
    }
    fn start_assets(&mut self, stop: Arc<CancelToken>, deliver: Deliver) {
        self.0.lock().unwrap().assets = Some(deliver.clone());
        let fake = self.clone();
        let retry = self.clone();
        let output = deliver.clone();
        deliver(StreamEvent::AssetRequests(AssetRequests {
            stop,
            request: Arc::new(move |asset, purpose| {
                let mut fake = fake.clone();
                let output = output.clone();
                std::thread::spawn(move || {
                    crate::client::take_network_touched();
                    let result = fake.media(&asset, purpose.clone());
                    let network = crate::client::take_network_touched();
                    if let Ok(Some(payload)) = result {
                        output(StreamEvent::Asset(
                            crate::client::desktop::assets::Completion {
                                network,
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
    ) -> Result<Option<String>> {
        Ok(None)
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

    fn contact(&mut self, mid: &str) -> Result<Option<crate::store::Friend>> {
        self.rpc(FakeRpc::Contact(mid.into()))?;
        if let Some(contact) = self.0.lock().unwrap().contacts.get(mid).cloned() {
            return Ok(Some(contact));
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
                    .config
                    .members
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .find(|m| m.contact.id == mid)
                    .map(|m| m.contact.clone())
            })
            .or_else(|| {
                state
                    .config
                    .chats
                    .iter()
                    .find(|c| c.id.0 == mid)
                    .map(|c| Friend {
                        profile: ContactProfile {
                            relation: Some(0),
                            ..Default::default()
                        },
                        id: mid.into(),
                        name: Some(c.title.clone()),
                        avatar: c.avatar.clone(),
                        group: c.is_group,
                        member_count: c.member_count.map(|n| n as usize),
                        ..Default::default()
                    })
            })
            .map_or(Ok(None), |row| Ok(Some(row)))
    }

    fn notice(&mut self, _mid: &str) -> Result<Value> {
        self.online()?;
        Ok(json!({"notice":{"noticeAntiFraudGeneral":{}}}))
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value> {
        self.online()?;
        Ok(
            json!({"chatMid":chat.0,"chatName":self.0.lock().unwrap().config.chats.iter().find(|c| c.id == *chat).map(|c| &c.title),"notificationDisabled":false,"extra":{"groupExtra":{}}}),
        )
    }
    fn profile_action(&mut self, action: &ProfileAction, _full: Value, _seq: u32) -> Result<()> {
        self.rpc(FakeRpc::Action(action.clone()))?;
        match action {
            ProfileAction::Rename { mid, name } | ProfileAction::AddFriend { mid, chat: name } => {
                let Some(mut f) = self.contact(mid)? else {
                    return Ok(());
                };
                if matches!(action, ProfileAction::Rename { .. }) {
                    let original = f.profile.original_name.clone().or_else(|| f.name.clone());
                    f.profile.relation = Some(0);
                    f.profile.original_name = original;
                    f.profile.overridden_name = Some(name.clone());
                    f.name = Some(name.clone());
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
                    .config
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
                    .config
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
            return Err(Failure::SignedOut(SignOutReason::AuthenticationFailed));
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
            chats: s.config.chats.clone(),
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
        if let Some(c) = s.config.chats.iter().find(|c| &c.id == chat) {
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
                title: f.name.unwrap_or_default(),
                is_group: f.group,
                member_count: f.member_count.map(|n| n as u32),
                avatar: f.avatar,
                ..Default::default()
            };
            s.config.chats.push(c.clone());
            return Ok(vec![c]);
        }
        Ok(vec![])
    }
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page> {
        self.rpc(FakeRpc::Messages(chat.clone()))?;
        let state = self.0.lock().unwrap();
        let rows = state
            .pages
            .get(chat)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                let cursor = Cursor {
                    sort_ts: m.created_ms,
                    sort_id: crate::store::server_sort(&m.server_id),
                };
                match kind {
                    PageKind::Recent | PageKind::Around { .. } => true,
                    PageKind::Older { before } => cursor < *before,
                }
            })
            .collect();
        Ok(Page {
            kind: kind.clone(),
            rows,
            reaches_start: true,
        })
    }
    fn finish_attachment(
        &mut self,
        _chat: &ChatId,
        _uploaded: Value,
        _seq: u32,
    ) -> Result<MessageInput> {
        Err(Failure::Bug(
            "fake upload does not use wire completion".into(),
        ))
    }
    fn prepare_attachment(
        &mut self,
        chat: ChatId,
        item: crate::attachments::Attachment,
        seq: u32,
    ) -> Result<UploadJob> {
        let mut fake = self.clone();
        Ok(Box::new(move |progress| {
            item.read().map_err(|_| {
                Failure::Refused(if item.path.is_file() {
                    Refusal::NotAccepted
                } else {
                    Refusal::FileMissing
                })
            })?;
            fake.online()?;
            let gate = fake.0.lock().unwrap().upload_gate.take();
            if let Some((entered, blocked)) = gate {
                let _ = entered.send(());
                let _ = blocked.recv();
            }
            let delay = fake.0.lock().unwrap().config.upload_delay;
            for tick in 0..=10 {
                progress(tick as f32 / 10.);
                std::thread::sleep(delay / 11);
            }
            fake.rpc(FakeRpc::Upload(item.name.clone()))?;
            Ok(Box::new(move |_backend: &mut dyn Backend| {
                let row = fake.fake_send(&chat, item.content(), seq, None)?;
                // SEND_MESSAGE carries the persisted request identity; SEND_CONTENT repeats
                // the same server identity after storage, without a matching request seq.
                let (deliver, revision) = {
                    let mut state = fake.0.lock().unwrap();
                    state.config.revision += 2;
                    (state.deliver.clone(), state.config.revision)
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
    fn send(
        &mut self,
        chat: &ChatId,
        text: &str,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<MessageInput> {
        self.fake_send(chat, Content::text(text), seq, reply)
    }
    fn send_sticker(
        &mut self,
        chat: &ChatId,
        sticker: &Content,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<MessageInput> {
        self.fake_send(chat, sticker.clone(), seq, reply)
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
        s.config.revision += 1;
        if let Some(deliver) = &s.deliver {
            deliver(StreamEvent::Operation(LiveOp {
                revision: s.config.revision,
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
        s.config.chats.push(row.clone());
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
        stop: Arc<CancelToken>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        self.rpc(FakeRpc::Stream(revision))?;
        let mut s = self.0.lock().unwrap();
        s.deliver = Some(deliver.clone());
        drop(s);
        Ok(Box::new(move || {
            if !deliver(StreamEvent::Connection(Ok(()))) {
                return;
            }
            while !stop.is_cancelled() {
                std::thread::park_timeout(Duration::from_millis(20));
            }
        }))
    }
    fn decode(&mut self, raw: Value) -> Result<LiveOp> {
        Ok(LiveOp {
            revision: raw["revision"]
                .as_i64()
                .ok_or(Failure::Bug("decode".into()))?,
            replay: raw["replay"].as_bool().unwrap_or(false),
            op: match raw["type"].as_u64() {
                Some(139 | 140) => reaction_operation(&raw, "self")?,
                Some(64 | 65) => Op::Withdraw(decode_id(
                    raw["param2"]
                        .as_str()
                        .ok_or(Failure::Bug("decode".into()))?,
                )?),
                _ => Op::Checkpoint,
            },
        })
    }
    fn refresh_token(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_failure_preserves_reason_without_touching_health() {
        use crate::client::error::SignInFailure;
        let mut fake = FakeBackend::signed_out();
        fake.offline(true);
        fake.fail_sign_in(Some(SignInFailure::Server));
        crate::client::take_network_touched();
        let result = fake.sign_in(Arc::new(CancelToken::new()), Arc::new(|_| {}));
        assert_eq!(
            result,
            Err(Failure::SignedOut(SignOutReason::Unavailable(
                SignInFailure::Server
            )))
        );
        assert!(!crate::client::take_network_touched());
        assert_eq!(fake.calls(), vec![FakeRpc::SignIn]);
    }
}
