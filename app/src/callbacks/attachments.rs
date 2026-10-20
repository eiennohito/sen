//! Selection and local preparation. Only the dialog's Send callback enqueues uploads.
use crate::AttachmentState as G;
use crate::*;
use line::attachments::Attachment;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub(crate) struct Selection {
    generation: u64,
    chat: Option<ChatId>,
    items: Vec<Attachment>,
}
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
    let g = ui.global::<G>();
    g.set_attachments_count(selection.items.iter().filter(|i| !i.too_large()).count() as i32);
    g.set_attachment_items(ModelRc::new(VecModel::from(
        selection
            .items
            .iter()
            .map(|i| AttachmentItem {
                name: i.name.clone().into(),
                size: presenter::format::file_size(i.size).into(),
                photo: i.photo.is_some(),
                picture: picture(&i.preview),
                error: if i.too_large() { "Too large" } else { "" }.into(),
            })
            .collect::<Vec<_>>(),
    )));
}
pub(crate) fn begin(
    ui: &AppWindow,
    desktop: &Desktop,
    selected: &std::sync::Arc<std::sync::Mutex<Selection>>,
    input: impl FnOnce() -> Result<Vec<PathBuf>, String> + Send + 'static,
) {
    if desktop.state.selected.is_none()
        || ui.global::<ShellState>().get_modal_open()
        || ui.global::<ComposerState>().get_composer_mode() == ComposerMode::Left
    {
        return;
    }
    let mut selection = selected.lock().unwrap();
    selection.generation += 1;
    selection.chat = desktop.state.selected.clone();
    for item in selection.items.drain(..) {
        item.delete_clipboard();
    }
    let generation = selection.generation;
    let g = ui.global::<G>();
    g.set_attachments_error("".into());
    g.set_attachments_chat(
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
    let selected = selected.clone();
    let root = desktop.clipboard_root.clone();
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
                            item.clipboard = path.parent() == Some(root.as_path());
                            prepared.items.push(item);
                        }
                        Err(error) => {
                            if path.parent() == Some(root.as_path()) {
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
        let _ = weak.upgrade_in_event_loop(move |ui| {
            complete(&ui, &mut selected.lock().unwrap(), prepared);
        });
    });
}
fn complete(ui: &AppWindow, selection: &mut Selection, prepared: Prepared) {
    let g = ui.global::<G>();
    if selection.generation == prepared.generation {
        for item in selection.items.drain(..) {
            item.delete_clipboard();
        }
        selection.items = prepared.items;
        present(ui, selection);
        g.set_attachments_error(prepared.errors.join("\n").into());
        g.set_attachments_open(!selection.items.is_empty() || !prepared.errors.is_empty());
    } else {
        for item in prepared.items {
            item.delete_clipboard();
        }
    }
}
fn uri_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| url::Url::parse(line.trim()).ok()?.to_file_path().ok())
        .collect()
}
fn chooser() -> Result<Vec<PathBuf>, String> {
    desktop_env::portal::choose(
        "OpenFile",
        "Attach files",
        HashMap::from([("multiple", true.into())]),
    )
    .map(|uris| {
        uris.unwrap_or_default()
            .into_iter()
            .flat_map(|uri| uri_paths(&uri))
            .collect()
    })
    .map_err(|_| "Couldn't open file chooser.".into())
}
pub(crate) fn install(ui: &AppWindow, actions: &Runtime) {
    global_callback!(ui, actions, G, on_attach, |ui, d| {
        begin(&ui, d, &d.attachments, chooser);
    });
    #[cfg(feature = "ui-debug")]
    dev::automation::inject_attachments(ui, actions);
    global_callback!(ui, actions, G, on_remove_attachment, |ui, d, index| {
        let mut selection = d.attachments.lock().unwrap();
        if (index as usize) < selection.items.len() {
            selection.items.remove(index as usize).delete_clipboard();
        }
        present(&ui, &selection);
    });
    global_callback!(ui, actions, G, on_cancel_attachments, |ui, d| {
        let g = ui.global::<G>();
        let mut selection = d.attachments.lock().unwrap();
        selection.generation += 1;
        for item in selection.items.drain(..) {
            item.delete_clipboard();
        }
        g.set_attachments_open(false);
        ui.invoke_focus_composer();
    });
    global_callback!(ui, actions, G, on_send_attachments, |ui, d| {
        let g = ui.global::<G>();
        let mut selection = d.attachments.lock().unwrap();
        if let Some(chat) = selection.chat.take() {
            let items = std::mem::take(&mut selection.items)
                .into_iter()
                .filter(|item| {
                    if item.too_large() {
                        item.delete_clipboard();
                        false
                    } else {
                        true
                    }
                })
                .collect();
            d.sender.send(Intent::SendAttachments { chat, items });
            g.set_attachments_open(false);
            ui.invoke_focus_composer();
        }
    });
    global_callback!(ui, actions, G, on_paste_attachment, |ui, d| {
        let weak = ui.as_weak();
        let root = d.clipboard_root.clone();
        let selected = d.attachments.clone();
        begin(&ui, d, &d.attachments, move || {
            let paste_text = || {
                weak.upgrade_in_event_loop({
                    let selected = selected.clone();
                    move |ui| paste_text(&ui, &selected)
                })
            };
            let Ok(mut clipboard) = arboard::Clipboard::new() else {
                let _ = paste_text();
                return Ok(vec![]);
            };
            if let Ok(image) = clipboard.get_image() {
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
    } => true);
}

pub(crate) fn drop_files(ui: &AppWindow, d: &Desktop, uris: &str) {
    let paths = uri_paths(uris);
    if !paths.is_empty() {
        begin(ui, d, &d.attachments, move || Ok(paths));
    }
}

fn paste_text(ui: &AppWindow, selected: &std::sync::Mutex<Selection>) {
    if selected
        .lock()
        .unwrap()
        .chat
        .as_ref()
        .is_some_and(|chat| chat.0 == ui.global::<ConversationState>().get_chat_id().as_str())
        && !ui.global::<ShellState>().get_modal_open()
    {
        ui.invoke_paste_text();
    }
}
