use super::*;

pub(super) fn install_reset(ui: &AppWindow, actions: &Runtime, session: &Rc<dev::Session>) {
    ui.global::<DemoState>().set_demo_reset_enabled(true);
    ui.global::<DemoState>()
        .set_demo_instance(std::env::var("LINE_UI_INSTANCE").unwrap_or_default().into());
    let session = Rc::downgrade(session);
    let weak = ui.as_weak();
    let actions = actions.clone();
    ui.global::<DemoState>().on_demo_automation(move |hook| {
        if hook != line_desktop_ui::DemoAutomation::Reset {
            return;
        }
        let weak = weak.clone();
        let runtime = actions.clone();
        let session = session.clone();
        actions.push(move |d| {
            let Some(old) = weak.upgrade() else { return };
            let Some(session) = session.upgrade() else {
                return;
            };
            let fresh = FakeBackend::canned();
            demo::configure(&fresh, "", None);
            let (worker, directory) =
                demo::worker(fresh.clone(), session.args.text_size, &demo::directory());
            worker.on_deltas(event_loop::wake);
            d.sender = worker.sender.clone();
            d.state = AppState::default();
            d.history = reading::Reading {
                opening_history: true,
                ..Default::default()
            };
            d.assets = assets::Assets::default();
            d.presenter = presenter::Presenter::default();
            d.shown_composer.clear();
            d.category = ChatCategory::All;
            d.reveal = None;
            d.viewer_asset = None;
            d.viewer_source = None;
            let size = old.window().size();
            session.workers.replace(worker);
            session._store_directory.replace(Some(directory));
            let ui = session
                .install_window(d, runtime.clone(), Some(&fresh))
                .expect("recreate demo window");
            ui.window().set_size(size);
            ui.global::<Palette>()
                .set_color_scheme(old.global::<Palette>().get_color_scheme());
            ui.window().set_position(old.window().position());
            ui.global::<ShellState>()
                .set_device_scale(old.global::<ShellState>().get_device_scale());
            ui.global::<DemoState>().set_demo_reset_generation(
                old.global::<DemoState>().get_demo_reset_generation() + 1,
            );
            d.render(&ui);
            ui.invoke_focus_search();
            ui.show().expect("show reset demo");
            old.hide().expect("hide previous demo");
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
    ui.global::<DemoState>().set_internal_error_seen(true);
    ui.global::<DemoState>()
        .set_internal_error(format!("Internal error: {kind} (see log)").into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
        if let Some(ui) = weak.upgrade() {
            ui.global::<DemoState>().set_internal_error("".into());
        }
    });
}

pub(crate) fn inject_attachments(ui: &AppWindow, actions: &Runtime) {
    let weak = ui.as_weak();
    let actions = actions.clone();
    ui.global::<DemoState>()
        .on_inject_attachments(move |paths| {
            let weak = weak.clone();
            actions.push(move |d| {
                if let Some(ui) = weak.upgrade()
                    && ui.global::<DemoState>().get_automation_background()
                {
                    let paths: Vec<std::path::PathBuf> =
                        serde_json::from_str(&paths).unwrap_or_default();
                    crate::callbacks::attachments::begin(&ui, d, move || Ok(paths));
                }
            });
        });
}
