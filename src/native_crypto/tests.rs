use super::*;
use crate::{HASH, runtime::Runtime};
use anyhow::Context;
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::Value;
use std::{fs, path::Path, time::Instant};

const CASES: usize = 320;
const TO: &str = "u8ae764e8e69e6bd4ecdd9b6ea0c40fce";
const FROM: &str = "uf1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6";

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/letter-sealing-v2.json")).unwrap()
}

fn decode(value: &Value) -> Vec<u8> {
    STANDARD.decode(value.as_str().unwrap()).unwrap()
}

fn wasm(test_name: &str) -> Result<Vec<u8>> {
    let path = std::env::var_os("LINE_CRYPTO_ORACLE_PATH")
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| crate::locations::Locations::from_environment().map(|paths| paths.wasm))
        .map_err(anyhow::Error::msg)?;
    wasm_at(&path).with_context(|| format!("SKIP native crypto oracle: {test_name}: fixture missing or invalid; set LINE_CRYPTO_ORACLE_PATH"))
}

fn wasm_at(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == HASH,
        "oracle hash"
    );
    Ok(bytes)
}

struct Inputs(u64);
impl Inputs {
    fn new() -> Self {
        Self(0x7439_6372_7970_746f)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }

    fn message(&mut self, case: usize) -> Vec<u8> {
        let len = [
            0, 1, 2, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256, 257, 1024, 4096, 65536,
        ]
        .get(case)
        .copied()
        .unwrap_or_else(|| self.next() as usize % 2049);
        self.bytes(len)
    }
}

struct Oracle {
    rt: Runtime,
    channel: u32,
    to: String,
    from: String,
}

impl Oracle {
    fn new(wasm: &[u8]) -> Result<Self> {
        let f = fixture();
        let mut rt = Runtime::new(wasm)?;
        let h = rt.bytes(&decode(&f["exported_key"]));
        let key = rt.static_call("Curve25519Key", "loadKey", &[h])?;
        rt.release(h)?;
        let h = rt.bytes(&decode(&f["peer_public_key"]));
        let channel = rt.method("Curve25519Key", "createChannel", key, &[h])?;
        rt.release(h)?;
        rt.destroy("Curve25519Key", key)?;
        let wrapped = rt.read_memory(7376, 193)?;
        let h = rt.bytes(&wrapped);
        let mid_key = rt.static_call("SecureKey", "loadKey", &[h])?;
        rt.release(h)?;
        let aes = rt.construct("AesKey", &[mid_key as u64])?;
        let mut mid = |mid: &str| -> Result<String> {
            let raw = (1..33)
                .step_by(2)
                .map(|i| u8::from_str_radix(&mid[i..i + 2], 16))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut ct = Self::bytes(&mut rt, "AesKey", "encrypt", aes, &raw)?;
            ensure!(ct.len() == 44, "opaque MID length");
            ct.truncate(32);
            Ok(format!(
                "{}{}",
                mid[..1].to_ascii_uppercase(),
                URL_SAFE_NO_PAD.encode(ct)
            ))
        };
        let to = mid(TO)?;
        let from = mid(FROM)?;
        rt.destroy("AesKey", aes)?;
        rt.destroy("SecureKey", mid_key)?;
        Ok(Self {
            rt,
            channel,
            to,
            from,
        })
    }

    fn bytes(rt: &mut Runtime, class: &str, name: &str, ptr: u32, data: &[u8]) -> Result<Vec<u8>> {
        let h = rt.bytes(data);
        let out = rt.method(class, name, ptr, &[h])?;
        rt.release(h)?;
        rt.output(out)
    }

    fn crypt(&mut self, name: &str, data: &[u8]) -> Result<Vec<u8>> {
        Self::bytes(&mut self.rt, "E2EEChannel", name, self.channel, data)
    }

    fn derive(&mut self, prefix: &[u8], suffix: &[u8]) -> Result<u32> {
        let p = self.rt.bytes(prefix);
        let s = self.rt.bytes(suffix);
        let key = self
            .rt
            .method("SecureKey", "deriveKey", self.channel, &[p, s])?;
        self.rt.release(p)?;
        self.rt.release(s)?;
        Ok(key)
    }

    fn hmac(&mut self, key: u32, data: &[u8]) -> Result<Vec<u8>> {
        let hmac = self.rt.construct("Hmac", &[key as u64])?;
        let out = Self::bytes(&mut self.rt, "Hmac", "digest", hmac, data)?;
        self.rt.destroy("Hmac", hmac)?;
        Ok(out)
    }

    fn v2(
        &mut self,
        context: &MessageContext<'_>,
        data: &[u8],
        sequence: Option<u64>,
    ) -> Result<Vec<u8>> {
        let to = self.rt.string(if context.to == TO {
            &self.to
        } else {
            &self.from
        })?;
        let from = self.rt.string(if context.from == FROM {
            &self.from
        } else {
            &self.to
        })?;
        let h = self.rt.bytes(data);
        let mut args = vec![
            to,
            from,
            context.sender_key_id as u64,
            context.receiver_key_id as u64,
            context.content_type as u64,
        ];
        let method = if let Some(n) = sequence {
            args.push(n);
            "encryptV2"
        } else {
            "decryptV2"
        };
        args.push(h);
        let out = self.rt.method("E2EEChannel", method, self.channel, &args)?;
        self.rt.free_string(to)?;
        self.rt.free_string(from)?;
        self.rt.release(h)?;
        self.rt.output(out)
    }
}

fn shared() -> [u8; 32] {
    decode(&fixture()["shared_secret"]).try_into().unwrap()
}

fn context() -> MessageContext<'static> {
    MessageContext {
        to: TO,
        from: FROM,
        sender_key_id: 42,
        receiver_key_id: 99,
        content_type: 0,
    }
}

#[test]
fn independent_go_vectors() -> Result<()> {
    let f = fixture();
    let peer_scalar = std::array::from_fn(|i| (i + 1) as u8);
    assert_eq!(
        x25519_public_key(&peer_scalar).as_slice(),
        decode(&f["peer_public_key"])
    );
    assert_eq!(
        x25519(
            &peer_scalar,
            decode(&f["public_key"]).as_slice().try_into()?
        ),
        shared()
    );
    let channel = Channel::new(shared());
    let text = f["plaintext"].as_str().unwrap().as_bytes();
    let v1 = decode(&f["ciphertext_v1"]);
    let v2 = decode(&f["ciphertext"]);
    assert_eq!(channel.decrypt_v1(&v1)?, text);
    assert_eq!(channel.encrypt_v1(v1[..8].try_into()?, text), v1);
    assert_eq!(channel.decrypt_v2(&context(), &v2)?, text);
    assert_eq!(
        channel.encrypt_v2(
            &context(),
            v2[..16].try_into()?,
            0x1_0000_0001,
            v2[24..28].try_into()?,
            text
        ),
        v2
    );
    let key = derive_key(&shared(), b"salt", b"info")?;
    assert_eq!(key.as_slice(), decode(&f["primitives"]["derived_key"]));
    assert_eq!(
        hmac_sha256(&key, b"probe").as_slice(),
        decode(&f["primitives"]["hmac"])
    );
    let aes = decode(&f["primitives"]["aes_ciphertext"]);
    assert_eq!(aes_encrypt(&key, aes[..12].try_into()?, b"probe"), aes);
    assert_eq!(aes_decrypt(&key, &aes)?, b"probe");
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn fixed_vectors_match_oracle() -> Result<()> {
    let wasm = wasm("fixed_vectors_match_oracle")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let f = fixture();
    let key = oracle.derive(b"salt", b"info")?;
    assert_eq!(
        oracle.hmac(key, b"probe")?,
        decode(&f["primitives"]["hmac"])
    );
    let aes = oracle.rt.construct("AesKey", &[key as u64])?;
    assert_eq!(
        Oracle::bytes(
            &mut oracle.rt,
            "AesKey",
            "decrypt",
            aes,
            &decode(&f["primitives"]["aes_ciphertext"])
        )?,
        b"probe"
    );
    assert_eq!(
        oracle.crypt("decryptV1", &decode(&f["ciphertext_v1"]))?,
        f["plaintext"].as_str().unwrap().as_bytes()
    );
    assert_eq!(
        oracle.v2(&context(), &decode(&f["ciphertext"]), None)?,
        f["plaintext"].as_str().unwrap().as_bytes()
    );
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn derive_and_hmac_match_oracle() -> Result<()> {
    let wasm = wasm("derive_and_hmac_match_oracle")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let mut inputs = Inputs::new();
    for case in 0..CASES {
        let prefix = inputs.message(case);
        let suffix = inputs.message((case + 5) % CASES);
        let data = inputs.message(case);
        assert_eq!(
            oracle.hmac(oracle.channel, &data)?,
            hmac_sha256(&shared(), &data)
        );
        let native = derive_key(&shared(), &prefix, &suffix);
        if prefix.is_empty() || suffix.is_empty() {
            assert!(native.is_err());
            assert!(oracle.derive(&prefix, &suffix).is_err());
            oracle = Oracle::new(wasm)?;
            continue;
        }
        let key = oracle.derive(&prefix, &suffix)?;
        let native = native?;
        assert_eq!(
            oracle.hmac(key, &data)?,
            hmac_sha256(&native, &data),
            "case {case}"
        );
        // A second use with AES independently checks all 32 derived key bytes.
        let aes = oracle.rt.construct("AesKey", &[key as u64])?;
        let ct = Oracle::bytes(&mut oracle.rt, "AesKey", "encrypt", aes, b"derived")?;
        assert_eq!(aes_decrypt(&native, &ct)?, b"derived");
        oracle.rt.destroy("AesKey", aes)?;
        oracle.rt.destroy("SecureKey", key)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn aes_matches_oracle() -> Result<()> {
    let wasm = wasm("aes_matches_oracle")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let mut inputs = Inputs::new();
    for case in 0..CASES {
        let prefix = inputs.bytes(32);
        let key = oracle.derive(&prefix, b"aes")?;
        let native = derive_key(&shared(), &prefix, b"aes")?;
        let aes = oracle.rt.construct("AesKey", &[key as u64])?;
        let data = inputs.message(case);
        let ct = Oracle::bytes(&mut oracle.rt, "AesKey", "encrypt", aes, &data)?;
        assert_eq!(
            aes_encrypt(&native, ct[..12].try_into()?, &data),
            ct,
            "case {case}"
        );
        let nonce: [u8; 12] = inputs.bytes(12).try_into().unwrap();
        let ct = aes_encrypt(&native, &nonce, &data);
        if data.is_empty() {
            assert!(aes_decrypt(&native, &ct).is_err());
            assert!(Oracle::bytes(&mut oracle.rt, "AesKey", "decrypt", aes, &ct).is_err());
            oracle = Oracle::new(wasm)?;
        } else {
            assert_eq!(aes_decrypt(&native, &ct)?, data);
            assert_eq!(
                Oracle::bytes(&mut oracle.rt, "AesKey", "decrypt", aes, &ct)?,
                data
            );
            oracle.rt.destroy("AesKey", aes)?;
            oracle.rt.destroy("SecureKey", key)?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn letter_sealing_matches_oracle() -> Result<()> {
    let wasm = wasm("letter_sealing_matches_oracle")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let mut inputs = Inputs::new();
    let root = oracle.channel;
    for case in 0..CASES {
        oracle.channel = root;
        let prefix = inputs.bytes(32);
        let key = oracle.derive(&prefix, b"channel")?;
        oracle.channel = oracle.rt.construct("E2EEChannel", &[key as u64])?;
        let native = Channel::new(derive_key(&shared(), &prefix, b"channel")?);
        let data = inputs.message(case);
        let v1 = oracle.crypt("encryptV1", &data)?;
        assert_eq!(
            native.encrypt_v1(v1[..8].try_into()?, &data),
            v1,
            "V1 case {case}"
        );
        assert_eq!(native.decrypt_v1(&v1)?, data);
        let salt: [u8; 8] = inputs.bytes(8).try_into().unwrap();
        assert_eq!(
            oracle.crypt("decryptV1", &native.encrypt_v1(&salt, &data))?,
            data
        );
        let ctx = MessageContext {
            sender_key_id: inputs.next() as u32,
            receiver_key_id: inputs.next() as u32,
            content_type: inputs.next() as u32,
            ..context()
        };
        let sequence = [0, 1, u32::MAX as u64, 0x1_0000_0001, u64::MAX]
            .get(case)
            .copied()
            .unwrap_or_else(|| inputs.next());
        let v2 = oracle.v2(&ctx, &data, Some(sequence))?;
        assert_eq!(
            native.encrypt_v2(
                &ctx,
                v2[..16].try_into()?,
                sequence,
                v2[24..28].try_into()?,
                &data
            ),
            v2,
            "V2 case {case}"
        );
        assert_eq!(native.decrypt_v2(&ctx, &v2)?, data);
        let salt: [u8; 16] = inputs.bytes(16).try_into().unwrap();
        let random: [u8; 4] = inputs.bytes(4).try_into().unwrap();
        assert_eq!(
            oracle.v2(
                &ctx,
                &native.encrypt_v2(&ctx, &salt, sequence, &random, &data),
                None
            )?,
            data
        );
        oracle.rt.destroy("E2EEChannel", oracle.channel)?;
        oracle.rt.destroy("SecureKey", key)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn tampering_is_rejected_by_both() -> Result<()> {
    let wasm = wasm("tampering_is_rejected_by_both")?;
    let wasm = wasm.as_slice();
    let native = Channel::new(shared());
    let text = b"Synthetic authenticated message spanning several cipher blocks.";
    let aes = aes_encrypt(&shared(), &[7; 12], text);
    let v1 = native.encrypt_v1(&[8; 8], text);
    let v2 = native.encrypt_v2(&context(), &[9; 16], 0x1_0000_0001, &[10; 4], text);
    for (method, frame, ciphertext_offset) in [
        ("decrypt", aes, 12),
        ("decryptV1", v1, 8),
        ("decryptV2", v2, 28),
    ] {
        let mut positions = vec![0, ciphertext_offset, frame.len() - 1];
        if method == "decryptV2" {
            positions.extend([16, 24]);
        }
        for position in positions {
            let mut oracle = Oracle::new(wasm)?;
            let aes_key = if method == "decrypt" {
                Some(oracle.rt.construct("AesKey", &[oracle.channel as u64])?)
            } else {
                None
            };
            let accepted = match aes_key {
                Some(key) => Oracle::bytes(&mut oracle.rt, "AesKey", "decrypt", key, &frame)?,
                None if method == "decryptV2" => oracle.v2(&context(), &frame, None)?,
                None => oracle.crypt(method, &frame)?,
            };
            assert_eq!(
                accepted, text,
                "oracle baseline for {method} bit at {position}"
            );
            let mut corrupt = frame.clone();
            corrupt[position] ^= 1;
            let rejected = match method {
                "decrypt" => aes_decrypt(&shared(), &corrupt),
                "decryptV1" => native.decrypt_v1(&corrupt),
                _ => native.decrypt_v2(&context(), &corrupt),
            };
            assert!(
                rejected.is_err(),
                "native accepted {method} bit at {position}"
            );
            let result = match aes_key {
                Some(key) => Oracle::bytes(&mut oracle.rt, "AesKey", "decrypt", key, &corrupt),
                None if method == "decryptV2" => oracle.v2(&context(), &corrupt, None),
                None => oracle.crypt(method, &corrupt),
            };
            assert!(
                result.is_err(),
                "oracle accepted {method} bit at {position}"
            );
        }
    }
    let f = fixture();
    for field in 0..5 {
        let mut ctx = context();
        match field {
            0 => ctx.to = FROM,
            1 => ctx.from = TO,
            2 => ctx.sender_key_id ^= 1,
            3 => ctx.receiver_key_id ^= 1,
            _ => ctx.content_type ^= 1,
        }
        let ct = decode(&f["ciphertext"]);
        let mut oracle = Oracle::new(wasm)?;
        assert_eq!(
            oracle.v2(&context(), &ct, None)?,
            f["plaintext"].as_str().unwrap().as_bytes()
        );
        assert!(native.decrypt_v2(&ctx, &ct).is_err());
        assert!(oracle.v2(&ctx, &ct, None).is_err());
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn secure_key_boundary() -> Result<()> {
    let wasm = wasm("secure_key_boundary")?;
    let wasm = wasm.as_slice();
    for len in [0, 16, 32, 48, 64, 193] {
        let mut rt = Runtime::new(wasm)?;
        let h = rt.bytes(&vec![0x42; len]);
        let error = rt
            .static_call("SecureKey", "loadKey", &[h])
            .expect_err("raw key import");
        let expected = if len < 32 {
            "invalid parameters"
        } else if len == 32 {
            "invalid format"
        } else {
            "operation not supported"
        };
        assert!(
            format!("{error:#}").contains(expected),
            "length {len}: {error:#}"
        );
    }
    let mut rt = Runtime::new(wasm)?;
    let token = rt.string(crate::TOKEN)?;
    let key = rt.static_call("SecureKey", "loadToken", &[token])?;
    rt.free_string(token)?;
    let p = rt.bytes(&Sha256::digest(crate::VERSION));
    let s = rt.bytes(&Sha256::digest(b"synthetic access token"));
    let derived = rt.method("SecureKey", "deriveKey", key, &[p, s])?;
    let hmac = rt.construct("Hmac", &[derived as u64])?;
    assert_eq!(
        Oracle::bytes(&mut rt, "Hmac", "digest", hmac, b"synthetic request")?.len(),
        32
    );
    let error = rt
        .method("SecureKey", "exportKey", key, &[])
        .expect_err("token export");
    assert!(
        format!("{error:#}").contains("illegal operation"),
        "{error:#}"
    );
    let mut rt = Runtime::new(wasm)?;
    let h = rt.bytes(&decode(&fixture()["exported_key"]));
    let key = rt.static_call("Curve25519Key", "loadKey", &[h])?;
    rt.release(h)?;
    let out = rt.method("Curve25519Key", "getPublicKey", key, &[])?;
    let public = rt.output(out)?;
    assert_eq!(public, decode(&fixture()["public_key"]));
    let mut basepoint = [0; 32];
    basepoint[0] = 9;
    let h = rt.bytes(&basepoint);
    let channel = rt.method("Curve25519Key", "createChannel", key, &[h])?;
    let hmac = rt.construct("Hmac", &[channel as u64])?;
    assert_eq!(
        Oracle::bytes(&mut rt, "Hmac", "digest", hmac, b"base point")?,
        hmac_sha256(&public, b"base point")
    );
    for curve_wrapper in [false, true] {
        let mut oracle = Oracle::new(wasm)?;
        let key = oracle.derive(b"salt", b"info")?;
        let (class, key) = if curve_wrapper {
            (
                "Curve25519Key",
                oracle.rt.construct("Curve25519Key", &[key as u64])?,
            )
        } else {
            ("SecureKey", key)
        };
        let error = oracle
            .rt
            .method(class, "exportKey", key, &[])
            .expect_err("derived key export");
        assert!(
            format!("{error:#}").contains("illegal operation"),
            "{class}: {error:#}"
        );
    }
    let mut rt = Runtime::new(wasm)?;
    let key = rt.static_call("Curve25519Key", "generate", &[1])?;
    let out = rt.method("Curve25519Key", "getPublicKey", key, &[])?;
    let public = rt.output(out)?;
    let out = rt.method("Curve25519Key", "exportKey", key, &[])?;
    let wrapped = rt.output(out)?;
    assert_eq!(wrapped.len(), 193);
    rt.destroy("Curve25519Key", key)?;
    // A fresh runtime proves persistence independently of the original engine cache.
    let mut rt = Runtime::new(wasm)?;
    let h = rt.bytes(&wrapped);
    let restored = rt.static_call("Curve25519Key", "loadKey", &[h])?;
    let out = rt.method("Curve25519Key", "getPublicKey", restored, &[])?;
    assert_eq!(rt.output(out)?, public);
    Ok(())
}

#[test]
#[ignore = "release benchmark; synthetic fixture, no account access"]
fn benchmark_decrypt() -> Result<()> {
    let wasm = wasm("benchmark_decrypt")?;
    let started = Instant::now();
    let mut oracle = Oracle::new(&wasm)?;
    let init = started.elapsed();
    let native = Channel::new(shared());
    let ct = decode(&fixture()["ciphertext"]);
    let expected = fixture()["plaintext"]
        .as_str()
        .context("fixture plaintext")?
        .as_bytes()
        .to_vec();
    for _ in 0..5 {
        assert_eq!(oracle.v2(&context(), &ct, None)?, expected);
    }
    let started = Instant::now();
    for _ in 0..100 {
        assert_eq!(
            std::hint::black_box(oracle.v2(&context(), std::hint::black_box(&ct), None)?),
            expected
        );
    }
    let oracle_us = started.elapsed().as_secs_f64() * 1e6 / 100.0;
    let started = Instant::now();
    for _ in 0..10000 {
        assert_eq!(
            std::hint::black_box(native.decrypt_v2(&context(), std::hint::black_box(&ct))?),
            expected
        );
    }
    let native_us = started.elapsed().as_secs_f64() * 1e6 / 10000.0;
    eprintln!(
        "V2 {}-byte plaintext, reused channel: oracle={oracle_us:.3} us native={native_us:.3} us speedup={:.1}x; oracle setup={:.1} ms",
        expected.len(),
        oracle_us / native_us,
        init.as_secs_f64() * 1000.0
    );
    Ok(())
}

// Build CBC and its valid MAC directly, without going through encrypt_v1's padding.
fn v1_padded_frame(padded: &[u8]) -> Vec<u8> {
    assert!(!padded.is_empty() && padded.len().is_multiple_of(16));
    let salt = [0x51; 8];
    let native = Channel::new(shared());
    let aes = Aes256::new((&native.key(&salt)).into());
    let mut previous = native.iv(&salt);
    let mut ciphertext = padded.to_vec();
    for chunk in ciphertext.as_chunks_mut::<16>().0 {
        let mut block = Block::<Aes256>::clone_from_slice(chunk);
        for (b, p) in block.iter_mut().zip(previous) {
            *b ^= p;
        }
        aes.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
        previous = block.into();
    }
    let mut mac = Block::<Aes256>::from(fold(&Sha256::digest(&ciphertext).into()));
    aes.encrypt_block(&mut mac);
    let mut frame = salt.to_vec();
    frame.extend_from_slice(&ciphertext);
    frame.extend_from_slice(&mac);
    frame
}

fn invalid_padding_frames() -> Vec<(&'static str, Vec<u8>)> {
    [
        ("short", vec![1, 2]),
        ("inconsistent", vec![4, 3, 4, 4]),
        ("zero", vec![0]),
        ("over block", vec![17]),
        ("maximum byte", vec![255]),
    ]
    .into_iter()
    .map(|(label, tail)| {
        let mut padded = [0x41; 32];
        padded[32 - tail.len()..].copy_from_slice(&tail);
        (label, v1_padded_frame(&padded))
    })
    .collect()
}

#[test]
fn native_rejects_authenticated_invalid_v1_padding() -> Result<()> {
    let native = Channel::new(shared());
    assert_eq!(native.decrypt_v1(&v1_padded_frame(&[16; 16]))?, b"");
    for (label, frame) in invalid_padding_frames() {
        assert_eq!(
            native.decrypt_v1(&frame).unwrap_err().to_string(),
            "invalid V1 padding",
            "{label}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn authenticated_invalid_v1_padding() -> Result<()> {
    let wasm = wasm("authenticated_invalid_v1_padding")?;
    let wasm = wasm.as_slice();
    native_rejects_authenticated_invalid_v1_padding()?;
    let valid = v1_padded_frame(&[16; 16]);
    for (label, frame) in invalid_padding_frames() {
        let mut oracle = Oracle::new(wasm)?;
        assert_eq!(oracle.crypt("decryptV1", &valid)?, b"");
        let error = oracle.crypt("decryptV1", &frame).expect_err(label);
        assert!(
            format!("{error:#}").contains("Failed to unpad the data: invalid format"),
            "{label}: {error:#}"
        );
    }
    Ok(())
}

fn hex32(hex: &str) -> [u8; 32] {
    assert_eq!(hex.len(), 64);
    std::array::from_fn(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
}

// https://www.rfc-editor.org/rfc/rfc7748.html#section-5.2 and section 6.1.
const RFC_SCALARS: [&str; 4] = [
    "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4",
    "4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d",
    "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
];

#[test]
fn rfc7748_vectors() {
    for (scalar, point, expected) in [
        (
            RFC_SCALARS[0],
            "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c",
            "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552",
        ),
        (
            RFC_SCALARS[1],
            "e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493",
            "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957",
        ),
    ] {
        assert_eq!(x25519(&hex32(scalar), &hex32(point)), hex32(expected));
    }
    let alice = hex32(RFC_SCALARS[2]);
    let bob = hex32(RFC_SCALARS[3]);
    let alice_public = hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
    let bob_public = hex32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
    let shared = hex32("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
    assert_eq!(x25519_public_key(&alice), alice_public);
    assert_eq!(x25519_public_key(&bob), bob_public);
    assert_eq!(x25519(&alice, &bob_public), shared);
    assert_eq!(x25519(&bob, &alice_public), shared);
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn x25519_matches_oracle() -> Result<()> {
    let wasm = wasm("x25519_matches_oracle")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let mut inputs = Inputs::new();
    for case in 0..CASES {
        let prefix = inputs.bytes(32);
        let scalar = derive_key(&shared(), &prefix, b"curve")?;
        let key = oracle.derive(&prefix, b"curve")?;
        let curve = oracle.rt.construct("Curve25519Key", &[key as u64])?;
        let h = oracle
            .rt
            .method("Curve25519Key", "getPublicKey", curve, &[])?;
        let public = oracle.rt.output(h)?;
        assert_eq!(public, x25519_public_key(&scalar), "public key {case}");
        // RFC scalars and seeded raw scalars on the peer side; the oracle's
        // scalar is independently known via deriveKey, never read from memory.
        let peer_scalar = RFC_SCALARS
            .get(case)
            .map(|s| hex32(s))
            .unwrap_or_else(|| inputs.bytes(32).try_into().unwrap());
        let point = x25519_public_key(&peer_scalar);
        let native = Channel::from_x25519(&scalar, &point);
        assert_eq!(
            native.shared,
            x25519(&peer_scalar, public.as_slice().try_into()?)
        );
        let h = oracle.rt.bytes(&point);
        let channel = oracle
            .rt
            .method("Curve25519Key", "createChannel", curve, &[h])?;
        oracle.rt.release(h)?;
        let probe = inputs.message(case);
        assert_eq!(
            oracle.hmac(channel, &probe)?,
            hmac_sha256(&native.shared, &probe),
            "ECDH {case}"
        );
        let frame = native.encrypt_v1(&[0x63; 8], &probe);
        assert_eq!(
            Oracle::bytes(&mut oracle.rt, "E2EEChannel", "decryptV1", channel, &frame)?,
            probe
        );
        oracle.rt.destroy("E2EEChannel", channel)?;
        oracle.rt.destroy("Curve25519Key", curve)?;
        oracle.rt.destroy("SecureKey", key)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn low_order_points_match_oracle() -> Result<()> {
    let wasm = wasm("low_order_points_match_oracle")?;
    let wasm = wasm.as_slice();
    // All seven standard small-order encodings, including noncanonical 0/1.
    let points = [
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0100000000000000000000000000000000000000000000000000000000000000",
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ];
    for point in points {
        let mut oracle = Oracle::new(wasm)?;
        let scalar = derive_key(&shared(), b"salt", b"info")?;
        let key = oracle.derive(b"salt", b"info")?;
        let curve = oracle.rt.construct("Curve25519Key", &[key as u64])?;
        let point = hex32(point);
        assert_eq!(x25519(&scalar, &point), [0; 32]);
        let native = Channel::from_x25519(&scalar, &point);
        assert_eq!(
            native.shared,
            hex32("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f")
        );
        let h = oracle.rt.bytes(&point);
        let channel = oracle
            .rt
            .method("Curve25519Key", "createChannel", curve, &[h])?;
        assert_eq!(
            oracle.hmac(channel, b"low order accepted")?,
            hmac_sha256(&native.shared, b"low order accepted")
        );
        let frame = native.encrypt_v1(&[0x64; 8], b"low order accepted");
        assert_eq!(
            Oracle::bytes(&mut oracle.rt, "E2EEChannel", "decryptV1", channel, &frame)?,
            b"low order accepted"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the local WASM oracle; run explicitly with --ignored"]
fn arbitrary_point_boundary() -> Result<()> {
    let wasm = wasm("arbitrary_point_boundary")?;
    let wasm = wasm.as_slice();
    let mut oracle = Oracle::new(wasm)?;
    let scalar = derive_key(&shared(), b"salt", b"info")?;
    let key = oracle.derive(b"salt", b"info")?;
    let curve = oracle.rt.construct("Curve25519Key", &[key as u64])?;
    let mut inputs = Inputs::new();
    let mut matches = 0;
    let mut divergent = 0;
    for case in 0..64 {
        let point: [u8; 32] = inputs.bytes(32).try_into().unwrap();
        let h = oracle.rt.bytes(&point);
        let channel = oracle
            .rt
            .method("Curve25519Key", "createChannel", curve, &[h])?;
        oracle.rt.release(h)?;
        let actual = oracle.hmac(channel, b"arbitrary point")?;
        let expected = hmac_sha256(&x25519(&scalar, &point), b"arbitrary point");
        if curve25519_dalek::montgomery::MontgomeryPoint(point)
            .to_edwards(0)
            .is_some()
        {
            assert_eq!(actual, expected, "curve point {case}");
            matches += 1;
        } else {
            assert_ne!(
                actual, expected,
                "twist point {case}: reevaluate documented boundary"
            );
            divergent += 1;
        }
        oracle.rt.destroy("E2EEChannel", channel)?;
    }
    assert!(matches > 0 && divergent > 0);
    eprintln!(
        "arbitrary u-coordinates: {matches} curve matches, {divergent} documented twist divergences"
    );
    Ok(())
}
