use crate::*;

#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) window: std::rc::Weak<RefCell<Option<AppWindow>>>,
    pub(crate) desktop: Rc<RefCell<Option<Desktop>>>,
    pub(crate) actions: ActionQueue<Desktop>,
    pub(crate) draining: Rc<std::cell::Cell<bool>>,
}
impl Runtime {
    pub(crate) fn push(&self, action: impl FnOnce(&mut Desktop) + 'static) {
        self.actions.push(action);
        if self.draining.get() {
            return;
        }
        let available = self.desktop.borrow().is_some();
        if available {
            self.update(|_| {});
        }
    }
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut Desktop) -> R) -> R {
        self.draining.set(true);
        let mut desktop = self.desktop.borrow_mut().take().unwrap();
        let result = f(&mut desktop);
        self.actions.drain(&mut desktop);
        *self.desktop.borrow_mut() = Some(desktop);
        self.draining.set(false);
        result
    }
}
