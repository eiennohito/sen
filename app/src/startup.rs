use crate::*;

pub(crate) struct Options {
    pub client_version: Option<String>,
    pub wasm_override: Option<std::path::PathBuf>,
    #[cfg(feature = "demo")]
    pub dev_args: dev::demo::Args,
    pub locations: line::locations::Locations,
}

pub(crate) fn start(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        locations,
        client_version,
        wasm_override,
        #[cfg(feature = "demo")]
        dev_args,
    } = options;
    #[cfg(not(feature = "demo"))]
    let launch = Launch {
        headless: false,
        automation: false,
        size: (980, 720),
        text_size: 15,
    };
    #[cfg(not(feature = "demo"))]
    select_platform()?;
    #[cfg(feature = "demo")]
    let (launch, dev) = dev::Session::start(dev_args)?;
    slint::set_xdg_app_id(desktop_env::desktop_integration::APP_ID)?;
    let instance = if !launch.headless {
        match desktop_env::desktop_integration::Instance::acquire_with_activation(
            !launch.automation,
        )? {
            desktop_env::desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_env::desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    emoji::load();
    let activations = instance.as_ref().map(|i| i.take_events());
    let assets = assets::Assets::default();
    let clipboard_root = locations
        .session
        .parent()
        .unwrap()
        .join("clipboard-attachments");
    let real_worker = || {
        let root = locations.session.parent().unwrap().join("accounts");
        Worker::start(
            StoreLocation::Account { root, mid: None },
            move || {
                LineBackend::open_local(
                    &locations.session,
                    wasm_override.clone(),
                    &locations.cache,
                    client_version.clone(),
                )
            },
            line::engine::WorkerOptions::default(),
        )
    };
    #[cfg(feature = "demo")]
    let worker = dev.worker(real_worker);
    #[cfg(not(feature = "demo"))]
    let worker = real_worker();
    let ui = window::create_app_window()?;
    if let Some(warning) = instance.as_ref().and_then(|i| i.warning.as_ref()) {
        ui.set_desktop_warning(warning.into());
    }
    ui.set_text_size(launch.text_size);
    ui.global::<State>()
        .set_automation_background(launch.automation);
    if !launch.headless && !launch.automation {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{Ime, WindowEvent},
        };
        let weak = ui.as_weak();
        let drops = Rc::new(RefCell::new(Vec::<std::path::PathBuf>::new()));
        let drop_timer = slint::Timer::default();
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
                    WindowEvent::DroppedFile(path) => {
                        if ui.get_show_conversation() && !ui.global::<State>().get_modal_open() {
                            drops.borrow_mut().push(path.clone());
                            let drops = drops.clone();
                            let weak = ui.as_weak();
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
                                        ui.global::<State>().invoke_drop_attachment(uris.into());
                                    }
                                },
                            );
                        }
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        ui.global::<State>().set_device_scale(*scale_factor as f32);
                    }
                    WindowEvent::Focused(active) => ui.global::<State>().set_window_active(*active),
                    WindowEvent::Ime(Ime::Preedit(text, _)) => ui.set_ime_active(!text.is_empty()),
                    WindowEvent::Ime(Ime::Commit(_)) | WindowEvent::Ime(Ime::Disabled) => {
                        ui.set_ime_active(false)
                    }
                    _ => {}
                }
            }
            EventResult::Propagate
        });
    }
    let desktop = Desktop {
        clipboard_root,
        history: reading::Reading {
            opening_history: true,
            ..Default::default()
        },
        assets,
        shown_composer: String::new(),
        reveal: None,
        viewer_asset: None,
        viewer_source: None,
        destination: viewer::save_dialog,
        notifications: if !launch.headless {
            Some(notifications::Notifications::start())
        } else {
            None
        },
        background: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender: worker.sender.clone(),
        query: String::new(),
        category: ChatCategory::All,
    };
    ui.window().set_size(slint::LogicalSize::new(
        launch.size.0 as f32,
        launch.size.1 as f32,
    ));
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade()
            && ui.global::<State>().get_background_mode()
            && ui.global::<State>().get_tray_available()
        {
            let _ = ui.hide();
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        }
    });
    desktop.presenter.install(&ui);
    let desktop = Runtime {
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: ActionQueue::default(),
        draining: Default::default(),
    };
    callbacks::install(&ui, desktop.clone());
    let weak = ui.as_weak();
    let d = desktop.clone();
    let worker = Rc::new(RefCell::new(worker));
    #[cfg(feature = "demo")]
    if dev.stage(&ui, &desktop, &worker)? {
        return Ok(());
    }
    let w = worker.clone();
    worker.borrow().on_deltas(event_loop::wake);
    event_loop::install(move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_env::desktop_integration::Activation::OpenChat(chat) = e {
                        d.update(|d| {
                            d.open_chat(&ui, ChatId(chat));
                            d.render(&ui);
                            ui.set_show_conversation(true);
                        });
                    }
                }
            }
            let deltas = w.borrow().deltas.try_iter().collect::<Vec<_>>();
            d.update(|d| d.apply_batch(&ui, deltas));
            d.update(|d| {
                d.background.drain(&ui);
                d.observe_reading(&ui);
                d.drain_notifications(&ui);
            });
        }
    });
    if !launch.headless {
        ui.show()?;
        slint::run_event_loop_until_quit()?;
        ui.hide()?;
    }
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    drop(worker);
    Ok(())
}

pub(crate) struct Launch {
    pub headless: bool,
    pub automation: bool,
    pub size: (u32, u32),
    pub text_size: i32,
}
pub(crate) fn select_platform() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
}
