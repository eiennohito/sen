//! Debug QR pairing against the same endpoints as LINE Chrome.
//! Rust owns protocol/state/WASM; system curl provides HTTPS and qrencode encodes PNG.
use crate::{DEFAULT_WASM, HASH, VERSION, runtime::Runtime, sign};
use anyhow::{Context, Result, bail, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fmt,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BASE: &str = "https://line-chrome-gw.line-apps.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";
const IMAGE_ID: u32 = 47101;

mod chat;
pub mod desktop;
pub mod pairing;

struct Options {
    command: String,
    wasm: PathBuf,
    state: PathBuf,
    qr_file: Option<PathBuf>,
    display: String,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(1);
        let command = args.next().unwrap_or_else(|| "help".into());
        let mut out = Self {
            command,
            wasm: DEFAULT_WASM.into(),
            state: "artifacts/auth/session.json".into(),
            qr_file: None,
            display: if io::stdout().is_terminal() {
                "kitty"
            } else {
                "none"
            }
            .into(),
        };
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("missing value for {arg}"))?;
            match arg.as_str() {
                "--wasm" => out.wasm = value.into(),
                "--state" => out.state = value.into(),
                "--qr-file" => out.qr_file = Some(value.into()),
                "--display" => out.display = value,
                _ => bail!("unknown option {arg}"),
            }
        }
        ensure!(
            ["kitty", "ascii", "none"].contains(&out.display.as_str()),
            "display must be kitty, ascii, or none"
        );
        Ok(out)
    }
}
fn private_write(path: &Path, data: &[u8]) -> Result<()> {
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
struct RpcError {
    method: String,
    http: u16,
    code: i64,
    status: i64,
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
fn quoted(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn curl_request(
    path: &str,
    body: &str,
    headers: &[(&str, &str)],
    timeout: u64,
) -> Result<(u16, Vec<u8>)> {
    ensure!(
        path.starts_with("/api/") && !path.contains(['\r', '\n']),
        "invalid API path"
    );
    let mut config = format!(
        "url = {}\nrequest = \"POST\"\nsilent\nshow-error\nconnect-timeout = 10\nmax-time = {}\nmax-filesize = 1048576\nwrite-out = \"\\n%{{http_code}}\"\ndata = {}\n",
        quoted(&format!("{BASE}{path}")),
        timeout,
        quoted(body)
    );
    for (name, value) in headers {
        ensure!(
            !name.contains(['\r', '\n', ':']) && !value.contains(['\r', '\n']),
            "invalid header"
        );
        config.push_str(&format!(
            "header = {}\n",
            quoted(&format!("{name}: {value}"))
        ));
    }
    // Tokens and pairing secrets stay on stdin, never in process arguments.
    let mut child = Command::new("curl")
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start curl")?;
    child
        .stdin
        .take()
        .context("curl stdin")?
        .write_all(config.as_bytes())?;
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success(),
        "HTTPS transport failed (curl exit {}); response not logged",
        out.status.code().unwrap_or(-1)
    );
    let stdout = out.stdout;
    let split = stdout
        .iter()
        .rposition(|b| *b == b'\n')
        .context("missing HTTP status")?;
    let status = std::str::from_utf8(&stdout[split + 1..])?.trim().parse()?;
    Ok((status, stdout[..split].to_vec()))
}
struct Api {
    rt: Runtime,
    key_cache: std::collections::HashMap<String, Value>,
    channels: std::collections::HashMap<String, (u32, u64)>,
    channel_tick: u64,
}
impl Api {
    fn new(wasm: &[u8]) -> Result<Self> {
        Ok(Self {
            rt: Runtime::new(wasm)?,
            key_cache: Default::default(),
            channels: Default::default(),
            channel_tick: 0,
        })
    }
    fn cached_channel(&mut self, key: &str) -> Option<u32> {
        self.channel_tick += 1;
        let entry = self.channels.get_mut(key)?;
        entry.1 = self.channel_tick;
        Some(entry.0)
    }
    fn cache_channel(&mut self, key: String, channel: u32) -> Result<()> {
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
    fn call(
        &mut self,
        path: &str,
        args: &Value,
        access: &str,
        session: &str,
        poll_seconds: u64,
    ) -> Result<Value> {
        let body = serde_json::to_string(args)?;
        let signature = sign(&mut self.rt, path, &body, access)?;
        let mut headers = vec![
            ("Content-Type", "application/json"),
            ("User-Agent", UA),
            ("x-line-chrome-version", VERSION),
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
        let (http, bytes) = curl_request(
            path,
            &body,
            &headers,
            if poll_seconds > 0 {
                poll_seconds + 10
            } else {
                30
            },
        )?;
        let response: Value = serde_json::from_slice(&bytes)
            .context("server returned invalid JSON; body not logged")?;
        let code = response["code"]
            .as_i64()
            .context("missing LINE response code")?;
        if http != 200 || code != 0 {
            return Err(RpcError {
                method: path.rsplit('/').next().unwrap_or("RPC").into(),
                http,
                code,
                status: response["data"]["statusCode"].as_i64().unwrap_or(0),
            }
            .into());
        }
        Ok(response["data"].clone())
    }
    fn qr(&mut self, method: &str, args: Value, session: &str, poll_seconds: u64) -> Result<Value> {
        let service = if poll_seconds > 0 {
            "SecondaryQrCodeLoginPermitNoticeService"
        } else {
            "SecondaryQrCodeLoginService"
        };
        self.call(
            &format!("/api/talk/thrift/LoginQrCode/{service}/{method}"),
            &json!([args]),
            "",
            session,
            poll_seconds,
        )
    }
    fn profile(&mut self, access: &str) -> Result<Value> {
        self.call(
            "/api/talk/thrift/Talk/TalkService/getProfile",
            &json!([0]),
            access,
            "",
            0,
        )
    }
}
fn bytes_method(
    rt: &mut Runtime,
    class: &str,
    name: &str,
    ptr: u32,
    args: &[u64],
) -> Result<Vec<u8>> {
    let h = rt
        .method(class, name, ptr, args)
        .with_context(|| format!("{class}.{name}"))?;
    rt.output(h)
}
fn decode(value: &str) -> Result<Vec<u8>> {
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(b) = engine.decode(value) {
            return Ok(b);
        }
    }
    bail!("invalid base64 key material")
}
fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty())
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
fn qr_png(text: &str) -> Result<Vec<u8>> {
    let mut child = Command::new("qrencode")
        .args(["-t", "PNG", "-s", "10", "-m", "4", "-o", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start qrencode")?;
    child
        .stdin
        .take()
        .context("QR stdin")?
        .write_all(text.as_bytes())?;
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success() && out.stdout.starts_with(b"\x89PNG\r\n\x1a\n"),
        "QR encoding failed"
    );
    Ok(out.stdout)
}
fn kitty_png(png: &[u8], out: &mut impl Write) -> Result<()> {
    let encoded = STANDARD.encode(png);
    let chunks = encoded.as_bytes().chunks(4096);
    let count = chunks.len();
    for (i, chunk) in chunks.enumerate() {
        let more = u8::from(i + 1 < count);
        if i == 0 {
            write!(
                out,
                "\x1b_Ga=T,f=100,t=d,i={IMAGE_ID},q=2,c=40,r=20,m={more};"
            )?
        } else {
            write!(out, "\x1b_Gm={more};")?
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    out.write_all(b"\r\n")?;
    out.flush()?;
    Ok(())
}
fn show_qr(text: &str, png: &[u8], display: &str) -> Result<()> {
    match display {
        "kitty" => kitty_png(png, &mut io::stdout().lock()),
        "ascii" => {
            let mut child = Command::new("qrencode")
                .args(["-t", "UTF8", "-m", "4"])
                .stdin(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .context("QR stdin")?
                .write_all(text.as_bytes())?;
            ensure!(child.wait()?.success(), "ASCII QR rendering failed");
            Ok(())
        }
        _ => Ok(()),
    }
}
fn save_state(path: &Path, state: &Value) -> Result<()> {
    private_write(path, &serde_json::to_vec_pretty(state)?)
}
fn login_key(rt: &mut Runtime, state: &Value) -> Result<u32> {
    let h = rt.bytes(&decode(required(state, "login_key")?)?);
    let key = rt
        .static_call("Curve25519Key", "loadKey", &[h])
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
        .method("Curve25519Key", "createChannel", key, &[h])
        .context("create phone key-transfer channel")?;
    rt.release(h)?;
    let h = rt.bytes(&decode(required(meta, "encryptedKeyChain")?)?);
    let chain = rt
        .method("E2EEChannel", "unwrapE2EEKeyChain", channel, &[h])
        .context("unwrap phone keychain")?;
    rt.release(h)?;
    let size = rt
        .method("E2EEKeychain", "size", chain, &[])
        .context("keychain size")?;
    ensure!((1..=64).contains(&size), "unexpected keychain size {size}");
    let mut keys = Vec::new();
    for i in 0..size {
        let h = rt
            .method("E2EEKeychain", "get", chain, &[i as u64])
            .context("keychain entry")?;
        let ptr = rt.object(h)?;
        let id = rt
            .method("E2EEKey", "getKeyId", ptr, &[])
            .context("key ID")?;
        let version = rt
            .method("E2EEKey", "getVersion", ptr, &[])
            .context("key version")?;
        let public = bytes_method(rt, "E2EEKey", "getPublicKey", ptr, &[])?;
        let exported = bytes_method(rt, "E2EEKey", "exportKey", ptr, &[])?;
        let h = rt.bytes(&exported);
        let loaded = rt
            .static_call("E2EEKey", "loadKey", &[h])
            .context("reload transferred key")?;
        rt.release(h)?;
        let reloaded = bytes_method(rt, "E2EEKey", "getPublicKey", loaded, &[])?;
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
    required(&state["token_v3_issue_result"], "accessToken")
}
fn validate_keys(rt: &mut Runtime, state: &Value) -> Result<usize> {
    let keys = state["e2ee_keys"]
        .as_array()
        .context("missing saved E2EE keys")?;
    ensure!(!keys.is_empty(), "no saved E2EE keys");
    for key in keys {
        let h = rt.bytes(&decode(required(key, "exported_key")?)?);
        let ptr = rt.static_call("E2EEKey", "loadKey", &[h])?;
        rt.release(h)?;
        let public = bytes_method(rt, "E2EEKey", "getPublicKey", ptr, &[])?;
        ensure!(
            public == decode(required(key, "public_key")?)?,
            "restored key mismatch"
        );
        rt.destroy("E2EEKey", ptr)?;
    }
    Ok(keys.len())
}
fn status(opts: &Options, wasm: &[u8], progress: &impl LoginUi) -> Result<()> {
    let mut state: Value =
        serde_json::from_slice(&fs::read(&opts.state).context("read saved session")?)?;
    ensure!(
        state["wasm_sha256"] == HASH,
        "saved session WASM version mismatch"
    );
    let mut api = Api::new(wasm)?;
    if state["e2ee_keys"].as_array().is_none_or(|k| k.is_empty()) {
        progress.set("Recovering E2EE keys from saved pairing metadata")?;
        let key = login_key(&mut api.rt, &state)?;
        state["e2ee_keys"] = json!(unwrap_keys(&mut api.rt, key, &state["login_metadata"])?);
        api.rt.destroy("Curve25519Key", key)?;
        save_state(&opts.state, &state)?;
    }
    let count = validate_keys(&mut api.rt, &state)?;
    progress.set(&format!(
        "Restored {count} Letter Sealing key(s) in a fresh WASM instance"
    ))?;
    let profile = api.profile(access(&state)?)?;
    ensure!(
        profile["mid"].as_str().is_some_and(|s| !s.is_empty()),
        "profile did not contain a user ID"
    );
    state["validated"] = json!(true);
    save_state(&opts.state, &state)?;
    progress.set("SUCCESS: saved session authenticated and E2EE keys restored")
}
trait LoginUi {
    fn set(&self, stage: &str) -> Result<()>;
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
    fn set(&self, stage: &str) -> Result<()> {
        Progress::set(self, stage)
    }
}
struct CliLogin<'a> {
    opts: &'a Options,
    progress: &'a Progress,
}
impl CliLogin<'_> {
    fn qr_path(&self) -> PathBuf {
        self.opts.qr_file.clone().unwrap_or_else(|| {
            self.opts
                .state
                .parent()
                .unwrap_or(Path::new("."))
                .join("qr.png")
        })
    }
}
impl LoginUi for CliLogin<'_> {
    fn set(&self, stage: &str) -> Result<()> {
        self.progress.set(stage)
    }
    fn qr(&self, link: &str, png: &[u8]) -> Result<()> {
        private_write(&self.qr_path(), png)?;
        println!(
            "Scan with LINE on your phone and approve the login. This uses the Chrome secondary-device slot."
        );
        show_qr(link, png, &self.opts.display)
    }
    fn scanned(&self) -> Result<()> {
        let _ = fs::remove_file(self.qr_path());
        if self.opts.display == "kitty" {
            print!("\x1b_Ga=d,d=I,i={IMAGE_ID},q=2;\x1b\\");
            io::stdout().flush()?;
        }
        Ok(())
    }
    fn pin(&self, pin: &str) -> Result<()> {
        println!("\nEnter this verification code on your phone: {pin}\n");
        io::stdout().flush()?;
        Ok(())
    }
}
fn login(opts: &Options, wasm: &[u8], progress: &Progress) -> Result<()> {
    login_with(opts, wasm, &CliLogin { opts, progress })
}
fn login_with(opts: &Options, wasm: &[u8], progress: &impl LoginUi) -> Result<()> {
    progress.check()?;
    ensure!(
        !opts.state.exists(),
        "session file already exists; use status or choose a new --state path"
    );
    let parent = opts
        .state
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let new_directory = !parent.exists();
    fs::create_dir_all(parent)?;
    if new_directory {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut api = Api::new(wasm)?;
    progress.set("Creating Chrome QR pairing session")?;
    let created = api.qr("createSession", json!({}), "", 0)?;
    let session = required(&created, "authSessionId")?.to_owned();
    let qr = api.qr("createQrCode", json!({"authSessionId":session}), "", 0)?;
    let key = api.rt.static_call("Curve25519Key", "generate", &[1])?;
    let public = bytes_method(&mut api.rt, "Curve25519Key", "getPublicKey", key, &[])?;
    let wrapped = bytes_method(&mut api.rt, "Curve25519Key", "exportKey", key, &[])?;
    let link = callback(required(&qr, "callbackUrl")?, &public)?;
    let png = qr_png(&link)?;
    progress.qr(&link, &png)?;
    progress.set("QR ready; waiting for phone scan")?;
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
                    .is_some_and(|e| e.code == 10052 && e.status == 410);
                if !expired || started.elapsed() > Duration::from_secs(300) {
                    return Err(error);
                }
            }
        }
    }
    ensure!(verified, "QR code expired before phone verification");
    progress.check()?;
    progress.scanned()?;
    progress.set("QR scanned; checking device certificate")?;
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
            progress.set("Waiting for phone PIN verification")?;
            api.qr(
                "checkPinCodeVerified",
                json!({"authSessionId":session}),
                &session,
                110,
            )?;
        }
    }
    progress.check()?;
    progress.set("Phone approved; requesting session credentials")?;
    let result=api.qr("qrCodeLoginV2",json!({"authSessionId":session,"systemName":"CHROMEOS","modelName":"CHROME","autoLoginIsRequired":false}),"",0)?;
    let mut token = result["tokenV3IssueResult"].clone();
    if token["accessToken"].as_str().is_none_or(|s| s.is_empty()) {
        token = json!({"accessToken":required(&result,"authToken")?});
    }
    let mut state = json!({"format":1,"version":VERSION,"wasm_sha256":HASH,"token_v3_issue_result":token,"certificate":result["certificate"],"login_key":STANDARD.encode(wrapped),"login_metadata":result["metaData"],"e2ee_keys":[],"validated":false});
    // Preserve the successful session and wrapped login key before any new
    // keychain operation, so a failure can be diagnosed/recovered with status.
    save_state(&opts.state, &state)?;
    progress.set("Session credentials received; unwrapping Letter Sealing keychain in WASM")?;
    let keys = unwrap_keys(&mut api.rt, key, &state["login_metadata"])?;
    state["e2ee_keys"] = json!(keys);
    save_state(&opts.state, &state)?;
    progress.set("Letter Sealing keys unwrapped and export/import verified")?;
    api.rt.destroy("Curve25519Key", key)?;
    let profile = api.profile(access(&state)?)?;
    ensure!(
        profile["mid"].as_str().is_some_and(|s| !s.is_empty()),
        "authenticated profile missing"
    );
    state["validated"] = json!(true);
    save_state(&opts.state, &state)?;
    progress.set("Authenticated getProfile succeeded; checking restart recovery")?;
    drop(api);
    status(opts, wasm, progress)
}
fn run() -> Result<()> {
    let opts = Options::parse()?;
    if ["help", "--help", "-h"].contains(&opts.command.as_str()) {
        println!(
            "Usage: line-auth login|status|chat-check|render-test [--wasm FILE] [--state FILE] [--qr-file FILE] [--display kitty|ascii|none]\n\nRequires curl; QR rendering also requires qrencode. Defaults to artifacts/auth/session.json.\nSession credentials and wrapped keys are stored in a mode-0600 debug file.\nProgress contains stage names only; tokens, QR URLs and key material are never logged.\nlogin pairs via your phone; status reimports keys and fetches your profile without logging in again.\nchat-check reads up to 10 recent chat previews (5 messages each), stops after one verified decryption, and logs counts only."
        );
        return Ok(());
    }
    if opts.command == "render-test" {
        let text = "LINE WASM QR renderer test";
        let png = qr_png(text)?;
        if let Some(p) = &opts.qr_file {
            private_write(p, &png)?;
        }
        show_qr(text, &png, &opts.display)?;
        return Ok(());
    }
    ensure!(
        ["login", "status", "chat-check"].contains(&opts.command.as_str()),
        "unknown command"
    );
    let wasm = fs::read(&opts.wasm).context("read LINE WASM")?;
    ensure!(
        format!("{:x}", Sha256::digest(&wasm)) == HASH,
        "unsupported WASM release"
    );
    let progress = Progress::new(&opts.state)?;
    let result = match opts.command.as_str() {
        "login" => login(&opts, &wasm, &progress),
        "chat-check" => chat::run(&opts, &wasm, &progress),
        _ => status(&opts, &wasm, &progress),
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
    fn curl_config_keeps_body_and_headers_in_single_values() {
        let raw = "[\"a\\b\",\"quote\\\"\",\"line\\n\"]";
        assert_eq!(
            quoted(raw),
            format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
        );
        assert_eq!(
            quoted("first\r\nheader: secret"),
            "\"first\\r\\nheader: secret\""
        );
    }
    #[test]
    fn kitty_image_chunks_roundtrip_to_original_png() {
        let png = vec![0x42; 12000];
        let mut out = Vec::new();
        kitty_png(&png, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let mut encoded = String::new();
        let messages: Vec<_> = s.split("\x1b_G").skip(1).collect();
        assert!(messages.len() > 1);
        for (i, message) in messages.iter().enumerate() {
            let (controls, data) = message.split_once(';').unwrap();
            let data = data.split("\x1b\\").next().unwrap();
            assert!(data.len() <= 4096 && data.len() % 4 == 0);
            encoded.push_str(data);
            assert!(controls.contains(if i + 1 == messages.len() {
                "m=0"
            } else {
                "m=1"
            }));
        }
        assert_eq!(STANDARD.decode(encoded).unwrap(), png);
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
