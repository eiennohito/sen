macro_rules! callback {($ui:ident, $actions:ident, $name:ident, |$u:ident,$d:ident $(,$arg:ident)*| $body:block)=>{{
        let weak=$ui.as_weak(); let actions=$actions.clone();
        $ui.$name(move |$($arg),*| {let weak=weak.clone(); actions.push(move |$d|{if let Some($u)=weak.upgrade() {$body}});});
    }};}
macro_rules! global_callback {($ui:ident, $actions:ident, $name:ident, |$u:ident,$d:ident $(,$arg:ident)*| $body:block)=>{{
        let weak=$ui.as_weak(); let actions=$actions.clone();
        $ui.global::<State>().$name(move |$($arg),*| {let weak=weak.clone(); actions.push(move |$d|{if let Some($u)=weak.upgrade() {$body}});});
    }};}
macro_rules! focus_bridge {
    ($ui:ident, $actions:ident, $on:ident,$invoke:ident) => {
        global_callback!($ui, $actions, $on, |ui, _d| {
            ui.$invoke();
        });
    };
}

pub(crate) mod attachments;
mod browser;
mod composer;
mod detail;
mod focus;
mod history;
mod settings;
mod viewer;

use crate::*;
pub(crate) fn install(ui: &AppWindow, actions: Runtime) {
    attachments::install(ui, &actions);
    composer::install(ui, &actions);
    history::install(ui, &actions);
    browser::install(ui, &actions);
    detail::install(ui, &actions);
    settings::install(ui, &actions);
    viewer::install(ui, &actions);
    focus::install(ui, &actions);
    global_callback!(ui, actions, on_open_link, |ui, _d, url| {
        let weak = ui.as_weak();
        let chat = ui.global::<State>().get_chat_id();
        std::thread::spawn(move || {
            let result = links::open(&url);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<State>();
                if g.get_chat_id() == chat {
                    if result.is_err() {
                        g.set_link_feedback(text::LINK_FAILURE.into());
                    } else if g.get_link_feedback() == text::LINK_FAILURE {
                        g.set_link_feedback("".into());
                    }
                }
            });
        });
    });
    rich_content::install(ui);
}
