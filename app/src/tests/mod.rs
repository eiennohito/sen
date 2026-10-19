use super::*;
fn message(_id: &str, text: &str) -> StoredMessage {
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
    let weak = ui.as_weak();
    ui.global::<State>().on_request_panel(move |panel| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<State>().set_panel(panel);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_pop_detail(move |all| {
        if let Some(ui) = weak.upgrade() {
            let g = ui.global::<State>();
            g.set_detail_depth(if all {
                0
            } else {
                (g.get_detail_depth() - 1).max(0)
            });
            if g.get_detail_depth() == 0 {
                g.set_panel(PanelState::None);
            }
        }
    });
    ui
}
mod grouping;
mod layers;
mod previews;
