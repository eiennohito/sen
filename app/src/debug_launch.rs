//! Automation is isolated on a virtual compositor; normal launch stays native.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Foreground,
    VirtualAutomation,
}
pub(crate) fn intent(
    requested: bool,
    debug_build: bool,
    demo: bool,
    socket: Option<&str>,
    display: Option<&str>,
) -> Result<Intent, String> {
    if !requested {
        return Ok(Intent::Foreground);
    }
    if !debug_build || !demo {
        return Err("--debug-background requires a ui-debug build and an offline demo.".into());
    }
    if !socket.is_some_and(|socket| {
        socket.starts_with("line-desktop-automation-") && Some(socket) == display
    }) {
        return Err("Launch background automation through tools/run-ui-debug.py so it uses a private virtual compositor.".into());
    }
    Ok(Intent::VirtualAutomation)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_launch_and_guarded_automation_have_distinct_intents() {
        assert_eq!(
            intent(false, false, false, None, None).unwrap(),
            Intent::Foreground
        );
        assert!(intent(true, false, true, None, None).is_err());
        assert!(intent(true, true, false, None, None).is_err());
        assert!(intent(true, true, true, None, None).is_err());
        assert!(intent(true, true, true, Some("wayland-0"), Some("wayland-0")).is_err());
        assert_eq!(
            intent(
                true,
                true,
                true,
                Some("line-desktop-automation-test"),
                Some("line-desktop-automation-test")
            )
            .unwrap(),
            Intent::VirtualAutomation
        );
    }
}
