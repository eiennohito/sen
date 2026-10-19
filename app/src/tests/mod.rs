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
fn test_window() -> AppWindow {
    let ui = window::create_app_window().unwrap();
    let worker = Worker::start(
        StoreLocation::Memory,
        || Ok(line::backend::FakeBackend::canned()),
        Default::default(),
    );
    let desktop = Desktop {
        clipboard_root: std::env::temp_dir(),
        history: Default::default(),
        shown_composer: String::new(),
        reveal: None,
        viewer_asset: None,
        viewer_source: None,
        destination: viewer::save_dialog,
        notifications: None,
        background: Default::default(),
        assets: Default::default(),
        state: AppState::default(),
        presenter: Default::default(),
        sender: worker.sender.clone(),
        query: String::new(),
        category: ChatCategory::All,
    };
    let runtime = Runtime {
        desktop: Rc::new(RefCell::new(Some(desktop))),
        actions: Default::default(),
        draining: Default::default(),
    };
    callbacks::install(&ui, runtime);
    ui
}
mod grouping;
mod layers;
pub(crate) mod previews;

mod rich_flow;

mod ui_regressions;
