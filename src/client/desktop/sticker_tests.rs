use super::*;

#[test]
fn owned_products_request_pages_and_decode_extension_fields() -> Result<()> {
    let mut first: Value = serde_json::from_str(include_str!(
        "../../../fixtures/owned-sticker-products.json"
    ))?;
    let expired = first["productList"][1].clone();
    first["productList"]
        .as_array_mut()
        .unwrap()
        .resize(1000, expired);
    let mut second = first.clone();
    second["offset"] = json!(1000);
    second["productList"] = json!([{
        "id":"618", "name":"More stickers", "latestVersion":10,
        "productTypeSummary":{"stickerSummary":{"stickerIdRanges":[{"start":400,"size":1}]}},
        "validUntil": i64::MAX.to_string()
    }]);
    let mut replies = vec![first, second].into_iter();
    let mut requests = Vec::new();
    let packages = load_sticker_packages("ja", "JP", |args| {
        requests.push(args);
        Ok(replies.next().expect("unexpected extra sticker page"))
    })?;
    assert_eq!(
        requests,
        vec![
            json!(["stickershop",0,1000,{"language":"ja","country":"JP"}]),
            json!(["stickershop",1000,1000,{"language":"ja","country":"JP"}]),
        ]
    );
    assert!(replies.next().is_none());
    assert_eq!(packages.len(), 2);
    assert_eq!(
        (&*packages[0].id, &*packages[0].name, &*packages[0].version),
        ("617", "Fixture stickers", "9")
    );
    let ids = packages[0]
        .stickers
        .iter()
        .map(|s| match s {
            Content::Sticker { pkg, id, ver, .. } => {
                assert_eq!(pkg, "617");
                assert_eq!(ver.as_deref(), Some("9"));
                id.as_str()
            }
            _ => panic!("expected sticker"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, ["100", "101", "200"]);
    assert_eq!(packages[1].version, "10");
    Ok(())
}

#[test]
fn sticker_language_matches_extension_supported_locales_and_fallback() {
    for (locale, expected) in [
        (Some("ja"), "ja"),
        (Some("ko_KR.UTF-8"), "ko"),
        (Some("ZH_hk.UTF-8"), "zh"),
        (Some("fr_FR.UTF-8"), "en"),
        (Some("POSIX"), "en"),
        (None, "en"),
    ] {
        assert_eq!(sticker_language(locale), expected);
    }
}

#[test]
fn online_profile_keeps_region_from_get_profile() -> Result<()> {
    use super::test_client::{client, server_responses};
    let module = crate::test_support::installed_module();
    let wasm = module.read()?;
    let rt = Runtime::new(&wasm, &module.version)?;
    let api = Api::with_runtime(rt, &wasm, &module.version)?;
    let (mut client, root) = client(api);
    let (transport, server) = server_responses(vec![
        (
            "getProfile",
            Box::new(|args| {
                assert_eq!(args, json!([0]));
                (
                    200,
                    json!({"message":"OK","data":{"mid":"U00000000000000000000000000000001","displayName":"Fixture","regionCode":"TW"}}),
                )
            }),
        ),
        (
            "getContactsV2",
            Box::new(|args| {
                assert_eq!(
                    args,
                    json!([{"targetUserMids":["U00000000000000000000000000000009"],"neededContactCalendarEvents":[]},2])
                );
                (
                    200,
                    json!({"message":"OK","data":{"contacts":{"U00000000000000000000000000000009":{"contact":{"displayName":"Contact"}}}}}),
                )
            }),
        ),
    ]);
    client.api.http = transport;
    // Stop before credential persistence; this fixture never accesses a keyring.
    client.api.session_path = None;
    assert_eq!(
        client.online_profile().unwrap_err().to_string(),
        "session path"
    );
    assert_eq!(client.api.state.profile.rest["regionCode"], "TW");
    let saved = serde_json::to_value(&client.api.state)?;
    assert_eq!(saved["profile"]["regionCode"], "TW");
    assert_eq!(
        client
            .contact("U00000000000000000000000000000009")?
            .unwrap()
            .name
            .as_deref(),
        Some("Contact")
    );
    server.join().unwrap();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn owned_product_without_valid_until_is_not_expired() -> Result<()> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../fixtures/owned-sticker-products.json"
    ))?;
    value["productList"] = json!([value["productList"][0].clone()]);
    value["productList"][0]
        .as_object_mut()
        .unwrap()
        .remove("validUntil");
    value["totalSize"] = json!(1);
    let packages = load_sticker_packages("en", "US", |_| Ok(value.clone()))?;
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].id, "617");
    Ok(())
}
