//! Explicit native sign-in. QR/PIN data stays in memory; one previous session is retained.
use super::*;
use crate::client::http::CancelToken;

/// Protocol progress, independent of desktop wording.
#[derive(Clone, Copy, Debug)]
pub enum Stage {
    WaitingKeyring,
    Authenticated,
    CreatingSession,
    AwaitScan,
    CheckingCertificate,
    AwaitPin,
    RequestCredentials,
    UnwrappingKeys,
    KeysVerified,
    CheckingKeys,
    Synthetic,
}

pub enum Event {
    Stage(Stage),
    Qr {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Scanned,
    Pin(String),
}
struct NativeLogin<'a, F> {
    emit: F,
    cancel: &'a CancelToken,
}
impl<F: Fn(Event)> NativeLogin<'_, F> {
    fn check(&self) -> Result<()> {
        ensure!(!self.cancel.is_cancelled(), "sign-in cancelled");
        Ok(())
    }
    fn set(&self, stage: Stage) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Stage(stage));
        Ok(())
    }
    fn qr(&self, image: image::RgbaImage) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Qr {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        });
        Ok(())
    }
    fn scanned(&self) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Scanned);
        Ok(())
    }
    fn pin(&self, pin: &str) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Pin(pin.into()));
        Ok(())
    }
}

fn login_with(
    state_path: &Path,
    api: &mut Api,
    progress: &NativeLogin<'_, impl Fn(Event)>,
) -> Result<()> {
    progress.check()?;
    ensure!(!state_path.exists(), "pairing session file already exists");
    progress.set(Stage::CreatingSession)?;
    let created = api.qr("createSession", json!({}), "", 0)?;
    let session = required(&created, "authSessionId")?.to_owned();
    let qr = api.qr("createQrCode", json!({"authSessionId":session}), "", 0)?;
    let key = api.rt.static_call(Call::Curve25519KeyGenerate, &[1])?;
    let public = bytes_method(&mut api.rt, Call::Curve25519KeyGetPublicKey, key, &[])?;
    let link = callback(required(&qr, "callbackUrl")?, &public)?;
    progress.qr(qr::rgba(&link)?)?;
    progress.set(Stage::AwaitScan)?;
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
                    .is_some_and(RpcError::qr_expired);
                if !expired || started.elapsed() > Duration::from_secs(300) {
                    return Err(error::phone_error(error));
                }
            }
        }
    }
    ensure!(verified, error::SignInFailure::Phone);
    progress.check()?;
    progress.scanned()?;
    progress.set(Stage::CheckingCertificate)?;
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
                    .is_some_and(RpcError::certificate_missing),
                "certificate check failed: {error}"
            );
            let pin = api.qr("createPinCode", json!({"authSessionId":session}), "", 0)?;
            let pin = required(&pin, "pinCode")?;
            ensure!(
                (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit()),
                "invalid verification PIN format"
            );
            progress.pin(pin)?;
            progress.set(Stage::AwaitPin)?;
            api.qr(
                "checkPinCodeVerified",
                json!({"authSessionId":session}),
                &session,
                110,
            )
            .map_err(error::phone_error)?;
        }
    }
    progress.check()?;
    progress.set(Stage::RequestCredentials)?;
    let result=api.qr("qrCodeLoginV2",json!({"authSessionId":session,"systemName":"CHROMEOS","modelName":"CHROME","autoLoginIsRequired":false}),"",0)?;
    let mut token = result["tokenV3IssueResult"].clone();
    if token["accessToken"].as_str().is_none_or(|s| s.is_empty()) {
        token = json!({"accessToken":required(&result,"authToken")?});
    }
    progress.set(Stage::UnwrappingKeys)?;
    let keys = unwrap_keys(&mut api.rt, key, &result["metaData"])?;
    let mut state = Session {
        format: 1,
        token_v3_issue_result: serde_json::from_value(token)?,
        certificate: result["certificate"].clone(),
        login_metadata: result["metaData"].clone(),
        e2ee_keys: keys,
        ..Default::default()
    };
    save_state(state_path, &state)?;
    progress.set(Stage::KeysVerified)?;
    api.rt.destroy("Curve25519Key", key)?;
    api.state = state.clone();
    let profile = api.profile()?;
    ensure!(
        profile["mid"].as_str().is_some_and(|s| !s.is_empty()),
        "authenticated profile missing"
    );
    state.profile = serde_json::from_value(profile)?;
    api.state = state.clone();
    save_state(state_path, &state)?;
    progress.set(Stage::CheckingKeys)?;
    validate_keys(api, &state)?;
    progress.set(Stage::Authenticated)
}

/// Never called at startup or on a network error: requires explicit user action.
/// A new pairing has separate encrypted draft storage, even for the same person.
pub fn sign_in(
    state: &Path,
    wasm_override: Option<PathBuf>,
    version_override: Option<String>,
    cancel: &CancelToken,
    emit: impl Fn(Event),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(super::error::SignInFailure::Cancelled.into());
    }
    let module = crate::crypto_module::resolve(wasm_override, version_override)?;
    sign_in_validated(
        state,
        module,
        NetworkActivity::default(),
        cancel,
        emit,
        |_, _| Ok(()),
    )
}

pub(crate) fn sign_in_validated<T>(
    state: &Path,
    module: crate::crypto_module::CryptoModule,
    activity: NetworkActivity,
    cancel: &CancelToken,
    emit: impl Fn(Event),
    validate: impl FnOnce(&Path, Api) -> Result<T>,
) -> Result<T> {
    let ui = NativeLogin { emit, cancel };
    ui.check()?;
    let wasm = module.read()?;
    let mut api = Api::with_runtime(module.runtime(&wasm)?, &wasm, &module.version)?;
    api.http.activity = activity;
    ui.set(Stage::WaitingKeyring)?;
    let probe_state = state.to_owned();
    let (send, receive) = async_channel::bounded(1);
    // Race prerequisite completion against cancellation without polling.
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let state = probe_state;
            // Fail local prerequisites before requesting a phone approval.
            let parent = state
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let probe_directory = || -> Result<u128> {
                private_directory(parent)?;
                let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                let probe = state.with_extension(format!("sign-in-{stamp}.check"));
                private_write(&probe, b"sign-in storage check")?;
                fs::remove_file(probe)?;
                Ok(stamp)
            };
            let stamp = probe_directory()
                .context(super::error::SignInFailure::StateDirectory(parent.into()))?;
            crate::credentials::check_writable().context(super::error::SignInFailure::Keyring)?;
            Ok(stamp)
        })();
        let _ = send.send_blocking(result);
    });
    let stamp = futures_lite::future::block_on(futures_lite::future::or(
        async {
            receive
                .recv()
                .await
                .context("sign-in prerequisite worker exited")?
        },
        async {
            cancel.cancelled().await;
            Err(error::SignInFailure::Cancelled.into())
        },
    ))?;
    ui.check()?;
    let parent = state
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staged = state.with_extension(format!("sign-in-{stamp}.json"));
    login_with(&staged, &mut api, &ui)?;
    let validated = validate(&staged, api)?;
    ui.check()?;
    install_session(state, &staged, Some(cancel))
        .context(super::error::SignInFailure::StateDirectory(parent.into()))?;
    Ok(validated)
}

fn install_session(state: &Path, staged: &Path, cancel: Option<&CancelToken>) -> Result<()> {
    ensure!(
        !cancel.is_some_and(|cancel| cancel.is_cancelled()),
        super::error::SignInFailure::Cancelled
    );
    let bytes = fs::read(staged)?;
    // Preserve old credentials so their separately encrypted drafts remain
    // recoverable. Never re-key or migrate drafts into a newly paired account.
    if state.exists() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let backup = state.with_extension(format!("before-sign-in-{stamp}.json"));
        private_write(&backup, &fs::read(state)?)?;
    }
    private_write(state, &bytes)?;
    let _ = fs::remove_file(staged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn missing_or_unsupported_crypto_fails_before_requesting_a_qr() -> Result<()> {
        use super::super::error::SignInFailure;
        let root =
            std::env::temp_dir().join(format!("line-pairing-prerequisites-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let state = root.join("session.json");
        let wasm = root.join("crypto.wasm");
        for exists in [false, true] {
            if exists {
                fs::write(&wasm, b"unsupported module")?;
            }
            let error = sign_in_validated(
                &state,
                crate::crypto_module::CryptoModule {
                    wasm: wasm.clone(),
                    version: "9.8.7".into(),
                },
                NetworkActivity::default(),
                &CancelToken::new(),
                |event| {
                    assert!(
                        matches!(event, Event::Stage(Stage::WaitingKeyring)),
                        "prerequisite failure requested a QR"
                    );
                },
                |_, _| Ok(()),
            )
            .unwrap_err();
            assert_eq!(
                error::sign_in_failure(&error, false),
                SignInFailure::CryptoModule(wasm.clone())
            );
            assert!(!state.exists());
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn qr_gateway_poll_timeout_keeps_waiting_until_the_scan_event() -> Result<()> {
        let module = crate::test_support::installed_module();

        use crate::client::http::tests::{accept, local_server};
        use std::io::{BufRead, BufReader, Read};
        let (http, server) = local_server(|listener| {
            for response in [
                json!({"code":0,"message":"OK","data":{"authSessionId":"synthetic"}}),
                json!({"code":0,"message":"OK","data":{"callbackUrl":"line://au/q/test","longPollingMaxCount":2}}),
                error::wire_fixture::qr_expired(),
                json!({"code":0,"message":"OK","data":{}}),
            ] {
                let mut socket = BufReader::new(accept(&listener));
                let mut line = String::new();
                socket.read_line(&mut line).unwrap();
                assert!(line.contains("SecondaryQrCodeLogin"));
                let mut length = 0;
                loop {
                    line.clear();
                    socket.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                socket.read_exact(&mut vec![0; length]).unwrap();
                let body = response.to_string();
                write!(
                    socket.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let wasm = module.read()?;
        let mut api = Api::new(&wasm, &module.version)?;
        api.http = http;
        let cancel = CancelToken::new();
        let events = std::cell::RefCell::new(Vec::new());
        let ui = NativeLogin {
            emit: |event| {
                if matches!(event, Event::Scanned) {
                    cancel.cancel();
                }
                events.borrow_mut().push(event);
            },
            cancel: &cancel,
        };
        let root = std::env::temp_dir().join(format!("line-qr-poll-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        assert!(login_with(&root.join("session.json"), &mut api, &ui).is_err());
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::Qr { .. }))
        );
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::Scanned))
        );
        server.join().unwrap();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn replacement_preserves_old_session_and_cancellation_never_pairs() -> Result<()> {
        let root = std::env::temp_dir().join(format!("line-pairing-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let state = root.join("session.json");
        let staged = root.join("new.json");
        private_write(&state, b"old session")?;
        private_write(&staged, b"new session")?;
        install_session(&state, &staged, None)?;
        assert_eq!(fs::read(&state)?, b"new session");
        let backup = fs::read_dir(&root)?
            .find_map(|e| {
                let e = e.ok()?;
                e.file_name()
                    .to_string_lossy()
                    .contains("before-sign-in")
                    .then(|| e.path())
            })
            .unwrap();
        assert_eq!(fs::read(&backup)?, b"old session");
        assert_eq!(fs::metadata(&backup)?.permissions().mode() & 0o777, 0o600);
        private_write(&staged, b"latest session")?;
        install_session(&state, &staged, None)?;
        assert_eq!(fs::read(&backup)?, b"old session");
        assert_eq!(fs::read_dir(&root)?.count(), 3);
        let cancel = CancelToken::new();
        cancel.cancel();
        private_write(&staged, b"cancelled session")?;
        assert!(install_session(&state, &staged, Some(&cancel)).is_err());
        assert_eq!(fs::read(&backup)?, b"old session");
        assert!(
            sign_in(
                &state,
                Some(root.join("unused.wasm")),
                None,
                &{
                    let token = CancelToken::new();
                    token.cancel();
                    token
                },
                |_| panic!("cancelled flow emitted an event")
            )
            .is_err()
        );
        assert_eq!(fs::read(&state)?, b"latest session");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
