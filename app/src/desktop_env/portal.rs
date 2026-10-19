//! Blocking XDG file chooser request; callers run this on a helper thread.
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Str},
};
pub(crate) fn choose(
    method: &str,
    title: &str,
    mut options: HashMap<&str, OwnedValue>,
) -> Result<Result<Vec<String>, u32>, Box<dyn std::error::Error>> {
    let bus = Connection::session()?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = format!(
        "line_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let sender = bus
        .unique_name()
        .ok_or("missing bus identity")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = Proxy::new(
        &bus,
        "org.freedesktop.portal.Desktop",
        path.as_str(),
        "org.freedesktop.portal.Request",
    )?;
    let mut signals = request.receive_signal("Response")?;
    let portal = Proxy::new(
        &bus,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    )?;
    options.insert("handle_token", Str::from(token).into());
    options.insert("modal", true.into());
    let returned: OwnedObjectPath = portal.call(method, &("", title, options))?;
    if returned.as_str() != path {
        return Err("unexpected portal request path".into());
    }
    let response = signals.next().ok_or("file chooser closed")?;
    let (code, mut results): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    if code != 0 {
        return Ok(Err(code));
    }
    let uris = Vec::<String>::try_from(results.remove("uris").ok_or("missing selection")?)?;
    Ok(Ok(uris))
}
