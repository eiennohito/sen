//! Read-only, bounded real-message decryption probe. Never persists chat data.
use super::*;

pub(super) fn talk(api: &mut Api, token: &str, method: &str, args: Value) -> Result<Value> {
    let cache_key = format!("{method}:{args}");
    let cacheable = matches!(method, "getE2EEPublicKey" | "getE2EEGroupSharedKey");
    if cacheable && let Some(value) = api.key_cache.get(&cache_key) {
        return Ok(value.clone());
    }
    let value = api.call(
        &format!("/api/talk/thrift/Talk/TalkService/{method}"),
        &args,
        token,
        "",
        0,
    )?;
    if cacheable {
        if api.key_cache.len() >= 256 {
            api.key_cache.clear();
        }
        api.key_cache.insert(cache_key, value.clone());
    }
    Ok(value)
}

pub(super) struct Envelope {
    pub(super) sender: u64,
    pub(super) receiver: u64,
    pub(super) version: u64,
    ciphertext: Vec<u8>,
}
impl Envelope {
    pub(super) fn parse(message: &Value) -> Result<Self> {
        let chunks = message["chunks"].as_array().context("missing chunks")?;
        ensure!(chunks.len() == 5, "expected five encrypted message chunks");
        let chunks = chunks
            .iter()
            .map(|c| decode(c.as_str().context("non-string chunk")?))
            .collect::<Result<Vec<_>>>()?;
        let key_id = |bytes: &[u8]| -> Result<u64> {
            let bytes: [u8; 4] = bytes.try_into().context("key ID must be four bytes")?;
            Ok(u32::from_be_bytes(bytes) as u64)
        };
        let version = match &message["contentMetadata"]["e2eeVersion"] {
            Value::Null => 2,
            Value::String(v) => v.parse().context("invalid E2EE version")?,
            _ => bail!("invalid E2EE version type"),
        };
        let order = match version {
            1 => [0, 1, 2],
            2 => [0, 2, 1],
            _ => bail!("unsupported E2EE version"),
        };
        Ok(Self {
            sender: key_id(&chunks[3])?,
            receiver: key_id(&chunks[4])?,
            version,
            ciphertext: order.iter().flat_map(|&i| chunks[i].clone()).collect(),
        })
    }
}

pub(super) fn own_key(rt: &mut Runtime, state: &Value, id: u64) -> Result<u32> {
    let key = state["e2ee_keys"]
        .as_array()
        .context("missing saved E2EE keys")?
        .iter()
        .find(|key| key["id"].as_u64() == Some(id))
        .context("matching own key unavailable")?;
    let h = rt.bytes(&decode(required(key, "exported_key")?)?);
    let ptr = rt.static_call("E2EEKey", "loadKey", &[h])?;
    rt.release(h)?;
    Ok(ptr)
}

pub(super) fn public_key(api: &mut Api, token: &str, mid: &str, id: u64) -> Result<Vec<u8>> {
    let key = talk(api, token, "getE2EEPublicKey", json!([mid, 1, id]))?;
    ensure!(key["keyId"].as_u64() == Some(id), "public key ID mismatch");
    let bytes = decode(required(&key, "keyData")?)?;
    ensure!(bytes.len() == 32, "invalid public key size");
    Ok(bytes)
}

pub(super) fn channel(rt: &mut Runtime, key: u32, public: &[u8]) -> Result<u32> {
    let h = rt.bytes(public);
    let ptr = rt.method("E2EEKey", "createChannel", key, &[h])?;
    rt.release(h)?;
    Ok(ptr)
}

pub(super) fn decrypt(
    api: &mut Api,
    state: &Value,
    my_mid: &str,
    message: &Value,
    envelope: &Envelope,
) -> Result<Vec<u8>> {
    let token = access(state)?;
    let to = required(message, "to")?;
    let from = required(message, "from")?;
    let outgoing = from == my_mid;
    let chat_mid = if outgoing {
        to
    } else if to == my_mid {
        from
    } else {
        to
    };
    // The gateway's opaque MIDs retain a type prefix: U = user, C/R = group/room.
    let direct = chat_mid.starts_with(['U', 'u']);
    let (private_id, public_id) = if outgoing {
        (envelope.sender, envelope.receiver)
    } else {
        (envelope.receiver, envelope.sender)
    };
    let cache_key = if direct {
        format!("user:{chat_mid}:{private_id}:{public_id}")
    } else {
        format!(
            "group:{chat_mid}:{}:{from}:{}",
            envelope.receiver, envelope.sender
        )
    };
    let ch = if let Some(channel) = api.cached_channel(&cache_key) {
        channel
    } else {
        let channel = if direct {
            let key = own_key(&mut api.rt, state, private_id)?;
            let public = public_key(api, token, chat_mid, public_id)?;
            let ch = channel(&mut api.rt, key, &public)?;
            api.rt.destroy("E2EEKey", key)?;
            ch
        } else {
            let shared = talk(
                api,
                token,
                "getE2EEGroupSharedKey",
                json!([1, chat_mid, envelope.receiver]),
            )?;
            ensure!(
                shared["groupKeyId"].as_u64() == Some(envelope.receiver),
                "group key ID mismatch"
            );
            ensure!(
                required(&shared, "receiver")? == my_mid,
                "group key receiver mismatch"
            );
            let private_id = shared["receiverKeyId"]
                .as_u64()
                .context("missing receiver key ID")?;
            let creator_id = shared["creatorKeyId"]
                .as_u64()
                .context("missing creator key ID")?;
            let public = public_key(api, token, required(&shared, "creator")?, creator_id)?;
            let key = own_key(&mut api.rt, state, private_id)?;
            let transfer = channel(&mut api.rt, key, &public)?;
            let h = api
                .rt
                .bytes(&decode(required(&shared, "encryptedSharedKey")?)?);
            let group_key = api
                .rt
                .method("E2EEChannel", "unwrapGroupSharedKey", transfer, &[h])?;
            api.rt.release(h)?;
            let public = public_key(api, token, from, envelope.sender)?;
            let ch = channel(&mut api.rt, group_key, &public)?;
            api.rt.destroy("E2EEKey", group_key)?;
            api.rt.destroy("E2EEChannel", transfer)?;
            api.rt.destroy("E2EEKey", key)?;
            ch
        };
        api.cache_channel(cache_key, channel)?;
        channel
    };
    let h = api.rt.bytes(&envelope.ciphertext);
    let plaintext = if envelope.version == 1 {
        bytes_method(&mut api.rt, "E2EEChannel", "decryptV1", ch, &[h])?
    } else {
        let to = api.rt.string(to)?;
        let from = api.rt.string(from)?;
        let content = message["contentType"]
            .as_u64()
            .context("missing content type")?;
        let bytes = bytes_method(
            &mut api.rt,
            "E2EEChannel",
            "decryptV2",
            ch,
            &[to, from, envelope.sender, envelope.receiver, content, h],
        )?;
        api.rt.free_string(to)?;
        api.rt.free_string(from)?;
        bytes
    };
    api.rt.release(h)?;
    Ok(plaintext)
}

pub(super) fn run(opts: &Options, wasm: &[u8], progress: &Progress) -> Result<()> {
    let state = crate::credentials::load(&opts.state)?;
    ensure!(
        state["wasm_sha256"] == HASH,
        "saved session WASM version mismatch"
    );
    let mut api = Api::new(wasm)?;
    validate_keys(&mut api.rt, &state)?;
    let profile = api.profile(access(&state)?)?;
    let my_mid = required(&profile, "mid")?;
    progress.set("Fetching bounded recent chat previews for decryption check")?;
    let response = talk(
        &mut api,
        access(&state)?,
        "getMessageBoxes",
        json!([{"activeOnly":true,"unreadOnly":false,"messageBoxCountLimit":10,
            "withUnreadCount":false,"lastMessagesPerMessageBoxCount":5},0]),
    )?;
    let boxes = response["messageBoxes"]
        .as_array()
        .context("missing message boxes")?;
    println!(
        "Fetched {} chat preview(s); more available: {}",
        boxes.len(),
        response["hasNext"].as_bool().unwrap_or(false)
    );
    let mut seen = 0;
    let mut encrypted = 0;
    let mut failed = 0;
    for chat in boxes.iter().take(10) {
        let Some(messages) = chat["lastMessages"].as_array() else {
            continue;
        };
        for message in messages.iter().take(5) {
            seen += 1;
            if message["chunks"].as_array().is_none_or(|c| c.is_empty()) {
                continue;
            }
            encrypted += 1;
            if !matches!(
                message["contentType"].as_u64(),
                Some(0 | 1 | 2 | 3 | 14 | 15)
            ) {
                continue;
            }
            let envelope = match Envelope::parse(message) {
                Ok(e) => e,
                Err(_) => {
                    println!("Skipped encrypted preview with unsupported envelope");
                    continue;
                }
            };
            // A thrown WASM exception poisons its instance. Each candidate gets
            // a fresh runtime, including failures during key preparation.
            let mut candidate = Api::new(wasm)?;
            match decrypt(&mut candidate, &state, my_mid, message, &envelope) {
                Ok(plaintext) => {
                    let decoded: Value = serde_json::from_slice(&plaintext)
                        .context("decrypted payload is not valid UTF-8 JSON; body not logged")?;
                    ensure!(decoded.is_object(), "decrypted payload is not an object");
                    println!(
                        "Verified E2EE V{} payload: {} bytes, text present: {}, location present: {}, file key present: {}",
                        envelope.version,
                        plaintext.len(),
                        decoded["text"].is_string(),
                        decoded["location"].is_object(),
                        decoded["keyMaterial"].is_string()
                    );
                    return progress.set("SUCCESS: real chat ciphertext decrypted in original WASM; plaintext not logged or saved");
                }
                Err(error) => {
                    failed += 1;
                    // Only application-defined errors reach here; no server body
                    // or message contents are included in RPC/runtime diagnostics.
                    println!("Encrypted preview attempt {failed} failed: {error:#}");
                }
            }
            if failed >= 5 {
                break;
            }
        }
        if failed >= 5 {
            break;
        }
    }
    bail!(
        "no verified decryption in bounded sample ({seen} previews, {encrypted} encrypted, {failed} failed attempts)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the locally obtained official WASM; synthetic fixture only"]
    fn cached_channel_is_reusable_across_messages() -> Result<()> {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/letter-sealing-v2.json"))?;
        let mut api = Api::new(&fs::read(
            crate::locations::Locations::from_environment()
                .map_err(anyhow::Error::msg)?
                .wasm,
        )?)?;
        let h = api.rt.bytes(&decode(required(&fixture, "exported_key")?)?);
        let key = api.rt.static_call("Curve25519Key", "loadKey", &[h])?;
        api.rt.release(h)?;
        let h = api
            .rt
            .bytes(&decode(required(&fixture, "peer_public_key")?)?);
        let channel = api.rt.method("Curve25519Key", "createChannel", key, &[h])?;
        api.rt.release(h)?;
        api.rt.destroy("Curve25519Key", key)?;
        let from = required(&fixture, "from")?;
        let to = required(&fixture, "to")?;
        api.cache_channel(format!("user:{from}:99:42"), channel)?;
        let ciphertext = decode(required(&fixture, "ciphertext_v1")?)?;
        let chunks = vec![
            STANDARD.encode(&ciphertext[..8]),
            STANDARD.encode(&ciphertext[8..ciphertext.len() - 16]),
            STANDARD.encode(&ciphertext[ciphertext.len() - 16..]),
            STANDARD.encode(42u32.to_be_bytes()),
            STANDARD.encode(99u32.to_be_bytes()),
        ];
        let message =
            json!({"from":from,"to":to,"chunks":chunks,"contentMetadata":{"e2eeVersion":"1"}});
        // Empty own-key state makes a channel cache miss fail before any network
        // call. Neither account state nor a real token is read by this test.
        let state = json!({"e2ee_keys":[],"token_v3_issue_result":{"accessToken":"synthetic-not-a-session"}});
        let envelope = Envelope::parse(&message)?;
        for _ in 0..2 {
            assert_eq!(
                decrypt(&mut api, &state, to, &message, &envelope)?,
                required(&fixture, "plaintext")?.as_bytes()
            );
        }
        assert_eq!(api.channels.len(), 1);
        Ok(())
    }

    #[test]
    fn chrome_chunks_reassemble_independent_ciphertexts() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/letter-sealing-v2.json")).unwrap();
        for version in [1, 2] {
            let ciphertext = decode(
                fixture[if version == 1 {
                    "ciphertext_v1"
                } else {
                    "ciphertext"
                }]
                .as_str()
                .unwrap(),
            )
            .unwrap();
            let chunks = if version == 1 {
                vec![
                    &ciphertext[..8],
                    &ciphertext[8..ciphertext.len() - 16],
                    &ciphertext[ciphertext.len() - 16..],
                ]
            } else {
                vec![&ciphertext[..16], &ciphertext[28..], &ciphertext[16..28]]
            };
            let mut chunks: Vec<_> = chunks.into_iter().map(|c| STANDARD.encode(c)).collect();
            chunks.extend([
                STANDARD.encode(42_u32.to_be_bytes()),
                STANDARD.encode(99_u32.to_be_bytes()),
            ]);
            let message =
                json!({"chunks":chunks,"contentMetadata":{"e2eeVersion":version.to_string()}});
            let envelope = Envelope::parse(&message).unwrap();
            assert_eq!(envelope.ciphertext, ciphertext);
            assert_eq!((envelope.sender, envelope.receiver), (42, 99));
        }
    }

    #[test]
    fn malformed_envelopes_are_rejected_before_entering_wasm() {
        assert!(Envelope::parse(&json!({"chunks":[]})).is_err());
        assert!(Envelope::parse(&json!({"chunks":["", "", "", "AA==", "AAAAAA=="]})).is_err());
        assert!(Envelope::parse(&json!({"chunks":["", "", "", "AAAAAA==", "AAAAAA=="],"contentMetadata":{"e2eeVersion":"3"}})).is_err());
    }
}
