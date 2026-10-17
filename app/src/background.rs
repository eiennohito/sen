//! Opt-in desktop lifecycle. Disk and bus work stay on bounded workers.
use crate::*;
#[derive(Default)]
pub(crate) struct Controller {
    tray: Option<tray::Tray>,
    autostart: Option<autostart::Manager>,
}
impl Controller {
    pub(crate) fn apply(&mut self, ui: &AppWindow, saved: &storage::Saved) {
        let state = ui.global::<State>();
        state.set_background_mode(saved.background_mode);
        state.set_autostart(saved.autostart);
        if saved.background_mode {
            self.start_tray(ui);
        }
    }
    pub(crate) fn capture(&mut self, ui: &AppWindow, saved: &mut storage::Saved) {
        let state = ui.global::<State>();
        let background = state.get_background_mode();
        if background != saved.background_mode {
            saved.background_mode = background;
            if background {
                self.start_tray(ui);
            } else {
                self.tray.take();
                state.set_tray_available(false);
                state.set_background_status("".into());
            }
        }
        let enabled = state.get_autostart();
        if enabled == saved.autostart {
            return;
        }
        if cfg!(feature = "ui-debug") {
            state.set_autostart(saved.autostart);
            state.set_autostart_status("Autostart requires an installed release build.".into());
            return;
        }
        if self.autostart.is_none() {
            let paths = autostart::directory().and_then(|directory| {
                std::env::current_exe()
                    .map(|executable| (directory, executable))
                    .map_err(|_| "Could not find the app executable.".into())
            });
            let (directory, executable) = match paths {
                Ok(paths) => paths,
                Err(error) => {
                    state.set_autostart(saved.autostart);
                    state.set_autostart_status(error.into());
                    return;
                }
            };
            let weak = ui.as_weak();
            self.autostart = Some(autostart::Manager::start(
                directory,
                executable,
                move |revision, enabled, result| {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        autostart_result(&ui, revision, enabled, result)
                    });
                },
            ));
        }
        saved.autostart = enabled;
        state.set_autostart_status("Updating autostart…".into());
        self.autostart.as_ref().unwrap().request(enabled);
    }
    fn start_tray(&mut self, ui: &AppWindow) {
        if self.tray.is_some() {
            return;
        }
        ui.global::<State>()
            .set_background_status("Starting desktop tray…".into());
        let (sender, events) = std::sync::mpsc::sync_channel(32);
        self.tray = Some(tray::Tray::start(move |event| {
            let _ = sender.try_send(event);
        }));
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                let (completed, completion) = std::sync::mpsc::sync_channel(1);
                if weak
                    .upgrade_in_event_loop(move |ui| {
                        tray_event(&ui, event);
                        let _ = completed.send(());
                    })
                    .is_err()
                {
                    break;
                }
                if completion
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    pub(crate) fn shutdown(&mut self) {
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
        if let Some(autostart) = self.autostart.take() {
            autostart.shutdown();
        }
    }
}
fn autostart_result(ui: &AppWindow, revision: u64, enabled: bool, result: Result<(), String>) {
    UI_STATE.with(|slot| {
        let Some(view) = slot.borrow().as_ref().cloned() else {
            return;
        };
        let mut view = view.borrow_mut();
        if view
            .background
            .autostart
            .as_ref()
            .is_none_or(|manager| manager.revision() != revision)
        {
            return;
        }
        match result {
            Ok(()) => ui.global::<State>().set_autostart_status(
                if enabled {
                    "Autostart enabled"
                } else {
                    "Autostart off"
                }
                .into(),
            ),
            Err(error) => {
                ui.global::<State>().set_autostart(!enabled);
                view.saved.autostart = !enabled;
                ui.global::<State>().set_autostart_status(error.into());
                view.persist(ui);
            }
        }
    });
}
fn tray_event(ui: &AppWindow, event: tray::Event) {
    UI_STATE.with(|slot| {
        let Some(view) = slot.borrow().as_ref().cloned() else { return; };
        let mut view = view.borrow_mut();
        let state = ui.global::<State>();
        match event {
            tray::Event::Available(available) => {
                if !state.get_background_mode() { return; }
                state.set_tray_available(available);
                if available { state.set_background_status("".into()); }
                else {
                    // A vanished tray must never strand a hidden app.
                    let _ = ui.show();
                    state.set_background_mode(false);
                    view.saved.background_mode = false;
                    view.background.tray.take();
                    state.set_background_status("Background mode is unavailable because no desktop tray host was found. Closing LINE will quit.".into());
                    view.persist(ui);
                }
            }
            tray::Event::Open => {
                let _ = ui.show();
                if !state.get_automation_background() {
                    use slint::winit_030::WinitWindowAccessor;
                    ui.window().with_winit_window(|window| { window.set_minimized(false); window.focus_window(); window.request_user_attention(Some(slint::winit_030::winit::window::UserAttentionType::Informational)); });
                }
            }
            tray::Event::Mute => {
                let muted = !state.get_notifications_paused();
                state.set_notifications_paused(muted);
                if let Some(tray) = &view.background.tray { tray.set_muted(muted); }
                if muted && let Some(attention) = &view.attention { attention.withdraw_notifications(); }
            }
            tray::Event::Quit => ui.invoke_quit(),
        }
    });
}
pub(crate) fn close_to_background(enabled: bool, tray_available: bool) -> bool {
    enabled && tray_available
}
pub(crate) fn install(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            let state = ui.global::<State>();
            if close_to_background(state.get_background_mode(), state.get_tray_available()) {
                state.set_window_active(false);
                let _ = ui.hide();
                return slint::CloseRequestResponse::KeepWindowShown;
            }
        }
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_requires_both_preference_and_available_tray() {
        assert!(!close_to_background(false, false));
        assert!(!close_to_background(false, true));
        assert!(!close_to_background(true, false));
        assert!(close_to_background(true, true));
    }
    #[test]
    fn close_hides_only_with_live_tray_and_loss_restores_draft() {
        std::thread::spawn(|| {
            let _platform = snapshot::install().unwrap();
            let ui = create_app_window().unwrap();
            ui.global::<State>().set_automation_background(true);
            ui.global::<State>().set_background_mode(true);
            ui.global::<State>().set_tray_available(true);
            ui.set_draft("日本語 draft 🐈".into());
            let view = Rc::new(RefCell::new(View {
                demo: true,
                ..Default::default()
            }));
            UI_STATE.with(|slot| *slot.borrow_mut() = Some(view));
            install(&ui);
            ui.show().unwrap();
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            assert!(!ui.window().is_visible());
            tray_event(&ui, tray::Event::Open);
            assert!(ui.window().is_visible());
            assert_eq!(ui.get_draft(), "日本語 draft 🐈");
            ui.hide().unwrap();
            tray_event(&ui, tray::Event::Available(false));
            assert!(
                ui.window().is_visible(),
                "lost tray stranded the hidden app"
            );
            assert!(!ui.global::<State>().get_background_mode());
            assert_eq!(ui.get_draft(), "日本語 draft 🐈");
            let quit = Rc::new(std::cell::Cell::new(false));
            let marker = quit.clone();
            ui.on_quit(move || marker.set(true));
            tray_event(&ui, tray::Event::Quit);
            assert!(quit.get());
            UI_STATE.with(|slot| slot.borrow_mut().take());
            ui.hide().unwrap();
        })
        .join()
        .unwrap();
    }
}
