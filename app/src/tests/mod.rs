use super::*;
fn message(id: &str, text: &str) -> account::MessageView {
    account::MessageView {
        id: id.into(),
        body: text.into(),
        timestamp: 1,
        delivered_time: 1,
        ..Default::default()
    }
}
fn test_window() -> AppWindow {
    let ui = create_app_window().unwrap();
    let weak = ui.as_weak();
    ui.global::<State>().on_request_panel(move |panel| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<State>().set_panel(panel);
        }
    });
    ui
}
mod grouping;
mod layers;
mod previews;
