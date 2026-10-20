//! Minimal org.freedesktop.secrets client: a plain session on the session bus
//! (the bus is local and the spec's DH variant adds nothing against its owner),
//! items in the default collection, unlock prompts bounded by a timeout.
use super::Keys;
use anyhow::{Context, Result, bail, ensure};
use std::{collections::HashMap, sync::mpsc, time::Duration};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const SERVICE: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// The user's desktop keyring.
pub struct Keyring;
impl Keyring {
    pub fn reachable(&self) -> Result<()> {
        let session = Session::open()?;
        let collection = session.default_collection()?;
        let proxy = session.proxy(collection.as_str(), "Collection")?;
        ensure!(
            !proxy.get_property::<bool>("Locked")?,
            "default keyring collection is locked"
        );
        Ok(())
    }
}
impl Keys for Keyring {
    fn get(&self, application: &str, id: &str) -> Result<Option<[u8; 32]>> {
        let session = Session::open()?;
        let service = session.proxy(PATH, "Service")?;
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            service.call("SearchItems", &(attributes(application, id),))?;
        let items = if unlocked.is_empty() && !locked.is_empty() {
            session.unlock(locked)?
        } else {
            unlocked
        };
        let Some(item) = items.into_iter().next() else {
            return Ok(None);
        };
        let mut secrets: HashMap<OwnedObjectPath, Secret> =
            service.call("GetSecrets", &(vec![item.clone()], &session.path))?;
        let (_, _, value, _) = secrets
            .remove(&item)
            .context("keyring returned no secret")?;
        let key = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("keyring secret has the wrong size"))?;
        Ok(Some(key))
    }
    fn put(&self, application: &str, id: &str, key: &[u8; 32]) -> Result<()> {
        let session = Session::open()?;
        let collection = session.default_collection()?;
        let proxy = session.proxy(collection.as_str(), "Collection")?;
        if proxy.get_property::<bool>("Locked")? {
            session.unlock(vec![collection.clone()])?;
        }
        let properties = HashMap::from([
            (
                "org.freedesktop.Secret.Item.Label",
                Value::from("閃 session key"),
            ),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(attributes(application, id)),
            ),
        ]);
        let secret: Secret = (
            session.path.clone(),
            Vec::new(),
            key.to_vec(),
            "application/octet-stream".into(),
        );
        let (item, prompt): (OwnedObjectPath, OwnedObjectPath) =
            proxy.call("CreateItem", &(properties, secret, false))?;
        if item.as_str() == "/" {
            session.prompt(prompt)?;
        }
        Ok(())
    }
}

fn attributes<'a>(application: &'a str, id: &'a str) -> HashMap<&'a str, &'a str> {
    HashMap::from([
        ("application", application),
        ("purpose", "session-key"),
        ("key-id", id),
    ])
}

struct Session {
    connection: Connection,
    path: OwnedObjectPath,
}
impl Session {
    fn open() -> Result<Self> {
        let connection = Connection::session()?;
        let service = Proxy::new(&connection, SERVICE, PATH, "org.freedesktop.Secret.Service")?;
        let (_, path): (OwnedValue, OwnedObjectPath) =
            service.call("OpenSession", &("plain", Value::from("")))?;
        Ok(Self { connection, path })
    }
    fn proxy(&self, path: &str, interface: &str) -> Result<Proxy<'static>> {
        Ok(Proxy::new(
            &self.connection,
            SERVICE,
            path.to_owned(),
            format!("org.freedesktop.Secret.{interface}"),
        )?)
    }
    fn default_collection(&self) -> Result<OwnedObjectPath> {
        let path: OwnedObjectPath = self
            .proxy(PATH, "Service")?
            .call("ReadAlias", &("default",))?;
        ensure!(
            path.as_str() != "/",
            "the keyring has no default collection"
        );
        Ok(path)
    }
    fn unlock(&self, objects: Vec<OwnedObjectPath>) -> Result<Vec<OwnedObjectPath>> {
        let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            self.proxy(PATH, "Service")?.call("Unlock", &(objects,))?;
        if prompt.as_str() == "/" {
            return Ok(unlocked);
        }
        let result = self.prompt(prompt)?;
        Vec::<OwnedObjectPath>::try_from(result).context("invalid keyring unlock response")
    }
    fn prompt(&self, prompt: OwnedObjectPath) -> Result<OwnedValue> {
        let proxy = self.proxy(prompt.as_str(), "Prompt")?;
        let mut completed = proxy.receive_signal("Completed")?;
        let (tx, rx) = mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let _ = tx.send(completed.next().map(|message| {
                message
                    .body()
                    .deserialize::<(bool, OwnedValue)>()
                    .map_err(anyhow::Error::from)
            }));
        });
        proxy.call::<_, _, ()>("Prompt", &("",))?;
        let outcome = match rx.recv_timeout(PROMPT_TIMEOUT) {
            Ok(outcome) => outcome,
            Err(_) => {
                // Dismissing emits Completed, which releases the waiter.
                let _ = proxy.call::<_, _, ()>("Dismiss", &());
                bail!("the keyring prompt timed out");
            }
        };
        let _ = waiter.join();
        let (dismissed, result) = outcome.context("keyring prompt ended without an answer")??;
        ensure!(!dismissed, "the keyring prompt was dismissed");
        Ok(result)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(proxy) = self.proxy(self.path.as_str(), "Session") {
            let _ = proxy.call::<_, _, ()>("Close", &());
        }
    }
}
