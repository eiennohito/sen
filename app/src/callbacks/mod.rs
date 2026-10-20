macro_rules! global_callback {($ui:ident, $actions:ident, $global:ty, $name:ident $(, [$($capture:ident),*])?, |$u:ident,$d:ident $(,$arg:ident)*| $body:block $(=> $result:expr)?)=>{{
        let weak=$ui.as_weak(); let actions=$actions.clone();
        $ui.global::<$global>().$name(move |$($arg),*| {let weak=weak.clone(); $($(let $capture = $capture.clone();)*)? actions.push(move |$d|{if let Some($u)=weak.upgrade() {$body}}); $($result)? });
    }};}
pub(crate) mod attachments;
mod auth;
mod browser;
mod composer;
mod content;
mod detail;
mod friends;
mod history;
mod new_chat;
mod settings;
mod shell;
mod viewer;

use crate::ContentState as G;
use crate::*;
pub(crate) fn install(ui: &AppWindow, actions: Runtime) {
    shell::install(ui, &actions);
    friends::install(ui, &actions);
    auth::install(ui, &actions);
    content::install(ui, &actions);
    attachments::install(ui, &actions);
    composer::install(ui, &actions);
    new_chat::install(ui, &actions);
    history::install(ui, &actions);
    browser::install(ui, &actions);
    detail::install(ui, &actions);
    settings::install(ui, &actions);
    viewer::install(ui, &actions);
    global_callback!(ui, actions, G, on_open_link, |ui, _d, url| {
        let weak = ui.as_weak();
        let chat = ui.global::<ConversationState>().get_chat_id();
        std::thread::spawn(move || {
            let result = rich_content::links::open(&url);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if ui.global::<ConversationState>().get_chat_id() == chat {
                    if result.is_err() {
                        ui.global::<ContentState>()
                            .set_link_feedback(text::LINK_FAILURE.into());
                    } else if ui.global::<ContentState>().get_link_feedback() == text::LINK_FAILURE
                    {
                        ui.global::<ContentState>().set_link_feedback("".into());
                    }
                }
            });
        });
    });
    ui.global::<ContentState>().on_copy_text({
        let weak = ui.as_weak();
        move |text| {
            if let Some(ui) = weak.upgrade() {
                slint::private_unstable_api::re_exports::WindowInner::from_pub(ui.window())
                    .context()
                    .platform()
                    .set_clipboard_text(&text, slint::platform::Clipboard::DefaultClipboard);
            }
        }
    });
    rich_content::install(ui);
}

fn message_key(id: &str) -> Option<Key> {
    id.parse().ok().map(Key)
}
