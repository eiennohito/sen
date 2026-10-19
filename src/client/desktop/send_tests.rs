use super::test_client::{Reply, client, server_responses};
use super::*;
use crate::client::error::wire_fixture::{self, GROUP_KEY_ROTATED, KEY_ROTATED, SENDER_KEY_STALE};

#[test]
fn rotated_peer_key_refetches_and_resends_once() -> Result<()> {
    let module = crate::test_support::installed_module();

    let wasm = module.read()?;
    let mut rt = Runtime::new(&wasm, &module.version)?;
    let own = rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
    let own_public = bytes_method(&mut rt, Call::Curve25519KeyGetPublicKey, own, &[])?;
    let mut receivers = Vec::new();
    let mut public_keys = Vec::new();
    for _ in 0..2 {
        let peer = rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
        public_keys.push(bytes_method(
            &mut rt,
            Call::Curve25519KeyGetPublicKey,
            peer,
            &[],
        )?);
        receivers.push(e2ee::channel(&mut rt, peer, &own_public)?);
    }
    let mut api = Api::with_runtime(rt, &wasm, &module.version)?;
    api.signature("/synthetic", "", "synthetic")?;
    let (mut client, root) = client(api);
    client.api.state.e2ee_keys = vec![SessionKey {
        id: 99,
        version: 1,
        public_key: String::new(),
        exported_key: String::new(),
        rest: Default::default(),
    }];
    client.api.own_keys.insert(99, own);
    for code in [KEY_ROTATED, SENDER_KEY_STALE, GROUP_KEY_ROTATED] {
        client.api.key_cache.clear();
        for (_, (channel, _)) in client.api.channels.drain() {
            client.api.rt.destroy("E2EEChannel", channel)?;
        }
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut steps: Vec<(&'static str, Reply)> = Vec::new();
        for (id, public) in [42u64, 43]
            .into_iter()
            .zip(&public_keys)
            .take(if code == SENDER_KEY_STALE { 1 } else { 2 })
        {
            let public = public.clone();
            steps.push(("negotiateE2EEPublicKey", Box::new(move |args| {
                assert_eq!(args, json!(["upeer"]));
                (200, json!({"code":0,"message":"OK","data":{"allowedTypes":[0],"specVersion":1,"publicKey":{"keyId":id}}}))
            })));
            steps.push((
                "getE2EEPublicKey",
                Box::new(move |args| {
                    assert_eq!(args, json!(["upeer", 1, id]));
                    (
                        200,
                        json!({"code":0,"message":"OK","data":{"keyId":id,"keyData":STANDARD.encode(public)}}),
                    )
                }),
            ));
            let sent = sent.clone();
            steps.push(("sendMessage", Box::new(move |args| {
                assert_eq!(args[0], 7);
                assert_eq!(args[1]["relatedMessageId"], "original");
                assert_eq!(args[1]["messageRelationType"], 3);
                assert_eq!(args[1]["relatedMessageServiceCode"], 2);
                assert_eq!(decode(args[1]["chunks"][4].as_str().unwrap()).unwrap(), (id as u32).to_be_bytes());
                sent.lock().unwrap().push(args[1].clone());
                if id == 42 {
                    (200, wire_fixture::talk(code))
                } else {
                    (200, json!({"code":0,"message":"OK","data":{"id":"sent","from":"uself","to":"upeer","contentType":0,"text":"hello"}}))
                }
            })));
        }
        let (mut transport, worker) = server_responses(steps);
        transport.client_version = module.version.clone();
        client.api.http = transport;
        let result = client.send_text_with_request(
            "upeer",
            "hello",
            7,
            Some(&crate::store::ReplyReference {
                message_id: "original".into(),
                service_code: Some(2),
            }),
        );
        if code == SENDER_KEY_STALE {
            let error = result.err().expect("stale sender key accepted");
            assert!(matches!(
                super::super::error::classify(&error),
                super::super::error::Failure::Refused(super::super::error::Refusal::Server(
                    crate::client::error::ServerRefusal::Talk(SENDER_KEY_STALE)
                ))
            ));
        } else {
            assert_eq!(result?.id, "sent");
        }
        assert!(!client.api.rt.is_poisoned());
        worker.join().unwrap();
        for (raw, receiver) in sent.lock().unwrap().iter().zip(receivers.iter().copied()) {
            // V1 chunks preserve nonce/body/tag order.
            let chunks = raw["chunks"].as_array().unwrap();
            let ciphertext = chunks[..3]
                .iter()
                .map(|v| decode(v.as_str().unwrap()))
                .collect::<Result<Vec<_>>>()?
                .concat();
            let h = client.api.rt.bytes(&ciphertext);
            let plaintext = bytes_method(
                &mut client.api.rt,
                Call::E2EEChannelDecryptV1,
                receiver,
                &[h],
            )?;
            client.api.rt.release(h)?;
            assert_eq!(
                serde_json::from_slice::<Value>(&plaintext)?,
                json!({"text":"hello"})
            );
        }
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn rpc_success_uses_http_and_message_and_allows_absent_data() -> Result<()> {
    let module = crate::test_support::installed_module();

    let wasm = module.read()?;
    let api = Api::new(&wasm, &module.version)?;
    let (mut client, root) = client(api);
    let (transport, server) = server_responses(vec![
        ("getProfile", Box::new(|_| (200, json!({"message":"OK"})))),
        (
            "getProfile",
            Box::new(|_| {
                (
                    200,
                    json!({"message":"OK", "code":0, "data":{"displayName":"Name"}}),
                )
            }),
        ),
        ("getProfile", Box::new(|_| (200, json!({"code":0})))),
        ("getProfile", Box::new(|_| (201, json!({"message":"OK"})))),
    ]);
    client.api.http = transport;
    assert_eq!(client.api.profile()?, Value::Null);
    assert_eq!(client.api.profile()?["displayName"], "Name");
    for http in [200, 201] {
        let error = client.api.profile().unwrap_err();
        let error = error.downcast_ref::<RpcError>().unwrap();
        assert_eq!((error.http, error.code), (http, 0));
    }
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn refused_key_fetch_renders_undecryptable_and_loads_following_messages() -> Result<()> {
    let module = crate::test_support::installed_module();

    let wasm = module.read()?;
    let api = Api::new(&wasm, &module.version)?;
    let (mut client, root) = client(api);
    let own = client
        .api
        .rt
        .static_call(Call::Curve25519KeyGenerate, &[1])?;
    client.api.own_keys.insert(99, own);
    let encrypted = json!({"id":"101","from":"upeer","to":"uself","chunks":["", "", "", "AAAAKg==", "AAAAYw=="],"contentType":0});
    let valid = json!({"id":"102","from":"upeer","to":"uself","text":"still here","contentType":0});
    let page = json!({"code":0,"message":"OK","data":[encrypted.clone(),valid]});
    let (http, server) = server_responses(vec![
        ("getRecentMessagesV2", Box::new(move |_| (200, page))),
        (
            "getE2EEPublicKey",
            Box::new(|_| (200, wire_fixture::refused_key())),
        ),
        (
            "getE2EEPublicKey",
            Box::new(|_| (200, wire_fixture::signed_out())),
        ),
    ]);
    client.api.http = http;
    let messages = client.messages("upeer")?;
    assert_eq!(messages[0].content, Content::Undecryptable);
    assert_eq!(messages[1].content.as_text(), "still here");
    let (_, live) = client
        .operation(&json!({"type":26,"message":encrypted}))?
        .unwrap();
    assert_eq!(live.content, Content::Undecryptable);
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}
