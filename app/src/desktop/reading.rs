use crate::ConversationState;
use crate::*;
impl crate::Desktop {
    pub(crate) fn observe_reading(&mut self, ui: &AppWindow) {
        let geometry = ui.global::<ConversationState>().get_reading();
        let effects = reading::observe(&mut self.history, &self.state, geometry);
        for intent in effects.intents {
            self.sender.send(intent);
        }
        if let Some((y, latest)) = effects.show {
            ui.global::<ConversationState>()
                .invoke_show_history(y, latest);
        }
        let geometry = ui.global::<ConversationState>().get_reading();
        for intent in reading::exposure(&mut self.history, &self.state, geometry) {
            self.sender.send(intent);
        }
        if let Some(y) = reading::restore(&mut self.history) {
            ui.invoke_history_jump(y);
        }
    }
    pub(crate) fn jump_to_message(&mut self, ui: &AppWindow, key: Key) {
        let origin = reading::return_anchor(
            &self.history,
            self.state.rows(),
            ui.global::<ConversationState>().get_reading(),
        );
        self.history.return_anchor = origin;
        self.history.opening_history = false;
        self.history.jump_to(
            key,
            ui.global::<ConversationState>().get_reading().visible_top,
        );
        ui.global::<ConversationState>().set_following(false);
        ui.global::<ConversationState>()
            .set_return_visible(origin.is_some());
        ui.global::<ConversationState>()
            .set_highlight_message(key.0.to_string().into());
        let weak = ui.as_weak();
        self.history.highlight_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(2),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<ConversationState>()
                        .set_highlight_message("".into());
                }
            },
        );
        let intent = self.state.subscribe_window(Anchor::Key(key), 80, 80);
        self.sender.send(intent);
    }
}
