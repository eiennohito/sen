use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::mpsc::SyncSender};
mod account;
mod attention;
mod attention_policy;
mod autostart;
mod background;
mod composer;
mod debug_launch;
mod demo;
mod desktop_integration;
mod links;
mod notifications;
mod snapshot;
mod storage;
mod tray;
mod unread;
mod viewer;
slint::include_modules!();
fn create_app_window() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    paging::install(&ui);
    unread::install(&ui);
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_settings(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_settings();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_friend_list(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_friend_list();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_save_photo(move || {
        if let Some(ui) = weak.upgrade() {
            viewer::save_photo(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_viewer_step(move |direction| {
        if let Some(ui) = weak.upgrade() {
            UI_STATE.with(|slot| {
                if let Some(state) = slot.borrow().as_ref() {
                    state.borrow_mut().step_photo(&ui, direction);
                }
            });
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_quit(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_quit();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>()
        .on_restore_editor(move |anchor, caret| {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_restore_editor(anchor, caret);
            }
        });
    let weak = ui.as_weak();
    ui.global::<State>().on_retry_message(move |id| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_retry_message(id);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_resolve_recovery(move |action| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_resolve_recovery(action);
        }
    });

    let weak = ui.as_weak();
    ui.global::<State>()
        .on_focus_reading_before_editor(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_focus_reading_before_editor();
            }
        });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_link(move |url| {
        if let Some(ui) = weak.upgrade() {
            if ui.get_demo() {
                ui.global::<State>().set_last_opened_link(url);
            } else {
                links::open(url.to_string());
            }
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_filter_category(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_filter_category();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_switch_chat(move |delta| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_switch_chat(delta);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_chat_list(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_chat_list();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_profile(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_show_profile();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_profile(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_profile();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_composer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_composer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_viewer(move |key| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_show_viewer(key);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_viewer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_viewer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_account_ready(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_account_ready();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_close_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_close_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_show_history(move |y, latest| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_show_history(y, latest);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_latest(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_latest();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_search(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_search();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_friend_search(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_friend_search();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_settings_opener(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_settings_opener();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_settings_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_settings_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_profile_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_profile_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_auth_start(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_auth_start();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_viewer_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_viewer_close();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_history(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_history();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_focus_sign_in_opener(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_focus_sign_in_opener();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_reset_viewer(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_reset_viewer();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_history_jump(move |y| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_history_jump(y);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_release_deferred(move || {
        // Focus/selection changes may be emitted inside a model mutation while
        // View is borrowed. Release queued updates on the next event turn.
        let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_release_deferred());
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_demo_event(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_demo_event(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_recover_message(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_recover_message(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_retry_storage(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_retry_storage();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_settings_changed();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_send(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_send();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_more(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_more();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_retry_history(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_retry_history();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_refresh();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_draft_edited(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_draft_edited();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_search_changed(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_search_changed(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_select_chat(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_select_chat(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_cancel_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_cancel_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_start_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_start_sign_in();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_reconnect(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_reconnect();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_photo_closed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_photo_closed();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_photo(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_photo(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_start_friend_chat(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_start_friend_chat();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_open_profile(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_open_profile(arg0);
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_filter_friends(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_filter_friends();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_refresh_friends(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_refresh_friends();
        }
    });
    let weak = ui.as_weak();
    ui.global::<State>().on_browse(move |arg0| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_browse(arg0);
        }
    });
    Ok(ui)
}

mod browser;
mod deferred;
mod history_window;
mod paging;
mod present;
mod rich_content;
mod view;
use present::*;
use view::View;

struct Options {
    paths: account::Paths,
    demo: bool,
    demo_rich: bool,
    demo_photos: bool,
    demo_groups: bool,
    demo_friends: bool,
    demo_notify: bool,
    demo_size: Option<(u32, u32)>,
    debug_background: bool,
    snapshot: Option<String>,
    snapshot_size: (u32, u32),
    snapshot_text_size: i32,
    snapshot_profile: bool,
    check: bool,
    explicit_paths: bool,
    locations: desktop_integration::Locations,
    configured_paths: bool,
}
fn options() -> Result<Options, Box<dyn std::error::Error>> {
    let locations = desktop_integration::Locations::from_environment()?;
    let mut out = Options {
        paths: account::Paths {
            state: locations.session.clone(),
            wasm: locations.wasm.clone(),
            cache: locations.cache.clone(),
        },
        explicit_paths: false,
        configured_paths: false,
        locations,
        demo: false,
        demo_rich: false,
        demo_photos: false,
        demo_groups: false,
        demo_friends: false,
        demo_notify: false,
        demo_size: None,
        debug_background: false,
        snapshot: None,
        snapshot_size: (980, 720),
        snapshot_text_size: 15,
        snapshot_profile: false,
        check: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"mcp_debug\":{},\"renderer\":\"software\"}}",
                    cfg!(feature = "ui-debug")
                );
                std::process::exit(0);
            }
            "--demo" => out.demo = true,
            "--debug-background" => out.debug_background = true,
            "--demo-notify" => {
                out.demo = true;
                out.demo_notify = true;
            }
            "--demo-rich" => {
                out.demo = true;
                out.demo_rich = true;
            }
            "--demo-friends" => {
                out.demo = true;
                out.demo_friends = true;
            }
            "--demo-photos" => {
                out.demo = true;
                out.demo_photos = true;
            }
            "--demo-groups" => {
                out.demo = true;
                out.demo_groups = true;
            }
            "--check-live" => out.check = true,
            "--state" => {
                out.paths.state = args.next().ok_or("missing state path")?.into();
                out.explicit_paths = true;
            }
            "--wasm" => {
                out.paths.wasm = args.next().ok_or("missing WASM path")?.into();
                out.explicit_paths = true;
            }
            "--cache-dir" => out.paths.cache = args.next().ok_or("missing cache path")?.into(),
            "--snapshot-size" | "--demo-size" => {
                let size = args.next().ok_or("missing WIDTHxHEIGHT")?;
                let (width, height) = size.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
                out.snapshot_size = (width.parse()?, height.parse()?);
                if !(360..=3840).contains(&out.snapshot_size.0)
                    || !(520..=2160).contains(&out.snapshot_size.1)
                {
                    return Err("size outside supported bounds".into());
                }
                if arg == "--demo-size" {
                    out.demo = true;
                    out.demo_size = Some(out.snapshot_size);
                }
            }
            "--snapshot-text-size" => {
                out.snapshot_text_size = args.next().ok_or("missing text size")?.parse()?;
                if !(12..=24).contains(&out.snapshot_text_size) {
                    return Err("text size must be 12–24".into());
                }
            }
            "--snapshot-profile" => {
                out.snapshot_profile = true;
                out.demo = true;
            }
            "--snapshot" => {
                out.snapshot = Some(args.next().ok_or("missing snapshot path")?);
                out.demo = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: line-desktop [--demo | --demo-photos | --demo-groups | --demo-friends] [--state FILE] [--wasm FILE] [--cache-dir DIR]\n       line-desktop --snapshot FILE.ppm [--snapshot-size WIDTHxHEIGHT] [--snapshot-text-size 12..24] [--snapshot-profile]\n       line-desktop --check-live [--state FILE] [--wasm FILE] [--cache-dir DIR]\n--demo-size WIDTHxHEIGHT sets an offline preview window size.\nDefault: use saved account. --demo and --snapshot are offline. --check-live reports counts only and never sends messages."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if !out.explicit_paths && !out.demo {
        out.configured_paths = out.locations.load_existing()?;
        out.paths.state = out.locations.session.clone();
        out.paths.wasm = out.locations.wasm.clone();
    }
    if out.check && out.demo {
        return Err("--check-live cannot be combined with offline modes".into());
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    let automation = debug_launch::intent(
        options.debug_background,
        cfg!(feature = "ui-debug"),
        options.demo,
        std::env::var("LINE_DESKTOP_AUTOMATION_SOCKET")
            .ok()
            .as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )? == debug_launch::Intent::VirtualAutomation;
    let instance = if options.snapshot.is_none() {
        match desktop_integration::Instance::acquire_with_activation(!automation)? {
            desktop_integration::Launch::Forwarded => return Ok(()),
            desktop_integration::Launch::Primary(instance) => Some(instance),
        }
    } else {
        None
    };
    if options.check {
        return account::check(&options.paths);
    }
    let snapshot = if options.snapshot.is_some() {
        Some(snapshot::install()?)
    } else {
        None
    };
    if snapshot.is_none() {
        let mut backend = slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into());
        if automation {
            backend = backend
                .with_winit_window_attributes_hook(|attributes| attributes.with_active(false));
        }
        backend.select()?;
    }
    slint::set_xdg_app_id("org.lineclient.Desktop")?;
    let ui = create_app_window()?;
    ui.global::<State>().set_automation_background(automation);
    background::install(&ui);
    if let Some(warning) = instance.as_ref().and_then(|i| i.warning.as_ref()) {
        ui.set_desktop_warning(warning.into());
    }
    let legacy_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing repository root")?
        .to_path_buf();
    let migration = !options.demo
        && !options.explicit_paths
        && !options.configured_paths
        && desktop_integration::Locations::legacy_available(&legacy_root);
    ui.set_migration_needed(migration);

    if snapshot.is_none() {
        use slint::winit_030::{
            EventResult, WinitWindowAccessor,
            winit::event::{ElementState, Ime, MouseButton, WindowEvent},
        };
        let weak = ui.as_weak();
        // Observe IME state without intercepting native composition or editing.
        // TextEdit does not expose its internal preedit state to Slint callers.
        let mut focused = false;
        let mut occluded = false;
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                match event {
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Back,
                        ..
                    } if !ui.get_ime_active() => ui.invoke_go_back(),
                    WindowEvent::Ime(Ime::Preedit(text, _)) => ui.set_ime_active(!text.is_empty()),
                    WindowEvent::Ime(Ime::Commit(_) | Ime::Disabled) => ui.set_ime_active(false),
                    WindowEvent::Focused(active) => {
                        focused = *active;
                        if !focused {
                            unread::revoke_now();
                            ui.set_ime_active(false);
                        }
                        ui.global::<State>().set_window_active(focused && !occluded);
                    }
                    WindowEvent::Occluded(hidden) => {
                        occluded = *hidden;
                        if occluded {
                            unread::revoke_now();
                        }
                        ui.global::<State>().set_window_active(focused && !occluded);
                    }
                    _ => {}
                }
            }
            EventResult::Propagate
        });
    }
    let view = View {
        demo: options.demo,
        attention: if options.snapshot.is_none() {
            Some(attention::Attention::start())
        } else {
            None
        },
        ..Default::default()
    };
    let state = Rc::new(RefCell::new(view));
    notifications::install(&ui, state.clone());
    ui.global::<State>()
        .set_demo_notifications(options.demo_notify);
    if options.demo_notify {
        ui.global::<State>().set_notifications_enabled(true);
    }
    rich_content::install(&ui, state.clone());
    UI_STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
    let worker = Rc::new(RefCell::new(None));
    if options.demo {
        ui.set_demo(true);
        ui.set_account_name("Offline preview".into());
        for (index, chat) in demo::examples().into_iter().enumerate() {
            let mut view = state.borrow_mut();
            view.ids.push(index.to_string());
            view.rows.push(ChatRow {
                index: index as i32,
                group: index == 1,
                outgoing_preview: chat.messages.last().is_some_and(|m| m.outgoing),
                time: chat
                    .messages
                    .last()
                    .map(|m| list_time(m.sort_key.parse().unwrap_or(0)))
                    .unwrap_or_default()
                    .into(),
                title: chat.title.into(),
                preview: single_line(chat.preview).into(),
                activity: order_key(3 - index as i64).into(),
                initial: chat.initial.into(),
                avatar: demo::avatar(index),
                ..Default::default()
            });
        }
        {
            let mut view = state.borrow_mut();
            view.friends = view
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| FriendRow {
                    id: i.to_string().into(),
                    name: row.title.clone(),
                    status: "こんにちは 🌿\nSynthetic profile".into(),
                    group: i == 1,
                    avatar: row.avatar.clone(),
                    ..Default::default()
                })
                .collect();
            view.friends.push(FriendRow {
                id: "new-friend".into(),
                name: "新しい友だち 🌸".into(),
                status: "まだ会話のない友だち".into(),
                ..Default::default()
            });
            view.friends_loaded = true;
            view.filter_friends(&ui);
        }
        state.borrow().filter(&ui);
        state.borrow_mut().select(&ui, 0);
    } else if !migration {
        *worker.borrow_mut() = Some(start_saved_account(&ui, &state, options.paths.clone()));
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let worker = worker.clone();
        let paths = RefCell::new(options.locations.clone());
        let cache = options.paths.cache.clone();
        ui.on_migration_choice(move |existing| {
            let Some(ui) = weak.upgrade() else { return; };
            if !ui.get_migration_needed() { return; }
            let mut paths = paths.borrow_mut();
            match paths.choose(&legacy_root, existing) {
                Ok(()) => {
                    ui.set_migration_needed(false);
                    let account_paths = account::Paths { state: paths.session.clone(), wasm: paths.wasm.clone(), cache: cache.clone() };
                    *worker.borrow_mut() = Some(start_saved_account(&ui, &state, account_paths));
                }
                Err(_) => ui.set_migration_error("Could not save the folder choice. Check local folder permissions and try again.".into()),
            }
        });
    }
    if let Some(instance) = instance.as_ref() {
        let events = instance.take_events();
        let weak = ui.as_weak();
        std::thread::Builder::new().name("desktop-activation".into()).spawn(move || {
            while let Ok(event) = events.recv() {
                // Fold a burst to one raise, keeping the newest chat action.
                let event = events.try_iter().fold(event, |previous, next| match next {
                    desktop_integration::Activation::OpenChat(_) => next,
                    desktop_integration::Activation::Show => previous,
                });
                let (completed, completion) = std::sync::mpsc::sync_channel(1);
                if weak.upgrade_in_event_loop(move |ui| {
                    let _ = ui.show();
                    use slint::winit_030::WinitWindowAccessor;
                    if !ui.global::<State>().get_automation_background() { ui.window().with_winit_window(|window| {
                        window.set_minimized(false);
                        window.focus_window();
                        window.request_user_attention(Some(slint::winit_030::winit::window::UserAttentionType::Informational));
                    }); }
                    if let desktop_integration::Activation::OpenChat(id) = event {
                        UI_STATE.with(|slot| {
                            if let Some(state) = slot.borrow().as_ref() {
                                let index = state.borrow().ids.iter().position(|candidate| candidate == &id);
                                if let Some(index) = index { state.borrow_mut().select(&ui, index); ui.set_show_conversation(true); }
                            }
                        });
                    }
                    let _ = completed.send(());
                }).is_err() { break; }
                // Keep at most one callback queued on the UI thread.
                if completion.recv().is_err() { break; }
            }
        })?;
    }
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_photo_closed(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().viewer_request = 0;
            ui.set_viewer_image(Default::default());
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_photo(move |key| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_viewer_photo(&ui, &key);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_reconnect(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().reconnect(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_start_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().begin_sign_in(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_cancel_sign_in(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().cancel_sign_in(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_browse(move |section| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().browse(&ui, section);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_refresh_friends(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().load_friends(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_filter_friends(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().filter_friends(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_open_profile(move |id| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().open_profile(&ui, &id);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_start_friend_chat(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().start_friend_chat(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_select_chat(move |index| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().select(&ui, index as usize);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_filter_category(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().filter(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_switch_chat(move |delta| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().switch_visible_chat(&ui, delta);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_search_changed(move |query| {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.query = query.to_lowercase();
            view.filter(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_draft_edited(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().persist(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().persist(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.generation += 1;
            ui.set_list_error("".into());
            ui.set_loading_chats(view.request(&ui, account::Command::Chats { next: false }));
            view.load(&ui, true);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_retry_history(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow().load(&ui, true);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_retry_storage(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.capture(&ui);
            if view
                .storage
                .as_ref()
                .is_some_and(|storage| storage.flush(view.saved.clone()))
            {
                ui.set_storage_error("".into());
            }
            view.send_availability(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_more(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_loading_chats(
                view.borrow()
                    .request(&ui, account::Command::Chats { next: true }),
            );
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_send(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().submit(&ui);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_recover_message(move |id| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().recover_outgoing(&ui, &id);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_retry_message(move |id| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().retry_outgoing(&ui, &id, false);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_resolve_recovery(move |action| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().resolve_recovery(&ui, &action);
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_demo_event(move |kind| {
        if let Some(ui) = weak.upgrade() {
            if kind == 11 {
                view.borrow_mut().demo_paged_history(&ui);
            } else {
                demo::scenario(&view, &ui, kind);
            }
        }
    });
    // Model changes wait until the thumb is released, without an idle timer.
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_release_deferred(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            let events = std::mem::take(&mut view.deferred);
            for event in events {
                view.update_render(&ui, event);
            }
        }
    });
    if options.demo_rich {
        demo::scenario(&state, &ui, 12);
    }
    if options.demo_photos {
        demo::scenario(&state, &ui, 5);
    }
    if options.demo_groups {
        demo::scenario(&state, &ui, 7);
    }
    if options.demo_friends {
        state.borrow_mut().browse(&ui, 1);
    }
    state.borrow().send_availability(&ui);
    if let (Some(window), Some(path)) = (snapshot, options.snapshot) {
        ui.show()?;
        ui.set_text_size(options.snapshot_text_size);
        ui.set_show_conversation(!options.demo_friends);
        if options.snapshot_profile {
            state.borrow_mut().open_profile(&ui, "0");
        }
        snapshot::save(&window, &path, options.snapshot_size)?;
        ui.hide()?;
    } else {
        if let Some((width, height)) = options.demo_size {
            ui.window()
                .set_size(slint::LogicalSize::new(width as f32, height as f32));
        }
        ui.show()?;
        slint::run_event_loop_until_quit()?;
        ui.hide()?;
    }
    let mut view = state.borrow_mut();
    view.capture(&ui);
    if let Some(storage) = &view.storage {
        storage.flush(view.saved.clone());
    }
    worker.borrow_mut().take();
    view.background.shutdown();
    if let Some(attention) = view.attention.take() {
        attention.withdraw_all();
        drop(attention);
    }
    UI_STATE.with(|slot| slot.borrow_mut().take());
    Ok(())
}
fn start_saved_account(
    ui: &AppWindow,
    state: &Rc<RefCell<View>>,
    paths: account::Paths,
) -> account::Worker {
    {
        let mut view = state.borrow_mut();
        view.open_storage(ui, &paths);
        view.paths = Some(paths.clone());
    }
    ui.set_chat_title("Your conversations".into());
    let weak = ui.as_weak();
    let worker = account::start(paths, move |event| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            UI_STATE.with(|slot| {
                if let Some(state) = slot.borrow().as_ref() {
                    state.borrow_mut().update(&ui, event);
                }
            });
        });
    });
    state.borrow_mut().commands = Some(worker.commands.clone());
    state.borrow_mut().read_gate = Some(worker.read_gate.clone());
    unread::set_gate(worker.read_gate.clone());
    state.borrow_mut().retain_send_requests();
    state.borrow_mut().pairing_cancel = Some(worker.pairing_cancel.clone());
    worker
}
thread_local! {static UI_STATE:RefCell<Option<Rc<RefCell<View>>>>=const{RefCell::new(None)};}

#[cfg(test)]
mod tests;
