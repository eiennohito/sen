use super::*;

pub(super) fn install_reset(
    ui: &AppWindow,
    actions: &Runtime,
    workers: Rc<RefCell<Worker>>,
    options: &window::Options,
) {
    ui.global::<ExtensionState>().set_demo_reset_enabled(true);
    ui.global::<ExtensionState>()
        .set_demo_instance(std::env::var("LINE_UI_INSTANCE").unwrap_or_default().into());
    let options = options.clone();
    let weak = ui.as_weak();
    let actions = actions.clone();
    ui.global::<ExtensionState>()
        .on_demo_automation(move |hook| {
            if hook != line_desktop_ui::DemoAutomation::Reset {
                return;
            }
            let weak = weak.clone();
            let workers = workers.clone();
            let runtime = actions.clone();
            let options = options.clone();
            actions.push(move |d| {
                let Some(old) = weak.upgrade() else { return };
                let fresh = FakeBackend::canned();
                demo::configure(&fresh, "", None);
                let worker = demo::worker(fresh.clone(), options.launch.text_size);
                worker.on_deltas(event_loop::wake);
                d.sender = worker.sender.clone();
                workers.replace(worker);
                d.state = AppState::default();
                d.history = reading::Reading {
                    opening_history: true,
                    ..Default::default()
                };
                d.assets = assets::Assets::default();
                d.presenter = presenter::Presenter::default();
                d.shown_composer.clear();
                d.query.clear();
                d.category = ChatCategory::All;
                d.reveal = None;
                d.viewer_asset = None;
                d.viewer_source = None;
                let mut options = options;
                let size = old.window().size().to_logical(old.window().scale_factor());
                options.launch.size = (size.width as u32, size.height as u32);
                options.color_scheme = old.global::<Palette>().get_color_scheme();
                options.fake = Some(fresh);
                let ui = window::install_window(d, runtime.clone(), &workers, &options)
                    .expect("recreate demo window");
                ui.window().set_position(old.window().position());
                ui.global::<State>()
                    .set_device_scale(old.global::<State>().get_device_scale());
                ui.global::<ExtensionState>().set_demo_reset_generation(
                    old.global::<ExtensionState>().get_demo_reset_generation() + 1,
                );
                d.render(&ui);
                ui.invoke_focus_search();
                ui.show().expect("show reset demo");
                old.hide().expect("hide previous demo");
                runtime.window.upgrade().unwrap().replace(Some(ui));
            });
        });
}

pub(super) fn launch(background: bool, demo: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !background {
        return Ok(());
    }
    if !demo {
        return Err("--debug-background requires a ui-debug build and an offline demo.".into());
    }
    if std::env::var("SLINT_BACKEND").as_deref() != Ok("headless-software") {
        return Err("headless automation requires the private launcher".into());
    }
    // The selector cannot select Slint 1.18's headless backend.
    slint::platform::set_platform(i_slint_backend_selector::create_backend()?)?;
    i_slint_backend_selector::init_testing_backends();
    Ok(())
}
pub(crate) fn internal_error(ui: &AppWindow, kind: &str) {
    ui.global::<ExtensionState>().set_internal_error_seen(true);
    ui.global::<ExtensionState>()
        .set_internal_error(format!("Internal error: {kind} (see log)").into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
        if let Some(ui) = weak.upgrade() {
            ui.global::<ExtensionState>().set_internal_error("".into());
        }
    });
}

pub(crate) fn inject_attachments(
    ui: &AppWindow,
    actions: &Runtime,
    selection: std::sync::Arc<std::sync::Mutex<crate::callbacks::attachments::Selection>>,
) {
    let weak = ui.as_weak();
    let actions = actions.clone();
    ui.global::<ExtensionState>()
        .on_inject_attachments(move |paths| {
            let weak = weak.clone();
            let selected = selection.clone();
            actions.push(move |d| {
                if let Some(ui) = weak.upgrade()
                    && ui.global::<ExtensionState>().get_automation_background()
                {
                    let paths: Vec<std::path::PathBuf> =
                        serde_json::from_str(&paths).unwrap_or_default();
                    crate::callbacks::attachments::begin(&ui, d, &selected, move || Ok(paths));
                }
            });
        });
}
