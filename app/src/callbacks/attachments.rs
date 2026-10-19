//! Selection and local preparation. Only the dialog's Send callback enqueues uploads.
use crate::*;
use line::attachments::{Attachment, FILE_LIMIT};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct Selection {
    generation: u64,
    chat: Option<ChatId>,
    items: Vec<Attachment>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Prepared {
    generation: u64,
    items: Vec<Attachment>,
    errors: Vec<String>,
}

fn picture(bytes: &[u8]) -> slint::Image {
    if let Ok(image) = image::load_from_memory(bytes) {
        let image = image.to_rgba8();
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            image.as_raw(),
            image.width(),
            image.height(),
        );
        slint::Image::from_rgba8(buffer)
    } else {
        Default::default()
    }
}
fn present(ui: &AppWindow, selection: &Selection) {
    let global = ui.global::<State>();
    global.set_attachments_count(
        selection
            .items
            .iter()
            .filter(|i| !i.too_large(FILE_LIMIT))
            .count() as i32,
    );
    global.set_attachment_items(ModelRc::new(VecModel::from(
        selection
            .items
            .iter()
            .map(|i| AttachmentItem {
                name: i.name.clone().into(),
                size: rich_content::file_size(i.size).into(),
                photo: i.photo.is_some(),
                picture: picture(&i.preview),
                error: if i.too_large(FILE_LIMIT) {
                    "Too large"
                } else {
                    ""
                }
                .into(),
            })
            .collect::<Vec<_>>(),
    )));
}
fn begin(
    ui: &AppWindow,
    desktop: &Desktop,
    selection: &mut Selection,
    input: impl FnOnce() -> Result<Vec<PathBuf>, String> + Send + 'static,
) {
    if desktop.state.selected.is_none()
        || ui.global::<State>().get_modal_open()
        || ui.global::<State>().get_chat_left()
    {
        return;
    }
    selection.generation += 1;
    selection.chat = desktop.state.selected.clone();
    for item in selection.items.drain(..) {
        item.delete_clipboard();
    }
    let generation = selection.generation;
    let global = ui.global::<State>();
    global.set_attachments_error("".into());
    global.set_attachments_chat(
        desktop
            .state
            .selected
            .as_ref()
            .and_then(|id| desktop.state.chats.get(id))
            .map(|c| c.chat.title.clone())
            .unwrap_or_default()
            .into(),
    );
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let mut prepared = Prepared {
            generation,
            items: vec![],
            errors: vec![],
        };
        match input() {
            Ok(paths) => {
                for path in paths {
                    match Attachment::prepare(&path) {
                        Ok(mut item) => {
                            item.clipboard = clipboard_root()
                                .is_ok_and(|root| path.parent() == Some(root.as_path()));
                            prepared.items.push(item);
                        }
                        Err(error) => {
                            if clipboard_root()
                                .is_ok_and(|root| path.parent() == Some(root.as_path()))
                            {
                                let _ = std::fs::remove_file(&path);
                            }
                            prepared.errors.push(format!(
                                "{}: {error}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        }
                    }
                }
            }
            Err(error) => prepared.errors.push(error),
        }
        let payload = serde_json::to_string(&prepared).expect("selection JSON");
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<State>()
                .invoke_attachments_prepared(payload.into());
        });
    });
}
fn uri_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| url::Url::parse(line.trim()).ok()?.to_file_path().ok())
        .collect()
}
fn chooser() -> Result<Vec<PathBuf>, String> {
    (|| -> Result<_, Box<dyn std::error::Error>> {
        use zbus::{
            blocking::{Connection, Proxy},
            zvariant::{OwnedObjectPath, OwnedValue, Value},
        };
        let bus = Connection::session()?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let token = format!(
            "line_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let sender = bus
            .unique_name()
            .ok_or("missing bus identity")?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = Proxy::new(
            &bus,
            "org.freedesktop.portal.Desktop",
            path.as_str(),
            "org.freedesktop.portal.Request",
        )?;
        let mut signals = request.receive_signal("Response")?;
        let portal = Proxy::new(
            &bus,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
        )?;
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("multiple", Value::from(true)),
            ("modal", Value::from(true)),
        ]);
        let returned: OwnedObjectPath = portal.call("OpenFile", &("", "Attach files", options))?;
        if returned.as_str() != path {
            return Err("unexpected portal request path".into());
        }
        let response = signals.next().ok_or("file chooser closed")?;
        let (code, mut results): (u32, HashMap<String, OwnedValue>) =
            response.body().deserialize()?;
        if code != 0 {
            return Ok(vec![]);
        }
        let uris = Vec::<String>::try_from(results.remove("uris").ok_or("missing selection")?)?;
        Ok(uris.into_iter().flat_map(|uri| uri_paths(&uri)).collect())
    })()
    .map_err(|_| "Couldn't open file chooser.".into())
}
pub(crate) fn install(ui: &AppWindow, actions: &Runtime) {
    macro_rules! bind {
        ($on:ident, $selected:ident, |$ui:ident, $d:ident $(,$arg:ident)*| $body:block) => {{
            let weak = ui.as_weak(); let runtime = actions.clone();
            ui.global::<State>().$on(move |$($arg),*| {
                let weak = weak.clone(); let $selected = $selected.clone();
                runtime.push(move |$d| { if let Some($ui) = weak.upgrade() { $body } });
            });
        }};
    }

    let selection = Rc::new(RefCell::new(Selection::default()));
    let selected = selection.clone();
    bind!(on_attach, selected, |ui, d| {
        begin(&ui, d, &mut selected.borrow_mut(), chooser);
    });
    let selected = selection.clone();
    bind!(on_drop_attachment, selected, |ui, d, uri| {
        let paths = uri_paths(&uri);
        if !paths.is_empty() {
            begin(&ui, d, &mut selected.borrow_mut(), move || Ok(paths));
        }
    });
    let selected = selection.clone();
    bind!(on_inject_attachments, selected, |ui, d, paths| {
        if cfg!(feature = "ui-debug") && ui.global::<State>().get_automation_background() {
            let paths: Vec<PathBuf> = serde_json::from_str(&paths).unwrap_or_default();
            begin(&ui, d, &mut selected.borrow_mut(), move || Ok(paths));
        }
    });
    let selected = selection.clone();
    bind!(on_attachments_prepared, selected, |ui, _d, json| {
        if let Ok(prepared) = serde_json::from_str::<Prepared>(&json) {
            let mut selection = selected.borrow_mut();
            if selection.generation == prepared.generation {
                for item in selection.items.drain(..) {
                    item.delete_clipboard();
                }
                selection.items = prepared.items;
                present(&ui, &selection);
                ui.global::<State>()
                    .set_attachments_error(prepared.errors.join("\n").into());
                ui.global::<State>().set_attachments_open(
                    !selection.items.is_empty() || !prepared.errors.is_empty(),
                );
            } else {
                for item in prepared.items {
                    item.delete_clipboard();
                }
            }
        }
    });
    let selected = selection.clone();
    bind!(on_remove_attachment, selected, |ui, _d, index| {
        let mut selection = selected.borrow_mut();
        if (index as usize) < selection.items.len() {
            selection.items.remove(index as usize).delete_clipboard();
        }
        present(&ui, &selection);
    });
    let selected = selection.clone();
    bind!(on_cancel_attachments, selected, |ui, _d| {
        let mut selection = selected.borrow_mut();
        selection.generation += 1;
        for item in selection.items.drain(..) {
            item.delete_clipboard();
        }
        ui.global::<State>().set_attachments_open(false);
        ui.invoke_focus_composer();
    });
    let selected = selection.clone();
    bind!(on_send_attachments, selected, |ui, d| {
        let mut selection = selected.borrow_mut();
        if let Some(chat) = selection.chat.take() {
            let items = std::mem::take(&mut selection.items)
                .into_iter()
                .filter(|item| {
                    if item.too_large(FILE_LIMIT) {
                        item.delete_clipboard();
                        false
                    } else {
                        true
                    }
                })
                .collect();
            d.sender.send(Intent::SendAttachments { chat, items });
            ui.global::<State>().set_attachments_open(false);
            ui.invoke_focus_composer();
        }
    });
    let selected = selection.clone();
    bind!(on_paste_text, selected, |ui, d| {
        if d.state.selected == selected.borrow().chat && !ui.global::<State>().get_modal_open() {
            let global = ui.global::<State>();
            global.set_paste_text_serial(global.get_paste_text_serial() + 1);
        }
    });
    let weak = ui.as_weak();
    let runtime = actions.clone();
    ui.global::<State>().on_paste_attachment(move || {
        let weak = weak.clone();
        let selected = selection.clone();
        runtime.push(move |d| {
            if let Some(ui) = weak.upgrade() {
                begin(&ui, d, &mut selected.borrow_mut(), move || {
                    let paste_text = || {
                        weak.upgrade_in_event_loop(|ui| ui.global::<State>().invoke_paste_text())
                    };
                    let Ok(mut clipboard) = arboard::Clipboard::new() else {
                        let _ = paste_text();
                        return Ok(vec![]);
                    };
                    if let Ok(image) = clipboard.get_image() {
                        let root = clipboard_root()?;
                        use std::os::unix::fs::DirBuilderExt;
                        std::fs::DirBuilder::new()
                            .recursive(true)
                            .mode(0o700)
                            .create(&root)
                            .map_err(|e| e.to_string())?;
                        static NEXT: AtomicU64 = AtomicU64::new(1);
                        let path = root.join(format!(
                            "Clipboard-{}-{}.png",
                            std::process::id(),
                            NEXT.fetch_add(1, Ordering::Relaxed)
                        ));
                        let rgba = image::RgbaImage::from_raw(
                            image.width as u32,
                            image.height as u32,
                            image.bytes.into_owned(),
                        )
                        .ok_or("Invalid clipboard image")?;
                        rgba.save(&path).map_err(|e| e.to_string())?;
                        Ok(vec![path])
                    } else {
                        let text = clipboard.get_text().unwrap_or_default();
                        let paths = uri_paths(&text);
                        if paths.is_empty() {
                            let _ = paste_text();
                        }
                        Ok(paths)
                    }
                });
            }
        });
        true
    });
}
fn clipboard_root() -> Result<PathBuf, String> {
    Ok(line::locations::Locations::from_environment()?
        .session
        .parent()
        .ok_or("Missing app data directory")?
        .join("clipboard-attachments"))
}
