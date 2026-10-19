//! Read-only, bounded real-message decryption probe. Never persists chat data.
use super::*;

pub(super) fn talk(api: &mut Api, token: &str, method: &str, args: Value) -> Result<Value> {
    let cache_key = format!("{method}:{args}");
    let cacheable = matches!(
        method,
        "getE2EEPublicKey"
            | "getE2EEGroupSharedKey"
            | "negotiateE2EEPublicKey"
            | "getLastE2EEGroupSharedKey"
    );
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

pub(super) fn own_key(api: &mut Api, state: &Value, id: u64) -> Result<u32> {
    if let Some(key) = api.own_keys.get(&id) {
        return Ok(*key);
    }
    let rt = &mut api.rt;
    let key = state["e2ee_keys"]
        .as_array()
        .context("missing saved E2EE keys")?
        .iter()
        .find(|key| key["id"].as_u64() == Some(id))
        .context("matching own key unavailable")?;
    let h = rt.bytes(&decode(required(key, "exported_key")?)?);
    let ptr = rt.static_call(Call::E2EEKeyLoadKey, &[h])?;
    rt.release(h)?;
    api.own_keys.insert(id, ptr);
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
    let ptr = rt.method(Call::E2EEKeyCreateChannel, key, &[h])?;
    rt.release(h)?;
    Ok(ptr)
}

/// Channel identity is shared by incoming messages, outgoing echoes and sends.
pub(super) struct Channel<'a> {
    pub mid: &'a str,
    pub own: u64,
    pub peer: u64,
    pub group: Option<u64>,
    pub sender: &'a str,
    pub sender_key: u64,
}
pub(super) fn message_channel(
    api: &mut Api,
    state: &Value,
    my_mid: &str,
    id: &Channel<'_>,
) -> Result<u32> {
    let token = access(state)?;
    let cache_key = match id.group {
        None => format!("user:{}:{}:{}", id.mid, id.own, id.peer),
        Some(group) => format!("group:{}:{group}:{}:{}", id.mid, id.sender, id.sender_key),
    };
    if let Some(channel) = api.cached_channel(&cache_key) {
        return Ok(channel);
    }
    let ch = if let Some(group) = id.group {
        let shared = talk(
            api,
            token,
            "getE2EEGroupSharedKey",
            json!([1, id.mid, group]),
        )?;
        ensure!(
            shared["groupKeyId"].as_u64() == Some(group),
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
        let key = own_key(api, state, private_id)?;
        let transfer = channel(&mut api.rt, key, &public)?;
        let h = api
            .rt
            .bytes(&decode(required(&shared, "encryptedSharedKey")?)?);
        let group_key = api
            .rt
            .method(Call::E2EEChannelUnwrapGroupSharedKey, transfer, &[h])?;
        api.rt.release(h)?;
        let public = if id.sender == my_mid {
            let own = own_key(api, state, id.sender_key)?;
            bytes_method(&mut api.rt, Call::E2EEKeyGetPublicKey, own, &[])?
        } else {
            public_key(api, token, id.sender, id.sender_key)?
        };
        let ch = channel(&mut api.rt, group_key, &public)?;
        api.rt.destroy("E2EEKey", group_key)?;
        api.rt.destroy("E2EEChannel", transfer)?;
        ch
    } else {
        let key = own_key(api, state, id.own)?;
        let public = public_key(api, token, id.mid, id.peer)?;
        channel(&mut api.rt, key, &public)?
    };
    api.cache_channel(cache_key, ch)?;
    Ok(ch)
}

pub(super) fn decrypt(
    api: &mut Api,
    state: &Value,
    my_mid: &str,
    message: &Value,
    envelope: &Envelope,
) -> Result<Vec<u8>> {
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
    let ch = message_channel(
        api,
        state,
        my_mid,
        &Channel {
            mid: chat_mid,
            own: private_id,
            peer: public_id,
            group: (!direct).then_some(envelope.receiver),
            sender: from,
            sender_key: envelope.sender,
        },
    )?;
    let h = api.rt.bytes(&envelope.ciphertext);
    let plaintext = if envelope.version == 1 {
        bytes_method(&mut api.rt, Call::E2EEChannelDecryptV1, ch, &[h])?
    } else {
        let to = api.rt.string(to)?;
        let from = api.rt.string(from)?;
        let content = message["contentType"]
            .as_u64()
            .context("missing content type")?;
        let bytes = bytes_method(
            &mut api.rt,
            Call::E2EEChannelDecryptV2,
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

pub(super) fn run(opts: &Options, wasm: &[u8], version: &str, progress: &Progress) -> Result<()> {
    let state = crate::credentials::load(&opts.state)?;
    let mut api = Api::new(wasm, version)?;
    validate_keys(&mut api, &state).context(error::SavedSessionFailure::Keys.error())?;
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
            match decrypt(&mut api, &state, my_mid, message, &envelope) {
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
                    api.reinitialize(wasm)?;
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
        let mut api = Api::new(
            &crate::test_support::read_fixture(&crate::test_support::fixture_path())?,
            crate::test_support::FIXTURE_VERSION,
        )?;
        let h = api.rt.bytes(&decode(required(&fixture, "exported_key")?)?);
        let key = api.rt.static_call(Call::Curve25519KeyLoadKey, &[h])?;
        api.rt.release(h)?;
        let h = api
            .rt
            .bytes(&decode(required(&fixture, "peer_public_key")?)?);
        let channel = api.rt.method(Call::Curve25519KeyCreateChannel, key, &[h])?;
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
