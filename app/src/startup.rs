use crate::*;

pub(crate) struct Options {
    pub client_version: Option<String>,
    pub wasm_override: Option<std::path::PathBuf>,
    #[cfg(feature = "demo")]
    pub demo_args: demo::Args,
    pub background: bool,
    pub locations: line::locations::Locations,
}

pub(crate) fn start(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        #[cfg(feature = "demo")]
        demo_args,
        background,
        locations,
        client_version,
        wasm_override,
    } = options;
    #[cfg(feature = "demo")]
    let demo::Args {
        enabled: demo,
        snapshot_path,
        size,
        text_size,
        scene,
        case,
        dark,
        snapshot_profile,
    } = demo_args;
    #[cfg(not(feature = "demo"))]
    let (demo, size, text_size) = (false, (980, 720), 15);
    #[cfg(feature = "demo")]
    let snapshot = snapshot_path
        .as_ref()
        .map(|_| snapshot::install())
        .transpose()?;
    #[cfg(feature = "demo")]
    let headless = snapshot.is_some();
    #[cfg(not(feature = "demo"))]
    let headless = false;
    let automation = background;
    if automation && (!cfg!(feature = "ui-debug") || !demo) {
        return Err("--debug-background requires a ui-debug build and an offline demo.".into());
    }
    if !headless {
        if automation {
            // Slint 1.18.1 supports headless in create_backend(), but not BackendSelector.
            // The owned launcher selects it before any platform initialization.
            if std::env::var("SLINT_BACKEND").as_deref() != Ok("headless-software") {
                return Err("headless automation requires the private launcher".into());
            }
            #[cfg(feature = "ui-debug")]
            {
                slint::platform::set_platform(i_slint_backend_selector::create_backend()?)?;
                i_slint_backend_selector::init_testing_backends();
            }
        } else {
            slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("software".into())
                .select()?;
        }
    }
    slint::set_xdg_app_id(desktop_integration::APP_ID)?;
    #[cfg(feature = "demo")]
    if headless {
        fix_snapshot_clock();
    }
    let instance = if !headless {
        match desktop_integration::Instance::acquire_with_activation(!automation)? {
            desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    emoji::load();
    let activations = instance.as_ref().map(|i| i.take_events());
    #[cfg(feature = "demo")]
    let fake = demo.then(|| {
        if scene == "signed-out" {
            FakeBackend::signed_out()
        } else {
            FakeBackend::canned()
        }
    });
    #[cfg(feature = "demo")]
    if let Some(fake) = &fake {
        demo::configure(fake, &scene, case.as_deref());
    }
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
    let worker = if let Some(fake) = &fake {
        demo::worker(fake.clone(), text_size)
    } else {
        real_worker()
    };
    #[cfg(not(feature = "demo"))]
    let worker = real_worker();
    let ui = window::create_app_window()?;
    #[cfg(feature = "demo")]
    if fake.is_some() {
        demo::prepare(&ui, &scene, dark);
    }
    if let Some(warning) = instance.as_ref().and_then(|i| i.warning.as_ref()) {
        ui.set_desktop_warning(warning.into());
    }
    ui.set_text_size(text_size);
    ui.global::<State>().set_automation_background(automation);
    if !headless && !automation {
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
    let destination = viewer::Destination::Dialog;
    #[cfg(feature = "demo")]
    let destination = if demo {
        demo::destination()
    } else {
        destination
    };
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
        destination,
        notifications: if !headless {
            Some(notifications::Notifications::start())
        } else {
            None
        },
        background: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender: worker.sender.clone(),
        query: String::new(),
        category: 0,
    };
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
    // Initial synthetic rendering uses the same FIFO as interactive rendering.
    #[cfg(feature = "demo")]
    if headless {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while desktop.update(|d| {
            if scene == "signed-out" {
                !matches!(d.state.connection, Connection::SignedOut { .. })
            } else {
                d.state.messages.is_empty()
            }
        }) {
            let delta = worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))?;
            desktop.update(|d| {
                d.apply_batch(&ui, std::iter::once(delta).chain(worker.deltas.try_iter()))
            });
        }
    }
    let weak = ui.as_weak();
    let d = desktop.clone();
    let worker = Rc::new(RefCell::new(worker));
    #[cfg(feature = "demo")]
    if let Some(fake) = &fake {
        demo::install(
            &ui,
            desktop.clone(),
            fake.clone(),
            worker.clone(),
            text_size,
        );
    }
    let w = worker.clone();
    worker.borrow().on_deltas(event_loop::wake);
    event_loop::install(move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_integration::Activation::OpenChat(chat) = e {
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
    ui.window()
        .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
    #[cfg(feature = "demo")]
    demo::stage(&ui, &desktop, fake.as_ref(), &scene, snapshot_profile);
    #[cfg(feature = "demo")]
    if snapshot_path.is_some() {
        demo::settle(&ui, &desktop, &worker.borrow());
    }
    #[cfg(feature = "ui-debug")]
    if demo && std::env::var_os("LINE_UI_LATENCY").is_some() {
        let window = snapshot
            .as_ref()
            .expect("latency measurement requires --snapshot");
        desktop.update(|d| latency::run(d, &ui, window));
        return Ok(());
    }
    #[cfg(feature = "demo")]
    demo::profile(&ui, &scene, snapshot_profile);
    #[cfg(feature = "demo")]
    if let (Some(window), Some(path)) = (snapshot, snapshot_path) {
        ui.show()?;
        snapshot::save(&window, &path, size)?;
        ui.hide()?;
    }
    if !headless {
        ui.show()?;
        slint::run_event_loop_until_quit()?;
        ui.hide()?;
    }
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    drop(worker);
    Ok(())
}
