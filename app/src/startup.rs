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
    let current_window = Rc::new(RefCell::new(None));
    let desktop = Runtime {
        window: Rc::downgrade(&current_window),
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: ActionQueue::default(),
        draining: Default::default(),
    };
    let worker = Rc::new(RefCell::new(worker));
    let options = window::Options {
        launch,
        warning: instance
            .as_ref()
            .and_then(|i| i.warning.as_ref())
            .map(|w| w.as_str().into())
            .unwrap_or_default(),
        color_scheme: slint::language::ColorScheme::Unknown,
        #[cfg(feature = "demo")]
        fake: dev.fake().cloned(),
    };
    #[cfg(feature = "demo")]
    let options = window::Options {
        color_scheme: dev.color_scheme(),
        ..options
    };
    let ui = desktop.update(|d| window::install_window(d, desktop.clone(), &worker, &options))?;
    current_window.replace(Some(ui.clone_strong()));
    #[cfg(feature = "demo")]
    if dev.stage(&ui, &desktop, &worker)? {
        return Ok(());
    }
    let w = worker.clone();
    worker.borrow().on_deltas(event_loop::wake);
    let d = desktop.clone();
    event_loop::install(move || {
        let ui = d
            .window
            .upgrade()
            .and_then(|window| window.borrow().as_ref().map(ComponentHandle::clone_strong));
        if let Some(ui) = ui {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_env::desktop_integration::Activation::OpenChat(chat) = e {
                        d.update(|d| {
                            d.open_chat(&ui, ChatId(chat));
                            d.render(&ui);
                            ui.global::<State>().set_show_conversation(true);
                        });
                    }
                }
            }
            let deltas = w.borrow().deltas.try_iter().collect::<Vec<_>>();
            d.update(|d| d.apply_batch(&ui, deltas));
            d.update(|d| {
                d.background.drain(&ui);
                // Unread and connection deltas can change exposure without changing reading geometry.
                d.observe_reading(&ui);
                d.drain_notifications(&ui);
            });
        }
    });
    if !options.launch.headless {
        ui.show()?;
        drop(ui);
        slint::run_event_loop_until_quit()?;
        current_window.borrow().as_ref().unwrap().hide()?;
    }
    let ui = current_window.borrow().as_ref().unwrap().clone_strong();
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    drop(worker);
    Ok(())
}

#[derive(Clone)]
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
