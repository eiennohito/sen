use crate::*;

pub(crate) fn start(
    locations: sen::locations::Locations,
    client_version: Option<String>,
    wasm_override: Option<std::path::PathBuf>,
    #[cfg(feature = "demo")] dev_args: dev::demo::Args,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(feature = "demo"))]
    select_platform()?;
    #[cfg(not(feature = "demo"))]
    let headless = false;
    #[cfg(feature = "demo")]
    let headless = dev_args.snapshot_path.is_some();
    #[cfg(not(feature = "demo"))]
    let automation = false;
    #[cfg(feature = "demo")]
    let automation = dev_args.background;
    let instance = if !headless {
        match desktop_env::desktop_integration::Instance::acquire_with_activation(!automation)? {
            desktop_env::desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_env::desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    let activations = instance.as_ref().map(|i| i.take_events());
    let assets = assets::Assets::default();
    let clipboard_root = locations.data.join("clipboard-attachments");
    let fonts = locations.fonts();
    let real_worker = || {
        let root = locations.data.clone();
        Worker::start(
            StoreLocation::Account {
                root,
                cache: locations.cache.clone(),
                mid: None,
            },
            move || {
                LineBackend::open_local(
                    &locations.session,
                    wasm_override.clone(),
                    &locations.cache,
                    client_version.clone(),
                )
            },
            sen::engine::WorkerOptions::default(),
        )
    };
    #[cfg(feature = "demo")]
    let dev = dev::Session::start(dev_args, real_worker)?;
    slint::set_xdg_app_id(desktop_env::desktop_integration::APP_ID)?;
    emoji::load(&fonts);
    #[cfg(not(feature = "demo"))]
    let worker = real_worker();
    #[cfg(feature = "demo")]
    let sender = dev.workers.borrow().sender.clone();
    #[cfg(not(feature = "demo"))]
    let sender = worker.sender.clone();
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
        viewer_name: String::new(),
        attachments: Default::default(),
        notifications: if !headless {
            Some(notifications::Notifications::start())
        } else {
            None
        },
        background: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender,
        category: ChatCategory::All,
    };
    let desktop = Runtime {
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: ActionQueue::default(),
        draining: Default::default(),
    };
    #[cfg(not(feature = "demo"))]
    let ui = desktop.update(|d| window::install_window(d, desktop.clone()))?;
    #[cfg(feature = "demo")]
    let ui = desktop.update(|d| dev.install_window(d, desktop.clone(), dev.fake.as_ref()))?;
    #[cfg(feature = "demo")]
    if dev.stage(&ui, &desktop)? {
        return Ok(());
    }
    #[cfg(not(feature = "demo"))]
    worker.on_deltas(event_loop::wake);
    #[cfg(not(feature = "demo"))]
    let current = {
        let weak = ui.as_weak();
        move || weak.upgrade()
    };
    #[cfg(feature = "demo")]
    let current = dev.window_getter();
    #[cfg(not(feature = "demo"))]
    let deltas = move || worker.deltas.try_iter().collect::<Vec<_>>();
    #[cfg(feature = "demo")]
    let deltas = dev.delta_drain();
    let d = desktop.clone();
    event_loop::install(move || {
        if let Some(ui) = current() {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_env::desktop_integration::Activation::OpenChat(chat) = e {
                        d.update(|d| {
                            d.open_conversation(&ui, ChatId(chat));
                        });
                    }
                }
            }
            d.update(|d| d.apply_batch(&ui, deltas()));
            d.update(|d| {
                d.background.drain(&ui);
                // Unread and connection deltas can change exposure without changing reading geometry.
                d.observe_reading(&ui);
                d.drain_notifications(&ui);
            });
        }
    });
    if !headless {
        ui.show()?;
        #[cfg(feature = "demo")]
        drop(ui);
        slint::run_event_loop_until_quit()?;
    }
    #[cfg(feature = "demo")]
    let ui = dev.window_getter()().unwrap();
    ui.hide()?;
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    Ok(())
}

pub(crate) fn select_platform() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
}
