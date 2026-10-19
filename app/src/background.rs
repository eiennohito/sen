//! Opt-in desktop lifecycle, independent of conversation state.
use crate::*;
#[derive(Default)]
pub struct Background {
    tray: Option<tray::Tray>,
    events: Option<std::sync::mpsc::Receiver<tray::Event>>,
    autostart_value: bool,
}
impl Background {
    pub fn preferences(&mut self, ui: &AppWindow, prefs: &DesktopPrefs) {
        let s = ui.global::<State>();
        if prefs.background && self.tray.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tray = Some(tray::Tray::start(move |e| {
                let _ = tx.send(e);
                crate::event_loop::wake();
            }));
            self.events = Some(rx);
        }
        if !prefs.background {
            if let Some(tray) = self.tray.take() {
                tray.shutdown();
            }
            self.events = None;
            s.set_tray_available(false);
        }
        if prefs.autostart != self.autostart_value {
            let result = autostart::directory().and_then(|directory| {
                let executable = std::env::current_exe()
                    .map_err(|_| "Could not find the executable for autostart.".to_owned())?;
                autostart::set_enabled(&directory, &executable, prefs.autostart)
            });
            s.set_autostart_status(result.err().unwrap_or_default().into());
            self.autostart_value = prefs.autostart;
        }
    }
    pub fn drain(&mut self, ui: &AppWindow) {
        let s = ui.global::<State>();
        if let Some(events) = &self.events {
            for event in events.try_iter() {
                match event {
                    tray::Event::Available(v) => s.set_tray_available(v),
                    tray::Event::Open => {
                        let _ = ui.show();
                    }
                    tray::Event::Mute => {
                        s.set_notifications_paused(!s.get_notifications_paused());
                        if let Some(tray) = &self.tray {
                            tray.set_muted(s.get_notifications_paused());
                        }
                    }
                    tray::Event::Quit => {
                        let _ = slint::quit_event_loop();
                    }
                }
            }
        }
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
    }
}
