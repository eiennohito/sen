use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

mod snapshot;

slint::include_modules!();

struct Conversation {
    title: &'static str,
    subtitle: &'static str,
    preview: &'static str,
    initial: &'static str,
    messages: Vec<MessageRow>,
    draft: SharedString,
}

fn message(body: &str, time: &str, outgoing: bool) -> MessageRow {
    MessageRow {
        body: body.into(),
        time: time.into(),
        outgoing,
    }
}

fn examples() -> Vec<Conversation> {
    vec![
        Conversation {
            title: "葵",
            subtitle: "Example conversation",
            preview: "また明日、駅の前で。",
            initial: "葵",
            messages: vec![
                message("明日は何時に待ち合わせる？", "10:41", false),
                message("11時でどう？ コーヒーも飲もう。", "10:42", true),
                message("いいね！\nまた明日、駅の前で。", "10:43", false),
            ],
            draft: "".into(),
        },
        Conversation {
            title: "Weekend plans",
            subtitle: "Example group · 3 members",
            preview: "The riverside route looks good.",
            initial: "W",
            messages: vec![
                message("Shall we take a walk on Saturday?", "09:18", false),
                message("The riverside route looks good.", "09:20", true),
            ],
            draft: "".into(),
        },
        Conversation {
            title: "Notes to self",
            subtitle: "Example conversation",
            preview: "A quiet place for a thought.",
            initial: "N",
            messages: vec![message("A quiet place for a thought.", "Yesterday", true)],
            draft: "".into(),
        },
    ]
}

fn show_conversation(ui: &AppWindow, conversations: &[Conversation], index: usize) {
    let conversation = &conversations[index];
    ui.set_selected_chat(index as i32);
    ui.set_chat_title(conversation.title.into());
    ui.set_chat_subtitle(conversation.subtitle.into());
    ui.set_messages(ModelRc::new(VecModel::from(conversation.messages.clone())));
    ui.set_draft(conversation.draft.clone());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: line-desktop [--snapshot FILE.ppm]\nOffline Slint preview; no credentials are read and no messages are sent."
        );
        return Ok(());
    }
    let snapshot_path = match args.as_slice() {
        [] => None,
        [flag, path] if flag == "--snapshot" => Some(path),
        _ => return Err("usage: line-desktop [--snapshot FILE.ppm]".into()),
    };
    let snapshot = if snapshot_path.is_some() {
        Some(snapshot::install()?)
    } else {
        None
    };
    // Only this renderer is compiled in. Selection failure is an error; never
    // silently acquire a GPU context or fall back to a different backend.
    if snapshot.is_none() {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }
    let ui = AppWindow::new()?;
    let conversations = Rc::new(RefCell::new(examples()));
    let rows: Vec<ChatRow> = conversations
        .borrow()
        .iter()
        .enumerate()
        .map(|(index, chat)| ChatRow {
            index: index as i32,
            title: chat.title.into(),
            preview: chat.preview.into(),
            initial: chat.initial.into(),
        })
        .collect();
    ui.set_chats(ModelRc::new(VecModel::from(rows.clone())));
    show_conversation(&ui, &conversations.borrow(), 0);

    let weak = ui.as_weak();
    let chats = conversations.clone();
    ui.on_select_chat(move |index| {
        if let Some(ui) = weak.upgrade() {
            let mut chats = chats.borrow_mut();
            let previous = ui.get_selected_chat() as usize;
            let next = index as usize;
            if previous != next && previous < chats.len() && next < chats.len() {
                // Read the committed draft only when switching conversations.
                // Do not rewrite the TextEdit binding while IME is composing.
                chats[previous].draft = ui.get_draft();
                show_conversation(&ui, &chats, next);
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_search_changed(move |query| {
        if let Some(ui) = weak.upgrade() {
            let query = query.to_lowercase();
            let filtered: Vec<_> = rows
                .iter()
                .filter(|row| row.title.to_lowercase().contains(&query))
                .cloned()
                .collect();
            ui.set_chats(ModelRc::new(VecModel::from(filtered)));
        }
    });
    if let (Some(window), Some(path)) = (snapshot, snapshot_path) {
        ui.show()?;
        snapshot::save(&window, path)?;
        ui.hide()?;
    } else {
        ui.run()?;
    }
    Ok(())
}
