use super::*;
pub(super) fn client(mut api: Api) -> (Client, PathBuf) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "line-crypto-send-tests-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    api.state.token_v3_issue_result.access_token = "synthetic".into();
    api.stream_access = Arc::new(std::sync::RwLock::new("synthetic".into()));
    api.session_path = Some(root.join("session.json"));
    let client = Client {
        api,
        my_mid: "U00000000000000000000000000000001".into(),
        obs_access: None,
        sequence: 0,
        cache: Arc::new(std::sync::Mutex::new(
            ImageCache::new(&root, "U00000000000000000000000000000001").unwrap(),
        )),
    };
    (client, root)
}

pub(crate) fn local_client(
    root: &Path,
    mid: &str,
    mut transport: http::Transport,
) -> Result<Client> {
    let module = crate::test_support::installed_module();
    let mut api = Api::new(&module)?;
    transport.client_version = module.version;
    api.http = transport;
    Client::from_api(
        &root.join("session.json"),
        |mid| Ok(Arc::new(std::sync::Mutex::new(ImageCache::new(root, mid)?))),
        serde_json::from_value(json!({
            "token_v3_issue_result":{"accessToken":"synthetic"},
            "e2ee_keys":[], "profile":{"mid":mid}
        }))?,
        api,
    )
}
