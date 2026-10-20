use crate::FriendsState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_filter_friends, |ui, d| {
        d.presenter.present_people(&ui, &d.state);
        d.presenter.request_status_assets(&mut d.assets, &d.sender);
    });
}
