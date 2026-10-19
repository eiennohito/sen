use crate::*;

pub(crate) fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.global::<State>()
        .on_avatar_tint(|id| presenter::format::avatar_tint(&id));
    ui.global::<State>()
        .on_name_tint(|id, background, dark| presenter::format::name_tint(&id, background, dark));
    let mut avatars: std::collections::VecDeque<(slint::Image, u32, slint::Image)> =
        std::collections::VecDeque::new();
    ui.global::<State>()
        .on_avatar_picture(move |source, width, scale| {
            if source.size().width == 0 {
                return source;
            }
            let edge = (width * scale).round().max(1.) as u32;
            if let Some((_, _, picture)) = avatars
                .iter()
                .find(|(old, size, _)| *old == source && *size == edge)
            {
                return picture.clone();
            }
            let Some(pixels) = source.to_rgba8() else {
                return source;
            };
            let pixels = crate::assets::circular_avatar(
                &line::client::desktop::assets::Pixels {
                    width: pixels.width(),
                    height: pixels.height(),
                    rgba: pixels.as_bytes().to_vec(),
                },
                edge,
            );
            let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &pixels.rgba,
                pixels.width,
                pixels.height,
            ));
            if avatars.len() == 256 {
                avatars.pop_front();
            }
            avatars.push_back((source, edge, picture.clone()));
            picture
        });
    ui.global::<State>()
        .on_has_text(|text| !text.trim().is_empty());
    focus::install(&ui);
    #[cfg(feature = "demo")]
    dev::install_window(&ui);
    macro_rules! bridge {
        ($on:ident, $invoke:ident $(, $arg:ident)*) => {{
            let weak = ui.as_weak();
            ui.global::<State>().$on(move |$($arg),*| {
                if let Some(ui) = weak.upgrade() { ui.$invoke($($arg),*); }
            });
        }};
    }
    bridge!(on_open_members, invoke_open_members);
    bridge!(on_filter_members, invoke_filter_members);
    bridge!(on_back_to_members, invoke_back_to_members);
    bridge!(on_open_member, invoke_open_member, id);
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    bridge!(on_close_settings, invoke_close_settings);
    bridge!(on_quit, invoke_quit);
    bridge!(on_restore_editor, invoke_restore_editor, anchor, caret);
    bridge!(on_retry_message, invoke_retry_message, id);
    bridge!(on_resolve_recovery, invoke_resolve_recovery, action);

    bridge!(on_filter_category, invoke_filter_category);
    bridge!(on_switch_chat, invoke_switch_chat, delta);
    bridge!(on_close_profile, invoke_close_profile);
    bridge!(on_show_viewer, invoke_show_viewer, key);
    bridge!(on_close_viewer, invoke_close_viewer);
    bridge!(on_account_ready, invoke_account_ready);
    bridge!(on_close_sign_in, invoke_close_sign_in);
    bridge!(on_open_sign_in, invoke_open_sign_in);
    bridge!(on_show_history, invoke_show_history, y, latest);
    bridge!(on_latest, invoke_latest);
    bridge!(on_reset_viewer, invoke_reset_viewer);
    bridge!(on_history_scroll, invoke_history_scroll, delta);
    bridge!(on_history_jump, invoke_history_jump, y);
    bridge!(on_recover_message, invoke_recover_message, arg0);
    bridge!(on_settings_changed, invoke_settings_changed);
    bridge!(on_send, invoke_send);
    bridge!(on_more, invoke_more);
    bridge!(on_retry_image, invoke_retry_image, key);
    bridge!(on_draft_edited, invoke_draft_edited);
    bridge!(on_search_changed, invoke_search_changed, arg0);
    bridge!(on_select_chat, invoke_select_chat, arg0);
    bridge!(on_cancel_sign_in, invoke_cancel_sign_in);
    bridge!(on_start_sign_in, invoke_start_sign_in);
    bridge!(on_photo_closed, invoke_photo_closed);
    bridge!(on_open_photo, invoke_open_photo, arg0);
    bridge!(on_start_friend_chat, invoke_start_friend_chat);
    bridge!(on_open_profile, invoke_open_profile, arg0);
    bridge!(on_filter_friends, invoke_filter_friends);
    bridge!(on_refresh_friends, invoke_refresh_friends);
    bridge!(on_browse, invoke_browse, arg0);
    Ok(ui)
}
