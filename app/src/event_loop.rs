//! Coalesce worker and desktop-service arrivals into one pending UI wakeup.
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
thread_local! {
    static RUNTIME: RefCell<Option<crate::Runtime>> = const { RefCell::new(None) };
    static DRAIN: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
}
static PENDING: AtomicBool = AtomicBool::new(false);
pub fn install(drain: impl Fn() + 'static) {
    DRAIN.with(|slot| *slot.borrow_mut() = Some(Rc::new(drain)));
    wake();
}
pub fn set_runtime(runtime: crate::Runtime) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(runtime));
}
// Drops completions after RUNTIME is cleared during shutdown.
pub fn push(action: impl FnOnce(&mut crate::Desktop) + 'static) {
    let runtime = RUNTIME.with(|slot| slot.borrow().clone());
    if let Some(runtime) = runtime {
        runtime.push(action);
    }
}
pub fn clear() {
    RUNTIME.with(|slot| slot.borrow_mut().take());
    DRAIN.with(|slot| slot.borrow_mut().take());
}
pub fn wake() {
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    if slint::invoke_from_event_loop(|| {
        // Clear before draining: arrivals during a drain schedule another pass.
        PENDING.store(false, Ordering::Release);
        let drain = DRAIN.with(|slot| slot.borrow().clone());
        if let Some(drain) = drain {
            drain();
        }
    })
    .is_err()
    {
        PENDING.store(false, Ordering::Release);
    }
}
