use super::test_client::{Reply, client, server_responses};
use super::*;

#[test]
fn rotated_peer_key_refetches_and_resends_once() -> Result<()> {
    let wasm = crate::test_support::read_fixture(&crate::test_support::fixture_path())?;
    let mut rt = Runtime::new(&wasm, crate::test_support::FIXTURE_VERSION)?;
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
        receivers.push(chat::channel(&mut rt, peer, &own_public)?);
    }
    let mut api = Api::with_runtime(rt, &wasm, crate::test_support::FIXTURE_VERSION)?;
    api.signature("/synthetic", "", "synthetic")?;
    let (mut client, root) = client(api);
    client.state["e2ee_keys"] = json!([{"id":99}]);
    client.api.own_keys.insert(99, own);
    for code in [82, 83, 84] {
        client.api.key_cache.clear();
        for (_, (channel, _)) in client.api.channels.drain() {
            client.api.rt.destroy("E2EEChannel", channel)?;
        }
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut steps: Vec<(&'static str, Reply)> = Vec::new();
        for (id, public) in [42u64, 43]
            .into_iter()
            .zip(&public_keys)
            .take(if code == 83 { 1 } else { 2 })
        {
            let public = public.clone();
            steps.push(("negotiateE2EEPublicKey", Box::new(move |args| {
                assert_eq!(args, json!(["upeer"]));
                (200, json!({"code":0,"data":{"allowedTypes":[0],"specVersion":1,"publicKey":{"keyId":id}}}))
            })));
            steps.push((
                "getE2EEPublicKey",
                Box::new(move |args| {
                    assert_eq!(args, json!(["upeer", 1, id]));
                    (
                        200,
                        json!({"code":0,"data":{"keyId":id,"keyData":STANDARD.encode(public)}}),
                    )
                }),
            ));
            let sent = sent.clone();
            steps.push(("sendMessage", Box::new(move |args| {
                assert_eq!(args[0], 7);
                assert_eq!(args[1]["relatedMessageId"], "original");
                assert_eq!(args[1]["messageRelationType"], 3);
                assert_eq!(args[1]["relatedMessageServiceCode"], 1);
                assert_eq!(decode(args[1]["chunks"][4].as_str().unwrap()).unwrap(), (id as u32).to_be_bytes());
                sent.lock().unwrap().push(args[1].clone());
                if id == 42 {
                    (200, json!({"code":code,"data":{}}))
                } else {
                    (200, json!({"code":0,"data":{"id":"sent","from":"uself","to":"upeer","contentType":0,"text":"hello"}}))
                }
            })));
        }
        let (mut transport, worker) = server_responses(steps);
        transport.client_version = crate::test_support::FIXTURE_VERSION.into();
        client.api.http = transport;
        let result = client.send_text_with_request(
            "upeer",
            "hello",
            7,
            Some(&crate::store::ReplyReference {
                message_id: "original".into(),
                service_code: Some(1),
            }),
        );
        if code == 83 {
            let error = result.err().expect("stale sender key accepted");
            assert!(matches!(
                error.downcast_ref::<SendFailure>(),
                Some(SendFailure::Refused)
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
fn void_calls_accept_gateway_ok_without_data_and_empty_success() -> Result<()> {
    let wasm = crate::test_support::read_fixture(&crate::test_support::fixture_path())?;
    let api = Api::new(&wasm, crate::test_support::FIXTURE_VERSION)?;
    let (mut client, root) = client(api);
    let (transport, server) = server_responses(vec![
        (
            "sendChatChecked",
            Box::new(|args| {
                assert_eq!(args, json!([7, "chat", "123", 0]));
                (200, json!({"message":"OK"}))
            }),
        ),
        (
            "react",
            Box::new(|args| {
                assert_eq!(
                    args,
                    json!([{"reqSeq":8,"messageId":"123","reactionType":{"predefinedReactionType":3}}])
                );
                (204, Value::Null)
            }),
        ),
        (
            "cancelReaction",
            Box::new(|args| {
                assert_eq!(args, json!([{"reqSeq":9,"messageId":"123"}]));
                (200, json!({"message":"OK"}))
            }),
        ),
        (
            "react",
            Box::new(|_| (200, json!({"message":"ERROR","code":5}))),
        ),
        ("getProfile", Box::new(|_| (200, Value::Null))),
    ]);
    client.api.http = transport;
    client.mark_read("chat", "123", 7)?;
    client.react("123", Some(3), 8)?;
    client.react("123", None, 9)?;
    assert!(matches!(
        error::classify(&client.react("123", Some(3), 10).unwrap_err()),
        Error::Server(5)
    ));
    let error = client.api.profile("synthetic").unwrap_err();
    assert!(
        matches!(error.downcast_ref::<Error>(), Some(Error::ResponseDecode { method, status: 200, content_type, body_len: 0 }) if method == "getProfile" && content_type.is_empty())
    );
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}
