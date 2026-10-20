use crate::*;
// Slint has no public API for saving and restoring the focused item.
use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};

pub(crate) fn install(ui: &AppWindow) {
    let saved = Rc::new(RefCell::new(Vec::new()));
    let reset = saved.clone();
    ui.global::<State>()
        .on_forget_detail_focus(move || reset.borrow_mut().clear());
    let record = saved.clone();
    let weak = ui.as_weak();
    ui.global::<State>().on_remember_detail_focus(move |reset| {
        if let Some(ui) = weak.upgrade() {
            if reset {
                record.borrow_mut().clear();
            }
            record.borrow_mut().push(
                WindowInner::from_pub(ui.window())
                    .focus_item
                    .borrow()
                    .clone(),
            );
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_restore_detail_focus(move |all| {
        if let Some(ui) = weak.upgrade() {
            let item = if all {
                let first = saved.borrow().first().cloned();
                saved.borrow_mut().clear();
                first
            } else {
                saved.borrow_mut().pop()
            };
            if let Some(item) = item.and_then(|i| i.upgrade()).filter(|i| i.is_visible()) {
                WindowInner::from_pub(ui.window()).set_focus_item(
                    &item,
                    true,
                    FocusReason::PopupActivation,
                );
            } else if ui.global::<State>().get_detail_open() {
                ui.invoke_focus_profile_close();
            } else {
                ui.invoke_focus_composer();
            }
        }
    });
}

#[cfg(feature = "demo")]
pub(crate) fn install_demo(ui: &AppWindow) {
    let saved = Rc::new(RefCell::new(None));
    let record = saved.clone();
    let weak = ui.as_weak();
    ui.global::<ExtensionState>()
        .on_remember_demo_focus(move || {
            if let Some(ui) = weak.upgrade() {
                *record.borrow_mut() = Some(
                    WindowInner::from_pub(ui.window())
                        .focus_item
                        .borrow()
                        .clone(),
                );
            }
        });
    let weak = ui.as_weak();
    ui.global::<ExtensionState>()
        .on_restore_demo_focus(move || {
            if let Some(ui) = weak.upgrade()
                && let Some(item) = saved.borrow_mut().take().and_then(|weak| weak.upgrade())
            {
                WindowInner::from_pub(ui.window()).set_focus_item(
                    &item,
                    true,
                    FocusReason::PopupActivation,
                );
            }
        });
}
