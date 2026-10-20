//! Opt-in offline measurements: LINE_UI_LATENCY=1 with --demo --snapshot PATH.
use super::*;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use std::{cell::Cell, time::Instant};
thread_local! { pub static RENDERS: Cell<usize> = const { Cell::new(0) }; }

pub fn run(d: &mut Desktop, ui: &AppWindow, window: &MinimalSoftwareWindow) {
    let fake = FakeBackend::canned();
    let template = d.state.chats.values().next().unwrap().clone();
    let message = (*d.state.rows().first().unwrap()).clone();
    d.state.chats.clear();
    d.state.chat_order.clear();
    let mut assets = Vec::new();
    for i in 0..300 {
        let mut chat = template.clone();
        chat.chat.id = ChatId(format!("latency-{i}"));
        chat.chat.title = format!("Conversation {i}");
        chat.chat.avatar = if i < 100 {
            let asset =
                demo_data::image_asset(&fake, &format!("latency-{i}"), demo_data::avatar(i % 3));
            assets.push(asset.key());
            Some(asset)
        } else {
            None
        };
        chat.preview = Some(message.clone());
        d.state.chat_order.push(chat.chat.id.clone());
        d.state.chats.insert(chat.chat.id.clone(), chat);
    }
    d.state.selected = Some(d.state.chat_order[0].clone());
    d.state.messages.clear();
    let count = std::env::var("LINE_UI_LATENCY_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    for i in 0..count {
        let mut row = message.clone();
        row.key = Key(100_000 + i);
        row.chat_id = d.state.selected.clone().unwrap();
        row.sort.sort_ts += i;
        d.state.messages.insert(row.key, row);
    }
    d.state.gaps.clear();
    d.render(ui);
    ui.show().unwrap();
    ui.global::<ShellState>().set_show_conversation(true);
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(980, 720);
    let mut draw = || {
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(pixels.make_mut_slice(), 980);
        });
    };
    draw();
    let image = demo_data::avatar(0).to_rgba8().unwrap();
    let pixels = std::sync::Arc::new(line::client::desktop::assets::Pixels {
        rgba: image.as_bytes().to_vec(),
        width: image.width(),
        height: image.height(),
    });
    RENDERS.set(0);
    let start = Instant::now();
    d.apply_batch(
        ui,
        assets.iter().map(|key| Delta::Asset {
            key: key.clone(),
            pixels: pixels.clone(),
        }),
    );
    println!(
        "LATENCY startup_assets=100 additional_full_renders={} batch_ms={:.3}",
        RENDERS.get(),
        start.elapsed().as_secs_f64() * 1000.
    );
    draw();
    for kind in ["asset", "switch"] {
        let mut render = Vec::new();
        let mut layout = Vec::new();
        let mut instantiate = Vec::new();
        for (i, key) in assets.iter().take(9).enumerate() {
            let start = Instant::now();
            if kind == "asset" {
                d.apply_batch(
                    ui,
                    [Delta::Asset {
                        key: key.clone(),
                        pixels: pixels.clone(),
                    }],
                );
            } else {
                d.state.selected = Some(d.state.chat_order[i + 1].clone());
                let rows: Vec<_> = d.state.rows().into_iter().cloned().collect();
                d.state.messages.clear();
                for mut row in rows {
                    row.key = Key(row.key.0 + 1000);
                    row.chat_id = d.state.selected.clone().unwrap();
                    d.state.messages.insert(row.key, row);
                }
                d.render(ui);
            }
            render.push(start.elapsed().as_secs_f64() * 1000.);
            let start = Instant::now();
            slint::private_unstable_api::re_exports::WindowInner::from_pub(ui.window())
                .ensure_tree_instantiated();
            instantiate.push(start.elapsed().as_secs_f64() * 1000.);
            let start = Instant::now();
            draw();
            layout.push(start.elapsed().as_secs_f64() * 1000.);
        }
        render.sort_by(f64::total_cmp);
        layout.sort_by(f64::total_cmp);
        instantiate.sort_by(f64::total_cmp);
        println!(
            "LATENCY {kind} instantiate_layout_median_ms={:.3}",
            instantiate[4]
        );
        println!(
            "LATENCY {kind} render_median_ms={:.3} layout_draw_median_ms={:.3}",
            render[4], layout[4]
        );
    }
    // Fixed-height side lists are measured separately before deciding whether
    // they need virtualization too.
    d.state.messages.clear();
    d.state.chats.clear();
    d.state.chat_order.clear();
    d.state.selected = None;
    let contacts = (0..300)
        .map(|i| line::store::Friend {
            profile: Default::default(),
            id: format!("contact-{i}"),
            name: Some(format!("Contact {i}")),
            status: "Available".into(),
            group: false,
            avatar: None,
            member_count: None,
        })
        .collect::<Vec<_>>();
    for kind in ["friends", "members"] {
        d.state.friends.clear();
        d.state.members.clear();
        if kind == "friends" {
            d.state.friends = contacts.clone();
            ui.global::<ShellState>().set_section(Section::Friends);
            d.state.panel = Panel::Friends;
        } else {
            d.state.members = contacts
                .iter()
                .cloned()
                .map(|contact| line::store::Member {
                    contact,
                    is_self: false,
                })
                .collect();
            d.state
                .push_detail(DetailPage::Members(ChatId("latency-members".into())));
        }
        d.render(ui);
        draw();
        let mut times = Vec::new();
        for i in 0..9 {
            if kind == "friends" {
                d.state.friends[0].status = i.to_string();
            } else {
                d.state.members[0].contact.status = i.to_string();
            }
            d.render(ui);
            let start = Instant::now();
            slint::private_unstable_api::re_exports::WindowInner::from_pub(ui.window())
                .ensure_tree_instantiated();
            times.push(start.elapsed().as_secs_f64() * 1000.);
            draw();
        }
        times.sort_by(f64::total_cmp);
        println!(
            "LATENCY {kind} rows=300 instantiate_layout_median_ms={:.3}",
            times[4]
        );
    }
}
