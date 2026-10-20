use line::{backend::*, client::http::CancelToken, store::*};
use std::sync::{Arc, mpsc};
use std::time::Duration;

// One gate for any scripted RPC; dropping the release sender also unblocks it.
type Chats = Box<dyn FnMut(Option<String>) -> Result<ChatPage> + Send>;

pub(super) struct Blocked {
    fake: FakeBackend,
    gate: Option<(FakeRpc, mpsc::Sender<()>, mpsc::Receiver<()>)>,
    skip: usize,
    pub(super) chats: Option<Chats>,
}
impl Blocked {
    pub(super) fn new(
        fake: FakeBackend,
        target: FakeRpc,
        skip: usize,
    ) -> (Self, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        (
            Self {
                fake,
                gate: Some((target, entered, resume)),
                skip,
                chats: None,
            },
            waiting,
            release,
        )
    }

    fn wait(&mut self, rpc: &FakeRpc) {
        if self
            .gate
            .as_ref()
            .is_some_and(|(target, _, _)| target == rpc)
        {
            if self.skip > 0 {
                self.skip -= 1;
                return;
            }
            let (_, entered, release) = self.gate.take().unwrap();
            let _ = entered.send(());
            let _ = release.recv_timeout(Duration::from_secs(5));
        }
    }
}
impl Backend for Blocked {
    fn set_network_activity(&mut self, activity: line::client::NetworkActivity) {
        self.fake.set_network_activity(activity);
    }
    fn finish_attachment(
        &mut self,
        chat: &ChatId,
        uploaded: serde_json::Value,
        seq: u32,
    ) -> Result<MessageInput> {
        self.fake.finish_attachment(chat, uploaded, seq)
    }
    fn contact(&mut self, mid: &str) -> Result<Option<line::store::Friend>> {
        self.fake.contact(mid)
    }
    fn notice(&mut self, mid: &str) -> Result<serde_json::Value> {
        self.fake.notice(mid)
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<serde_json::Value> {
        self.fake.full_chat(chat)
    }
    fn profile_action(&mut self, action: &line::backend::ProfileAction, seq: u32) -> Result<()> {
        self.fake.profile_action(action, seq)
    }

    fn sign_in(&mut self, c: Arc<CancelToken>, d: PairDeliver) -> Result<()> {
        self.fake.sign_in(c, d)
    }
    fn friends(&mut self) -> Result<Vec<line::store::Friend>> {
        self.fake.friends()
    }
    fn members(&mut self, c: &ChatId) -> Result<Vec<line::store::Member>> {
        self.fake.members(c)
    }
    fn start_assets(&mut self, s: Arc<CancelToken>, d: Deliver) -> Result<()> {
        self.fake.start_assets(s, d)
    }
    fn authorize(&mut self, a: bool) -> Result<Option<String>> {
        self.fake.authorize(a)
    }
    fn session(&self) -> Result<bool> {
        self.fake.session()
    }
    fn cached_media(
        &mut self,
        mid: &str,
        asset: &line::client::desktop::assets::Asset,
        purpose: line::client::desktop::assets::Purpose,
    ) -> Result<Option<line::client::desktop::assets::Payload>> {
        self.fake.cached_media(mid, asset, purpose)
    }
    fn prepare_attachment(
        &mut self,
        chat: ChatId,
        item: line::attachments::Attachment,
        seq: u32,
    ) -> Result<line::backend::UploadJob> {
        self.fake.prepare_attachment(chat, item, seq)
    }
    fn profile(&mut self) -> Result<Profile> {
        self.fake.profile()
    }
    fn chats(&mut self, n: Option<String>) -> Result<ChatPage> {
        self.wait(&FakeRpc::Chats);
        if let Some(chats) = &mut self.chats {
            chats(n)
        } else {
            self.fake.chats(n)
        }
    }
    fn details(&mut self, c: &ChatId) -> Result<Vec<ChatInput>> {
        self.fake.details(c)
    }
    fn messages(&mut self, c: &ChatId, k: &PageKind) -> Result<Page> {
        self.wait(&FakeRpc::Messages(c.clone()));
        self.fake.messages(c, k)
    }
    fn send(
        &mut self,
        c: &ChatId,
        t: &str,
        s: u32,
        reply: Option<&line::store::ReplyReference>,
    ) -> Result<MessageInput> {
        self.wait(&FakeRpc::Send(c.clone()));
        self.fake.send(c, t, s, reply)
    }
    fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
        self.fake.react(id, kind, seq)
    }
    fn sticker_packages(&mut self) -> Result<Vec<line::store::StickerPackage>> {
        self.fake.sticker_packages()
    }
    fn send_sticker(
        &mut self,
        chat: &ChatId,
        sticker: &line::store::Content,
        seq: u32,
        reply: Option<&line::store::ReplyReference>,
    ) -> Result<MessageInput> {
        self.fake.send_sticker(chat, sticker, seq, reply)
    }
    fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput> {
        self.fake.create_chat(name, targets, seq)
    }
    fn unsend(&mut self, i: &ServerId, s: u32) -> Result<()> {
        self.fake.unsend(i, s)
    }
    fn receipts(&mut self, c: &ChatId) -> Result<Vec<Option<ServerId>>> {
        self.fake.receipts(c)
    }
    fn mark_read(&mut self, c: &ChatId, i: &ServerId, seq: u32) -> Result<()> {
        self.fake.mark_read(c, i, seq)
    }
    fn stream(
        &mut self,
        r: i64,
        s: Arc<CancelToken>,
        d: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        self.fake.stream(r, s, d)
    }
    fn decode(&mut self, r: line::client::operation::RawOperation) -> Result<LiveOp> {
        self.fake.decode(r)
    }
    fn refresh_token(&mut self) -> Result<()> {
        self.fake.refresh_token()
    }
}
