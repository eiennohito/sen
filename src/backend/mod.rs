//! RPC boundary shared by the real account and synthetic runs.
use crate::client::http::CancelToken;
use crate::client::{
    NetworkActivity,
    operation::{OperationType, RawOperation},
};
use crate::store::*;
use serde_json::Value;
use std::sync::Arc;

pub use crate::client::error::{Failure, Refusal, SignOutReason};
pub type Result<T> = std::result::Result<T, Failure>;
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
    pub stop: Arc<CancelToken>,
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
        renew: bool,
        reply: async_channel::Sender<
            std::result::Result<Option<String>, crate::client::desktop::assets::Failure>,
        >,
    },
    Resync {
        revision: Option<i64>,
        complete: async_channel::Sender<Result<()>>,
    },
    Operation(LiveOp),
    Raw(RawOperation),
    RefreshToken {
        reply: async_channel::Sender<Result<()>>,
    },
    Connection(Result<()>),
    Exited,
    Connecting,
    Retry {
        error: Failure,
    },
}
pub type Deliver = Arc<dyn Fn(StreamEvent) -> bool + Send + Sync>;
pub type FinishUpload = Box<dyn FnOnce(&mut dyn Backend) -> Result<MessageInput> + Send>;
pub type UploadJob =
    Box<dyn FnOnce(NetworkActivity, &mut dyn FnMut(f32)) -> Result<FinishUpload> + Send>;
/// Each stream invocation runs on its own thread. Returning is supervised by the engine.
pub trait Backend: Send {
    fn set_network_activity(&mut self, activity: NetworkActivity);
    fn session(&self) -> Result<bool>;
    fn sign_in(&mut self, cancel: Arc<CancelToken>, deliver: PairDeliver) -> Result<()>;
    fn friends(&mut self) -> Result<Vec<crate::store::Friend>>;
    fn members(&mut self, chat: &ChatId) -> Result<Vec<crate::store::Member>>;
    /// Publish AssetRequests before returning. Its handles enqueue independently
    /// of RPCs, and asset completions use this delivery channel directly.
    fn start_assets(&mut self, stop: Arc<CancelToken>, deliver: Deliver) -> Result<()>;
    fn authorize(&mut self, renew: bool) -> Result<Option<String>>;
    /// Cache-only access; must never authorize or download an asset.
    fn cached_media(
        &mut self,
        mid: &str,
        asset: &crate::client::desktop::assets::Asset,
        purpose: crate::client::desktop::assets::Purpose,
    ) -> Result<Option<crate::client::desktop::assets::Payload>>;
    fn contact(&mut self, mid: &str) -> Result<Option<crate::store::Friend>>;
    fn notice(&mut self, mid: &str) -> Result<Value>;
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value>;
    fn profile_action(&mut self, action: &ProfileAction, seq: u32) -> Result<()>;
    fn profile(&mut self) -> Result<Profile>;
    fn chats(&mut self, cursor: Option<String>) -> Result<ChatPage>;
    fn details(&mut self, chat: &ChatId) -> Result<Vec<ChatInput>>;
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page>;
    fn prepare_attachment(
        &mut self,
        chat: ChatId,
        item: crate::attachments::Attachment,
        seq: u32,
    ) -> Result<UploadJob>;
    fn finish_attachment(
        &mut self,
        chat: &ChatId,
        uploaded: Value,
        seq: u32,
    ) -> Result<MessageInput>;
    fn send(
        &mut self,
        chat: &ChatId,
        text: &str,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<MessageInput>;
    fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()>;
    fn sticker_packages(&mut self) -> Result<Vec<StickerPackage>>;
    fn send_sticker(
        &mut self,
        chat: &ChatId,
        sticker: &Content,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<MessageInput>;
    fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput>;
    fn unsend(&mut self, id: &ServerId, seq: u32) -> Result<()>;
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>>;
    fn mark_read(&mut self, chat: &ChatId, id: &ServerId, seq: u32) -> Result<()>;
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<CancelToken>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>>;
    fn decode(&mut self, operation: RawOperation) -> Result<LiveOp>;
    fn refresh_token(&mut self) -> Result<()>;
}

#[cfg(any(test, feature = "demo"))]
mod fake;
#[cfg(any(test, feature = "demo"))]
pub use fake::{FakeBackend, FakeConfig, FakeRpc};

enum SessionClient {
    Open(Box<crate::client::desktop::Client>),
    Absent(NetworkActivity),
}

pub struct LineBackend {
    cache: std::path::PathBuf,
    session_failure: Option<crate::client::error::SignInFailure>,
    fetchers: Option<Arc<crate::client::desktop::assets::Fetchers>>,
    media_auth: Option<(Arc<CancelToken>, Deliver)>,
    client: SessionClient,
    state: std::path::PathBuf,
    wasm_override: Option<std::path::PathBuf>,
    version_override: Option<String>,
}
impl LineBackend {
    fn client(&mut self) -> Result<&mut crate::client::desktop::Client> {
        match &mut self.client {
            SessionClient::Open(client) => Ok(client),
            SessionClient::Absent(_) => Err(Failure::SignedOut(SignOutReason::NoSession)),
        }
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
                let reason = crate::client::error::saved_session_failure(&error)?;
                (None, Some(reason))
            }
        };
        Ok(Self {
            fetchers: None,
            media_auth: None,
            cache: cache.into(),
            client: match client {
                Some(client) => SessionClient::Open(Box::new(client)),
                None => SessionClient::Absent(NetworkActivity::default()),
            },
            session_failure,
            state: state.into(),
            wasm_override,
            version_override,
        })
    }
}
fn rpc(error: anyhow::Error) -> Failure {
    let failure = crate::client::error::classify(&error);
    if !matches!(failure, Failure::Bug(_)) {
        crate::reporting::source(error.as_ref());
    }
    failure
}
fn asset_authorization(
    renew: bool,
    cancel: &CancelToken,
    deliver: &Deliver,
) -> std::result::Result<Option<String>, crate::client::desktop::assets::Failure> {
    let (reply, rx) = async_channel::bounded(1);
    if !deliver(StreamEvent::Authorize { renew, reply }) {
        return Err(crate::client::desktop::assets::Failure::Cancelled);
    }
    cancel
        .recv(&rx)
        .unwrap_or(Err(crate::client::desktop::assets::Failure::Cancelled))
}

fn refresh_stream_token(stop: &CancelToken, deliver: &Deliver) -> Result<()> {
    let (reply, rx) = async_channel::bounded(1);
    if !deliver(StreamEvent::RefreshToken { reply }) {
        return Err(Failure::SignedOut(SignOutReason::AuthenticationFailed));
    }
    stop.recv(&rx)
        .unwrap_or(Err(Failure::SignedOut(SignOutReason::AuthenticationFailed)))
}

impl Backend for LineBackend {
    fn set_network_activity(&mut self, activity: NetworkActivity) {
        match &mut self.client {
            SessionClient::Open(client) => client.set_network_activity(activity),
            SessionClient::Absent(pending) => *pending = activity,
        }
    }
    fn finish_attachment(
        &mut self,
        chat: &ChatId,
        uploaded: Value,
        seq: u32,
    ) -> Result<MessageInput> {
        self.client()?
            .finish_attachment(&chat.0, uploaded, seq)
            .map_err(rpc)
    }

    fn session(&self) -> Result<bool> {
        match &self.session_failure {
            Some(reason) => Err(Failure::SignedOut(SignOutReason::Unavailable(
                reason.clone(),
            ))),
            None => Ok(matches!(self.client, SessionClient::Open(_))),
        }
    }
    fn sign_in(&mut self, cancel: Arc<CancelToken>, deliver: PairDeliver) -> Result<()> {
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
        let activity = match &self.client {
            SessionClient::Open(client) => client.network_activity(),
            SessionClient::Absent(activity) => activity.clone(),
        };
        self.client = SessionClient::Absent(activity.clone());
        let mut client = pairing::sign_in_validated(
            &self.state,
            module,
            activity,
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
            |staged, api| {
                desktop::Client::from_api(
                    staged,
                    &self.cache,
                    serde_json::from_value(crate::credentials::load(staged)?)
                        .map_err(|_| crate::client::error::SignInFailure::SavedSession)?,
                    api,
                )
            },
        )
        .map_err(|error| {
            crate::reporting::log("sign-in", "pairing", error.as_ref());
            Failure::SignedOut(SignOutReason::Unavailable(
                crate::client::error::sign_in_failure(&error, cancel.is_cancelled()),
            ))
        })?;
        client.installed_at(&self.state);
        self.client = SessionClient::Open(Box::new(client));
        self.session_failure = None;
        Ok(())
    }

    fn friends(&mut self) -> Result<Vec<crate::store::Friend>> {
        self.client()?.friends().map_err(rpc)
    }
    fn members(&mut self, chat: &ChatId) -> Result<Vec<crate::store::Member>> {
        self.client()?.members(&chat.0).map_err(rpc)
    }
    fn start_assets(&mut self, stop: Arc<CancelToken>, deliver: Deliver) -> Result<()> {
        self.media_auth = Some((stop.clone(), deliver.clone()));
        let auth = deliver.clone();
        let client = self.client()?;
        let queue_stop = stop.clone();
        self.fetchers = Some(Arc::new(client.asset_fetchers(
            stop,
            Default::default(),
            move |renew, cancel| asset_authorization(renew, cancel, &auth),
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
            stop: queue_stop,
            request: Arc::new(move |asset, purpose| {
                requests.enqueue(asset, purpose);
            }),
            retry: Arc::new(move |key| {
                retry.retry(&key);
            }),
        }));
        Ok(())
    }
    fn authorize(&mut self, renew: bool) -> Result<Option<String>> {
        self.client()?.media_token(renew).map(Some).map_err(rpc)
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
            Purpose::Original => match asset.original() {
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
    fn contact(&mut self, mid: &str) -> Result<Option<crate::store::Friend>> {
        self.client()?.contact(mid).map_err(rpc)
    }
    fn notice(&mut self, mid: &str) -> Result<Value> {
        self.client()?.profile_notice(mid).map_err(rpc)
    }
    fn full_chat(&mut self, chat: &ChatId) -> Result<Value> {
        self.client()?.full_chat(&chat.0).map_err(rpc)
    }
    fn profile_action(&mut self, action: &ProfileAction, seq: u32) -> Result<()> {
        match action {
            ProfileAction::Rename { mid, name } => self.client()?.rename_contact(mid, name, seq),
            ProfileAction::AddFriend { mid, chat } => self.client()?.add_friend(mid, chat, seq),
            ProfileAction::Mute { chat, disabled } => {
                self.client()?.mute_chat(&chat.0, *disabled, seq)
            }
            ProfileAction::Leave { chat } => self.client()?.leave_chat(&chat.0, seq),
        }
        .map_err(rpc)
    }
    fn profile(&mut self) -> Result<Profile> {
        let data = self.client()?.online_profile().map_err(rpc)?;
        serde_json::from_value(data).map_err(|_| Failure::Bug("decode".into()))
    }
    fn chats(&mut self, cursor: Option<String>) -> Result<ChatPage> {
        let page = self.client()?.chats(cursor.as_deref()).map_err(rpc)?;
        Ok(ChatPage {
            chats: page.chats,
            previews: page.previews,
            more: page.more,
            cursor: page.cursor,
        })
    }

    fn details(&mut self, chat: &ChatId) -> Result<Vec<ChatInput>> {
        self.client()?.chat_detail(&chat.0).map_err(rpc)
    }
    fn messages(&mut self, chat: &ChatId, kind: &PageKind) -> Result<Page> {
        const MESSAGE_PAGE_SIZE: usize = 50;
        let (rows, reaches_start) = match kind {
            PageKind::Around { id } => self.client()?.around(&chat.0, id.as_str()).map_err(rpc)?,
            PageKind::Older { before } => self
                .client()?
                .older(
                    &chat.0,
                    before.sort_id.trim_start_matches('0'),
                    before.sort_ts,
                )
                .map_err(rpc)?,
            PageKind::Recent => {
                let rows = self.client()?.refresh_messages(&chat.0).map_err(rpc)?;
                let start = rows.len() < MESSAGE_PAGE_SIZE;
                (rows, start)
            }
        };
        Ok(Page {
            kind: kind.clone(),
            rows,
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
            return Err(Failure::Refused(Refusal::FileMissing));
        }
        let (stop, deliver) = self
            .media_auth
            .clone()
            .ok_or(Failure::Bug("media authorization unavailable".into()))?;
        let upload = self
            .client()?
            .prepare_attachment(&chat.0, item, move || {
                asset_authorization(true, &stop, &deliver)?
                    .ok_or_else(|| anyhow::anyhow!("missing media token"))
            })
            .map_err(rpc)?;
        Ok(Box::new(move |activity, progress| {
            let uploaded = upload(activity, progress).map_err(rpc)?;
            Ok(Box::new(move |backend: &mut dyn Backend| {
                backend.finish_attachment(&chat, uploaded, seq)
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
        self.client()?
            .send_text_with_request(&chat.0, text, seq, reply)
            .map_err(rpc)
    }
    fn react(&mut self, id: &ServerId, kind: Option<u32>, seq: u32) -> Result<()> {
        self.client()?.react(id.as_str(), kind, seq).map_err(rpc)
    }
    fn sticker_packages(&mut self) -> Result<Vec<StickerPackage>> {
        self.client()?.sticker_packages().map_err(rpc)
    }
    fn send_sticker(
        &mut self,
        chat: &ChatId,
        sticker: &Content,
        seq: u32,
        reply: Option<&crate::store::ReplyReference>,
    ) -> Result<MessageInput> {
        self.client()?
            .send_sticker(&chat.0, sticker, seq, reply)
            .map_err(rpc)
    }
    fn create_chat(&mut self, name: &str, targets: &[String], seq: u32) -> Result<ChatInput> {
        self.client()?.create_chat(name, targets, seq).map_err(rpc)
    }
    fn unsend(&mut self, id: &ServerId, seq: u32) -> Result<()> {
        self.client()?.unsend(id.as_str(), seq).map_err(rpc)
    }
    fn receipts(&mut self, chat: &ChatId) -> Result<Vec<Option<ServerId>>> {
        Ok(self
            .client()?
            .read_ranges(&chat.0)
            .map_err(rpc)?
            .into_iter()
            .filter_map(|r| {
                if r.open_end {
                    Some(None)
                } else {
                    decode_id(&r.end).ok().map(Some)
                }
            })
            .collect())
    }
    fn mark_read(&mut self, chat: &ChatId, id: &ServerId, seq: u32) -> Result<()> {
        self.client()?
            .mark_read(&chat.0, id.as_str(), seq)
            .map_err(rpc)
    }
    fn stream(
        &mut self,
        revision: i64,
        stop: Arc<CancelToken>,
        deliver: Deliver,
    ) -> Result<Box<dyn FnOnce() + Send>> {
        let replay_through = self.client()?.latest_revision().map_err(rpc)?;
        let client_version = self.client()?.client_version().to_owned();
        let access = self.client()?.stream_access();
        Ok(Box::new(move || {
            use crate::client::desktop::sync::{self, Event, SyncState};
            let refresh_deliver = deliver.clone();
            let result = sync::receive_with_auth(
                &client_version,
                revision as u64,
                &stop,
                |renew| {
                    if renew {
                        refresh_stream_token(&stop, &refresh_deliver)?;
                    }
                    Ok(access.read().unwrap().clone())
                },
                |e| {
                    deliver(match e {
                        Event::Operation(mut v) => {
                            let rev: Option<i64> = crate::client::wire_number(&v.data["revision"]);
                            v.replay = rev.is_some_and(|r| r <= replay_through);
                            StreamEvent::Raw(v)
                        }
                        Event::State(s) => match s {
                            SyncState::Connected => StreamEvent::Connection(Ok(())),
                            SyncState::Reconnecting => StreamEvent::Connecting,
                        },
                        Event::Failure { error } => StreamEvent::Retry { error },
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
            if !stop.is_cancelled()
                && let Err(error) = result
            {
                deliver(StreamEvent::Retry {
                    error: crate::client::error::classify(&error),
                });
            }
        }))
    }
    fn decode(&mut self, operation: RawOperation) -> Result<LiveOp> {
        let kind = operation.kind;
        let raw = &operation.data;
        let revision =
            crate::client::wire_number(&raw["revision"]).ok_or(Failure::Bug("decode".into()))?;
        let chat = || wire_string(raw, "param1").map(|s| ChatId(s.into()));
        let id = || decode_id(wire_string(raw, "param2")?);
        let op = match kind {
            OperationType::SendMessage
            | OperationType::ReceiveMessage
            | OperationType::SendContent => {
                let message = self.client()?.operation(&operation).map_err(rpc)?;
                if let Some(mut m) = message {
                    m.rev = revision;
                    m.request_seq = crate::client::wire_number(&raw["reqSeq"])
                        .filter(|seq| *seq > 0)
                        .or(m.request_seq);
                    Op::Message(m.into())
                } else {
                    Op::Checkpoint
                }
            }
            OperationType::NotifiedReadMessage => Op::Receipt {
                chat: chat()?,
                up_to: decode_id(
                    raw["param3"]
                        .as_str()
                        .ok_or(Failure::Bug("decode".into()))?,
                )?,
            },
            OperationType::SendChatChecked => Op::Read {
                chat: chat()?,
                up_to: id()?,
            },
            OperationType::DestroyMessage | OperationType::NotifiedDestroyMessage => {
                Op::Withdraw(id()?)
            }
            OperationType::SendReaction | OperationType::NotifiedSendReaction => {
                reaction_operation(kind, raw, self.client()?.local_mid())?
            }
            // Unhandled operation types still advance the revision checkpoint.
            _ => Op::Checkpoint,
        };
        Ok(LiveOp {
            revision,
            replay: operation.replay,
            op,
        })
    }
    fn refresh_token(&mut self) -> Result<()> {
        self.client()?.refresh_token().map_err(rpc)
    }
}

fn reaction_operation(kind: OperationType, raw: &Value, own_mid: &str) -> Result<Op> {
    let id = decode_id(
        raw["param1"]
            .as_str()
            .ok_or(Failure::Bug("decode".into()))?,
    )?;
    let data: Value = serde_json::from_str(
        raw["param2"]
            .as_str()
            .ok_or(Failure::Bug("decode".into()))?,
    )
    .map_err(|_| Failure::Bug("decode".into()))?;
    let chat = wire_string(&data, "chatMid")?;
    let curr = match data.get("curr") {
        Some(Value::Null) => None,
        Some(value) => match crate::client::desktop::reaction_kind(value) {
            Some(kind) => Some(kind),
            None => return Ok(Op::Checkpoint),
        },
        None => return Ok(Op::Checkpoint),
    };
    // Extension 3.7.2 main.js: SEND_REACTION uses self; NOTIFIED_SEND_REACTION uses param3.
    let actor = if kind == OperationType::SendReaction {
        own_mid
    } else {
        match wire_string(raw, "param3") {
            Ok(actor) => actor,
            Err(_) => return Ok(Op::Recent(ChatId(chat.into()))),
        }
    };
    if actor.is_empty() {
        return Err(Failure::Bug("decode".into()));
    }
    Ok(Op::Reaction {
        id,
        actor: actor.into(),
        curr,
    })
}

fn wire_string<'a>(raw: &'a Value, key: &str) -> Result<&'a str> {
    raw[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Failure::Bug("decode".into()))
}

fn decode_id(value: &str) -> Result<ServerId> {
    ServerId::new(value).map_err(|_| Failure::Bug("decode".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_reply_wait_wakes_for_reply_or_cancellation() {
        use std::{sync::mpsc, thread, time::Duration};
        for (authorize, cancel) in [(true, false), (true, true), (false, false), (false, true)] {
            let stop = Arc::new(CancelToken::new());
            let worker_stop = stop.clone();
            let (events, pending) = mpsc::channel();
            let deliver: Deliver = Arc::new(move |event| events.send(event).is_ok());
            let (finished, done) = mpsc::channel();
            let worker = thread::spawn(move || {
                if authorize {
                    assert_eq!(
                        asset_authorization(false, &worker_stop, &deliver),
                        if cancel {
                            Err(crate::client::desktop::assets::Failure::Cancelled)
                        } else {
                            Ok(Some("token".into()))
                        }
                    );
                } else {
                    assert_eq!(
                        refresh_stream_token(&worker_stop, &deliver),
                        if cancel {
                            Err(Failure::SignedOut(SignOutReason::AuthenticationFailed))
                        } else {
                            Ok(())
                        }
                    );
                }
                finished.send(()).unwrap();
            });
            let event = pending.recv_timeout(Duration::from_secs(3)).unwrap();
            match &event {
                StreamEvent::Authorize { reply, .. } if authorize => {
                    if cancel {
                        stop.cancel();
                    } else {
                        reply.try_send(Ok(Some("token".into()))).unwrap();
                    }
                }
                StreamEvent::RefreshToken { reply } if !authorize => {
                    if cancel {
                        stop.cancel();
                    } else {
                        reply.try_send(Ok(())).unwrap();
                    }
                }
                _ => panic!("unexpected stream request"),
            }
            done.recv_timeout(Duration::from_secs(3))
                .expect("stream reply stayed blocked");
            worker.join().unwrap();
            drop(event);
        }
    }

    #[test]
    fn reaction_actor_uses_the_observed_wire_field() {
        let mut raw = json!({"type":140,"param1":"10","param2":json!({"chatMid":"chat","curr":{"predefinedReactionType":2},"fromUserMid":"guessed"}).to_string()});
        assert!(
            matches!(reaction_operation(OperationType::NotifiedSendReaction, &raw, "self").unwrap(), Op::Recent(chat) if chat.0 == "chat")
        );
        raw["param3"] = "actor".into();
        assert!(
            matches!(reaction_operation(OperationType::NotifiedSendReaction, &raw, "self").unwrap(), Op::Reaction { actor, .. } if actor == "actor")
        );
        for value in [Value::Null, json!(""), json!(3)] {
            assert!(wire_string(&json!({"param1":value}), "param1").is_err());
        }
    }

    #[test]
    fn unknown_live_reactions_checkpoint_and_only_null_removes() {
        for kind in [
            OperationType::NotifiedSendReaction,
            OperationType::SendReaction,
        ] {
            let raw =
                |data: Value| json!({"param1":"10", "param2":data.to_string(), "param3":"actor"});
            for curr in [
                json!({"customReactionType":{"id":"new"}}),
                json!({}),
                json!({"predefinedReactionType":"bad"}),
            ] {
                assert!(matches!(
                    reaction_operation(kind, &raw(json!({"chatMid":"chat", "curr":curr})), "self")
                        .unwrap(),
                    Op::Checkpoint
                ));
            }
            assert!(matches!(
                reaction_operation(kind, &raw(json!({"chatMid":"chat"})), "self").unwrap(),
                Op::Checkpoint
            ));
            assert!(matches!(
                reaction_operation(kind, &raw(json!({"chatMid":"chat", "curr":null})), "self")
                    .unwrap(),
                Op::Reaction { curr: None, .. }
            ));
            for curr in [
                json!({"predefinedReactionType":2}),
                json!({"predefinedReactionType":"2"}),
            ] {
                assert!(matches!(
                    reaction_operation(kind, &raw(json!({"chatMid":"chat", "curr":curr})), "self")
                        .unwrap(),
                    Op::Reaction {
                        curr: Some(crate::store::ReactionKind::Predefined(2)),
                        ..
                    }
                ));
            }
        }
    }

    #[test]
    fn invalid_wire_ids_fail_decoding() {
        let mut backend = FakeBackend::canned();
        for id in ["", "01", "-1", "x", "123456789012345678901"] {
            assert!(
                backend
                    .decode(json!({"type":64,"revision":1,"param2":id}).into())
                    .is_err()
            );
            assert!(
                reaction_operation(
                    OperationType::NotifiedSendReaction,
                    &json!({
                        "type":140,"param1":id,"param2":r#"{"chatMid":"chat"}"#
                    }),
                    "self"
                )
                .is_err()
            );
        }
    }
}
