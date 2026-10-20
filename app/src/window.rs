use crate::*;

#[derive(Clone)]
pub(crate) struct Options {
    pub launch: startup::Launch,
    pub warning: slint::SharedString,
    pub color_scheme: slint::language::ColorScheme,
    #[cfg(feature = "demo")]
    pub fake: Option<FakeBackend>,
}

pub(crate) fn install_window(
    d: &mut Desktop,
    runtime: Runtime,
    workers: &Rc<RefCell<Worker>>,
    options: &Options,
) -> Result<AppWindow, slint::PlatformError> {
    let ui = create_app_window()?;
    let launch = &options.launch;
    ui.window().set_size(slint::LogicalSize::new(
        launch.size.0 as f32,
        launch.size.1 as f32,
    ));
    install_events(&ui, runtime.clone(), !launch.headless && !launch.automation);
    ui.global::<SettingsState>().set_text_size(launch.text_size);
    ui.global::<ShellState>()
        .set_desktop_warning(options.warning.clone());
    ui.global::<Palette>()
        .set_color_scheme(options.color_scheme);
    d.presenter.install(&ui);
    callbacks::install(&ui, runtime.clone());
    #[cfg(feature = "demo")]
    {
        ui.global::<ExtensionState>()
            .set_automation_background(launch.automation);
        if let Some(fake) = &options.fake {
            d.destination = dev::demo::destination();
            dev::demo::prepare(&ui);
            dev::demo::install(&ui, runtime, fake.clone(), workers.clone(), options);
        }
    }
    #[cfg(not(feature = "demo"))]
    let _ = workers;
    Ok(ui)
}

pub(crate) fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.global::<ContentState>()
        .on_avatar_tint(|id| presenter::format::avatar_tint(&id));
    ui.global::<ContentState>()
        .on_name_tint(|id, background, dark| presenter::format::name_tint(&id, background, dark));
    let mut avatars: std::collections::VecDeque<(slint::Image, u32, slint::Image)> =
        std::collections::VecDeque::new();
    ui.global::<ContentState>()
        .on_avatar_picture(move |source, width, scale| {
            if source.size().width == 0 {
                return source;
            }
            let edge = (width * scale).round().max(1.) as u32;
            if let Some((_, _, picture)) = avatars
                .iter()
                .find(|(old, size, _)| *old == source && *size == edge)
            {
                return picture.clone();
            }
            let Some(pixels) = source.to_rgba8() else {
                return source;
            };
            let pixels = crate::assets::circular_avatar(
                &line::client::desktop::assets::Pixels {
                    width: pixels.width(),
                    height: pixels.height(),
                    rgba: pixels.as_bytes().to_vec(),
                },
                edge,
            );
            let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &pixels.rgba,
                pixels.width,
                pixels.height,
            ));
            if avatars.len() == 256 {
                avatars.pop_front();
            }
            avatars.push_back((source, edge, picture.clone()));
            picture
        });
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
