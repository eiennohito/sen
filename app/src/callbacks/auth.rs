use crate::AuthState as G;
use crate::*;

pub(super) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_start_sign_in, |ui, d| {
        let g = ui.global::<G>();
        g.set_auth_phase(AuthPhase::Preparing);
        g.set_auth_error("".into());
        g.set_auth_qr(slint::Image::default());
        g.set_auth_pin("".into());
        g.set_auth_stage(text::PREPARING_SIGN_IN.into());
        d.sender.send(Intent::SignIn);
    });
    global_callback!(ui, actions, G, on_cancel_sign_in, |ui, d| {
        let g = ui.global::<G>();
        g.set_auth_phase(AuthPhase::Cancelling);
        d.sender.cancel_sign_in();
        g.set_auth_stage(text::CANCELLING_SIGN_IN.into());
    });
}
