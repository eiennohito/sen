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
pub(crate) fn install(ui: &AppWindow, actions: Runtime, fake: Option<FakeBackend>) {
    attachments::install(ui, &actions);
    composer::install(ui, &actions);
    history::install(ui, &actions);
    browser::install(ui, &actions);
    detail::install(ui, &actions);
    settings::install(ui, &actions);
    viewer::install(ui, &actions);
    focus::install(ui, &actions);
    let link_actions = actions.clone();
    ui.global::<State>().on_open_link(move |url| {
        link_actions.push(move |_| links::open(url.to_string()));
    });
    if let Some(fake) = fake {
        demo::install(ui, actions, fake);
    }
    rich_content::install(ui);
}
