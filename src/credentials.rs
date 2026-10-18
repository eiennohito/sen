//! Session credentials at rest.
//!
//! The session file is sealed with AES-256-GCM; its key lives in the desktop
//! Secret Service (org.freedesktop.secrets), one item per sealed file. Legacy
//! plaintext files are still readable and are sealed by `load_migrating` only
//! after a verified round trip. Sealed files are never downgraded.
//!
//! `LINE_DESKTOP_CREDENTIALS=plaintext-file` is the development store for
//! agent and debug instances without a keyring (e.g. the private D-Bus session
//! of `tools/run-ui-debug.py`): it keeps the mode-0600 plaintext file and
//! refuses sealed files.
use crate::client::private_write;
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};

const FORMAT: &str = "line-desktop-sealed-session";
pub const ENVIRONMENT: &str = "LINE_DESKTOP_CREDENTIALS";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Keyring,
    PlaintextFile,
}
impl Store {
    pub fn from_environment() -> Result<Self> {
        Self::parse(std::env::var(ENVIRONMENT).ok().as_deref())
    }
    fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("" | "keyring") => Ok(Self::Keyring),
            Some("plaintext-file") => Ok(Self::PlaintextFile),
            Some(_) => bail!("{ENVIRONMENT} must be keyring or plaintext-file"),
        }
    }
}

/// Where sealing keys are kept. The Secret Service in production; a map in tests.
pub trait Keys {
    fn get(&self, id: &str) -> Result<Option<[u8; 32]>>;
    fn put(&self, id: &str, key: &[u8; 32]) -> Result<()>;
}

pub fn load(path: &Path) -> Result<Value> {
    load_with(path, Store::from_environment()?, &secret_service::Keyring)
}
pub fn save(path: &Path, state: &Value) -> Result<()> {
    save_with(
        path,
        state,
        Store::from_environment()?,
        &secret_service::Keyring,
    )
}
/// Like `load`, and seals a legacy plaintext file when the keyring is
/// reachable. Call once per account start, before the sync thread can save.
pub fn load_migrating(path: &Path) -> Result<Value> {
    load_migrating_with(path, Store::from_environment()?, &secret_service::Keyring)
}
/// Fails early, before a phone approval, when new credentials could not be sealed.
pub fn check_writable() -> Result<()> {
    match Store::from_environment()? {
        Store::PlaintextFile => Ok(()),
        Store::Keyring => secret_service::Keyring
            .reachable()
            .context("the desktop keyring (Secret Service) is unavailable"),
    }
}

fn parse(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).context("saved session is not valid JSON")
}
fn sealed(value: &Value) -> bool {
    value["format"] == FORMAT
}

pub fn load_with(path: &Path, store: Store, keys: &impl Keys) -> Result<Value> {
    let envelope = parse(&fs::read(path).context("read saved session")?)?;
    if !sealed(&envelope) {
        return Ok(envelope);
    }
    ensure!(
        store == Store::Keyring,
        "the saved session is sealed with the desktop keyring; unset {ENVIRONMENT} to use it"
    );
    open(&envelope, keys)
}

pub fn save_with(path: &Path, state: &Value, store: Store, keys: &impl Keys) -> Result<()> {
    let plain = serde_json::to_vec_pretty(state)?;
    let existing = fs::read(path).ok().and_then(|bytes| parse(&bytes).ok());
    let existing_id = existing
        .as_ref()
        .filter(|value| sealed(value))
        .map(|value| value["key_id"].as_str().map(str::to_owned))
        .map(|id| id.context("sealed session has no key ID"))
        .transpose()?;
    if store == Store::PlaintextFile {
        ensure!(
            existing_id.is_none(),
            "refusing to replace a keyring-sealed session with plaintext"
        );
        return private_write(path, &plain);
    }
    let sealed = match existing_id {
        Some(id) => {
            let key = keys
                .get(&id)?
                .context("the keyring no longer holds this session's key")?;
            seal(&id, &key, &plain)?
        }
        None => match new_key(keys) {
            Ok((id, key)) => seal(&id, &key, &plain)?,
            // Keep a refreshed token usable rather than losing it; a plaintext
            // file is sealed at the next start with a reachable keyring.
            Err(_) if existing.is_some() => return private_write(path, &plain),
            Err(error) => return Err(error),
        },
    };
    private_write(path, &sealed)
}

pub fn load_migrating_with(path: &Path, store: Store, keys: &impl Keys) -> Result<Value> {
    let original = fs::read(path).context("read saved session")?;
    let state = load_with(path, store, keys)?;
    if store == Store::PlaintextFile || sealed(&parse(&original)?) {
        return Ok(state);
    }
    let Ok((id, key)) = new_key(keys) else {
        return Ok(state);
    };
    let bytes = seal(&id, &key, &original)?;
    // Verify the stored key and the envelope before the plaintext goes away.
    let stored = keys.get(&id)?.context("keyring did not keep the new key")?;
    ensure!(
        open(&parse(&bytes)?, &MapKeys::single(&id, stored))? == state,
        "sealed session did not round-trip"
    );
    // Do not overwrite a concurrent save made since the first read.
    if fs::read(path)? == original {
        private_write(path, &bytes)?;
    }
    Ok(state)
}

fn new_key(keys: &impl Keys) -> Result<(String, [u8; 32])> {
    let mut random = [0u8; 48];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let id = random[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let key: [u8; 32] = random[16..].try_into().unwrap();
    keys.put(&id, &key)?;
    Ok((id, key))
}
fn aad(id: &str) -> Vec<u8> {
    format!("{FORMAT}/1/{id}").into_bytes()
}
fn seal(id: &str, key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 12];
    fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let ciphertext = Aes256Gcm::new(key.into())
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plain,
                aad: &aad(id),
            },
        )
        .map_err(|_| anyhow::anyhow!("session sealing failed"))?;
    Ok(serde_json::to_vec_pretty(&json!({
        "format": FORMAT,
        "version": 1,
        "key_id": id,
        "nonce": STANDARD.encode(nonce),
        "ciphertext": STANDARD.encode(ciphertext),
    }))?)
}
fn open(envelope: &Value, keys: &impl Keys) -> Result<Value> {
    ensure!(
        envelope["version"] == 1,
        "unsupported sealed session version"
    );
    let id = envelope["key_id"]
        .as_str()
        .context("sealed session has no key ID")?;
    let nonce: [u8; 12] = STANDARD
        .decode(envelope["nonce"].as_str().context("sealed session nonce")?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("sealed session nonce size"))?;
    let ciphertext = STANDARD.decode(
        envelope["ciphertext"]
            .as_str()
            .context("sealed session ciphertext")?,
    )?;
    let key = keys
        .get(id)?
        .context("the desktop keyring does not hold this session's key")?;
    let plain = Aes256Gcm::new((&key).into())
        .decrypt(
            (&nonce).into(),
            Payload {
                msg: &ciphertext,
                aad: &aad(id),
            },
        )
        .map_err(|_| anyhow::anyhow!("sealed session failed authentication"))?;
    parse(&plain)
}

/// In-memory keys: verification of a fresh envelope, and tests.
#[derive(Default)]
pub struct MapKeys(std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>);
impl MapKeys {
    fn single(id: &str, key: [u8; 32]) -> Self {
        Self(std::sync::Mutex::new([(id.to_owned(), key)].into()))
    }
}
impl Keys for MapKeys {
    fn get(&self, id: &str) -> Result<Option<[u8; 32]>> {
        Ok(self.0.lock().unwrap().get(id).copied())
    }
    fn put(&self, id: &str, key: &[u8; 32]) -> Result<()> {
        self.0.lock().unwrap().insert(id.to_owned(), *key);
        Ok(())
    }
}

mod secret_service;
pub use secret_service::Keyring;

#[cfg(test)]
mod tests;
