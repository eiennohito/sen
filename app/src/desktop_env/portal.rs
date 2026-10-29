//! Blocking XDG portal requests; callers run these on a helper thread.
use std::{
    collections::HashMap,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{Fd, OwnedObjectPath, OwnedValue, Str},
};
type Response = (u32, HashMap<String, OwnedValue>);
fn request(
    interface: &str,
    mut options: HashMap<&str, OwnedValue>,
    call: impl FnOnce(&Proxy, HashMap<&str, OwnedValue>) -> zbus::Result<OwnedObjectPath>,
) -> Result<Response, Box<dyn std::error::Error>> {
    #[cfg(test)]
    if std::env::var("SEN_TEST_BUS").as_deref() != Ok("1") {
        return Err("tests require a private dbus-run-session".into());
    }
    let bus = Connection::session()?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = format!(
        "sen_{}_{}",
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
        interface,
    )?;
    options.insert("handle_token", Str::from(token).into());
    let returned = call(&portal, options)?;
    if returned.as_str() != path {
        return Err("unexpected portal request path".into());
    }
    let response = signals.next().ok_or("portal request closed")?;
    Ok(response.body().deserialize()?)
}
pub(crate) fn choose(
    method: &str,
    title: &str,
    mut options: HashMap<&str, OwnedValue>,
) -> Result<Result<Vec<String>, u32>, Box<dyn std::error::Error>> {
    options.insert("modal", true.into());
    let (code, mut results) = request("org.freedesktop.portal.FileChooser", options, |p, o| {
        p.call(method, &("", title, o))
    })?;
    if code != 0 {
        return Ok(Err(code));
    }
    let uris = Vec::<String>::try_from(results.remove("uris").ok_or("missing selection")?)?;
    Ok(Ok(uris))
}
/// Hands a read-only descriptor to the default application for its type.
pub(crate) fn open_file(path: &Path) -> Result<Result<(), u32>, Box<dyn std::error::Error>> {
    use std::os::fd::AsFd;
    let file = std::fs::File::open(path)?;
    let options = HashMap::from([("writable", false.into()), ("ask", false.into())]);
    let (code, _) = request("org.freedesktop.portal.OpenURI", options, |p, o| {
        p.call("OpenFile", &("", Fd::from(file.as_fd()), o))
    })?;
    Ok(if code == 0 { Ok(()) } else { Err(code) })
}
