//! Shared client protocol against the same endpoints as LINE Chrome.
//! Rust owns protocol/state/WASM, all HTTPS (RPC, SSE, media) and QR encoding.
pub mod error;
pub mod operation;
use crate::runtime::{Call, Runtime};
use anyhow::{Context, Result, bail, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BASE: &str = "https://line-chrome-gw.line-apps.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

pub mod desktop;
mod e2ee;
#[cfg(feature = "harness")]
pub mod harness;
pub mod http;
pub mod pairing;
mod qr;

pub(crate) fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    private_directory(parent)?;
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
use error::RpcError;
thread_local! { static REQUEST_CANCEL: std::cell::RefCell<Option<std::sync::Arc<crate::client::http::CancelToken>>> = const { std::cell::RefCell::new(None) }; }
fn set_request_cancel(cancel: std::sync::Arc<crate::client::http::CancelToken>) {
    REQUEST_CANCEL.with(|slot| *slot.borrow_mut() = Some(cancel));
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Session {
    #[serde(default)]
    format: u32,
    token_v3_issue_result: Tokens,
    #[serde(default)]
    certificate: Value,
    #[serde(default)]
    login_metadata: Value,
    e2ee_keys: Vec<SessionKey>,
    profile: Profile,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Profile {
    mid: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Tokens {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(
        rename = "refreshToken",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    refresh_token: Option<String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SessionKey {
    id: u64,
    #[serde(default)]
    version: u32,
    public_key: String,
    exported_key: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => {}
        Err(error) => return Err(error.into()),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(crate) struct Api {
    rt: Runtime,
    wasm: Vec<u8>,
    http: http::Transport,
    key_cache: std::collections::HashMap<String, Value>,
    channels: std::collections::HashMap<e2ee::Channel, (u32, u64)>,
    channel_tick: u64,
    state: Session,
    session_path: Option<PathBuf>,
    stream_access: std::sync::Arc<std::sync::RwLock<String>>,
    signing_key: Option<(String, SigningKey)>,
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
            let key = signing_key(&mut self.rt, access, &self.http.client_version)?;
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

    #[cfg(test)]
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
            state: Session::default(),
            session_path: None,
            stream_access: Default::default(),
        })
    }
    fn cached_channel(&mut self, key: &e2ee::Channel) -> Option<u32> {
        self.channel_tick += 1;
        let entry = self.channels.get_mut(key)?;
        entry.1 = self.channel_tick;
        Some(entry.0)
    }
    fn cache_channel(&mut self, key: e2ee::Channel, channel: u32) -> Result<()> {
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
        let (http, _, bytes) = self.http.request(
            path,
            &body,
            &headers,
            Duration::from_secs(if poll_seconds > 0 {
                poll_seconds + 10
            } else {
                30
            }),
        )?;
        decode_response(path, http, &bytes)
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
    pub(super) fn call(
        &mut self,
        path: &str,
        args: &serde_json::Value,
        session: &str,
        poll_seconds: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let token = access(&self.state)
            .map_err(|_| error::Failure::SignedOut(error::SignOutReason::NoSession))?
            .to_owned();
        let result = self.call_once(path, args, &token, session, poll_seconds);
        if !error::needs_refresh(&result) {
            return result;
        }
        self.refresh_session()?;
        let token = access(&self.state)?.to_owned();
        let result = self.call_once(path, args, &token, session, poll_seconds);
        if error::needs_refresh(&result) {
            Err(error::Failure::SignedOut(error::SignOutReason::TokenRefresh).into())
        } else {
            result
        }
    }
    pub(super) fn refresh_session(&mut self) -> anyhow::Result<()> {
        let token = self
            .state
            .token_v3_issue_result
            .refresh_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .ok_or(error::Failure::SignedOut(
                error::SignOutReason::TokenRefresh,
            ))?
            .to_owned();
        let access = access(&self.state)?.to_owned();
        let args = serde_json::json!({"refreshToken": token});
        let result = self.call_once("/api/auth/tokenRefresh", &args, &access, "", 0);
        let result = if result
            .as_ref()
            .err()
            .and_then(|error| error.downcast_ref::<RpcError>())
            .is_some_and(RpcError::retry_refresh)
        {
            self.call_once("/api/auth/tokenRefresh", &args, &access, "", 0)
        } else {
            result
        };
        let value = result.map_err(|error| {
            if error
                .downcast_ref::<RpcError>()
                .is_some_and(RpcError::refresh_rejected)
            {
                anyhow::Error::new(error::Failure::SignedOut(
                    error::SignOutReason::TokenRefresh,
                ))
            } else {
                error
            }
        })?;
        let access = value["accessToken"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or(error::Failure::Bug("refresh missing access token".into()))?
            .to_owned();
        self.state.token_v3_issue_result.access_token = access.clone();
        if let Some(refresh) = value["refreshToken"].as_str() {
            self.state.token_v3_issue_result.refresh_token = Some(refresh.into());
        }
        for (name, value) in value
            .as_object()
            .ok_or(error::Failure::Bug("refresh token response".into()))?
        {
            if name != "accessToken" && name != "refreshToken" {
                self.state
                    .token_v3_issue_result
                    .rest
                    .insert(name.clone(), value.clone());
            }
        }
        if let Some(path) = &self.session_path {
            save_state(path, &self.state)?;
        }
        *self.stream_access.write().unwrap() = access;
        Ok(())
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
fn save_state(path: &Path, state: &Session) -> Result<()> {
    crate::credentials::save(path, &serde_json::to_value(state)?)
}
fn unwrap_keys(rt: &mut Runtime, key: u32, meta: &Value) -> Result<Vec<SessionKey>> {
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
        let public = bytes_method(rt, Call::E2EEKeyGetPublicKey, ptr, &[])?;
        let exported = bytes_method(rt, Call::E2EEKeyExportKey, ptr, &[])?;
        keys.push(SessionKey {
            id: id.into(),
            version: rt.method(Call::E2EEKeyGetVersion, ptr, &[])?,
            rest: Default::default(),
            public_key: STANDARD.encode(public),
            exported_key: STANDARD.encode(exported),
        });
    }
    rt.destroy("E2EEKeychain", chain)?;
    rt.destroy("E2EEChannel", channel)?;
    Ok(keys)
}
fn access(state: &Session) -> Result<&str> {
    ensure!(
        !state.token_v3_issue_result.access_token.is_empty(),
        error::SignInFailure::SavedSession
    );
    Ok(&state.token_v3_issue_result.access_token)
}
fn validate_keys(api: &mut Api, state: &Session) -> Result<()> {
    ensure!(!state.e2ee_keys.is_empty(), "no saved E2EE keys");
    for key in &state.e2ee_keys {
        let ptr = e2ee::own_key(api, state, key.id)?;
        let public = bytes_method(&mut api.rt, Call::E2EEKeyGetPublicKey, ptr, &[])?;
        ensure!(
            public.len() == 32 && public == decode(&key.public_key)?,
            "restored key mismatch"
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_session_keeps_the_current_format_and_all_fields() -> Result<()> {
        let original = json!({
            "format":1,"token_v3_issue_result":{"accessToken":"fake-access","refreshToken":"fake-refresh","expiresIn":123,"extra":{"nested":[true,null]}},
            "certificate":"fake-certificate","login_metadata":{"publicKey":"fake","other":[1,2]},
            "e2ee_keys":[{"id":42,"version":1,"public_key":"fake-public","exported_key":"fake-export","extra":false}],
            "profile":{"mid":"ufake","displayName":"Fake","nested":{"x":1}},"other":[null,"extra"]
        });
        let loaded: Session = serde_json::from_value(original.clone())?;
        assert_eq!(loaded.profile.mid, "ufake");
        let saved = serde_json::to_value(&loaded)?;
        let reloaded: Session = serde_json::from_value(saved.clone())?;
        assert_eq!(saved, original);
        assert_eq!(serde_json::to_value(reloaded)?, original);
        Ok(())
    }
    #[test]
    fn private_directory_forces_only_the_final_directory() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("line-private-dir-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let parent = root.join("ancestor");
        let final_dir = parent.join("private");
        private_directory(&final_dir)?;
        assert_eq!(
            fs::metadata(&final_dir)?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&parent)?.permissions().mode() & 0o777,
            fs::metadata(&root)?.permissions().mode() & 0o777
        );
        fs::set_permissions(&final_dir, fs::Permissions::from_mode(0o755))?;
        private_directory(&final_dir)?;
        assert_eq!(
            fs::metadata(&final_dir)?.permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn qr_callback_encodes_public_key_as_query_parameter() {
        let url = callback("line://au/q/test?session=abc", &[0xfb; 32]).unwrap();
        assert!(url.starts_with("line://au/q/test?session=abc&secret="));
        assert!(url.contains("%2B"));
        assert!(url.ends_with("&e2eeVersion=1"));
        assert!(callback("file:///tmp/test", &[0; 32]).is_err());
    }
}

/// Explicit, request-scoped transport activity shared with detached upload work.
#[derive(Clone, Default)]
pub struct NetworkActivity(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl NetworkActivity {
    pub(crate) fn record(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub(crate) fn touched(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub const TOKEN: &str = "wODdrvWqmdP4Zliay-iF3cz3KZcK0ekrial868apg06TXeCo7A1hIQO0ESElHg6D";

/// Derive and retain the HMAC object for one version/access-token pair.
pub struct SigningKey {
    hmac: u32,
    derived: u32,
}
impl SigningKey {
    pub fn destroy(self, rt: &mut Runtime) -> Result<()> {
        rt.destroy("Hmac", self.hmac)?;
        rt.destroy("SecureKey", self.derived)
    }
}
pub fn signing_key(rt: &mut Runtime, access: &str, client_version: &str) -> Result<SigningKey> {
    let token = rt.string(TOKEN)?;
    let sk = rt.static_call(Call::SecureKeyLoadToken, &[token])?;
    rt.free_string(token)?;
    let cv = rt.bytes(&Sha256::digest(client_version.as_bytes()));
    let at = rt.bytes(&Sha256::digest(access.as_bytes()));
    let dk = rt.method(Call::SecureKeyDeriveKey, sk, &[cv, at])?;
    rt.release(cv)?;
    rt.release(at)?;
    let hmac = rt.construct(Call::HmacNew, &[dk as u64])?;
    rt.destroy("SecureKey", sk)?;
    Ok(SigningKey { hmac, derived: dk })
}
pub fn sign(rt: &mut Runtime, key: &SigningKey, path: &str, body: &str) -> Result<String> {
    let msg = rt.bytes(format!("{path}{body}").as_bytes());
    let out = rt.method(Call::HmacDigest, key.hmac, &[msg])?;
    rt.release(msg)?;
    Ok(STANDARD.encode(rt.output(out)?))
}

pub(crate) fn decode_response(
    path: &str,
    http: u16,
    bytes: &[u8],
) -> anyhow::Result<serde_json::Value> {
    let response: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(response) => response,
        Err(_) if http != 200 => {
            return Err(RpcError {
                gateway: 0,
                method: path.rsplit('/').next().unwrap_or("RPC").into(),
                http,
                code: 0,
                status: 0,
            }
            .into());
        }
        Err(_) => {
            return Err(error::Failure::Bug(format!(
                "response decode failed: status={http} body-length={}",
                bytes.len()
            ))
            .into());
        }
    };
    if http == 200 && response["message"] == "OK" {
        return Ok(response
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Null));
    }
    let gateway = response["code"].as_i64().unwrap_or(if http == 200 {
        error::UNKNOWN_GATEWAY
    } else {
        0
    });
    let code = if gateway == error::TALK_EXCEPTION {
        response["data"]["code"]
            .as_i64()
            .ok_or(error::Failure::Bug("missing TalkException code".into()))?
    } else {
        0
    };
    Err(RpcError {
        gateway,
        method: path.rsplit('/').next().unwrap_or("RPC").into(),
        http,
        code,
        status: response["data"]["statusCode"].as_i64().unwrap_or(0),
    }
    .into())
}
