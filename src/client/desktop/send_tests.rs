use super::test_client::client;
use super::*;
use crate::client::error::wire_fixture::{self, GROUP_KEY_ROTATED, KEY_ROTATED, SENDER_KEY_STALE};
use crate::client::http::test_support::{Reply, server_responses};

#[test]
fn rotated_peer_key_refetches_and_resends_once() -> Result<()> {
    let module = crate::test_support::installed_module();

    let wasm = module.read()?;
    let mut rt = Runtime::from_module(crate::test_support::compiled_module(), &module.version)?;
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
            steps.push(("/api/talk/thrift/Talk/TalkService/negotiateE2EEPublicKey", Box::new(move |args| {
                assert_eq!(args, json!(["U00000000000000000000000000000002"]));
                (200, json!({"code":0,"message":"OK","data":{"allowedTypes":[0],"specVersion":1,"publicKey":{"keyId":id}}}))
            })));
            steps.push((
                "/api/talk/thrift/Talk/TalkService/getE2EEPublicKey",
                Box::new(move |args| {
                    assert_eq!(args, json!(["U00000000000000000000000000000002", 1, id]));
                    (
                        200,
                        json!({"code":0,"message":"OK","data":{"keyId":id,"keyData":STANDARD.encode(public)}}),
                    )
                }),
            ));
            let sent = sent.clone();
            steps.push(("/api/talk/thrift/Talk/TalkService/sendMessage", Box::new(move |args| {
                assert_eq!(args[0], 7);
                assert_eq!(args[1]["relatedMessageId"], "123");
                assert_eq!(args[1]["messageRelationType"], 3);
                assert_eq!(args[1]["relatedMessageServiceCode"], 2);
                assert_eq!(decode(args[1]["chunks"][4].as_str().unwrap()).unwrap(), (id as u32).to_be_bytes());
                sent.lock().unwrap().push(args[1].clone());
                if id == 42 {
                    (200, wire_fixture::talk(code))
                } else {
                    (200, json!({"code":0,"message":"OK","data":{"id":"104","from":"U00000000000000000000000000000001","to":"U00000000000000000000000000000002","contentType":0,"text":"hello"}}))
                }
            })));
        }
        let (mut transport, worker) = server_responses(steps);
        transport.client_version = module.version.clone();
        client.api.http = transport;
        let result = client.send_text_with_request(
            "U00000000000000000000000000000002",
            "hello",
            7,
            Some(&crate::store::ReplyReference {
                message_id: crate::store::ServerId::new("123").unwrap(),
                service_code: Some(2),
            }),
        );
        if code == SENDER_KEY_STALE {
            let error = result.expect_err("stale sender key accepted");
            assert!(matches!(
                super::super::error::classify(&error),
                super::super::error::Failure::Refused(super::super::error::Refusal::Server(
                    crate::client::error::ServerRefusal::Talk(SENDER_KEY_STALE)
                ))
            ));
        } else {
            assert_eq!(result?.server_id.as_str(), "104");
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
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    let (transport, server) = server_responses(vec![
        (
            "/api/talk/thrift/Talk/TalkService/getProfile",
            Box::new(|_| (200, json!({"message":"OK"}))),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getProfile",
            Box::new(|_| {
                (
                    200,
                    json!({"message":"OK", "code":0, "data":{"displayName":"Name"}}),
                )
            }),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getProfile",
            Box::new(|_| (200, json!({"code":0}))),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getProfile",
            Box::new(|_| (201, json!({"message":"OK"}))),
        ),
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

    let api = Api::new(&module)?;
    let (mut client, root) = client(api);
    let own = client
        .api
        .rt
        .static_call(Call::Curve25519KeyGenerate, &[1])?;
    client.api.own_keys.insert(99, own);
    let encrypted = json!({"id":"101","from":"U00000000000000000000000000000002","to":"U00000000000000000000000000000001","chunks":["", "", "", "AAAAKg==", "AAAAYw=="],"contentType":0});
    let valid = json!({"id":"102","from":"U00000000000000000000000000000002","to":"U00000000000000000000000000000001","text":"still here","contentType":0});
    let page = json!({"code":0,"message":"OK","data":[encrypted.clone(),valid]});
    let (http, server) = server_responses(vec![
        (
            "/api/talk/thrift/Talk/TalkService/getRecentMessagesV2",
            Box::new(move |_| (200, page)),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getE2EEPublicKey",
            Box::new(|_| (200, wire_fixture::refused_key())),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getE2EEPublicKey",
            Box::new(|_| (200, wire_fixture::signed_out())),
        ),
    ]);
    client.api.http = http;
    let messages = client.refresh_messages("U00000000000000000000000000000002")?;
    assert_eq!(messages[0].body, Content::Undecryptable);
    assert_eq!(messages[1].body.as_text(), "still here");
    for kind in [25, 43] {
        let live = client.operation(&json!({"type":kind,"message":{"id":"103", "from":"U00000000000000000000000000000001", "to":"U00000000000000000000000000000002", "text":"outgoing", "contentType":0}}).into())?.expect("outgoing operation was dropped");
        assert_eq!(live.chat_id.0, "U00000000000000000000000000000002");
        assert_eq!(live.body.as_text(), "outgoing");
        assert!(live.outgoing);
    }
    let live = client
        .operation(&json!({"type":26,"message":encrypted}).into())?
        .unwrap();
    assert_eq!(live.body, Content::Undecryptable);
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_message_rows_do_not_hide_valid_history_or_invent_reactions() -> Result<()> {
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    let (http, server) = server_responses(vec![(
        "/api/talk/thrift/Talk/TalkService/getRecentMessagesV2",
        Box::new(|_| {
            (
                200,
                json!({"code":0,"message":"OK","data":[
                    {"id":"bad", "text":"invalid"},
                    {"id":"01", "text":"noncanonical"},
                    {"id":"123456789012345678901", "text":"too long"},
                    {"text":"missing ID"},
                    {"id":"100", "from":"U00000000000000000000000000000002", "text":"valid history", "createdTime":"123", "reactions":[
                        {"fromUserMid":"other", "reactionType":null},
                        {"fromUserMid":"other", "reactionType":{}},
                        {"fromUserMid":"other", "reactionType":{"unknownReactionType":{}}},
                        {"fromUserMid":"other", "reactionType":{"predefinedReactionType":"invalid"}},
                        {"fromUserMid":"other", "reactionType":{"predefinedReactionType":2}}
                    ]}
                ]}),
            )
        }),
    )]);
    client.api.http = http;
    let messages = client.refresh_messages("U00000000000000000000000000000002")?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body.as_text(), "valid history");
    assert_eq!(messages[0].created_ms, 123);
    assert_eq!(
        messages[0].reactions,
        vec![Reaction {
            actor: "other".into(),
            kind: ReactionKind::Predefined(2)
        }]
    );
    let invalid =
        json!({"type":26, "revision":"5", "message":{"from":"U00000000000000000000000000000002", "to":"U00000000000000000000000000000001", "id":"bad"}})
            .into();
    assert!(client.operation(&invalid)?.is_none());
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn wire_request_sequences_and_contact_numbers_survive_decoding() -> Result<()> {
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    for field in ["reqSeq", "requestSeq"] {
        for seq in [json!(7), json!("7")] {
            let mut raw = json!({"id":"100", "text":"body"});
            raw[field] = seq;
            assert_eq!(
                client
                    .message("U00000000000000000000000000000002", &raw)?
                    .unwrap()
                    .request_seq,
                Some(7)
            );
        }
    }
    assert_eq!(
        client
            .message("chat", &json!({"id":"100", "reqSeq":0}))?
            .unwrap()
            .request_seq,
        None
    );
    for favorite in [json!(7), json!("7"), Value::Null, json!("bad"), json!({})] {
        let profile: crate::store::ContactProfile =
            serde_json::from_value(json!({"favoriteTime":favorite}))?;
        assert_eq!(
            profile.favorite_time,
            if favorite == json!(7) || favorite == json!("7") {
                Some(7)
            } else {
                None
            }
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn obs_token_renewal_replaces_the_shared_cached_token() -> Result<()> {
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    let (http, server) = server_responses(vec![
        (
            "/api/talk/thrift/Talk/TalkService/acquireEncryptedAccessToken",
            Box::new(|_| {
                (
                    200,
                    json!({"code":0,"message":"OK","data":"x\u{001e}stale\u{001f}rest"}),
                )
            }),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/acquireEncryptedAccessToken",
            Box::new(|_| {
                (
                    200,
                    json!({"code":0,"message":"OK","data":"x\u{001e}fresh\u{001f}rest"}),
                )
            }),
        ),
    ]);
    client.api.http = http;
    assert_eq!(client.media_token(false)?, "stale");
    assert_eq!(client.media_token(false)?, "stale");
    assert_eq!(client.media_token(true)?, "fresh");
    assert_eq!(client.media_token(false)?, "fresh");
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_contacts_do_not_discard_friends_or_fail_lookup() -> Result<()> {
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    const GOOD: &str = "U00000000000000000000000000000002";
    const BAD: &str = "U00000000000000000000000000000003";
    let contacts = || {
        json!({"code":0,"message":"OK","data":{"contacts":{
            (GOOD):{"contact":{"displayName":"Good"}}, (BAD):{"contact":{"displayName":"Bad","type":null}}
        }}})
    };
    let (http, server) = server_responses(vec![
        (
            "/api/talk/thrift/Talk/TalkService/getAllContactIds",
            Box::new(|_| (200, json!({"code":0,"message":"OK","data":[GOOD,BAD]}))),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getContactsV2",
            Box::new(move |_| (200, contacts())),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getAllChatMids",
            Box::new(|_| {
                (
                    200,
                    json!({"code":0,"message":"OK","data":{"memberChatMids":[]}}),
                )
            }),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getContactsV2",
            Box::new(move |_| (200, contacts())),
        ),
        (
            "/api/talk/thrift/Talk/TalkService/getContactsV2",
            Box::new(move |_| (200, contacts())),
        ),
    ]);
    client.api.http = http;
    let friends = client.friends()?;
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].name.as_deref(), Some("Good"));
    assert!(client.contact(BAD)?.is_none());
    assert_eq!(client.contact(GOOD)?.unwrap().name.as_deref(), Some("Good"));
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn sticker_to_type_uses_uppercase_mid_prefixes_and_unknown_defaults_to_user() -> Result<()> {
    let api = Api::without_crypto()?;
    let (mut client, root) = client(api);
    let mut steps: Vec<(&'static str, Reply)> = Vec::new();
    for (prefix, expected) in [
        ('U', 0),
        ('C', 2),
        ('R', 1),
        ('u', 0),
        ('c', 0),
        ('r', 0),
        ('X', 0),
    ] {
        steps.push(("/api/talk/thrift/Talk/TalkService/sendMessage", Box::new(move |args| {
            assert_eq!(args[1]["toType"], expected);
            (200, json!({"code":0,"message":"OK","data":{"id":"100","from":"U00000000000000000000000000000001","to":format!("{prefix}{:032x}", 2),"contentType":7}}))
        })));
    }
    let (http, server) = server_responses(steps);
    client.api.http = http;
    let sticker = Content::Sticker {
        pkg: "1".into(),
        id: "2".into(),
        ver: None,
        asset: None,
        text: None,
    };
    for prefix in ['U', 'C', 'R', 'u', 'c', 'r', 'X'] {
        client.send_sticker(&format!("{prefix}{:032x}", 2), &sticker, 1, None)?;
    }
    server.join().unwrap();
    fs::remove_dir_all(root)?;
    Ok(())
}
