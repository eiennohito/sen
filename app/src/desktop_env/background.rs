//! Opt-in desktop lifecycle, independent of conversation state.
use crate::*;
#[derive(Default)]
pub struct Background {
    pub(crate) available: bool,
    tray: Option<desktop_env::tray::Tray>,
    events: Option<std::sync::mpsc::Receiver<desktop_env::tray::Event>>,
    autostart_value: bool,
}
impl Background {
    pub fn preferences(&mut self, ui: &AppWindow, prefs: &DesktopPrefs) {
        if prefs.background && self.tray.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tray = Some(desktop_env::tray::Tray::start(move |e| {
                let _ = tx.send(e);
                crate::event_loop::wake();
            }));
            self.events = Some(rx);
        }
        if !prefs.background {
            self.tray = None;
            self.events = None;
            self.available = false;
        }
        if prefs.autostart != self.autostart_value {
            let result = desktop_env::autostart::directory().and_then(|directory| {
                let executable = std::env::current_exe()
                    .map_err(|_| "Could not find the executable for autostart.".to_owned())?;
                desktop_env::autostart::set_enabled(&directory, &executable, prefs.autostart)
            });
            ui.global::<SettingsState>()
                .set_autostart_status(result.err().unwrap_or_default().into());
            self.autostart_value = prefs.autostart;
        }
    }
    pub fn drain(&mut self, ui: &AppWindow) {
        if let Some(events) = &self.events {
            for event in events.try_iter() {
                match event {
                    desktop_env::tray::Event::Available(v) => self.available = v,
                    desktop_env::tray::Event::Open => {
                        let _ = ui.show();
                    }
                    desktop_env::tray::Event::Mute => {
                        ui.global::<SettingsState>().set_notifications_paused(
                            !ui.global::<SettingsState>().get_notifications_paused(),
                        );
                        if let Some(tray) = &self.tray {
                            tray.set_muted(ui.global::<SettingsState>().get_notifications_paused());
                        }
                    }
                    desktop_env::tray::Event::Quit => {
                        let _ = slint::quit_event_loop();
                    }
                }
            }
        }
    }
}
