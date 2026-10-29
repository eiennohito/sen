pub(crate) mod autostart;
pub(crate) mod background;
pub(crate) mod desktop_integration;
pub(crate) mod portal;
pub(crate) mod tray;

/// Blocking: shows a local file in the default application for its type.
pub(crate) fn open_file(path: &std::path::Path) -> Result<(), String> {
    #[cfg(feature = "demo")]
    if crate::dev::DEMO_SESSION.get().is_some() {
        return Ok(());
    }
    #[cfg(test)]
    if std::env::var("SEN_TEST_BUS").as_deref() != Ok("1") {
        return Err("tests require a private dbus-run-session".into());
    }
    match portal::open_file(path) {
        Ok(Ok(())) => return Ok(()),
        // The user dismissed the portal's application chooser.
        Ok(Err(1)) => return Ok(()),
        Ok(Err(code)) => sen::reporting::log(
            "open",
            "xdg-open-fallback",
            &std::io::Error::other(format!("portal response {code}")),
        ),
        Err(error) => sen::reporting::log("open", "xdg-open-fallback", error.as_ref()),
    }
    match std::process::Command::new("xdg-open").arg(path).status() {
        Ok(status) if status.success() => Ok(()),
        _ => Err("No application could open this file.".into()),
    }
}
