use crate::*;
#[cfg(feature = "ui-debug")]
pub(crate) mod automation;
pub(crate) mod demo;
mod demo_data;
#[cfg(feature = "ui-debug")]
mod latency;

pub(crate) fn install_focus(ui: &AppWindow) {
    focus::install_demo(ui);
    #[cfg(feature = "ui-debug")]
    {
        let weak = ui.as_weak();
        ui.global::<ExtensionState>().on_focused_control(move || {
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
    pub(crate) fn fake(&self) -> Option<&FakeBackend> {
        self.fake.as_ref()
    }
    pub(crate) fn color_scheme(&self) -> slint::language::ColorScheme {
        if self.args.dark {
            slint::language::ColorScheme::Dark
        } else {
            slint::language::ColorScheme::Unknown
        }
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

fn image_mut(
    content: &mut line::store::Content,
) -> &mut Option<line::client::desktop::assets::Asset> {
    use line::store::Content;
    match content {
        Content::Photo { asset, .. } | Content::Sticker { asset, .. } => asset,
        Content::Video { preview, .. } => preview,
        _ => panic!("content has no image"),
    }
}
