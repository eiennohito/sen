use anyhow::{Context, Result, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use line::{
    client::{sign, signing_key},
    runtime::{Call, Runtime},
};
use std::{env, fs, path::Path};

const TO: &str = "U8ae764e8e69e6bd4ecdd9b6ea0c40fce";
const FROM: &str = "Uf1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6";

fn replay_har(rt: &mut Runtime, path: &Path, version: &str) -> Result<usize> {
    let har: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let entries = har["log"]["entries"].as_array().context("HAR entries")?;
    let mut checked = 0;
    for e in entries {
        let req = &e["request"];
        let url = req["url"].as_str().context("request URL")?;
        let Some(path) = url.strip_prefix("https://line-chrome-gw.line-apps.com/") else {
            continue;
        };
        let path = format!("/{path}");
        let headers = req["headers"].as_array().context("request headers")?;
        let get = |name: &str| -> Option<&str> {
            headers
                .iter()
                .find(|h| {
                    h["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .and_then(|h| h["value"].as_str())
        };
        let Some(expected) = get("x-hmac") else {
            continue;
        };
        ensure!(
            get("x-line-chrome-version").unwrap_or(version) == version,
            "unsupported captured version"
        );
        let body = req["postData"]["text"].as_str().unwrap_or("");
        // At startup Chrome signs with an empty token argument even while the
        // browser sends a session cookie. Do not substitute the cookie here.
        let key = signing_key(rt, get("x-line-access").unwrap_or(""), version)?;
        let actual = sign(rt, &key, &path, body)?;
        key.destroy(rt)?;
        ensure!(actual == expected, "signature mismatch for {path}");
        checked += 1;
    }
    ensure!(checked > 0, "HAR contained no signed Chrome requests");
    println!("PASS: {checked}/{checked} captured Chrome request signatures match");
    Ok(checked)
}
fn method_bytes(rt: &mut Runtime, call: Call, this: u32, args: &[u64]) -> Result<Vec<u8>> {
    let h = rt.method(call, this, args)?;
    rt.output(h)
}

struct ChannelPair {
    sender: u32,
    receiver: u32,
    alice: u32,
    bob: u32,
}
impl ChannelPair {
    fn new(rt: &mut Runtime) -> Result<Self> {
        let alice = rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
        let bob = rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
        let alice_pub = method_bytes(rt, Call::Curve25519KeyGetPublicKey, alice, &[])?;
        let bob_pub = method_bytes(rt, Call::Curve25519KeyGetPublicKey, bob, &[])?;
        ensure!(
            alice_pub.len() == 32 && bob_pub.len() == 32,
            "X25519 public-key length"
        );
        let a = rt.bytes(&alice_pub);
        let b = rt.bytes(&bob_pub);
        let sender = rt.method(Call::Curve25519KeyCreateChannel, alice, &[b])?;
        let receiver = rt.method(Call::Curve25519KeyCreateChannel, bob, &[a])?;
        rt.release(a)?;
        rt.release(b)?;
        Ok(Self {
            sender,
            receiver,
            alice,
            bob,
        })
    }
    fn destroy(self, rt: &mut Runtime) -> Result<()> {
        rt.destroy("E2EEChannel", self.sender)?;
        rt.destroy("E2EEChannel", self.receiver)?;
        rt.destroy("Curve25519Key", self.alice)?;
        rt.destroy("Curve25519Key", self.bob)?;
        Ok(())
    }
}

struct Mids {
    to: String,
    from: String,
}
impl Mids {
    fn synthetic(rt: &mut Runtime) -> Result<Self> {
        // Chrome's V2 API accepts opaque encrypted MIDs, not ordinary LINE MIDs.
        // For this offline test only, mint valid opaque IDs with the embedded
        // MID key. 7376 is a release-specific immutable-data offset in 3.7.2.
        // A real client passes the opaque IDs returned by LINE directly.
        let wrapped = rt.read_memory(7376, 193)?;
        let h = rt.bytes(&wrapped);
        let sk = rt.static_call(Call::SecureKeyLoadKey, &[h])?;
        rt.release(h)?;
        let aes = rt.construct(Call::AesKeyNew, &[sk as u64])?;
        let mut encode = |mid: &str| -> Result<String> {
            let raw = (1..33)
                .step_by(2)
                .map(|i| Ok(u8::from_str_radix(&mid[i..i + 2], 16)?))
                .collect::<Result<Vec<u8>>>()?;
            let h = rt.bytes(&raw);
            let mut ct = method_bytes(rt, Call::AesKeyEncrypt, aes, &[h])?;
            rt.release(h)?;
            ensure!(ct.len() == 44, "opaque MID ciphertext size");
            // MID obfuscation uses the first 4 bytes of the GCM tag.
            ct.truncate(ct.len() - 12);
            Ok(format!(
                "{}{}",
                mid[..1].to_ascii_uppercase(),
                URL_SAFE_NO_PAD.encode(ct)
            ))
        };
        let to = encode(TO)?;
        let from = encode(FROM)?;
        rt.destroy("AesKey", aes)?;
        rt.destroy("SecureKey", sk)?;
        ensure!(to.len() == 44 && from.len() == 44, "opaque MID lengths");

        Ok(Self { to, from })
    }
}
fn channel_crypt(
    rt: &mut Runtime,
    call: Call,
    ch: u32,
    mids: &Mids,
    data: &[u8],
    seq: Option<u64>,
    sender_id: u64,
) -> Result<Vec<u8>> {
    if call == Call::E2EEChannelDecryptV2 {
        ensure!(data.len() >= 44, "V2 ciphertext too short");
    }
    let to = rt.string(&mids.to)?;
    let from = rt.string(&mids.from)?;
    let h = rt.bytes(data);
    let mut args = vec![to, from, sender_id, 99, 0];
    if let Some(n) = seq {
        args.push(n)
    }
    args.push(h);
    let out = rt.method(call, ch, &args)?;
    // On a WASM exception Runtime is poisoned and must be discarded. The
    // early return above deliberately avoids calling into an unwound stack.
    rt.free_string(to)?;
    rt.free_string(from)?;
    rt.release(h)?;
    rt.output(out)
}
fn roundtrips(rt: &mut Runtime, mids: &Mids) -> Result<()> {
    let empty = rt.bytes(&[]);
    let aes = rt.static_call(Call::AesKeyGenerate, &[32, empty])?;
    rt.release(empty)?;
    let plaintext = "Native Rust + original LINE WASM: 日本語 🔐\0binary".as_bytes();
    let h = rt.bytes(plaintext);
    let encrypted = method_bytes(rt, Call::AesKeyEncrypt, aes, &[h])?;
    rt.release(h)?;
    let h = rt.bytes(&encrypted);
    let decrypted = method_bytes(rt, Call::AesKeyDecrypt, aes, &[h])?;
    rt.release(h)?;
    ensure!(decrypted == plaintext, "AES roundtrip");
    rt.destroy("AesKey", aes)?;
    println!("PASS: WASM AesKey encrypt/decrypt (Unicode and embedded NUL)");
    let pair = ChannelPair::new(rt)?;
    let message = br#"{"text":"Hello from a browser-free Rust WASM host"}"#;
    let h = rt.bytes(message);
    let encrypted = method_bytes(rt, Call::E2EEChannelEncryptV1, pair.sender, &[h])?;
    rt.release(h)?;
    let h = rt.bytes(&encrypted);
    let decrypted = method_bytes(rt, Call::E2EEChannelDecryptV1, pair.receiver, &[h])?;
    rt.release(h)?;
    ensure!(decrypted == message, "Letter Sealing V1 roundtrip");
    println!("PASS: WASM X25519 + Letter Sealing V1 across two independent keys");
    // Exercise the full i64 ABI rather than silently truncating to i32.
    let sequence = 0x1_0000_0001;
    let encrypted = channel_crypt(
        rt,
        Call::E2EEChannelEncryptV2,
        pair.sender,
        mids,
        message,
        Some(sequence),
        42,
    )?;
    ensure!(
        encrypted.len() == message.len() + 44,
        "V2 ciphertext format"
    );
    ensure!(
        u64::from_be_bytes(encrypted[16..24].try_into()?) == sequence,
        "64-bit sequence preservation"
    );
    let decrypted = channel_crypt(
        rt,
        Call::E2EEChannelDecryptV2,
        pair.receiver,
        mids,
        &encrypted,
        None,
        42,
    )?;
    ensure!(decrypted == message, "Letter Sealing V2 roundtrip");
    println!("PASS: WASM X25519 + Letter Sealing V2 (64-bit sequence preserved)");
    pair.destroy(rt)?;
    Ok(())
}
fn fixture() -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(include_bytes!(
        "../fixtures/letter-sealing-v2.json"
    ))?)
}
fn fixture_bytes(f: &serde_json::Value, name: &str) -> Result<Vec<u8>> {
    Ok(STANDARD.decode(f[name].as_str().context("fixture field")?)?)
}
fn fixture_channel(rt: &mut Runtime, f: &serde_json::Value) -> Result<(u32, u32)> {
    let h = rt.bytes(&fixture_bytes(f, "exported_key")?);
    let key = rt.static_call(Call::Curve25519KeyLoadKey, &[h])?;
    rt.release(h)?;
    let pubkey = method_bytes(rt, Call::Curve25519KeyGetPublicKey, key, &[])?;
    ensure!(
        pubkey == fixture_bytes(f, "public_key")?,
        "imported fixture public key"
    );
    let peer = rt.bytes(&fixture_bytes(f, "peer_public_key")?);
    let channel = rt.method(Call::Curve25519KeyCreateChannel, key, &[peer])?;
    rt.release(peer)?;
    Ok((key, channel))
}
fn external_fixture(rt: &mut Runtime, mids: &Mids) -> Result<()> {
    let f = fixture()?;
    ensure!(f["to"] == TO && f["from"] == FROM, "fixture identities");
    let (key, channel) = fixture_channel(rt, &f)?;
    let expected = f["plaintext"]
        .as_str()
        .context("fixture plaintext")?
        .as_bytes();
    let h = rt.bytes(&fixture_bytes(&f, "ciphertext_v1")?);
    let plaintext = method_bytes(rt, Call::E2EEChannelDecryptV1, channel, &[h])?;
    rt.release(h)?;
    ensure!(plaintext == expected, "independent V1 plaintext");

    let ciphertext = fixture_bytes(&f, "ciphertext")?;
    let plaintext = channel_crypt(
        rt,
        Call::E2EEChannelDecryptV2,
        channel,
        mids,
        &ciphertext,
        None,
        42,
    )?;
    ensure!(plaintext == expected, "independent V2 plaintext");
    println!("PASS: original WASM decrypted independent Go V1 and V2 ciphertext fixtures");
    println!("  {}", std::str::from_utf8(&plaintext)?);
    rt.destroy("E2EEChannel", channel)?;
    rt.destroy("Curve25519Key", key)?;
    Ok(())
}
fn rejection_checks(wasm: &[u8], version: &str) -> Result<()> {
    let f = fixture()?;
    for (label, mutate, sender_id) in [
        ("tampered authentication tag", true, 42),
        ("wrong sender key ID (AAD)", false, 43),
    ] {
        let mut rt = Runtime::new(wasm, version)?;
        let mids = Mids::synthetic(&mut rt)?;
        let (_, channel) = fixture_channel(&mut rt, &f)?;
        let mut ct = fixture_bytes(&f, "ciphertext")?;
        if mutate {
            *ct.last_mut().context("empty ciphertext")? ^= 1;
        }
        let error = channel_crypt(
            &mut rt,
            Call::E2EEChannelDecryptV2,
            channel,
            &mids,
            &ct,
            None,
            sender_id,
        )
        .expect_err("invalid ciphertext accepted");
        ensure!(
            format!("{error:#}").contains("authentication failure"),
            "unexpected rejection reason: {error:#}"
        );
        ensure!(
            rt.static_call(Call::Curve25519KeyGenerate, &[1]).is_err(),
            "exception did not poison instance"
        );
        println!("PASS: WASM rejects {label}; failed instance cannot be reused");
    }
    Ok(())
}
fn export_test_key(wasm: &[u8], path: &str, version: &str) -> Result<()> {
    let mut rt = Runtime::new(wasm, version)?;
    let key = rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
    let public = method_bytes(&mut rt, Call::Curve25519KeyGetPublicKey, key, &[])?;
    let exported = method_bytes(&mut rt, Call::Curve25519KeyExportKey, key, &[])?;
    fs::write(
        path,
        serde_json::to_vec_pretty(
            &serde_json::json!({"exported_key":STANDARD.encode(exported),"public_key":STANDARD.encode(public)}),
        )?,
    )?;
    println!("Wrote synthetic test key to {path}");
    Ok(())
}
fn benchmark(wasm: &[u8], version: &str) -> Result<()> {
    use std::time::Instant;
    const COUNT: u32 = 20;
    let mut rt = Runtime::new(wasm, version)?;
    let mids = Mids::synthetic(&mut rt)?;
    let fixture = fixture()?;
    let plaintext = fixture["plaintext"]
        .as_str()
        .context("fixture text")?
        .as_bytes();
    // Compare the previous send lifetime (import + channel per send) with the
    // session's retained channel. RPC time is excluded from this offline probe.
    let load_channel = |rt: &mut Runtime| -> Result<(u32, u32)> {
        let h = rt.bytes(&fixture_bytes(&fixture, "exported_key")?);
        let key = rt.static_call(Call::Curve25519KeyLoadKey, &[h])?;
        rt.release(h)?;
        let h = rt.bytes(&fixture_bytes(&fixture, "peer_public_key")?);
        let ch = rt.method(Call::Curve25519KeyCreateChannel, key, &[h])?;
        rt.release(h)?;
        Ok((key, ch))
    };
    let started = Instant::now();
    for seq in 1..=COUNT {
        let (key, ch) = load_channel(&mut rt)?;
        channel_crypt(
            &mut rt,
            Call::E2EEChannelEncryptV2,
            ch,
            &mids,
            plaintext,
            Some(seq.into()),
            42,
        )?;
        rt.destroy("E2EEChannel", ch)?;
        rt.destroy("Curve25519Key", key)?;
    }
    let send_before = started.elapsed().as_secs_f64() * 1000. / f64::from(COUNT);
    let (key, ch) = load_channel(&mut rt)?;
    let started = Instant::now();
    for seq in 1..=COUNT {
        channel_crypt(
            &mut rt,
            Call::E2EEChannelEncryptV2,
            ch,
            &mids,
            plaintext,
            Some(seq.into()),
            42,
        )?;
    }
    let send_after = started.elapsed().as_secs_f64() * 1000. / f64::from(COUNT);
    let history =
        line::client::harness::benchmark_history(wasm, version, &fixture, &mids.to, &mids.from)?;
    let started = Instant::now();
    let expected_key = signing_key(&mut rt, "synthetic-access", version)?;
    let expected = sign(&mut rt, &expected_key, "/synthetic", "{}")?;
    expected_key.destroy(&mut rt)?;
    for _ in 0..COUNT {
        let hmac = signing_key(&mut rt, "synthetic-access", version)?;
        ensure!(
            sign(&mut rt, &hmac, "/synthetic", "{}")? == expected,
            "signature mismatch"
        );
        hmac.destroy(&mut rt)?;
    }
    let signing_before = started.elapsed().as_secs_f64() * 1000. / f64::from(COUNT + 1);
    let hmac = signing_key(&mut rt, "synthetic-access", version)?;
    let started = Instant::now();
    for _ in 0..COUNT {
        ensure!(
            sign(&mut rt, &hmac, "/synthetic", "{}")? == expected,
            "cached signature mismatch"
        );
    }
    let signing_after = started.elapsed().as_secs_f64() * 1000. / f64::from(COUNT);
    hmac.destroy(&mut rt)?;
    rt.destroy("E2EEChannel", ch)?;
    rt.destroy("Curve25519Key", key)?;
    println!("send_encrypt_ms={send_before:.2} -> {send_after:.2}");
    println!("history_45_decrypt_5_missing_keys_ms={history:.2}");
    println!("request_sign_ms={signing_before:.2} -> {signing_after:.2}");
    println!(
        "Offline fixture, release build; previous/current object lifetimes, no RPC time. History includes five missing device keys and retains its channel."
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "--help" || s == "-h") {
        println!(
            "Usage: crypto-harness [WASM [HAR]]\n       crypto-harness --export-test-key OUTPUT_JSON [WASM]\n       crypto-harness --bench [WASM]"
        );
        return Ok(());
    }
    let export = args.first().is_some_and(|s| s == "--export-test-key");
    let bench = args.first().is_some_and(|s| s == "--bench");
    let override_path = if export {
        args.get(2)
    } else if bench {
        args.get(1)
    } else {
        args.first()
    };
    let module = line::crypto_module::resolve(override_path.map(Into::into), None)?;
    let version = module.version.as_str();
    let bytes = module.read()?;
    if export {
        return export_test_key(
            &bytes,
            args.get(1).context("missing output filename")?,
            version,
        );
    }
    if bench {
        ensure!(args.len() <= 2, "usage: crypto-harness --bench [WASM]");
        return benchmark(&bytes, version);
    }
    ensure!(args.len() <= 2, "usage: crypto-harness [WASM [HAR]]");
    println!("LINE Chrome {version}, installed crypto module");
    let mut rt = Runtime::new(&bytes, version)?;
    println!(
        "PASS: instantiated WASM and discovered {} embind classes",
        rt.bindings().len()
    );
    if let Some(har) = args.get(1) {
        replay_har(&mut rt, Path::new(har), version)?;
    }
    let mids = Mids::synthetic(&mut rt)?;
    external_fixture(&mut rt, &mids)?;
    roundtrips(&mut rt, &mids)?;
    rejection_checks(&bytes, version)?;
    println!(
        "All checks passed. Rust + Wasmi; no JavaScript engine, translated crypto, or LINE network calls."
    );
    Ok(())
}
