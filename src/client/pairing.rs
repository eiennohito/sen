//! Explicit native sign-in. QR/PIN data stays in memory; old sessions are retained.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub enum Event {
    Stage(String),
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
    fn set(&self, stage: &str) -> Result<()> {
        self.check()?;
        (self.emit)(Event::Stage(stage.into()));
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
    wasm_path: &Path,
    cancel: &AtomicBool,
    emit: impl Fn(Event),
) -> Result<()> {
    let ui = NativeLogin { emit, cancel };
    ui.check()?;
    let wasm = fs::read(wasm_path).context("read crypto module")?;
    ensure!(
        format!("{:x}", Sha256::digest(&wasm)) == HASH,
        "unsupported crypto module"
    );
    // Fail local prerequisites before requesting a phone approval.
    let parent = state
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let new_directory = !parent.exists();
    fs::create_dir_all(parent)?;
    if new_directory {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let probe = state.with_extension(format!("sign-in-{stamp}.check"));
    private_write(&probe, b"sign-in storage check")?;
    fs::remove_file(probe)?;
    crate::credentials::check_writable()?;
    ui.check()?;
    let staged = state.with_extension(format!("sign-in-{stamp}.json"));
    let opts = Options {
        command: "login".into(),
        state: staged.clone(),
        wasm: wasm_path.into(),
        qr_file: None,
        display: "none".into(),
    };
    login_with(&opts, &wasm, &ui)?;
    ui.check()?;
    install_session(state, &staged)?;
    Ok(())
}

fn install_session(state: &Path, staged: &Path) -> Result<()> {
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
        install_session(&state, &staged)?;
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
                &root.join("unused.wasm"),
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
