use super::*;
pub(super) fn install_reset(
    ui: &AppWindow,
    actions: &Runtime,
    fake: Rc<RefCell<FakeBackend>>,
    workers: Rc<RefCell<Worker>>,
    text_size: i32,
) {
    let weak = ui.as_weak();
    let fake = fake.clone();
    let actions = actions.clone();
    let reset_worker = workers;
    ui.global::<State>().set_demo_reset_enabled(true);
    ui.global::<State>()
        .set_demo_instance(std::env::var("LINE_UI_INSTANCE").unwrap_or_default().into());
    ui.global::<State>().on_demo_automation(move |hook| {
        if hook != line_desktop_ui::DemoAutomation::Reset {
            return;
        }
        let weak = weak.clone();
        let fake = fake.clone();
        let reset_worker = reset_worker.clone();
        actions.push(move |d| {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_focus_search();
                ui.global::<State>().invoke_forget_detail_focus();
                ui.global::<State>().invoke_cancel_attachments();
                let fresh = FakeBackend::canned();
                demo::configure(&fresh, "", None);
                let worker = demo::worker(fresh.clone(), text_size);
                worker.on_deltas(event_loop::wake);
                d.sender = worker.sender.clone();
                reset_worker.replace(worker);
                *fake.borrow_mut() = fresh;
                d.state = AppState::default();
                d.history = reading::Reading {
                    opening_history: true,
                    ..Default::default()
                };
                d.assets = assets::Assets::default();
                d.presenter = presenter::Presenter::default();
                d.shown_composer.clear();
                d.query.clear();
                d.category = ChatCategory::All;
                d.reveal = None;
                d.viewer_asset = None;
                d.viewer_source = None;
                // Preserve geometry/derived bindings and instance identity; reset
                // writable presentation state from the same defaults as a fresh UI.
                let defaults = AppWindow::new().expect("create demo reset defaults");
                let state = ui.global::<State>();
                let fresh_state = defaults.global::<State>();
                state.set_composer_reply(fresh_state.get_composer_reply());
                state.set_attachment_injection_paths(fresh_state.get_attachment_injection_paths());
                state.set_attachments_open(fresh_state.get_attachments_open());
                state.set_attachments_chat(fresh_state.get_attachments_chat());
                state.set_attachments_error(fresh_state.get_attachments_error());
                state.set_attachments_count(fresh_state.get_attachments_count());
                state.set_attachment_items(fresh_state.get_attachment_items());
                state.set_paste_text_serial(fresh_state.get_paste_text_serial());
                state.set_history_virtual(fresh_state.get_history_virtual());
                state.set_virtual_history_height(fresh_state.get_virtual_history_height());
                state.set_plain_history_height(fresh_state.get_plain_history_height());
                state.set_account_initial(fresh_state.get_account_initial());
                state.set_chat_id(fresh_state.get_chat_id());
                state.set_chat_initial(fresh_state.get_chat_initial());
                state.set_chat_avatar(fresh_state.get_chat_avatar());
                state.set_panel(fresh_state.get_panel());
                state.set_unread_conversations(fresh_state.get_unread_conversations());
                state.set_unread_stale(fresh_state.get_unread_stale());
                state.set_unread_divider(fresh_state.get_unread_divider());
                state.set_unread_locating(fresh_state.get_unread_locating());
                state.set_unread_boundary_unknown(fresh_state.get_unread_boundary_unknown());
                state.set_section(fresh_state.get_section());
                state.set_category(fresh_state.get_category());
                state.set_reaction_error_id(fresh_state.get_reaction_error_id());
                state.set_sticker_tabs(fresh_state.get_sticker_tabs());
                state.set_sticker_items(fresh_state.get_sticker_items());
                state.set_sticker_package(fresh_state.get_sticker_package());
                state.set_new_chat_open(fresh_state.get_new_chat_open());
                state.set_new_chat_busy(fresh_state.get_new_chat_busy());
                state.set_new_chat_uncertain(fresh_state.get_new_chat_uncertain());
                state.set_new_chat_error(fresh_state.get_new_chat_error());
                state.set_new_chat_query(fresh_state.get_new_chat_query());
                state.set_new_chat_name(fresh_state.get_new_chat_name());
                state.set_new_chat_placeholder(fresh_state.get_new_chat_placeholder());
                state.set_new_chat_count(fresh_state.get_new_chat_count());
                state.set_new_chat_friends(fresh_state.get_new_chat_friends());
                state.set_new_chat_selected(fresh_state.get_new_chat_selected());
                state.set_friends(fresh_state.get_friends());
                state.set_friend_result(fresh_state.get_friend_result());
                state.set_friend_row_height(fresh_state.get_friend_row_height());
                state.set_friend_query(fresh_state.get_friend_query());
                state.set_groups_expanded(fresh_state.get_groups_expanded());
                state.set_friends_expanded(fresh_state.get_friends_expanded());
                state.set_friends_y(fresh_state.get_friends_y());
                state.set_member_result(fresh_state.get_member_result());
                state.set_detail_depth(fresh_state.get_detail_depth());
                state.set_detail_page(fresh_state.get_detail_page());
                state.set_detail_busy(fresh_state.get_detail_busy());
                state.set_detail_running(fresh_state.get_detail_running());
                state.set_viewer_from_detail(fresh_state.get_viewer_from_detail());
                state.set_detail_error(fresh_state.get_detail_error());
                state.set_conversation_error(fresh_state.get_conversation_error());
                state.set_friends_error(fresh_state.get_friends_error());
                state.set_chat_list_error(fresh_state.get_chat_list_error());
                state.set_profile_friend(fresh_state.get_profile_friend());
                state.set_profile_addable(fresh_state.get_profile_addable());
                state.set_profile_own(fresh_state.get_profile_own());
                state.set_profile_favourite(fresh_state.get_profile_favourite());
                state.set_profile_original(fresh_state.get_profile_original());
                state.set_profile_notice(fresh_state.get_profile_notice());
                state.set_profile_name_runs(fresh_state.get_profile_name_runs());
                state.set_profile_status_runs(fresh_state.get_profile_status_runs());
                state.set_group_room(fresh_state.get_group_room());
                state.set_group_muted(fresh_state.get_group_muted());
                state.set_group_left(fresh_state.get_group_left());
                state.set_group_writable(fresh_state.get_group_writable());
                state.set_group_count(fresh_state.get_group_count());
                state.set_invitee_count(fresh_state.get_invitee_count());
                state.set_chat_left(fresh_state.get_chat_left());
                state.set_detail_dialog(fresh_state.get_detail_dialog());
                state.set_members(fresh_state.get_members());
                state.set_member_query(fresh_state.get_member_query());
                state.set_chat_group(fresh_state.get_chat_group());
                state.set_member_count(fresh_state.get_member_count());
                state.set_browser_focus_hidden(fresh_state.get_browser_focus_hidden());
                state.set_profile_initial(fresh_state.get_profile_initial());
                state.set_profile_name(fresh_state.get_profile_name());
                state.set_profile_id(fresh_state.get_profile_id());
                state.set_profile_status(fresh_state.get_profile_status());
                state.set_profile_group(fresh_state.get_profile_group());
                state.set_profile_avatar(fresh_state.get_profile_avatar());
                state.set_profile_available(fresh_state.get_profile_available());
                state.set_ime_active(fresh_state.get_ime_active());
                state.set_text_size(fresh_state.get_text_size());
                state.set_chats(fresh_state.get_chats());
                state.set_messages(fresh_state.get_messages());
                state.set_selected_chat(fresh_state.get_selected_chat());
                state.set_chat_title(fresh_state.get_chat_title());
                state.set_chat_subtitle(fresh_state.get_chat_subtitle());
                state.set_draft(fresh_state.get_draft());
                state.set_enter_sends(fresh_state.get_enter_sends());
                state.set_editor_caret(fresh_state.get_editor_caret());
                state.set_editor_anchor(fresh_state.get_editor_anchor());
                state.set_composer_focused(fresh_state.get_composer_focused());
                state.set_unsend_feedback_id(fresh_state.get_unsend_feedback_id());
                state.set_recovery_kind(fresh_state.get_recovery_kind());
                state.set_recovery_id(fresh_state.get_recovery_id());
                state.set_recovery_focus_id(fresh_state.get_recovery_focus_id());
                state.set_recovery_focus_action(fresh_state.get_recovery_focus_action());
                state.set_send_pending(fresh_state.get_send_pending());
                state.set_send_reason(fresh_state.get_send_reason());
                state.set_list_pressed(fresh_state.get_list_pressed());
                state.set_list_release_pending(fresh_state.get_list_release_pending());
                state.set_background_mode(fresh_state.get_background_mode());
                state.set_tray_available(fresh_state.get_tray_available());
                state.set_autostart(fresh_state.get_autostart());
                state.set_background_status(fresh_state.get_background_status());
                state.set_autostart_status(fresh_state.get_autostart_status());
                state.set_notifications_paused(fresh_state.get_notifications_paused());
                state.set_notifications_enabled(fresh_state.get_notifications_enabled());
                state.set_notification_preview(fresh_state.get_notification_preview());
                state.set_notification_sound(fresh_state.get_notification_sound());
                state.set_notification_mention_exception(
                    fresh_state.get_notification_mention_exception(),
                );
                state.set_notification_group_events(fresh_state.get_notification_group_events());
                state.set_notification_available(fresh_state.get_notification_available());
                state.set_notification_status(fresh_state.get_notification_status());
                state.set_transient_notice(fresh_state.get_transient_notice());
                state.set_viewer_image(fresh_state.get_viewer_image());
                state.set_viewer_status(fresh_state.get_viewer_status());
                state.set_viewer_name(fresh_state.get_viewer_name());
                state.set_viewer_error(fresh_state.get_viewer_error());
                state.set_viewer_retryable(fresh_state.get_viewer_retryable());
                state.set_viewer_loading(fresh_state.get_viewer_loading());
                state.set_viewer_key(fresh_state.get_viewer_key());
                state.set_viewer_zoom(fresh_state.get_viewer_zoom());
                state.set_viewer_has_previous(fresh_state.get_viewer_has_previous());
                state.set_viewer_has_next(fresh_state.get_viewer_has_next());
                state.set_viewer_save_status(fresh_state.get_viewer_save_status());
                state.set_viewer_saving(fresh_state.get_viewer_saving());
                state.set_viewer_return_key(fresh_state.get_viewer_return_key());
                state.set_viewer_return_serial(fresh_state.get_viewer_return_serial());
                state.set_connection(fresh_state.get_connection());
                state.set_auth_cancel_confirm(fresh_state.get_auth_cancel_confirm());
                state.set_auth_cancelling(fresh_state.get_auth_cancelling());
                state.set_auth_qr(fresh_state.get_auth_qr());
                state.set_auth_pin(fresh_state.get_auth_pin());
                state.set_auth_stage(fresh_state.get_auth_stage());
                state.set_auth_error(fresh_state.get_auth_error());
                state.set_session_cause(fresh_state.get_session_cause());
                state.set_auth_prerequisite_missing(fresh_state.get_auth_prerequisite_missing());
                state.set_title_override(fresh_state.get_title_override());
                state.set_link_feedback(fresh_state.get_link_feedback());
                state.set_demo_scenarios_open(fresh_state.get_demo_scenarios_open());
                state.set_more_chats(fresh_state.get_more_chats());
                state.set_has_chat(fresh_state.get_has_chat());
                state.set_storage_error(fresh_state.get_storage_error());
                state.set_desktop_warning(fresh_state.get_desktop_warning());
                state.set_connection_status(fresh_state.get_connection_status());
                state.set_has_account(fresh_state.get_has_account());
                state.set_account_mid(fresh_state.get_account_mid());
                state.set_account_name(fresh_state.get_account_name());
                state.set_show_conversation(fresh_state.get_show_conversation());
                state.set_following(fresh_state.get_following());
                state.set_new_count(fresh_state.get_new_count());
                state.set_history_y(fresh_state.get_history_y());
                state.set_history_dragging(fresh_state.get_history_dragging());
                state.set_query(fresh_state.get_query());
                state.set_list_y(fresh_state.get_list_y());
                state.set_editor_y(fresh_state.get_editor_y());
                state.set_search_list_y(fresh_state.get_search_list_y());
                state.set_searching(fresh_state.get_searching());
                state.set_result_focus(fresh_state.get_result_focus());
                state.set_history_extent(fresh_state.get_history_extent());
                state.set_file_save_id(fresh_state.get_file_save_id());
                state.set_file_save_status(fresh_state.get_file_save_status());
                state.set_file_saving(fresh_state.get_file_saving());
                state.set_last_opened_link(fresh_state.get_last_opened_link());
                state.set_history_cover_inset(fresh_state.get_history_cover_inset());
                state.set_older_start_known(fresh_state.get_older_start_known());
                state.set_older_status(fresh_state.get_older_status());
                state.set_highlight_message(fresh_state.get_highlight_message());
                state.set_return_visible(fresh_state.get_return_visible());
                state.set_history_needs_refresh(fresh_state.get_history_needs_refresh());
                state.set_history_budget_warning(fresh_state.get_history_budget_warning());
                state.set_history_can_follow(fresh_state.get_history_can_follow());
                state.set_history_interacting(fresh_state.get_history_interacting());
                d.presenter.install(&ui);
                ui.set_text_size(text_size);
                ui.invoke_focus_search();
                d.render(&ui);
                state.set_demo_reset_generation(state.get_demo_reset_generation() + 1);
            }
        });
    });
}
pub(super) fn launch(background: bool, demo: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !background {
        return Ok(());
    }
    if !demo {
        return Err("--debug-background requires a ui-debug build and an offline demo.".into());
    }
    if std::env::var("SLINT_BACKEND").as_deref() != Ok("headless-software") {
        return Err("headless automation requires the private launcher".into());
    }
    // The selector cannot select Slint 1.18's headless backend.
    slint::platform::set_platform(i_slint_backend_selector::create_backend()?)?;
    i_slint_backend_selector::init_testing_backends();
    Ok(())
}
pub(crate) fn internal_error(ui: &AppWindow, kind: &str) {
    ui.set_internal_error_seen(true);
    ui.set_internal_error(format!("Internal error: {kind} (see log)").into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_internal_error("".into());
        }
    });
}

pub(crate) fn inject_attachments(
    ui: &AppWindow,
    actions: &Runtime,
    selection: std::sync::Arc<std::sync::Mutex<crate::callbacks::attachments::Selection>>,
) {
    let weak = ui.as_weak();
    let actions = actions.clone();
    ui.global::<State>().on_inject_attachments(move |paths| {
        let weak = weak.clone();
        let selected = selection.clone();
        actions.push(move |d| {
            if let Some(ui) = weak.upgrade()
                && ui.global::<State>().get_automation_background()
            {
                let paths: Vec<std::path::PathBuf> =
                    serde_json::from_str(&paths).unwrap_or_default();
                crate::callbacks::attachments::begin(&ui, d, &selected, move || Ok(paths));
            }
        });
    });
}
