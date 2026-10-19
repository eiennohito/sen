use crate::*;
impl crate::Desktop {
    pub(crate) fn reading_geometry(ui: &AppWindow) -> reading::Geometry {
        let global = ui.global::<State>();
        reading::Geometry {
            y: ui.get_history_y(),
            at_bottom: global.get_following()
                && global.get_history_extent() + ui.get_history_y()
                    <= global.get_text_size() as f32,
            visible_top: global.get_history_visible_top(),
            height: global.get_history_viewport_height(),
            pill_height: global.get_history_pill_height(),
            active: global.get_window_active(),
            modal: global.get_modal_open(),
            dragging: ui.get_history_dragging(),
            interacting: global.get_history_interacting(),
            narrow: global.get_narrow(),
            conversation: ui.get_show_conversation(),
            older_start_known: global.get_older_start_known(),
        }
    }
    pub(crate) fn observe_reading(&mut self, ui: &AppWindow) {
        let effects = reading::observe(&mut self.history, &self.state, Self::reading_geometry(ui));
        for intent in effects.intents {
            self.sender.send(intent);
        }
        if let Some((y, latest)) = effects.show {
            ui.invoke_show_history(y, latest);
        }
        for intent in reading::exposure(&mut self.history, &self.state, Self::reading_geometry(ui))
        {
            self.sender.send(intent);
        }
        if let Some(y) = reading::restore(&mut self.history) {
            ui.invoke_history_jump(y);
        }
    }
    pub(crate) fn jump_to_message(&mut self, ui: &AppWindow, key: Key) {
        let origin =
            reading::return_anchor(&self.history, self.state.rows(), Self::reading_geometry(ui));
        self.history.return_anchor = origin;
        self.history.opening_history = false;
        self.history
            .jump_to(key, ui.global::<State>().get_history_visible_top());
        ui.set_following(false);
        ui.global::<State>().set_return_visible(origin.is_some());
        ui.global::<State>()
            .set_highlight_message(key.0.to_string().into());
        let weak = ui.as_weak();
        self.history.highlight_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(2),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<State>().set_highlight_message("".into());
                }
            },
        );
        let intent = self.state.subscribe_window(Anchor::Key(key), 80, 80);
        self.sender.send(intent);
    }
}
