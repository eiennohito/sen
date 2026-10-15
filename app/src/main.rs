use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::mpsc::SyncSender};

mod account;
mod demo;
mod snapshot;
slint::include_modules!();

struct View {
    rows: Vec<ChatRow>,
    ids: Vec<String>,
    drafts: HashMap<String, SharedString>,
    current: String,
    messages: Vec<MessageRow>,
    generation: u64,
    query: String,
    commands: Option<SyncSender<account::Command>>,
    demo: bool,
}

impl View {
    fn filter(&self, ui: &AppWindow) {
        ui.set_chats(ModelRc::new(VecModel::from(
            self.rows
                .iter()
                .filter(|row| row.title.to_lowercase().contains(&self.query))
                .cloned()
                .collect::<Vec<_>>(),
        )));
    }
    fn select(&mut self, ui: &AppWindow, index: usize) {
        let Some(id) = self.ids.get(index).cloned() else {
            return;
        };
        if id == self.current {
            return;
        }
        if !self.current.is_empty() {
            self.drafts.insert(self.current.clone(), ui.get_draft());
        }
        self.current = id;
        ui.set_has_chat(true);
        self.generation += 1;
        ui.set_selected_chat(index as i32);
        ui.set_chat_title(self.rows[index].title.clone());
        ui.set_draft(self.drafts.get(&self.current).cloned().unwrap_or_default());
        self.messages.clear();
        ui.set_messages(ModelRc::default());
        if self.demo {
            self.messages = demo::examples()[index].messages.clone();
            ui.set_chat_subtitle(demo::examples()[index].subtitle.into());
            ui.set_messages(ModelRc::new(VecModel::from(self.messages.clone())));
        } else {
            ui.set_chat_subtitle("Loading recent messages…".into());
            self.request(
                ui,
                account::Command::Messages {
                    id: self.current.clone(),
                    generation: self.generation,
                },
            );
        }
    }
    fn request(&self, ui: &AppWindow, command: account::Command) -> bool {
        if self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok())
        {
            true
        } else {
            ui.set_status("Account worker unavailable or busy. Try again shortly.".into());
            false
        }
    }
    fn update(&mut self, ui: &AppWindow, event: account::Update) {
        match event {
            account::Update::Connected(name) => {
                ui.set_account_name(name.into());
                ui.set_connected(true);
                ui.set_status("Loading conversations…".into());
            }
            account::Update::Chats {
                chats,
                append,
                more,
            } => {
                if !append {
                    self.rows.clear();
                    self.ids.clear();
                }
                for chat in chats {
                    if let Some(index) = self.ids.iter().position(|id| id == &chat.id) {
                        self.rows[index].title = chat.title.into();
                        self.rows[index].preview = chat.preview.into();
                    } else {
                        let index = self.rows.len() as i32;
                        self.rows.push(ChatRow {
                            index,
                            title: chat.title.clone().into(),
                            initial: chat.title.chars().next().unwrap_or('·').to_string().into(),
                            preview: chat.preview.into(),
                            avatar_key: chat.avatar_key.into(),
                            ..Default::default()
                        });
                        self.ids.push(chat.id);
                    }
                }
                self.filter(ui);
                ui.set_more_chats(more);
                ui.set_status(
                    format!("{} conversations · Refresh to update", self.ids.len()).into(),
                );
                if let Some(index) = self.ids.iter().position(|id| id == &self.current) {
                    ui.set_selected_chat(index as i32);
                    ui.set_chat_title(self.rows[index].title.clone());
                } else {
                    if !self.current.is_empty() {
                        self.drafts.insert(self.current.clone(), ui.get_draft());
                    }
                    self.current.clear();
                    if !self.rows.is_empty() {
                        self.select(ui, 0);
                    } else {
                        ui.set_has_chat(false);
                        self.messages.clear();
                        ui.set_messages(ModelRc::default());
                        ui.set_chat_title("No conversations".into());
                    }
                }
            }
            account::Update::Messages {
                id,
                generation,
                messages,
            } => {
                if id != self.current || generation != self.generation {
                    return;
                }
                self.messages = messages
                    .into_iter()
                    .map(|message| {
                        let reactions: Vec<_> = message
                            .reactions
                            .into_iter()
                            .map(|r| ReactionRow {
                                label: r.label.into(),
                                image_key: r.image_key.into(),
                                ..Default::default()
                            })
                            .collect();
                        let time = chrono::DateTime::from_timestamp_millis(message.timestamp)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%m/%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        MessageRow {
                            body: message.body.into(),
                            time: time.into(),
                            outgoing: message.outgoing,
                            image_key: message.image_key.into(),
                            reactions: ModelRc::new(VecModel::from(reactions)),
                            ..Default::default()
                        }
                    })
                    .collect();
                ui.set_chat_subtitle(format!("{} recent messages", self.messages.len()).into());
                ui.set_messages(ModelRc::new(VecModel::from(self.messages.clone())));
            }
            account::Update::Image { key, pixels } => {
                let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &pixels.rgba,
                    pixels.width,
                    pixels.height,
                ));
                let mut changed = false;
                for row in &mut self.rows {
                    if row.avatar_key == key {
                        row.avatar = image.clone();
                        changed = true;
                    }
                }
                if changed {
                    self.filter(ui);
                }
                let mut changed = false;
                for row in &mut self.messages {
                    if row.image_key == key {
                        row.picture = image.clone();
                        row.has_picture = true;
                        changed = true;
                    }
                    let mut reactions: Vec<_> = row.reactions.iter().collect();
                    for reaction in &mut reactions {
                        if reaction.image_key == key {
                            reaction.picture = image.clone();
                            reaction.has_picture = true;
                            changed = true;
                        }
                    }
                    row.reactions = ModelRc::new(VecModel::from(reactions));
                }
                if changed {
                    ui.set_messages(ModelRc::new(VecModel::from(self.messages.clone())));
                }
            }
            account::Update::Sent {
                id,
                generation,
                text,
            } => {
                ui.set_sending(false);
                if self.drafts.get(&id).is_some_and(|draft| draft == &text) {
                    self.drafts.remove(&id);
                }
                if id == self.current && generation == self.generation && ui.get_draft() == text {
                    ui.set_draft("".into());
                }
                ui.set_status("Message sent".into());
            }
            account::Update::Error {
                message,
                sending,
                generation,
            } => {
                if sending {
                    ui.set_sending(false);
                }
                if !sending && generation.is_some_and(|g| g != self.generation) {
                    return;
                }
                ui.set_status(message.into());
                if generation.is_some() && !sending {
                    ui.set_chat_subtitle("Could not load messages · use Refresh to retry".into());
                }
            }
        }
    }
}

struct Options {
    paths: account::Paths,
    demo: bool,
    snapshot: Option<String>,
    check: bool,
}
fn options() -> Result<Options, Box<dyn std::error::Error>> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("artifacts/cache"))
        .join("line-desktop");
    let mut out = Options {
        paths: account::Paths {
            state: "artifacts/auth/session.json".into(),
            wasm: line_wasm_poc::DEFAULT_WASM.into(),
            cache,
        },
        demo: false,
        snapshot: None,
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
            "--check-live" => out.check = true,
            "--state" => out.paths.state = args.next().ok_or("missing state path")?.into(),
            "--wasm" => out.paths.wasm = args.next().ok_or("missing WASM path")?.into(),
            "--cache-dir" => out.paths.cache = args.next().ok_or("missing cache path")?.into(),
            "--snapshot" => {
                out.snapshot = Some(args.next().ok_or("missing snapshot path")?);
                out.demo = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: line-desktop [--demo] [--state FILE] [--wasm FILE] [--cache-dir DIR]\n       line-desktop --snapshot FILE.ppm\n       line-desktop --check-live [--state FILE] [--wasm FILE] [--cache-dir DIR]\nDefault: use saved account. --demo and --snapshot are offline. --check-live reports counts only and never sends messages."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if out.check && out.demo {
        return Err("--check-live cannot be combined with offline modes".into());
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    if options.check {
        return account::check(&options.paths);
    }
    let snapshot = if options.snapshot.is_some() {
        Some(snapshot::install()?)
    } else {
        None
    };
    if snapshot.is_none() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }
    slint::set_xdg_app_id("org.lineclient.Desktop")?;
    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(View {
        rows: Vec::new(),
        ids: Vec::new(),
        drafts: HashMap::new(),
        current: String::new(),
        messages: Vec::new(),
        generation: 0,
        query: String::new(),
        commands: None,
        demo: options.demo,
    }));
    let worker = if options.demo {
        ui.set_demo(true);
        ui.set_account_name("Offline preview".into());
        ui.set_status("Messages are examples".into());
        for (index, chat) in demo::examples().into_iter().enumerate() {
            let mut view = state.borrow_mut();
            view.ids.push(index.to_string());
            view.rows.push(ChatRow {
                index: index as i32,
                title: chat.title.into(),
                preview: chat.preview.into(),
                initial: chat.initial.into(),
                ..Default::default()
            });
        }
        state.borrow().filter(&ui);
        state.borrow_mut().select(&ui, 0);
        None
    } else {
        ui.set_status("Opening saved account…".into());
        ui.set_chat_title("Your conversations".into());
        let weak = ui.as_weak();
        let events = state.clone();
        // This Rc stays on the UI thread: the worker publishes through a weak
        // component handle, and the UI callback accesses the global controller.
        UI_STATE.with(|slot| *slot.borrow_mut() = Some(events));
        let worker = account::start(options.paths, move |event| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                UI_STATE.with(|slot| {
                    if let Some(state) = slot.borrow().as_ref() {
                        state.borrow_mut().update(&ui, event);
                    }
                })
            });
        });
        state.borrow_mut().commands = Some(worker.commands.clone());
        Some(worker)
    };
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_select_chat(move |index| {
        if let Some(ui) = weak.upgrade() {
            view.borrow_mut().select(&ui, index as usize);
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
    ui.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            let mut view = view.borrow_mut();
            view.generation += 1;
            view.request(&ui, account::Command::Chats { next: false });
            if !view.current.is_empty() {
                view.request(
                    &ui,
                    account::Command::Messages {
                        id: view.current.clone(),
                        generation: view.generation,
                    },
                );
            }
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_more(move || {
        if let Some(ui) = weak.upgrade() {
            view.borrow()
                .request(&ui, account::Command::Chats { next: true });
        }
    });
    let weak = ui.as_weak();
    let view = state.clone();
    ui.on_send(move || {
        if let Some(ui) = weak.upgrade() {
            let view = view.borrow();
            if view.demo || ui.get_sending() || view.current.is_empty() {
                return;
            }
            let text = ui.get_draft().to_string();
            if text.trim().is_empty() {
                return;
            }
            if view.request(
                &ui,
                account::Command::Send {
                    id: view.current.clone(),
                    generation: view.generation,
                    text,
                },
            ) {
                ui.set_sending(true);
                ui.set_status("Sending…".into());
            }
        }
    });
    if let (Some(window), Some(path)) = (snapshot, options.snapshot) {
        ui.show()?;
        snapshot::save(&window, &path)?;
        ui.hide()?;
    } else {
        ui.run()?;
    }
    drop(worker);
    UI_STATE.with(|slot| slot.borrow_mut().take());
    Ok(())
}

thread_local! {static UI_STATE:RefCell<Option<Rc<RefCell<View>>>>=const{RefCell::new(None)};}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_network_responses_do_not_replace_current_chat_or_new_draft() {
        let _window = snapshot::install().unwrap();
        let ui = AppWindow::new().unwrap();
        let mut view = View {
            rows: Vec::new(),
            ids: Vec::new(),
            drafts: HashMap::new(),
            current: "current-chat".into(),
            messages: vec![MessageRow {
                body: "current message".into(),
                ..Default::default()
            }],
            generation: 2,
            query: String::new(),
            commands: None,
            demo: false,
        };
        view.update(
            &ui,
            account::Update::Messages {
                id: "previous-chat".into(),
                generation: 1,
                messages: Vec::new(),
            },
        );
        assert_eq!(view.messages.len(), 1);
        ui.set_sending(true);
        ui.set_draft("new text typed after clicking Send".into());
        view.update(
            &ui,
            account::Update::Sent {
                id: "current-chat".into(),
                generation: 2,
                text: "older submitted text".into(),
            },
        );
        assert!(!ui.get_sending());
        assert_eq!(ui.get_draft(), "new text typed after clicking Send");
        ui.set_chat_subtitle("current history".into());
        view.update(
            &ui,
            account::Update::Error {
                message: "old request failed".into(),
                sending: false,
                generation: Some(1),
            },
        );
        assert_eq!(ui.get_chat_subtitle(), "current history");
    }
}
