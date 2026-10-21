use crate::*;
#[cfg(feature = "ui-debug")]
pub(crate) mod automation;
pub(crate) mod demo;
mod demo_data;
#[cfg(feature = "ui-debug")]
mod latency;

pub(crate) fn install_focus(ui: &AppWindow) {
    remember_focus(ui);
    #[cfg(feature = "ui-debug")]
    {
        let weak = ui.as_weak();
        ui.global::<DemoState>().on_focused_control(move || {
            use i_slint_core::item_tree::ParentItemTraversalMode;
            use slint::private_unstable_api::re_exports::{AccessibleStringProperty, WindowInner};
            let Some(ui) = weak.upgrade() else {
                return "{}".into();
            };
            let window = WindowInner::from_pub(ui.window());
            let popup = window.active_popups().last().and_then(|popup| {
                use i_slint_core::window::PopupWindowLocation;
                let PopupWindowLocation::ChildWindow(position) = &popup.location else {
                    return None;
                };
                Some(serde_json::json!({
                    "x": position.x, "y": position.y,
                }))
            });
            let mut item = window.focus_item.borrow().upgrade();
            while let Some(current) = item {
                let label = current
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .unwrap_or_default();
                if current.is_accessible() && !label.is_empty() {
                    let geometry = current.geometry();
                    let position = current.map_to_window(geometry.origin);
                    return serde_json::json!({
                        "label": label.as_str(),
                        "visible": current.is_visible(),
                        "role": format!("{:?}", current.accessible_role()),
                        "x": position.x, "y": position.y,
                        "width": geometry.size.width, "height": geometry.size.height,
                        "popup": popup,
                    })
                    .to_string()
                    .into();
                }
                item = current.parent_item(ParentItemTraversalMode::FindAllParents);
            }
            serde_json::json!({"popup": popup}).to_string().into()
        });
    }
}

pub(crate) struct Session {
    args: demo::Args,
    pub(crate) fake: Option<FakeBackend>,
    window: Rc<RefCell<Option<AppWindow>>>,
    pub(crate) workers: Rc<RefCell<Worker>>,
    _store_directory: RefCell<Option<tempfile::TempDir>>,
    snapshot: Option<Rc<slint::platform::software_renderer::MinimalSoftwareWindow>>,
}
impl Session {
    pub(crate) fn start(
        args: demo::Args,
        real: impl FnOnce() -> Worker,
    ) -> Result<Rc<Self>, Box<dyn std::error::Error>> {
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
        let (worker, store_directory) = match &fake {
            Some(fake) => {
                DEMO_SESSION.set(()).expect("start demo session once");
                let (worker, directory) =
                    demo::worker(fake.clone(), args.text_size, &demo::directory());
                (worker, Some(directory))
            }
            None => (real(), None),
        };
        worker.on_deltas(event_loop::wake);
        Ok(Rc::new(Self {
            window: Default::default(),
            workers: Rc::new(RefCell::new(worker)),
            _store_directory: RefCell::new(store_directory),
            args,
            fake,
            snapshot,
        }))
    }
    pub(crate) fn install_window(
        self: &Rc<Self>,
        d: &mut Desktop,
        runtime: Runtime,
        fake: Option<&FakeBackend>,
    ) -> Result<AppWindow, slint::PlatformError> {
        let ui = window::create_app_window()?;
        let (width, height) = (self.args.size.0 as f32, self.args.size.1 as f32);
        // Headless snapshots and UI checks render at --demo-scale (2 for HiDPI captures),
        // sized in physical pixels: the testing backend converts logical sizes at scale 1.
        // Desktop windows keep the system's scale.
        if self.snapshot.is_some() || self.args.background {
            let scale = self.args.scale;
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                    scale_factor: scale,
                });
            ui.window().set_size(slint::PhysicalSize::new(
                (width * scale).round() as u32,
                (height * scale).round() as u32,
            ));
        } else {
            ui.window().set_size(slint::LogicalSize::new(width, height));
        }
        window::configure(
            &ui,
            d,
            runtime.clone(),
            self.snapshot.is_none() && !self.args.background,
        );
        ui.global::<Palette>().set_color_scheme(if self.args.dark {
            slint::language::ColorScheme::Dark
        } else {
            slint::language::ColorScheme::Unknown
        });
        ui.global::<DemoState>()
            .set_automation_background(self.args.background);
        if let Some(fake) = fake {
            demo::prepare(&ui);
            demo::install(&ui, runtime, fake.clone(), self);
        }
        self.window.replace(Some(ui.clone_strong()));
        Ok(ui)
    }
    pub(crate) fn window_getter(&self) -> impl Fn() -> Option<AppWindow> + use<> {
        let window = self.window.clone();
        move || window.borrow().as_ref().map(ComponentHandle::clone_strong)
    }
    pub(crate) fn delta_drain(&self) -> impl Fn() -> Vec<Delta> + use<> {
        let workers = self.workers.clone();
        move || workers.borrow().deltas.try_iter().collect()
    }
    pub(crate) fn stage(
        &self,
        ui: &AppWindow,
        desktop: &Runtime,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let worker = &self.workers;
        let args = &self.args;
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
        if args.enabled && std::env::var_os("SEN_UI_LATENCY").is_some() {
            let window = self
                .snapshot
                .as_ref()
                .expect("latency measurement requires --snapshot");
            desktop.update(|d| latency::run(d, ui, window));
            return Ok(true);
        }
        if args.snapshot_profile || args.scene == "member-profile" {
            ui.global::<DetailState>()
                .invoke_open_profile("alice".into());
        }
        if let (Some(window), Some(path)) = (self.snapshot.as_ref(), args.snapshot_path.as_ref()) {
            ui.show()?;
            let (width, height) = args.size;
            let scaled = |v: u32| (v as f32 * args.scale).round() as u32;
            snapshot::save(window, path, (scaled(width), scaled(height)))?;
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

fn image_mut(
    content: &mut sen::store::Content,
) -> &mut Option<sen::client::desktop::assets::Asset> {
    use sen::store::Content;
    match content {
        Content::Photo { asset, .. } | Content::Sticker { asset, .. } => asset,
        Content::Video { preview, .. } => preview,
        _ => panic!("content has no image"),
    }
}

pub(crate) fn remember_focus(ui: &AppWindow) {
    let saved = Rc::new(RefCell::new(None));
    let record = saved.clone();
    let weak = ui.as_weak();
    ui.global::<DemoState>().on_remember_demo_focus(move || {
        if let Some(ui) = weak.upgrade() {
            *record.borrow_mut() = Some(ui.invoke_focus_opener());
        }
    });
    let weak = ui.as_weak();
    ui.global::<DemoState>().on_restore_demo_focus(move || {
        if let Some(ui) = weak.upgrade()
            && let Some(opener) = saved.borrow_mut().take()
        {
            ui.invoke_restore_opener(opener);
        }
    });
}

pub(crate) static DEMO_SESSION: std::sync::OnceLock<()> = std::sync::OnceLock::new();
