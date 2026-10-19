use crate::*;

pub(crate) fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.global::<State>().on_avatar_tint(|id| avatar_tint(&id));
    ui.global::<State>().on_name_tint(|id, background, dark| {
        let tint = avatar_tint(&id);
        if dark {
            return tint;
        }
        let luminance = |color: slint::Color| {
            [color.red(), color.green(), color.blue()]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(c, weight)| {
                    let c = f64::from(c) / 255.;
                    weight
                        * if c <= 0.04045 {
                            c / 12.92
                        } else {
                            ((c + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum::<f64>()
        };
        let mut factor = 1.;
        loop {
            let color = slint::Color::from_rgb_u8(
                (f64::from(tint.red()) * factor) as u8,
                (f64::from(tint.green()) * factor) as u8,
                (f64::from(tint.blue()) * factor) as u8,
            );
            if (luminance(background) + 0.05) / (luminance(color) + 0.05) >= 4.5 {
                return color;
            }
            factor *= 0.95;
        }
    });
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
    #[cfg(feature = "demo")]
    {
        use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};
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
                } else if ui.get_detail_open() {
                    ui.invoke_focus_profile_close();
                } else {
                    ui.invoke_focus_composer();
                }
            }
        });
    }
    #[cfg(feature = "demo")]
    {
        use slint::private_unstable_api::re_exports::{FocusReason, WindowInner};
        let saved = Rc::new(RefCell::new(None));
        let record = saved.clone();
        let weak = ui.as_weak();
        ui.global::<State>().on_remember_demo_focus(move || {
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
        ui.global::<State>().on_restore_demo_focus(move || {
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
    bridge!(on_focus_cycle_end, invoke_focus_cycle_end);
    bridge!(on_focus_after_browser, invoke_focus_after_browser);
    bridge!(on_focus_after_conversation, invoke_focus_after_conversation);
    bridge!(on_focus_before_layer, invoke_focus_before_layer);
    bridge!(on_focus_after_layer, invoke_focus_after_layer);
    bridge!(on_close_settings, invoke_close_settings);
    bridge!(on_focus_friend_list, invoke_focus_friend_list);
    bridge!(on_quit, invoke_quit);
    bridge!(on_restore_editor, invoke_restore_editor, anchor, caret);
    bridge!(on_retry_message, invoke_retry_message, id);
    bridge!(on_resolve_recovery, invoke_resolve_recovery, action);

    bridge!(
        on_focus_reading_before_editor,
        invoke_focus_reading_before_editor
    );
    bridge!(on_filter_category, invoke_filter_category);
    bridge!(on_switch_chat, invoke_switch_chat, delta);
    bridge!(on_focus_chat_list, invoke_focus_chat_list);
    bridge!(on_show_profile, invoke_focus_profile_close);
    bridge!(on_close_profile, invoke_close_profile);
    bridge!(on_focus_composer, invoke_focus_composer);
    bridge!(on_show_viewer, invoke_show_viewer, key);
    bridge!(on_close_viewer, invoke_close_viewer);
    bridge!(on_account_ready, invoke_account_ready);
    bridge!(on_close_sign_in, invoke_close_sign_in);
    bridge!(on_open_sign_in, invoke_open_sign_in);
    bridge!(on_show_history, invoke_show_history, y, latest);
    bridge!(on_latest, invoke_latest);
    bridge!(on_focus_search, invoke_focus_search);
    bridge!(on_focus_friend_search, invoke_focus_friend_search);
    bridge!(on_focus_settings_opener, invoke_focus_settings_opener);
    bridge!(on_focus_settings_close, invoke_focus_settings_close);
    bridge!(on_focus_profile_close, invoke_focus_profile_close);
    bridge!(on_focus_auth_start, invoke_focus_auth_start);
    bridge!(on_focus_viewer_close, invoke_focus_viewer_close);
    bridge!(on_focus_history, invoke_focus_history);
    bridge!(on_focus_connection_first, invoke_focus_connection_first);
    bridge!(on_focus_sign_in_opener, invoke_focus_sign_in_opener);
    bridge!(on_reset_viewer, invoke_reset_viewer);
    bridge!(on_history_scroll, invoke_history_scroll, delta);
    bridge!(on_history_jump, invoke_history_jump, y);
    #[cfg(feature = "demo")]
    bridge!(on_demo_event, invoke_demo_event, arg0);
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

fn avatar_tint(id: &str) -> slint::Color {
    let hash = id.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let hues = [
        0xDCCEDC, 0xC6D9E8, 0xC9DDD0, 0xE7D4BD, 0xD2D2EA, 0xE8CCD0, 0xDADDBE, 0xBEDDDA,
    ];
    let rgb = hues[hash as usize % hues.len()];
    slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
