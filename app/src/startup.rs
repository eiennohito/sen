use crate::*;

pub(crate) struct Options {
    pub demo: bool,
    pub snapshot_path: Option<String>,
    pub size: (u32, u32),
    pub text_size: i32,
    pub scene: String,
    pub case: Option<String>,
    pub dark: bool,
    pub snapshot_profile: bool,
    pub background: bool,
    pub locations: line_wasm_poc::locations::Locations,
}

pub(crate) fn start(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        demo,
        snapshot_path,
        size,
        text_size,
        scene,
        case,
        dark,
        snapshot_profile,
        background,
        locations,
    } = options;
    let automation = debug_launch::intent(
        background,
        cfg!(feature = "ui-debug"),
        demo,
        std::env::var("LINE_DESKTOP_AUTOMATION_SOCKET")
            .ok()
            .as_deref(),
    )? == debug_launch::Intent::VirtualAutomation;
    let snapshot = if snapshot_path.is_some() {
        Some(snapshot::install()?)
    } else {
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
        None
    };
    slint::set_xdg_app_id("org.lineclient.Desktop")?;
    if snapshot.is_some() {
        fix_snapshot_clock();
    }
    let instance = if snapshot.is_none() {
        match desktop_integration::Instance::acquire_with_activation(!automation)? {
            desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    let activations = instance.as_ref().map(|i| i.take_events());
    let fake = demo.then(|| {
        if scene == "signed-out" {
            FakeBackend::signed_out()
        } else {
            FakeBackend::canned()
        }
    });
    if let Some(fake) = &fake {
        demo::configure(fake, &scene, case.as_deref());
    }
    let assets = assets::Assets::default();
    let projection = assets.clone();
    let worker = if let Some(fake) = &fake {
        demo::worker(fake.clone(), text_size, projection)
    } else {
        let root = locations.session.parent().unwrap().join("accounts");
        Worker::start_projected(
            StoreLocation::Account { root, mid: None },
            move || LineBackend::open_local(&locations.session, &locations.wasm, &locations.cache),
            Default::default(),
            move |delta| projection.project(delta),
        )
    };
    let ui = window::create_app_window()?;
    if fake.is_some() {
        demo::prepare(&ui, &scene, dark);
    }
    if let Some(warning) = instance.as_ref().and_then(|i| i.warning.as_ref()) {
        ui.set_desktop_warning(warning.into());
    }
    ui.set_text_size(text_size);
    ui.global::<State>().set_automation_background(automation);
    if snapshot.is_none() && !automation {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{Ime, WindowEvent},
        };
        let weak = ui.as_weak();
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
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
        history: reading::Reading {
            opening_history: true,
            ..Default::default()
        },
        assets,
        shown_composer: String::new(),
        reveal: None,
        viewer_asset: None,
        destination: if demo {
            demo::destination()
        } else {
            viewer::Destination::Dialog
        },
        notifications: if snapshot.is_none() {
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
    callbacks::install(&ui, desktop.clone(), fake.clone());
    // Initial synthetic rendering uses the same FIFO as interactive rendering.
    if snapshot.is_some() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while desktop.update(|d| {
            if scene == "signed-out" {
                d.state.connection != Connection::SignedOut
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
    let worker = Rc::new(worker);
    let w = worker.clone();
    worker.on_deltas(event_loop::wake);
    event_loop::install(move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(events) = &activations {
                for e in events.try_iter() {
                    let _ = ui.show();
                    if let desktop_integration::Activation::OpenChat(chat) = e {
                        d.update(|d| {
                            let intent = d.state.select(ChatId(chat));
                            d.sender.send(intent);
                            d.render(&ui);
                            ui.set_show_conversation(true);
                        });
                    }
                }
            }
            d.update(|d| d.apply_batch(&ui, w.deltas.try_iter()));
            d.update(|d| {
                d.background.apply(&ui);
                d.observe_reading(&ui);
                d.drain_notifications(&ui);
            });
        }
    });
    ui.window()
        .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
    demo::stage(&ui, &desktop, fake.as_ref(), &scene, snapshot_profile);
    if snapshot_path.is_some() {
        demo::settle(&ui, &desktop, &worker);
    }
    #[cfg(feature = "ui-debug")]
    if demo && std::env::var_os("LINE_UI_LATENCY").is_some() {
        let window = snapshot
            .as_ref()
            .expect("latency measurement requires --snapshot");
        desktop.update(|d| latency::run(d, &ui, window));
        return Ok(());
    }
    demo::profile(&ui, &scene, snapshot_profile);
    if let (Some(window), Some(path)) = (snapshot, snapshot_path) {
        ui.show()?;
        snapshot::save(&window, &path, size)?;
        ui.hide()?;
    } else {
        ui.show()?;
        slint::run_event_loop_until_quit()?;
        ui.hide()?;
    }
    desktop.update(|d| d.capture_draft(&ui));
    event_loop::clear();
    drop(worker);
    Ok(())
}
