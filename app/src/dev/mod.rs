use crate::*;
#[cfg(feature = "ui-debug")]
pub(crate) mod automation;
pub(crate) mod demo;
mod demo_data;
#[cfg(feature = "ui-debug")]
mod latency;

pub(crate) fn install_window(ui: &AppWindow) {
    focus::install_demo(ui);
    let weak = ui.as_weak();
    ui.global::<State>().on_demo_event(move |kind| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_demo_event(kind);
        }
    });
}

pub(crate) struct Session {
    args: demo::Args,
    fake: Option<FakeBackend>,
    snapshot: Option<Rc<slint::platform::software_renderer::MinimalSoftwareWindow>>,
}
impl Session {
    pub(crate) fn start(
        args: demo::Args,
    ) -> Result<(startup::Launch, Self), Box<dyn std::error::Error>> {
        let snapshot = args
            .snapshot_path
            .as_ref()
            .map(|_| snapshot::install())
            .transpose()?;
        let headless = snapshot.is_some();
        #[cfg(feature = "ui-debug")]
        if !headless {
            automation::launch(args.background, args.enabled)?;
        }
        if !headless && !args.background {
            startup::select_platform()?;
        }
        if headless {
            presenter::format::set_clock(snapshot_time);
        }
        let fake = args.enabled.then(|| {
            if args.scene == "signed-out" {
                FakeBackend::signed_out()
            } else {
                FakeBackend::canned()
            }
        });
        if let Some(fake) = &fake {
            demo::configure(fake, &args.scene, args.case.as_deref());
        }
        let launch = startup::Launch {
            headless,
            automation: args.background,
            size: args.size,
            text_size: args.text_size,
        };
        Ok((
            launch,
            Self {
                args,
                fake,
                snapshot,
            },
        ))
    }
    pub(crate) fn worker(&self, real: impl FnOnce() -> Worker) -> Worker {
        self.fake
            .as_ref()
            .map(|fake| demo::worker(fake.clone(), self.args.text_size))
            .unwrap_or_else(real)
    }
    pub(crate) fn stage(
        self,
        ui: &AppWindow,
        desktop: &Runtime,
        worker: &Rc<RefCell<Worker>>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let args = self.args;
        if let Some(fake) = &self.fake {
            demo::prepare(ui, args.dark);
            desktop.update(|d| d.destination = demo::destination());
            demo::install(
                ui,
                desktop.clone(),
                fake.clone(),
                #[cfg(feature = "ui-debug")]
                worker.clone(),
                #[cfg(feature = "ui-debug")]
                args.text_size,
            );
        }
        if self.snapshot.is_some() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while desktop.update(|d| {
                if args.scene == "signed-out" {
                    !matches!(d.state.connection, Connection::SignedOut { .. })
                } else {
                    d.state.messages.is_empty()
                }
            }) {
                let worker = worker.borrow();
                let delta = worker
                    .deltas
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))?;
                desktop.update(|d| {
                    d.apply_batch(ui, std::iter::once(delta).chain(worker.deltas.try_iter()))
                });
            }
        }
        demo::stage(
            ui,
            desktop,
            self.fake.as_ref(),
            &args.scene,
            args.snapshot_profile,
        );
        if args.snapshot_path.is_some() {
            demo::settle(ui, desktop, &worker.borrow());
        }
        #[cfg(feature = "ui-debug")]
        if args.enabled && std::env::var_os("LINE_UI_LATENCY").is_some() {
            let window = self
                .snapshot
                .as_ref()
                .expect("latency measurement requires --snapshot");
            desktop.update(|d| latency::run(d, ui, window));
            return Ok(true);
        }
        demo::profile(ui, &args.scene, args.snapshot_profile);
        if let (Some(window), Some(path)) = (self.snapshot, args.snapshot_path) {
            ui.show()?;
            snapshot::save(&window, &path, args.size)?;
            ui.hide()?;
        }
        Ok(false)
    }
}
fn snapshot_time() -> i64 {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 10, 18, 14, 0, 0)
        .unwrap()
        .timestamp_millis()
}
pub(crate) fn rendered() {
    #[cfg(feature = "ui-debug")]
    latency::RENDERS.set(latency::RENDERS.get() + 1);
}
