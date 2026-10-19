use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

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
fn store_selection_is_explicit() {
    assert_eq!(Store::parse(None).unwrap(), Store::Keyring);
    assert_eq!(Store::parse(Some("keyring")).unwrap(), Store::Keyring);
    assert_eq!(
        Store::parse(Some("plaintext-file")).unwrap(),
        Store::PlaintextFile
    );
    assert!(Store::parse(Some("plaintext")).is_err());
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
fn keyring_refuses_plaintext_without_reading_keys_or_changing_file() {
    let root = root("plaintext");
    let path = root.join("session.json");
    let bytes = serde_json::to_vec_pretty(&synthetic()).unwrap();
    private_write(&path, &bytes).unwrap();
    for keys_available in [true, false] {
        let result = if keys_available {
            load_with(&path, Store::Keyring, &MapKeys::default())
        } else {
            load_with(&path, Store::Keyring, &Unreachable)
        };
        assert!(matches!(
            result
                .unwrap_err()
                .downcast_ref::<crate::client::error::Error>(),
            Some(crate::client::error::Error::SignIn(
                crate::client::error::SignInFailure::SavedSession(
                    SavedSessionFailure::CredentialsStore
                )
            ))
        ));
    }
    assert!(save_with(&path, &json!({}), Store::Keyring, &MapKeys::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        load_with(&path, Store::PlaintextFile, &Unreachable).unwrap(),
        synthetic()
    );
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
    fs::remove_dir_all(root).unwrap();
}

mod fake_service {
    //! A synthetic org.freedesktop.secrets for a private bus. Only the calls
    //! the client makes; every collection starts locked behind a prompt.
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use zbus::{
        object_server::SignalEmitter,
        zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
    };
    type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);
    pub const COLLECTION: &str = "/org/freedesktop/secrets/collection/login";

    #[derive(Default)]
    pub struct Data {
        pub items: Vec<(HashMap<String, String>, Vec<u8>)>,
        pub locked: bool,
        pub prompts: u32,
    }
    pub type Shared = Arc<Mutex<Data>>;
    fn item(index: usize) -> OwnedObjectPath {
        ObjectPath::try_from(format!("{COLLECTION}/{index}"))
            .unwrap()
            .into()
    }

    pub struct Service(pub Shared);
    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl Service {
        fn open_session(
            &self,
            algorithm: &str,
            _input: OwnedValue,
        ) -> (OwnedValue, OwnedObjectPath) {
            assert_eq!(algorithm, "plain");
            (
                Value::from("").try_into().unwrap(),
                ObjectPath::try_from("/org/freedesktop/secrets/session/1")
                    .unwrap()
                    .into(),
            )
        }
        fn read_alias(&self, name: &str) -> OwnedObjectPath {
            assert_eq!(name, "default");
            ObjectPath::try_from(COLLECTION).unwrap().into()
        }
        fn search_items(
            &self,
            attributes: HashMap<String, String>,
        ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
            let data = self.0.lock().unwrap();
            let found = data
                .items
                .iter()
                .enumerate()
                .filter(|(_, (stored, _))| attributes.iter().all(|(k, v)| stored.get(k) == Some(v)))
                .map(|(index, _)| item(index))
                .collect();
            if data.locked {
                (Vec::new(), found)
            } else {
                (found, Vec::new())
            }
        }
        async fn unlock(
            &self,
            objects: Vec<OwnedObjectPath>,
            #[zbus(object_server)] server: &zbus::ObjectServer,
        ) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
            if !self.0.lock().unwrap().locked {
                return (objects, ObjectPath::try_from("/").unwrap().into());
            }
            let path = ObjectPath::try_from("/org/freedesktop/secrets/prompt/1").unwrap();
            let _ = server.remove::<Prompt, _>(path.clone()).await;
            server
                .at(path.clone(), Prompt(self.0.clone(), objects))
                .await
                .unwrap();
            (Vec::new(), path.into())
        }
        fn get_secrets(
            &self,
            items: Vec<OwnedObjectPath>,
            session: OwnedObjectPath,
        ) -> HashMap<OwnedObjectPath, Secret> {
            let data = self.0.lock().unwrap();
            assert!(!data.locked, "secret read from a locked collection");
            items
                .into_iter()
                .map(|path| {
                    let index: usize = path.as_str().rsplit('/').next().unwrap().parse().unwrap();
                    let value = data.items[index].1.clone();
                    (
                        path,
                        (
                            session.clone(),
                            Vec::new(),
                            value,
                            "application/octet-stream".into(),
                        ),
                    )
                })
                .collect()
        }
    }

    pub struct Collection(pub Shared);
    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl Collection {
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.0.lock().unwrap().locked
        }
        fn create_item(
            &self,
            properties: HashMap<String, OwnedValue>,
            secret: Secret,
            replace: bool,
        ) -> (OwnedObjectPath, OwnedObjectPath) {
            assert!(replace);
            let mut data = self.0.lock().unwrap();
            assert!(!data.locked, "item created in a locked collection");
            let attributes: HashMap<String, String> =
                properties["org.freedesktop.Secret.Item.Attributes"]
                    .try_clone()
                    .unwrap()
                    .try_into()
                    .unwrap();
            data.items.push((attributes, secret.2));
            (
                item(data.items.len() - 1),
                ObjectPath::try_from("/").unwrap().into(),
            )
        }
    }

    pub struct Prompt(Shared, Vec<OwnedObjectPath>);
    #[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
    impl Prompt {
        async fn prompt(&self, _window: &str, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
            {
                let mut data = self.0.lock().unwrap();
                data.locked = false;
                data.prompts += 1;
            }
            let result = Value::from(self.1.clone());
            Self::completed(&emitter, false, &result).await.unwrap();
        }
        fn dismiss(&self) {}
        #[zbus(signal)]
        async fn completed(
            emitter: &SignalEmitter<'_>,
            dismissed: bool,
            result: &Value<'_>,
        ) -> zbus::Result<()>;
    }
    pub struct Session;
    #[zbus::interface(name = "org.freedesktop.Secret.Session")]
    impl Session {
        fn close(&self) {}
    }
}

#[test]
#[ignore = "requires dbus-run-session and LINE_DESKTOP_TEST_BUS=1"]
fn synthetic_secret_service_contract() {
    assert_eq!(
        std::env::var("LINE_DESKTOP_TEST_BUS").as_deref(),
        Ok("1"),
        "refusing to use the real desktop keyring"
    );
    use fake_service::*;
    let data = Shared::default();
    data.lock().unwrap().locked = true;
    let _server = zbus::blocking::connection::Builder::session()
        .unwrap()
        .serve_at("/org/freedesktop/secrets", Service(data.clone()))
        .unwrap()
        .serve_at(COLLECTION, Collection(data.clone()))
        .unwrap()
        .serve_at("/org/freedesktop/secrets/session/1", Session)
        .unwrap()
        .name("org.freedesktop.secrets")
        .unwrap()
        .build()
        .unwrap();
    let root = root("service");
    let path = root.join("session.json");
    Keyring.reachable().unwrap();
    // Locked collection: one unlock prompt, then the item is created.
    save_with(&path, &synthetic(), Store::Keyring, &Keyring).unwrap();
    assert_eq!(data.lock().unwrap().prompts, 1);
    {
        let data = data.lock().unwrap();
        let (attributes, secret) = &data.items[0];
        assert_eq!(attributes["application"], "line-desktop");
        assert_eq!(attributes["purpose"], "session-key");
        assert_eq!(secret.len(), 32);
    }
    assert_eq!(
        load_with(&path, Store::Keyring, &Keyring).unwrap(),
        synthetic()
    );
    // Relock: reading the key prompts again and still succeeds.
    data.lock().unwrap().locked = true;
    assert_eq!(
        load_with(&path, Store::Keyring, &Keyring).unwrap(),
        synthetic()
    );
    assert_eq!(data.lock().unwrap().prompts, 2);
    assert_eq!(Keyring.get("absent").unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}
