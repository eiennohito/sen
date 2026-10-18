//! Opt-in desktop lifecycle, independent of conversation state.
use crate::*;
#[derive(Default)]
pub struct Background {
    tray: Option<tray::Tray>,
    events: Option<std::sync::mpsc::Receiver<tray::Event>>,
    autostart: Option<autostart::Manager>,
    autostart_value: bool,
}
impl Background {
    pub fn apply(&mut self, ui: &AppWindow) {
        let s = ui.global::<State>();
        if s.get_background_mode() && self.tray.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tray = Some(tray::Tray::start(move |e| {
                let _ = tx.send(e);
            }));
            self.events = Some(rx);
        }
        if !s.get_background_mode() {
            if let Some(tray) = self.tray.take() {
                tray.shutdown();
            }
            self.events = None;
            s.set_tray_available(false);
        }
        if s.get_autostart() != self.autostart_value {
            if self.autostart.is_none()
                && !cfg!(feature = "ui-debug")
                && let (Ok(dir), Ok(exe)) = (autostart::directory(), std::env::current_exe())
            {
                let weak = ui.as_weak();
                self.autostart = Some(autostart::Manager::start(dir, exe, move |_, _, result| {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.global::<State>()
                            .set_autostart_status(result.err().unwrap_or_default().into())
                    });
                }));
            }
            if let Some(manager) = &self.autostart {
                manager.request(s.get_autostart());
                s.set_autostart_status(
                    format!("Updating autostart ({})", manager.revision()).into(),
                );
            }
            self.autostart_value = s.get_autostart();
        }
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
        if let Some(manager) = self.autostart.take() {
            manager.shutdown();
        }
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
    }
}
