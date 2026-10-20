use super::*;
pub(crate) fn message(text: &str) -> StoredMessage {
    StoredMessage {
        key: Key(1),
        chat_id: ChatId("chat".into()),
        server_id: None,
        local_id: None,
        request_seq: None,
        sort: line::store::Cursor {
            sort_ts: 1,
            sort_id: "1".into(),
        },
        sender_id: String::new(),
        outgoing: false,
        body: Content::text(text),
        reactions: vec![],
        reactions_rev: 0,
        body_rev: None,
        delivery: Delivery::Sent,
        created_at_local: 1,
        mentioned: false,
        reply: None,
        skip_badge: false,
    }
}
pub(crate) fn test_window(
    backend: line::backend::FakeBackend,
    seed: impl FnOnce(&mut Desktop, &Worker, &AppWindow),
) -> (AppWindow, Runtime, Worker) {
    let worker = Worker::start(
        StoreLocation::Memory,
        {
            let mut backend = Some(backend);
            move || Ok(backend.take().unwrap())
        },
        Default::default(),
    );
    let ui = window::create_app_window().unwrap();
    let mut desktop = desktop(worker.sender.clone());
    desktop.presenter.install(&ui);
    seed(&mut desktop, &worker, &ui);
    let runtime = Runtime {
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: Default::default(),
        draining: Default::default(),
    };
    callbacks::install(&ui, runtime.clone());
    (ui, runtime, worker)
}

mod grouping;
mod layers;
pub(crate) mod previews;
mod search;

mod rich_flow;

mod ui_regressions;

mod messages;

fn element(ui: &AppWindow, label: &str) -> i_slint_backend_testing::ElementHandle {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(ui, label)
        .next()
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn visible(ui: &AppWindow, label: &str) -> bool {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(ui, label)
        .next()
        .is_some()
}
fn search(ui: &AppWindow) -> i_slint_backend_testing::ElementHandle {
    element(ui, "Find a conversation")
}

pub(crate) fn image_file(image: &slint::Image) -> std::path::PathBuf {
    thread_local! { static FIXTURES: line::backend::FakeBackend = line::backend::FakeBackend::canned(); }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let pixels = image.to_rgba8().unwrap();
    FIXTURES.with(|fake| {
        fake.set_image(
            &line::client::desktop::assets::Asset::avatar(&format!(
                "/fixture-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
            .unwrap()
            .original()
            .unwrap(),
            line::client::desktop::assets::Pixels {
                width: pixels.width(),
                height: pixels.height(),
                rgba: pixels.as_bytes().to_vec(),
            },
        )
    })
}

pub(crate) fn desktop(sender: Sender) -> Desktop {
    Desktop {
        clipboard_root: std::env::temp_dir(),
        history: Default::default(),
        shown_composer: String::new(),
        reveal: None,
        viewer_asset: None,
        viewer_source: None,
        viewer_name: String::new(),
        attachments: Default::default(),
        notifications: None,
        background: Default::default(),
        assets: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender,
        category: ChatCategory::All,
    }
}
