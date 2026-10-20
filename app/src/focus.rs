use crate::*;
// Slint has no public API for saving and restoring the focused item.
use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};

pub(crate) fn install(ui: &AppWindow) {
    // One application window, replaced when the offline demo resets.
    thread_local! {
        static INPUT_WINDOW: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    }
    INPUT_WINDOW.with_borrow_mut(|window| {
        if window.is_none() {
            use i_slint_core::context::set_window_event_hook;
            use i_slint_core::input::key_codes;
            use slint::platform::WindowEvent;
            let chords = std::cell::Cell::new(0u8);
            let previous = set_window_event_hook(None).unwrap();
            set_window_event_hook(Some(Box::new(move |adapter, event, result| {
                INPUT_WINDOW.with_borrow(|window| {
                    if let Some(ui) = window.as_ref().and_then(slint::Weak::upgrade) {
                        // Touch taps aren't delivered to this hook.
                        match event {
                            WindowEvent::KeyPressed { text }
                            | WindowEvent::KeyPressRepeated { text }
                            | WindowEvent::KeyReleased { text } => {
                                let key = text.chars().next();
                                let chord = match key {
                                    Some(key_codes::Control) => 1,
                                    Some(key_codes::ControlR) => 2,
                                    Some(key_codes::Meta) => 4,
                                    Some(key_codes::MetaR) => 8,
                                    _ => 0,
                                };
                                if matches!(event, WindowEvent::KeyReleased { .. }) {
                                    chords.set(chords.get() & !chord);
                                } else if chord != 0 {
                                    chords.set(chords.get() | chord);
                                } else if chords.get() == 0
                                    && !matches!(
                                        key,
                                        Some(
                                            key_codes::Shift
                                                | key_codes::ShiftR
                                                | key_codes::Alt
                                                | key_codes::AltGr
                                                | key_codes::CapsLock
                                        )
                                    )
                                {
                                    ui.global::<State>().set_keyboard_navigation(true);
                                }
                            }
                            WindowEvent::WindowActiveChanged(false) => chords.set(0),
                            WindowEvent::PointerPressed { .. } => {
                                ui.global::<State>().set_keyboard_navigation(false)
                            }
                            _ => {}
                        }
                    }
                });
                if let Some(previous) = &previous {
                    previous(adapter, event, result);
                }
            })))
            .unwrap();
        }
        *window = Some(ui.as_weak());
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_tab_focus(move |back| {
        if let Some(ui) = weak.upgrade() {
            let window = WindowInner::from_pub(ui.window());
            if back {
                window.focus_previous_item();
            } else {
                window.focus_next_item();
            }
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_text_input_focused(move || {
        weak.upgrade().is_some_and(|ui| {
            WindowInner::from_pub(ui.window())
                .focus_item
                .borrow()
                .upgrade()
                .is_some_and(|item| {
                    item.downcast::<slint::private_unstable_api::re_exports::TextInput>()
                        .is_some_and(|input| !input.as_pin_ref().read_only())
                })
        })
    });
    let saved = Rc::new(RefCell::new(Vec::new()));
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
