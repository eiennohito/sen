//! Explicit native sign-in. QR/PIN data stays in memory; old sessions are retained.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Protocol progress, independent of desktop wording. Display retains CLI logs.
#[derive(Clone, Copy, Debug)]
pub enum Stage {
    WaitingKeyring,
    RecoveringKeys,
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
    RestoredKeys(usize),
}
impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingKeyring => f.write_str("Waiting for the keyring to unlock…"),
            Self::RecoveringKeys => f.write_str("Recovering E2EE keys from saved pairing metadata"),
            Self::Authenticated => {
                f.write_str("SUCCESS: saved session authenticated and E2EE keys restored")
            }
            Self::CreatingSession => f.write_str("Creating Chrome QR pairing session"),
            Self::AwaitScan => f.write_str("QR ready; waiting for phone scan"),
            Self::CheckingCertificate => f.write_str("QR scanned; checking device certificate"),
            Self::AwaitPin => f.write_str("Waiting for phone PIN verification"),
            Self::RequestCredentials => {
                f.write_str("Phone approved; requesting session credentials")
            }
            Self::UnwrappingKeys => f.write_str(
                "Session credentials received; unwrapping Letter Sealing keychain in WASM",
            ),
            Self::KeysVerified => {
                f.write_str("Letter Sealing keys unwrapped and export/import verified")
            }
            Self::CheckingKeys => {
                f.write_str("Authenticated getProfile succeeded; validating saved keys")
            }
            Self::Synthetic => f.write_str("Synthetic sign-in"),
            Self::RestoredKeys(count) => write!(
                f,
                "Restored {count} Letter Sealing key(s) in a fresh WASM instance"
            ),
        }
    }
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
    cancel: &'a AtomicBool,
}
impl<F: Fn(Event)> LoginUi for NativeLogin<'_, F> {
    fn check(&self) -> Result<()> {
        ensure!(!self.cancel.load(Ordering::Relaxed), "sign-in cancelled");
        Ok(())
    }
    fn set(&self, stage: Stage) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Stage(stage));
        Ok(())
    }
    fn qr(&self, _link: &str, png: &[u8]) -> Result<()> {
        self.check()?;
        let image = image::load_from_memory(png)?.to_rgba8();
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

/// Never called at startup or on a network error: requires explicit user action.
/// A new pairing has separate encrypted draft storage, even for the same person.
pub fn sign_in(
    state: &Path,
    wasm_override: Option<PathBuf>,
    version_override: Option<String>,
    cancel: &AtomicBool,
    emit: impl Fn(Event),
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(super::error::Error::SignIn(super::error::SignInFailure::Cancelled).into());
    }
    let module = crate::crypto_module::resolve(wasm_override, version_override)?;
    sign_in_validated(state, module, cancel, emit, |_, _, _| Ok(()))
}

pub(crate) fn sign_in_validated<T>(
    state: &Path,
    module: crate::crypto_module::CryptoModule,
    cancel: &AtomicBool,
    emit: impl Fn(Event),
    validate: impl FnOnce(&Path, Api, Vec<u8>) -> Result<T>,
) -> Result<T> {
    let ui = NativeLogin { emit, cancel };
    ui.check()?;
    let wasm = module.read()?;
    let mut api = Api::new(&wasm, &module.version)?;
    ui.set(Stage::WaitingKeyring)?;
    let probe_state = state.to_owned();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    // Filesystem probes and Secret Service may block without reading our token.
    // Poll their result on the pairing owner so cancellation also releases the
    // network queue. The probe owns no pairing session and never publishes UI.
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let state = probe_state;
            // Fail local prerequisites before requesting a phone approval.
            let parent = state
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let probe_directory = || -> Result<u128> {
                let new_directory = !parent.exists();
                fs::create_dir_all(parent)?;
                if new_directory {
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
                }
                let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                let probe = state.with_extension(format!("sign-in-{stamp}.check"));
                private_write(&probe, b"sign-in storage check")?;
                fs::remove_file(probe)?;
                Ok(stamp)
            };
            let stamp = probe_directory().context(super::error::Error::SignIn(
                super::error::SignInFailure::StateDirectory(parent.into()),
            ))?;
            crate::credentials::check_writable().context(super::error::Error::SignIn(
                super::error::SignInFailure::Keyring,
            ))?;
            Ok(stamp)
        })();
        let _ = send.send(result);
    });
    let stamp = loop {
        ui.check()?;
        match receive.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => break result?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("sign-in prerequisite worker exited")
            }
        }
    };
    ui.check()?;
    let parent = state
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staged = state.with_extension(format!("sign-in-{stamp}.json"));
    let opts = Options {
        command: "login".into(),
        state: staged.clone(),
        wasm_override: None,
        version_override: None,
        qr_file: None,
        display: "none".into(),
    };
    login_with(&opts, &mut api, &ui)?;
    let validated = validate(&staged, api, wasm)?;
    ui.check()?;
    install_session(state, &staged, Some(cancel)).context(super::error::Error::SignIn(
        super::error::SignInFailure::StateDirectory(parent.into()),
    ))?;
    Ok(validated)
}

fn install_session(state: &Path, staged: &Path, cancel: Option<&AtomicBool>) -> Result<()> {
    let bytes = fs::read(staged)?;
    // Preserve old credentials so their separately encrypted drafts remain
    // recoverable. Never re-key or migrate drafts into a newly paired account.
    if state.exists() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let backup = state.with_extension(format!("before-sign-in-{stamp}.json"));
        private_write(&backup, &fs::read(state)?)?;
    }
    ensure!(
        !cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)),
        super::error::Error::SignIn(super::error::SignInFailure::Cancelled)
    );
    private_write(state, &bytes)?;
    let _ = fs::remove_file(staged);
    Ok(())
}

/// Classify at the pairing boundary before protocol diagnostics are discarded.
pub fn failure(error: &anyhow::Error, cancelled: bool) -> super::error::SignInFailure {
    use super::error::{Error, SignInFailure};
    if cancelled {
        return SignInFailure::Cancelled;
    }
    if let Some(Error::SignIn(reason)) = error.downcast_ref::<Error>() {
        return reason.clone();
    }
    if let Some(e) = error.downcast_ref::<super::RpcError>() {
        return if e.code == 10052 || e.status == 410 {
            SignInFailure::Phone
        } else {
            SignInFailure::Server
        };
    }
    if matches!(error.downcast_ref::<Error>(), Some(Error::Offline(_))) {
        return SignInFailure::Network;
    }
    if error
        .chain()
        .any(|e| e.is::<std::io::Error>() || e.is::<ureq::Error>())
    {
        return SignInFailure::Network;
    }
    if error.to_string() == "QR code expired before phone verification" {
        return SignInFailure::Phone;
    }
    SignInFailure::Internal
}

pub(super) fn phone_error(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<super::RpcError>().is_some() {
        error.context(super::error::Error::SignIn(
            super::error::SignInFailure::Phone,
        ))
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                &AtomicBool::new(false),
                |event| {
                    assert!(
                        matches!(event, Event::Stage(Stage::WaitingKeyring)),
                        "prerequisite failure requested a QR"
                    );
                },
                |_, _, _| Ok(()),
            )
            .unwrap_err();
            assert_eq!(
                failure(&error, false),
                if exists {
                    SignInFailure::CryptoChanged {
                        version: "9.8.7".into(),
                        missing: "WASM module".into(),
                    }
                } else {
                    SignInFailure::CryptoModule(wasm.clone())
                }
            );
            assert!(!state.exists());
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn transport_and_phone_errors_keep_their_categories() {
        use super::super::error::{Error, NetCause, SignInFailure};
        let offline =
            anyhow::Error::new(Error::Offline(NetCause::Unreachable)).context("transport");
        assert_eq!(failure(&offline, false), SignInFailure::Network);
        let rpc = super::super::RpcError {
            method: "checkPinCodeVerified".into(),
            http: 403,
            code: 1,
            status: 403,
        };
        assert_eq!(
            failure(&phone_error(rpc.into()), false),
            SignInFailure::Phone
        );
        assert_eq!(failure(&offline, true), SignInFailure::Cancelled);
    }

    #[test]
    fn qr_png_reaches_the_desktop_as_rgba() -> Result<()> {
        let link = "line://au/q/synthetic?secret=synthetic&e2eeVersion=1";
        let modules = qrcodegen::QrCode::encode_text(link, qrcodegen::QrCodeEcc::Low)?.size();
        let png = qr_png(link)?;
        let events = std::cell::RefCell::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let ui = NativeLogin {
            emit: |event| events.borrow_mut().push(event),
            cancel: &cancel,
        };
        ui.qr("unused", &png)?;
        let Event::Qr {
            width,
            height,
            rgba,
        } = events.borrow_mut().pop().unwrap()
        else {
            panic!("expected QR image");
        };
        let size = ((modules + 8) * 10) as u32;
        assert_eq!((width, height), (size, size));
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        let (pixels, remainder) = rgba.as_chunks::<4>();
        assert!(remainder.is_empty());
        assert!(
            pixels
                .iter()
                .all(|pixel| matches!(pixel, [0, 0, 0, 255] | [255, 255, 255, 255]))
        );
        assert!(pixels.contains(&[0, 0, 0, 255]));
        assert!(pixels.contains(&[255, 255, 255, 255]));
        cancel.store(true, Ordering::Relaxed);
        assert!(ui.qr("unused", &png).is_err());
        assert!(events.borrow().is_empty());
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
        assert_eq!(fs::metadata(backup)?.permissions().mode() & 0o777, 0o600);
        assert!(
            sign_in(
                &state,
                Some(root.join("unused.wasm")),
                None,
                &AtomicBool::new(true),
                |_| panic!("cancelled flow emitted an event")
            )
            .is_err()
        );
        assert_eq!(fs::read(&state)?, b"new session");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
