use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

/// In-memory keys for credential store tests.
#[derive(Default)]
struct MapKeys(std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>);
impl Keys for MapKeys {
    fn get(&self, id: &str) -> Result<Option<[u8; 32]>> {
        Ok(self.0.lock().unwrap().get(id).copied())
    }
    fn put(&self, id: &str, key: &[u8; 32]) -> Result<()> {
        self.0.lock().unwrap().insert(id.to_owned(), *key);
        Ok(())
    }
}

struct Unreachable;
impl Keys for Unreachable {
    fn get(&self, _: &str) -> Result<Option<[u8; 32]>> {
        bail!("no keyring")
    }
    fn put(&self, _: &str, _: &[u8; 32]) -> Result<()> {
        bail!("no keyring")
    }
}

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("line-credentials-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
fn synthetic() -> Value {
    json!({"format":1,"login_key":"c3ludGhldGljIG9ubHk=","token_v3_issue_result":{"refreshToken":"synthetic-refresh"}})
}

#[test]
fn default_store_cannot_reach_the_desktop_keyring() {
    assert!(std::env::var_os(ENVIRONMENT).is_none());
    assert_eq!(
        check_writable().unwrap_err().root_cause().to_string(),
        "tests must not use the desktop keyring"
    );
}

#[test]
fn sealed_file_hides_credentials_and_round_trips() {
    let root = root("seal");
    let path = root.join("session.json");
    let keys = MapKeys::default();
    save_with(&path, &synthetic(), Store::Keyring, &keys).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-refresh"));
    assert!(!String::from_utf8_lossy(&bytes).contains("c3ludGhldGljIG9ubHk="));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        load_with(&path, Store::Keyring, &keys).unwrap(),
        synthetic()
    );
    // Later saves keep the same keyring item, so backups stay readable.
    let id = parse(&bytes).unwrap()["key_id"].clone();
    let mut refreshed = synthetic();
    refreshed["token_v3_issue_result"]["refreshToken"] = json!("rotated");
    save_with(&path, &refreshed, Store::Keyring, &keys).unwrap();
    assert_eq!(parse(&fs::read(&path).unwrap()).unwrap()["key_id"], id);
    assert_eq!(keys.0.lock().unwrap().len(), 1);
    assert_eq!(load_with(&path, Store::Keyring, &keys).unwrap(), refreshed);
    // A tampered envelope or a missing key is an error, never a fallback.
    let mut envelope = parse(&fs::read(&path).unwrap()).unwrap();
    envelope["key_id"] = json!("00");
    fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(load_with(&path, Store::Keyring, &keys).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stores_refuse_downgrades_and_require_keys() {
    let root = root("downgrade");
    let path = root.join("session.json");
    let keys = MapKeys::default();
    save_with(&path, &synthetic(), Store::Keyring, &keys).unwrap();
    let sealed_bytes = fs::read(&path).unwrap();
    assert!(load_with(&path, Store::PlaintextFile, &keys).is_err());
    assert!(save_with(&path, &json!({}), Store::PlaintextFile, &keys).is_err());
    assert!(save_with(&path, &json!({}), Store::Keyring, &Unreachable).is_err());
    assert_eq!(fs::read(&path).unwrap(), sealed_bytes);
    let fresh = root.join("fresh.json");
    assert!(save_with(&fresh, &synthetic(), Store::Keyring, &Unreachable).is_err());
    assert!(!fresh.exists());
    // The development store writes and reads plaintext.
    let dev = root.join("dev.json");
    save_with(&dev, &synthetic(), Store::PlaintextFile, &Unreachable).unwrap();
    assert_eq!(parse(&fs::read(&dev).unwrap()).unwrap(), synthetic());
    let bytes = fs::read(&dev).unwrap();
    let result = load_with(&dev, Store::Keyring, &Unreachable);
    assert!(matches!(
        result
            .unwrap_err()
            .downcast_ref::<crate::client::error::SignInFailure>(),
        Some(crate::client::error::SignInFailure::SessionStoreMismatch)
    ));
    assert!(save_with(&dev, &json!({}), Store::Keyring, &MapKeys::default()).is_err());
    assert_eq!(fs::read(&dev).unwrap(), bytes);
    assert_eq!(
        load_with(&dev, Store::PlaintextFile, &Unreachable).unwrap(),
        synthetic()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_key_refuses_save_and_load_without_creating_a_key() {
    let root = root("missing-key");
    let path = root.join("session.json");
    let keys = MapKeys::default();
    save_with(&path, &synthetic(), Store::Keyring, &keys).unwrap();
    let sealed_bytes = fs::read(&path).unwrap();
    keys.0.lock().unwrap().clear();

    assert!(save_with(&path, &json!({}), Store::Keyring, &keys).is_err());
    assert!(load_with(&path, Store::Keyring, &keys).is_err());
    assert_eq!(fs::read(&path).unwrap(), sealed_bytes);
    assert!(keys.0.lock().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_preserves_unreadable_or_invalid_existing_sessions() {
    let root = root("invalid-existing");
    let keys = MapKeys::default();
    for store in [Store::Keyring, Store::PlaintextFile] {
        let path = root.join("session.json");
        let bytes = b"{incomplete session";
        fs::write(&path, bytes).unwrap();
        assert!(save_with(&path, &synthetic(), store, &keys).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        // A directory deterministically fails reads even when tests run as root.
        fs::create_dir(&path).unwrap();
        assert!(save_with(&path, &synthetic(), store, &keys).is_err());
        assert!(path.is_dir());
        fs::remove_dir(&path).unwrap();
    }
    assert!(keys.0.lock().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saved_session_fixtures_preserve_all_fields() -> Result<()> {
    // Current fixtures: a synthetic Session serialized by save_with, copied verbatim.
    // The sealed fixture uses only the public test key [0x42; 32], never a keyring.
    // ecc06f0f939f2982c795d779266c649b49f4d665 added the typed Session defaults.
    // session-pre-format.json hand-writes that accepted shape without format or
    // refreshToken. No pre-format writer exists in git history: even the first
    // writer (6e058db0d8048abd2054fcdf2d15559656ef6730) wrote format: 1.
    let root = root("session-fixtures");
    let keys = MapKeys::default();
    keys.put("synthetic-session-fixture", &[0x42; 32])?;
    for (name, bytes, store) in [
        (
            "current",
            include_bytes!("../../fixtures/session-current.json").as_slice(),
            Store::PlaintextFile,
        ),
        (
            "sealed",
            include_bytes!("../../fixtures/session-sealed.json").as_slice(),
            Store::Keyring,
        ),
        (
            "pre-format",
            include_bytes!("../../fixtures/session-pre-format.json").as_slice(),
            Store::PlaintextFile,
        ),
    ] {
        let path = root.join(format!("{name}.json"));
        private_write(&path, bytes)?;
        let mut original = load_with(&path, store, &keys)?;
        let loaded: crate::client::Session = serde_json::from_value(original.clone())?;
        // The absent format defaults to zero; every original field must survive.
        if name == "pre-format" {
            original["format"] = json!(0);
        }
        let saved = serde_json::to_value(loaded)?;
        assert_eq!(saved, original, "{name}: typed session changed a field");
        save_with(&path, &saved, store, &keys)?;
        let reloaded = load_with(&path, store, &keys)?;
        assert_eq!(reloaded, original, "{name}: save/load changed a field");
        let typed: crate::client::Session = serde_json::from_value(reloaded)?;
        assert_eq!(
            serde_json::to_value(typed)?,
            original,
            "{name}: reload changed a field"
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_session_failure_kinds_stay_distinct() -> Result<()> {
    let root = root("failure-kinds");
    let path = root.join("session.json");
    let keys = MapKeys::default();
    let kind = |result: Result<Value>| {
        result
            .unwrap_err()
            .downcast_ref::<SignInFailure>()
            .unwrap()
            .kind()
    };
    assert_eq!(
        kind(load_with(&path, Store::PlaintextFile, &keys)),
        "saved-session-io"
    );
    private_write(&path, b"invalid JSON")?;
    assert_eq!(
        kind(load_with(&path, Store::PlaintextFile, &keys)),
        "saved-session-corrupt"
    );
    fs::remove_file(&path)?;
    save_with(&path, &synthetic(), Store::Keyring, &keys)?;
    assert_eq!(
        kind(load_with(&path, Store::PlaintextFile, &keys)),
        "saved-session-store-mismatch"
    );
    assert_eq!(
        kind(load_with(&path, Store::Keyring, &Unreachable)),
        "keyring"
    );
    keys.0.lock().unwrap().clear();
    assert_eq!(
        kind(load_with(&path, Store::Keyring, &keys)),
        "saved-session-key-missing"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
