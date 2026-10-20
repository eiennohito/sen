use crate::*;
use slint::private_unstable_api::re_exports::WindowInner;

macro_rules! global_callback {
    ($ui:ident, $global:ty, $callback:ident, $method:ident) => {
        let weak = $ui.as_weak();
        $ui.global::<$global>().$callback(move || {
            if let Some(ui) = weak.upgrade() {
                ui.$method();
            }
        });
    };
}

pub(crate) fn install(ui: &AppWindow) {
    install_input(ui);
    global_callback!(ui, ShellState, on_focus_composer, invoke_focus_composer);
    global_callback!(
        ui,
        ShellState,
        on_focus_connection_first,
        invoke_focus_connection_first
    );
    global_callback!(
        ui,
        ShellState,
        on_focus_friend_search,
        invoke_focus_friend_search
    );
    global_callback!(
        ui,
        ConversationState,
        on_focus_history,
        invoke_focus_history
    );
    global_callback!(ui, ShellState, on_focus_search, invoke_focus_search);
    global_callback!(
        ui,
        ShellState,
        on_focus_settings_opener,
        invoke_focus_settings_opener
    );
    let weak = ui.as_weak();
    ui.global::<ShellState>().on_tab_focus(move |back| {
        if let Some(ui) = weak.upgrade() {
            let window = WindowInner::from_pub(ui.window());
            if back {
                window.focus_previous_item();
            } else {
                window.focus_next_item();
            }
        }
    });
    DETAIL_FOCUS.with_borrow_mut(Vec::clear);
}

thread_local! {
    static DETAIL_FOCUS: RefCell<Vec<FocusOpener>> = const { RefCell::new(Vec::new()) };
}
pub(crate) fn remember_detail(ui: &AppWindow, reset: bool) {
    DETAIL_FOCUS.with_borrow_mut(|saved| {
        if reset {
            saved.clear();
        }
        saved.push(ui.invoke_focus_opener());
    });
}
pub(crate) fn restore_detail(ui: &AppWindow, all: bool) {
    let opener = DETAIL_FOCUS.with_borrow_mut(|saved| {
        if all {
            let first = saved.first().copied();
            saved.clear();
            first
        } else {
            saved.pop()
        }
    });
    if let Some(opener) = opener {
        ui.invoke_restore_opener(opener);
    } else if ui.global::<DetailState>().get_detail_open() {
        ui.invoke_focus_profile_close();
    } else {
        ui.invoke_focus_composer();
    }
}

pub(crate) fn install_input(ui: &AppWindow) {
    thread_local! {
        static WINDOW: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    }
    let installed = WINDOW.with_borrow_mut(|window| window.replace(ui.as_weak()).is_some());
    if installed {
        return;
    }
    let current = || WINDOW.with_borrow(|window| window.as_ref().and_then(slint::Weak::upgrade));
    use i_slint_core::context::set_window_event_hook;
    use i_slint_core::input::key_codes;
    use slint::platform::WindowEvent;
    let chords = std::cell::Cell::new(0u8);
    let previous = set_window_event_hook(None).unwrap();
    set_window_event_hook(Some(Box::new(move |adapter, event, result| {
        if let Some(ui) = current() {
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
                        ui.global::<ShellState>().set_keyboard_navigation(true);
                    }
                }
                WindowEvent::WindowActiveChanged(false) => chords.set(0),
                WindowEvent::PointerPressed { .. } => {
                    ui.global::<ShellState>().set_keyboard_navigation(false)
                }
                _ => {}
            }
        }
        if let Some(previous) = &previous {
            previous(adapter, event, result);
        }
    })))
    .unwrap();
}
