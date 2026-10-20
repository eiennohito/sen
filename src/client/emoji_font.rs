//! Disk-backed bitmap emoji font; network work runs on its own startup thread.
use super::http::Transport;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const SOURCE: &str =
    "https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/2D/fonts/NotoColorEmoji.ttf";
const LICENSE: &str = "https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/2D/fonts/LICENSE";
const MONTH: u64 = 30 * 24 * 60 * 60;
const CAP: usize = 50 * 1024 * 1024;

/// Reject outline-only builds and malformed fonts before replacing an installed font.
fn validate(data: &[u8]) -> Result<()> {
    let face = ttf_parser::Face::parse(data, 0)?;
    ensure!(face.tables().cbdt.is_some(), "missing CBDT/CBLC tables");
    ensure!(
        ['😀', '🌸', '❤']
            .into_iter()
            .all(|ch| face.glyph_index(ch).is_some()),
        "missing emoji cmap entries"
    );
    let mut collection = fontique::Collection::new(fontique::CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    ensure!(
        !collection
            .register_fonts(fontique::Blob::new(Arc::new(data.to_vec())), None)
            .is_empty(),
        "no fonts registered"
    );
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    checked: u64,
    etag: Option<String>,
    modified: Option<String>,
}

/// One attempt per startup; callbacks run on the worker and must marshal UI work.
pub fn start(fonts: std::path::PathBuf, ready: impl FnOnce(Vec<u8>) + Send + 'static) {
    if let Err(error) = spawn(fonts, || Transport::new(""), ready) {
        crate::reporting::log("emoji-font", "download", &error);
    }
}
fn spawn(
    fonts: std::path::PathBuf,
    transport: impl FnOnce() -> Result<Transport> + Send + 'static,
    ready: impl FnOnce(Vec<u8>) + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("sen-emoji-font".into())
        .spawn(move || {
            let result = transport().and_then(|transport| update(&fonts, &transport, now()));
            match result {
                Ok(Some(data)) => ready(data),
                Ok(None) => {}
                Err(error) => crate::reporting::log("emoji-font", "download", error.as_ref()),
            }
        })
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn update(fonts: &Path, transport: &Transport, now: u64) -> Result<Option<Vec<u8>>> {
    let font = fonts.join("NotoColorEmoji.ttf");
    let state_path = fonts.join("emoji-update.json");
    let mut state: State = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let exists = font.is_file();
    if exists && now.saturating_sub(state.checked) < MONTH {
        return Ok(None);
    }
    // Record attempts too: an offline monthly check must not run on every launch.
    state.checked = now;
    super::private_write(&state_path, &serde_json::to_vec(&state)?)?;
    let mut headers = Vec::new();
    if exists {
        if let Some(etag) = state.etag.as_deref() {
            headers.push(("if-none-match", etag));
        }
        if let Some(modified) = state.modified.as_deref() {
            headers.push(("if-modified-since", modified));
        }
    }
    let (status, response, data) = transport.emoji_font(SOURCE, &headers, CAP)?;
    if status == 304 {
        ensure!(exists, "304 without an installed font");
        return Ok(None);
    }
    validate(&data)?;
    let license_path = fonts.join("OFL.txt");
    if !license_path.is_file() {
        let (_, _, license) = transport.emoji_font(LICENSE, &[], 1024 * 1024)?;
        ensure!(
            license
                .windows(b"SIL OPEN FONT LICENSE".len())
                .any(|s| s == b"SIL OPEN FONT LICENSE"),
            "invalid OFL license"
        );
        super::private_write(&license_path, &license)?;
    }
    super::private_write(&font, &data)?;
    state.etag = response
        .get("etag")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    state.modified = response
        .get("last-modified")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    super::private_write(&state_path, &serde_json::to_vec(&state)?)?;
    Ok(Some(data))
}

#[cfg(test)]
mod tests;
