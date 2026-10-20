use crate::*;

#[cfg(not(feature = "demo"))]
pub(crate) fn install_window(
    d: &mut Desktop,
    runtime: Runtime,
    warning: &str,
) -> Result<AppWindow, slint::PlatformError> {
    let ui = create_app_window()?;
    ui.window().set_size(slint::LogicalSize::new(980., 720.));
    configure(&ui, d, runtime, warning, true);
    Ok(ui)
}
pub(crate) fn configure(
    ui: &AppWindow,
    d: &mut Desktop,
    runtime: Runtime,
    warning: &str,
    native_events: bool,
) {
    install_events(ui, runtime.clone(), native_events);
    ui.global::<ShellState>()
        .set_desktop_warning(warning.into());
    let assets = d.assets.clone();
    let sender = d.sender.clone();
    d.presenter
        .images
        .on_missing(move |key| assets.reload(key, &sender));
    d.presenter.install(ui);
    callbacks::install(ui, runtime);
}

pub(crate) fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    presenter::format::install(&ui);
    ui.global::<ContentState>()
        .on_avatar_tint(|id| presenter::format::avatar_tint(&id));
    ui.global::<ContentState>()
        .on_name_tint(|id, background, dark| presenter::format::name_tint(&id, background, dark));
    focus::install(&ui);
    #[cfg(feature = "demo")]
    dev::install_focus(&ui);
    let weak = ui.as_weak();
    ui.global::<ConversationState>().on_history_jump(move |y| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_history_jump(y);
        }
    });
    let weak = ui.as_weak();
    ui.global::<ConversationState>()
        .on_history_scroll(move |delta| {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_history_scroll(delta);
            }
        });
    ui.global::<ShellState>().on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    Ok(ui)
}

pub(crate) fn install_events(ui: &AppWindow, runtime: Runtime, native_events: bool) {
    if native_events {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{Ime, WindowEvent},
        };
        let weak = ui.as_weak();
        let drops = Rc::new(RefCell::new(Vec::<std::path::PathBuf>::new()));
        let drop_timer = slint::Timer::default();
        let runtime = runtime.clone();
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
                    WindowEvent::DroppedFile(path) => {
                        if ui.global::<ShellState>().get_show_conversation()
                            && !ui.global::<ShellState>().get_modal_open()
                        {
                            drops.borrow_mut().push(path.clone());
                            let drops = drops.clone();
                            let weak = ui.as_weak();
                            let runtime = runtime.clone();
                            drop_timer.start(
                                slint::TimerMode::SingleShot,
                                std::time::Duration::from_millis(80),
                                move || {
                                    let uris = std::mem::take(&mut *drops.borrow_mut())
                                        .iter()
                                        .filter_map(|path| url::Url::from_file_path(path).ok())
                                        .map(|uri| uri.to_string())
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    if let Some(ui) = weak.upgrade() {
                                        runtime.push(move |d| {
                                            callbacks::attachments::drop_files(&ui, d, &uris)
                                        });
                                    }
                                },
                            );
                        }
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        ui.global::<ShellState>()
                            .set_device_scale(*scale_factor as f32);
                    }
                    WindowEvent::Focused(active) => {
                        ui.global::<ShellState>().set_window_active(*active)
                    }
                    WindowEvent::Ime(Ime::Preedit(text, _)) => {
                        ui.global::<ShellState>().set_ime_active(!text.is_empty())
                    }
                    WindowEvent::Ime(Ime::Commit(_)) | WindowEvent::Ime(Ime::Disabled) => {
                        ui.global::<ShellState>().set_ime_active(false)
                    }
                    _ => {}
                }
            }
            EventResult::Propagate
        });
    }
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade()
            && ui.global::<SettingsState>().get_background_mode()
            && runtime.update(|d| d.background.available)
        {
            let _ = ui.hide();
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        }
    });
}
