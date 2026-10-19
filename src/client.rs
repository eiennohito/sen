//! Shared client protocol against the same endpoints as LINE Chrome.
//! Rust owns protocol/state/WASM, all HTTPS (RPC, SSE, media) and QR encoding.
pub mod error;
use crate::{
    runtime::{Call, Runtime},
    sign,
};
use anyhow::{Context, Result, bail, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
pub use error::Error;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BASE: &str = "https://line-chrome-gw.line-apps.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

#[cfg(feature = "harness")]
pub use chat::benchmark_history;
mod chat;
pub mod desktop;
mod http;
pub mod pairing;
mod qr;
use qr::png as qr_png;

pub(crate) fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .context("output filename")?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[derive(Debug)]
pub(crate) struct RpcError {
    pub(crate) gateway: i64,
    pub(crate) method: String,
    pub(crate) http: u16,
    pub(crate) code: i64,
    pub(crate) status: i64,
}
impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: HTTP {}, LINE code {}, status {}",
            self.method, self.http, self.code, self.status
        )
    }
}
impl std::error::Error for RpcError {}
thread_local! { static REQUEST_CANCEL: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> = const { std::cell::RefCell::new(None) }; }
fn set_request_cancel(cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    REQUEST_CANCEL.with(|slot| *slot.borrow_mut() = Some(cancel));
}

pub(crate) struct Api {
    rt: Runtime,
    wasm: Vec<u8>,
    http: http::Transport,
    key_cache: std::collections::HashMap<String, Value>,
    channels: std::collections::HashMap<chat::Channel, (u32, u64)>,
    channel_tick: u64,
    state: Value,
    session_path: Option<PathBuf>,
    stream_access: std::sync::Arc<std::sync::RwLock<String>>,
    signing_key: Option<(String, crate::SigningKey)>,
    own_keys: std::collections::HashMap<u64, u32>,
}
impl Api {
    fn signature(&mut self, path: &str, body: &str, access: &str) -> Result<String> {
        self.prepare_runtime()?;
        if self
            .signing_key
            .as_ref()
            .is_none_or(|(token, _)| token != access)
        {
            let key = crate::signing_key(&mut self.rt, access, &self.http.client_version)?;
            if let Some((_, old)) = self.signing_key.replace((access.into(), key)) {
                old.destroy(&mut self.rt)?;
            }
        }
        sign(
            &mut self.rt,
            &self.signing_key.as_ref().expect("prepared signing key").1,
            path,
            body,
        )
    }
    fn prepare_runtime(&mut self) -> Result<()> {
        if self.rt.is_poisoned() {
            self.rt
                .reinitialize(&self.wasm, &self.http.client_version)?;
            self.channels.clear();
            self.own_keys.clear();
            self.signing_key = None;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "harness"))]
    fn new(wasm: &[u8], client_version: &str) -> Result<Self> {
        Self::with_runtime(Runtime::new(wasm, client_version)?, wasm, client_version)
    }
    fn with_runtime(rt: Runtime, wasm: &[u8], client_version: &str) -> Result<Self> {
        Ok(Self {
            rt,
            wasm: wasm.to_vec(),
            http: http::Transport::new(client_version)?,
            signing_key: None,
            own_keys: Default::default(),
            key_cache: Default::default(),
            channels: Default::default(),
            channel_tick: 0,
            state: Value::Null,
            session_path: None,
            stream_access: Default::default(),
        })
    }
    fn cached_channel(&mut self, key: &chat::Channel) -> Option<u32> {
        self.channel_tick += 1;
        let entry = self.channels.get_mut(key)?;
        entry.1 = self.channel_tick;
        Some(entry.0)
    }
    fn cache_channel(&mut self, key: chat::Channel, channel: u32) -> Result<()> {
        const MAX_CHANNELS: usize = 64;
        if self.channels.len() >= MAX_CHANNELS {
            let oldest = self
                .channels
                .iter()
                .min_by_key(|(_, (_, tick))| tick)
                .map(|(key, _)| key.clone())
                .context("oldest channel")?;
            let (pointer, _) = self.channels.remove(&oldest).context("cached channel")?;
            self.rt.destroy("E2EEChannel", pointer)?;
        }
        self.channel_tick += 1;
        self.channels.insert(key, (channel, self.channel_tick));
        Ok(())
    }
    fn call_once(
        &mut self,
        path: &str,
        args: &Value,
        access: &str,
        session: &str,
        poll_seconds: u64,
    ) -> Result<Value> {
        let body = serde_json::to_string(args)?;
        let signature = self.signature(path, &body, access)?;
        let mut headers = vec![
            ("Content-Type", "application/json"),
            ("User-Agent", UA),
            ("x-line-chrome-version", self.http.client_version.as_str()),
            ("x-lal", "en_US"),
            ("x-hmac", signature.as_str()),
        ];
        let cookie = format!("lct={access}");
        if !access.is_empty() {
            headers.extend([("x-line-access", access), ("Cookie", cookie.as_str())]);
        }
        if !session.is_empty() {
            headers.push(("X-Line-Session-ID", session));
        }
        let millis = (poll_seconds * 1000).to_string();
        if poll_seconds > 0 {
            headers.push(("X-LST", millis.as_str()));
        }
        let (http, content_type, bytes) = self.http.request(
            path,
            &body,
            &headers,
            Duration::from_secs(if poll_seconds > 0 {
                poll_seconds + 10
            } else {
                30
            }),
        )?;
        error::decode_response(path, http, &content_type, &bytes)
    }
    fn qr(&mut self, method: &str, args: Value, session: &str, poll_seconds: u64) -> Result<Value> {
        let service = if poll_seconds > 0 {
            "SecondaryQrCodeLoginPermitNoticeService"
        } else {
            "SecondaryQrCodeLoginService"
        };
        self.call_once(
            &format!("/api/talk/thrift/LoginQrCode/{service}/{method}"),
            &json!([args]),
            "",
            session,
            poll_seconds,
        )
    }
    fn profile(&mut self) -> Result<Value> {
        self.call(
            "/api/talk/thrift/Talk/TalkService/getProfile",
            &json!([0]),
            "",
            0,
        )
    }
}
fn bytes_method(rt: &mut Runtime, call: Call, ptr: u32, args: &[u64]) -> Result<Vec<u8>> {
    let h = rt
        .method(call, ptr, args)
        .with_context(|| format!("{call:?}"))?;
    rt.output(h)
}
fn decode(value: &str) -> Result<Vec<u8>> {
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(b) = engine.decode(value) {
            return Ok(b);
        }
    }
    Err(anyhow::Error::new(error::Failure::Bug("decode".into()))
        .context("invalid base64 key material"))
}
fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(error::Failure::Bug("decode".into()))
        .with_context(|| format!("missing {field}"))
}
fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char)
        } else {
            out.push_str(&format!("%{b:02X}"))
        }
    }
    out
}
fn callback(base: &str, public: &[u8]) -> Result<String> {
    ensure!(
        (base.starts_with("line://") || base.starts_with("https://"))
            && !base.contains(['\r', '\n', '#']),
        "invalid QR callback URL"
    );
    Ok(format!(
        "{base}{}secret={}&e2eeVersion=1",
        if base.contains('?') { "&" } else { "?" },
        percent_encode(&STANDARD.encode(public))
    ))
}
fn save_state(path: &Path, state: &Value) -> Result<()> {
    crate::credentials::save(path, state)
}
fn unwrap_keys(rt: &mut Runtime, key: u32, meta: &Value) -> Result<Vec<Value>> {
    ensure!(
        meta["errorCode"].is_null() || meta["errorCode"].as_str() == Some("SUCCESS"),
        "phone returned an E2EE key-transfer error"
    );
    let peer = decode(required(meta, "publicKey")?)?;
    ensure!(peer.len() == 32, "phone public-key size");
    let h = rt.bytes(&peer);
    let channel = rt
        .method(Call::Curve25519KeyCreateChannel, key, &[h])
        .context("create phone key-transfer channel")?;
    rt.release(h)?;
    let h = rt.bytes(&decode(required(meta, "encryptedKeyChain")?)?);
    let chain = rt
        .method(Call::E2EEChannelUnwrapE2EEKeyChain, channel, &[h])
        .context("unwrap phone keychain")?;
    rt.release(h)?;
    let size = rt
        .method(Call::E2EEKeychainSize, chain, &[])
        .context("keychain size")?;
    ensure!((1..=64).contains(&size), "unexpected keychain size {size}");
    let mut keys = Vec::new();
    for i in 0..size {
        let h = rt
            .method(Call::E2EEKeychainGet, chain, &[i as u64])
            .context("keychain entry")?;
        let ptr = rt.object(h)?;
        let id = rt
            .method(Call::E2EEKeyGetKeyId, ptr, &[])
            .context("key ID")?;
        let version = rt
            .method(Call::E2EEKeyGetVersion, ptr, &[])
            .context("key version")?;
        let public = bytes_method(rt, Call::E2EEKeyGetPublicKey, ptr, &[])?;
        let exported = bytes_method(rt, Call::E2EEKeyExportKey, ptr, &[])?;
        let h = rt.bytes(&exported);
        let loaded = rt
            .static_call(Call::E2EEKeyLoadKey, &[h])
            .context("reload transferred key")?;
        rt.release(h)?;
        let reloaded = bytes_method(rt, Call::E2EEKeyGetPublicKey, loaded, &[])?;
        ensure!(
            public.len() == 32 && public == reloaded,
            "E2EE export/import public-key mismatch"
        );
        rt.destroy("E2EEKey", loaded)?;
        keys.push(json!({"id":id,"version":version,"public_key":STANDARD.encode(public),"exported_key":STANDARD.encode(exported)}));
    }
    rt.destroy("E2EEKeychain", chain)?;
    rt.destroy("E2EEChannel", channel)?;
    Ok(keys)
}
fn access(state: &Value) -> Result<&str> {
    let token = required(&state["token_v3_issue_result"], "accessToken")?;
    ensure!(
        !token.is_empty(),
        error::SavedSessionFailure::Corrupt.error()
    );
    Ok(token)
}
fn validate_keys(api: &mut Api, state: &Value) -> Result<usize> {
    let keys = state["e2ee_keys"]
        .as_array()
        .context("missing saved E2EE keys")?;
    ensure!(!keys.is_empty(), "no saved E2EE keys");
    for key in keys {
        let ptr = chat::own_key(
            api,
            state,
            key["id"].as_u64().context("missing own key ID")?,
        )?;
        let public = bytes_method(&mut api.rt, Call::E2EEKeyGetPublicKey, ptr, &[])?;
        ensure!(
            public == decode(required(key, "public_key")?)?,
            "restored key mismatch"
        );
    }
    Ok(keys.len())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qr_callback_encodes_public_key_as_query_parameter() {
        let url = callback("line://au/q/test?session=abc", &[0xfb; 32]).unwrap();
        assert!(url.starts_with("line://au/q/test?session=abc&secret="));
        assert!(url.contains("%2B"));
        assert!(url.ends_with("&e2eeVersion=1"));
        assert!(callback("file:///tmp/test", &[0; 32]).is_err());
    }
}

thread_local! {
    static NETWORK_TOUCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn network_touched() {
    NETWORK_TOUCHED.set(true);
}
pub(crate) fn take_network_touched() -> bool {
    NETWORK_TOUCHED.replace(false)
}
