//! Debug QR pairing against the same endpoints as LINE Chrome.
//! Rust owns protocol/state/WASM, all HTTPS (RPC, SSE, media) and QR encoding.
pub mod error;
use crate::{
    locations::Locations,
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
    env, fmt,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
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

struct Options {
    command: String,
    wasm_override: Option<PathBuf>,
    version_override: Option<String>,
    state: PathBuf,
    qr_file: Option<PathBuf>,
    display: String,
}
impl Options {
    fn parse() -> Result<Self> {
        Self::parse_from(env::args().skip(1), io::stdout().is_terminal(), || {
            Locations::from_environment().map_err(anyhow::Error::msg)
        })
    }
    fn parse_from(
        mut args: impl Iterator<Item = String>,
        terminal: bool,
        resolve_locations: impl FnOnce() -> Result<Locations>,
    ) -> Result<Self> {
        let command = args.next().unwrap_or_else(|| "help".into());
        let mut wasm = None;
        let mut client_version = None;
        let mut state = None;
        let mut qr_file = None;
        let mut display = String::from(if terminal { "ascii" } else { "none" });
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("missing value for {arg}"))?;
            match arg.as_str() {
                "--client-version" => client_version = Some(value),
                "--wasm" => wasm = Some(PathBuf::from(value)),
                "--state" => state = Some(PathBuf::from(value)),
                "--qr-file" => qr_file = Some(value.into()),
                "--display" => display = value,
                _ => bail!("unknown option {arg}"),
            }
        }
        ensure!(
            ["ascii", "none"].contains(&display.as_str()),
            "display must be ascii or none"
        );
        if ["login", "status", "chat-check"].contains(&command.as_str()) && state.is_none() {
            let locations = resolve_locations()?;
            state.get_or_insert(locations.session);
        }
        Ok(Self {
            command,
            version_override: client_version,
            wasm_override: wasm,
            state: state.unwrap_or_default(),
            qr_file,
            display,
        })
    }
}
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
struct Progress {
    path: PathBuf,
}
impl Progress {
    fn new(state: &Path) -> Result<Self> {
        let path = state.with_extension("progress.json");
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let new_directory = !parent.exists();
            fs::create_dir_all(parent)?;
            if new_directory {
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(Self { path })
    }
    fn set(&self, stage: &str) -> Result<()> {
        println!("{stage}");
        io::stdout().flush()?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        private_write(
            &self.path,
            &serde_json::to_vec(&json!({"stage":stage,"unix_time":timestamp}))?,
        )
    }
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
fn show_qr(text: &str, display: &str) -> Result<()> {
    if display == "ascii" {
        let mut stdout = io::stdout().lock();
        let colour = stdout.is_terminal();
        qr::write_text(text, &mut stdout, colour)?;
    }
    Ok(())
}
fn save_state(path: &Path, state: &Value) -> Result<()> {
    crate::credentials::save(path, state)
}
fn login_key(rt: &mut Runtime, state: &Value) -> Result<u32> {
    let h = rt.bytes(&decode(required(state, "login_key")?)?);
    let key = rt
        .static_call(Call::Curve25519KeyLoadKey, &[h])
        .context("restore pairing key")?;
    rt.release(h)?;
    Ok(key)
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
fn status(
    opts: &Options,
    wasm: &[u8],
    module: &crate::crypto_module::CryptoModule,
    progress: &impl LoginUi,
) -> Result<()> {
    let mut state = crate::credentials::load(&opts.state)?;
    let mut api = Api::with_runtime(module.runtime(wasm)?, wasm, &module.version)?;
    if state["e2ee_keys"].as_array().is_none_or(|k| k.is_empty()) {
        progress.set(pairing::Stage::RecoveringKeys)?;
        let key = login_key(&mut api.rt, &state)?;
        state["e2ee_keys"] = json!(unwrap_keys(&mut api.rt, key, &state["login_metadata"])?);
        api.rt.destroy("Curve25519Key", key)?;
        save_state(&opts.state, &state)?;
    }
    let count =
        validate_keys(&mut api, &state).context(error::SavedSessionFailure::Keys.error())?;
    progress.set(pairing::Stage::RestoredKeys(count))?;
    api.state = state.clone();
    let profile = api.profile()?;
    ensure!(
        profile["mid"].as_str().is_some_and(|s| !s.is_empty()),
        "profile did not contain a user ID"
    );
    state["profile"] = profile;
    state["validated"] = json!(true);
    save_state(&opts.state, &state)?;
    progress.set(pairing::Stage::Authenticated)
}
trait LoginUi {
    fn set(&self, stage: pairing::Stage) -> Result<()>;
    fn qr(&self, _link: &str, _png: &[u8]) -> Result<()> {
        Ok(())
    }
    fn scanned(&self) -> Result<()> {
        Ok(())
    }
    fn pin(&self, _pin: &str) -> Result<()> {
        Ok(())
    }
    fn check(&self) -> Result<()> {
        Ok(())
    }
}
impl LoginUi for Progress {
    fn set(&self, stage: pairing::Stage) -> Result<()> {
        Progress::set(self, &stage.to_string())
    }
}
struct CliLogin<'a> {
    state: &'a Path,
    qr_file: Option<&'a Path>,
    display: &'a str,
    progress: &'a Progress,
}
impl CliLogin<'_> {
    fn qr_path(&self) -> PathBuf {
        self.qr_file
            .map(Path::to_owned)
            .unwrap_or_else(|| self.state.parent().unwrap_or(Path::new(".")).join("qr.png"))
    }
}
impl LoginUi for CliLogin<'_> {
    fn set(&self, stage: pairing::Stage) -> Result<()> {
        self.progress.set(&stage.to_string())
    }
    fn qr(&self, link: &str, png: &[u8]) -> Result<()> {
        private_write(&self.qr_path(), png)?;
        println!(
            "Scan with LINE on your phone and approve the login. This uses the Chrome secondary-device slot."
        );
        show_qr(link, self.display)
    }
    fn scanned(&self) -> Result<()> {
        let _ = fs::remove_file(self.qr_path());
        Ok(())
    }
    fn pin(&self, pin: &str) -> Result<()> {
        println!("\nEnter this verification code on your phone: {pin}\n");
        io::stdout().flush()?;
        Ok(())
    }
}
fn login(
    opts: &Options,
    wasm: &[u8],
    module: &crate::crypto_module::CryptoModule,
    progress: &Progress,
) -> Result<()> {
    let mut api = Api::with_runtime(module.runtime(wasm)?, wasm, &module.version)?;
    login_with(
        &opts.state,
        &mut api,
        &CliLogin {
            state: &opts.state,
            qr_file: opts.qr_file.as_deref(),
            display: &opts.display,
            progress,
        },
    )
}
fn login_with(state_path: &Path, api: &mut Api, progress: &impl LoginUi) -> Result<()> {
    progress.check()?;
    ensure!(
        !state_path.exists(),
        "session file already exists; use status or choose a new --state path"
    );
    let parent = state_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let new_directory = !parent.exists();
    fs::create_dir_all(parent)?;
    if new_directory {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    progress.set(pairing::Stage::CreatingSession)?;
    let created = api.qr("createSession", json!({}), "", 0)?;
    let session = required(&created, "authSessionId")?.to_owned();
    let qr = api.qr("createQrCode", json!({"authSessionId":session}), "", 0)?;
    let key = api.rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
    let public = bytes_method(&mut api.rt, Call::Curve25519KeyGetPublicKey, key, &[])?;
    let wrapped = bytes_method(&mut api.rt, Call::Curve25519KeyExportKey, key, &[])?;
    let link = callback(required(&qr, "callbackUrl")?, &public)?;
    let png = qr_png(&link)?;
    progress.qr(&link, &png)?;
    progress.set(pairing::Stage::AwaitScan)?;
    let interval = qr["longPollingIntervalSec"]
        .as_u64()
        .unwrap_or(30)
        .clamp(1, 110);
    let count = qr["longPollingMaxCount"]
        .as_u64()
        .unwrap_or(4)
        .clamp(1, 120);
    let started = Instant::now();
    let mut verified = false;
    for _ in 0..count {
        progress.check()?;
        match api.qr(
            "checkQrCodeVerified",
            json!({"authSessionId":session}),
            &session,
            interval,
        ) {
            Ok(_) => {
                verified = true;
                break;
            }
            Err(error) => {
                let expired = error
                    .downcast_ref::<RpcError>()
                    .is_some_and(|e| e.gateway == 10052 && e.status == 410);
                if !expired || started.elapsed() > Duration::from_secs(300) {
                    return Err(pairing::phone_error(error));
                }
            }
        }
    }
    ensure!(verified, error::Error::SignIn(error::SignInFailure::Phone));
    progress.check()?;
    progress.scanned()?;
    progress.set(pairing::Stage::CheckingCertificate)?;
    match api.qr(
        "verifyCertificate",
        json!({"authSessionId":session,"certificate":""}),
        "",
        0,
    ) {
        Ok(_) => {}
        Err(error) => {
            ensure!(
                error
                    .downcast_ref::<RpcError>()
                    .is_some_and(|e| e.http == 400),
                "certificate check failed: {error}"
            );
            let pin = api.qr("createPinCode", json!({"authSessionId":session}), "", 0)?;
            let pin = required(&pin, "pinCode")?;
            ensure!(
                (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit()),
                "invalid verification PIN format"
            );
            progress.pin(pin)?;
            progress.set(pairing::Stage::AwaitPin)?;
            api.qr(
                "checkPinCodeVerified",
                json!({"authSessionId":session}),
                &session,
                110,
            )
            .map_err(pairing::phone_error)?;
        }
    }
    progress.check()?;
    progress.set(pairing::Stage::RequestCredentials)?;
    let result=api.qr("qrCodeLoginV2",json!({"authSessionId":session,"systemName":"CHROMEOS","modelName":"CHROME","autoLoginIsRequired":false}),"",0)?;
    let mut token = result["tokenV3IssueResult"].clone();
    if token["accessToken"].as_str().is_none_or(|s| s.is_empty()) {
        token = json!({"accessToken":required(&result,"authToken")?});
    }
    let mut state = json!({"format":1,"token_v3_issue_result":token,"certificate":result["certificate"],"login_key":STANDARD.encode(wrapped),"login_metadata":result["metaData"],"e2ee_keys":[],"validated":false});
    // Preserve the successful session and wrapped login key before any new
    // keychain operation, so a failure can be diagnosed/recovered with status.
    save_state(state_path, &state)?;
    progress.set(pairing::Stage::UnwrappingKeys)?;
    let keys = unwrap_keys(&mut api.rt, key, &state["login_metadata"])?;
    state["e2ee_keys"] = json!(keys);
    save_state(state_path, &state)?;
    progress.set(pairing::Stage::KeysVerified)?;
    api.rt.destroy("Curve25519Key", key)?;
    api.state = state.clone();
    let profile = api.profile()?;
    ensure!(
        profile["mid"].as_str().is_some_and(|s| !s.is_empty()),
        "authenticated profile missing"
    );
    state["profile"] = profile;
    state["validated"] = json!(true);
    save_state(state_path, &state)?;
    progress.set(pairing::Stage::CheckingKeys)?;
    validate_keys(api, &state)?;
    progress.set(pairing::Stage::Authenticated)
}
fn run() -> Result<()> {
    let mut opts = Options::parse()?;
    if ["help", "--help", "-h"].contains(&opts.command.as_str()) {
        println!(
            "Usage: line-auth login|status|chat-check|render-test [--wasm FILE] [--client-version VERSION] [--state FILE] [--qr-file FILE] [--display ascii|none]\n\nDefault session: $XDG_DATA_HOME/line-desktop/session.json (fallback $HOME/.local/share/line-desktop/session.json).\nDefault WASM: newest installed LINE for Chrome extension (Chrome/Chromium profiles under $XDG_CONFIG_HOME, default $HOME/.config).\n--display defaults to ascii on a TTY, none otherwise; --qr-file writes a PNG.\nSession credentials and wrapped keys are sealed in a mode-0600 file whose key is kept in the desktop keyring (Secret Service).\nLINE_DESKTOP_CREDENTIALS=plaintext-file keeps an unsealed file instead (debug and agent instances without a keyring).\nProgress contains stage names only; tokens, QR URLs and key material are never logged.\nlogin pairs via your phone; status reimports keys and fetches your profile without logging in again.\nchat-check reads up to 10 recent chat previews (5 messages each), stops after one verified decryption, and logs counts only."
        );
        return Ok(());
    }
    if opts.command == "render-test" {
        let text = "LINE WASM QR renderer test";
        let png = qr_png(text)?;
        if let Some(p) = &opts.qr_file {
            private_write(p, &png)?;
        }
        show_qr(text, &opts.display)?;
        return Ok(());
    }
    ensure!(
        ["login", "status", "chat-check"].contains(&opts.command.as_str()),
        "unknown command"
    );
    let module =
        crate::crypto_module::resolve(opts.wasm_override.take(), opts.version_override.take())?;
    let wasm = module.read()?;
    let progress = Progress::new(&opts.state)?;
    let result = match opts.command.as_str() {
        "login" => login(&opts, &wasm, &module, &progress),
        "chat-check" => chat::run(&opts, &wasm, &module, &progress),
        _ => status(&opts, &wasm, &module, &progress),
    };
    if result.is_err() {
        progress.set(
            "FAILED: see terminal error; saved credentials, if present, can be retried with status",
        )?;
    }
    result
}
pub fn cli_main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_defaults_overrides_and_display_modes() -> Result<()> {
        let root = env::temp_dir().join(format!("line-cli-paths-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let locations = Locations {
            config_home: "/config".into(),
            session: root.join("data/session.json"),
            cache: root.join("cache"),
        };
        let parse = |args: &[&str], terminal| {
            Options::parse_from(args.iter().map(|s| (*s).into()), terminal, || {
                Ok(locations.clone())
            })
        };
        for terminal in [true, false] {
            let opts = parse(&["status"], terminal)?;
            assert_eq!(opts.state, locations.session);
            assert!(opts.wasm_override.is_none());
            assert_eq!(opts.display, if terminal { "ascii" } else { "none" });
        }
        let opts = parse(
            &[
                "login",
                "--state",
                "custom.json",
                "--wasm",
                "custom.wasm",
                "--qr-file",
                "custom.png",
                "--display",
                "none",
            ],
            true,
        )?;
        assert_eq!(opts.state, Path::new("custom.json"));
        assert_eq!(
            opts.wasm_override.as_deref(),
            Some(Path::new("custom.wasm"))
        );
        assert_eq!(opts.qr_file.as_deref(), Some(Path::new("custom.png")));
        assert_eq!(opts.display, "none");
        assert_eq!(
            parse(&["status", "--display", "ascii"], false)?.display,
            "ascii"
        );
        assert!(parse(&["status", "--display", "graphics"], true).is_err());
        assert!(parse(&["status", "--state"], false).is_err());
        let parse = |args: &[&str]| {
            Options::parse_from(args.iter().map(|s| (*s).into()), false, || {
                Ok(locations.clone())
            })
        };
        let opts = parse(&["status"])?;
        assert_eq!(opts.state, locations.session);
        assert!(opts.wasm_override.is_none());
        let opts = parse(&["status", "--state", "custom.json"])?;
        assert_eq!(opts.state, Path::new("custom.json"));
        assert!(opts.wasm_override.is_none());
        let opts = parse(&["status", "--wasm", "custom.wasm"])?;
        assert_eq!(
            opts.wasm_override.as_deref(),
            Some(Path::new("custom.wasm"))
        );
        assert_eq!(opts.state, locations.session);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn cli_resolves_locations_only_when_a_default_is_needed() -> Result<()> {
        let parse = |args: &[&str]| {
            let called = std::cell::Cell::new(false);
            let result = Options::parse_from(args.iter().map(|s| (*s).into()), false, || {
                called.set(true);
                bail!("HOME must be an absolute path")
            });
            (result, called.get())
        };
        for args in [
            vec![],
            vec!["help"],
            vec!["--help"],
            vec!["-h"],
            vec!["render-test"],
            vec!["render-test", "--qr-file", "test.png"],
        ] {
            let (result, called) = parse(&args);
            result?;
            assert!(!called);
        }
        for command in ["login", "status", "chat-check"] {
            let (result, called) =
                parse(&[command, "--state", "custom.json", "--wasm", "custom.wasm"]);
            let opts = result?;
            assert!(!called);
            assert_eq!(opts.state, Path::new("custom.json"));
            assert_eq!(
                opts.wasm_override.as_deref(),
                Some(Path::new("custom.wasm"))
            );
            for args in [vec![command], vec![command, "--wasm", "custom.wasm"]] {
                let (result, called) = parse(&args);
                assert!(called);
                assert_eq!(
                    result.err().unwrap().to_string(),
                    "HOME must be an absolute path"
                );
            }
        }
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

thread_local! {
    static NETWORK_TOUCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn network_touched() {
    NETWORK_TOUCHED.set(true);
}
pub(crate) fn take_network_touched() -> bool {
    NETWORK_TOUCHED.replace(false)
}
