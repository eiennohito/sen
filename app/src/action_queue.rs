/// UI callbacks record actions, including callbacks invoked while an action is
/// being presented. The sole drain releases the queue borrow before dispatch.
pub struct ActionQueue<T>(std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<Action<T>>>>);
type Action<T> = Box<dyn FnOnce(&mut T)>;
impl<T> Default for ActionQueue<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}
impl<T> Clone for ActionQueue<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> ActionQueue<T> {
    pub fn push(&self, action: impl FnOnce(&mut T) + 'static) {
        self.0.borrow_mut().push_back(Box::new(action));
    }
    pub fn drain(&self, target: &mut T) {
        loop {
            let action = self.0.borrow_mut().pop_front();
            match action {
                Some(action) => action(target),
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reentrant_panel_requests_are_queued_without_borrowing_the_controller() {
        use super::ActionQueue;
        use sen::controller::{AppState, Panel};
        let mut state = AppState::default();
        let actions = ActionQueue::<AppState>::default();
        let callbacks = actions.clone();
        actions.push(move |state| {
            state.panel = Panel::Settings;
            // A presenter-triggered callback only records another action.
            callbacks.push(|state| state.panel = Panel::Detail);
            assert_eq!(state.panel, Panel::Settings);
        });
        actions.drain(&mut state);
        assert_eq!(state.panel, Panel::Detail);
    }
}
