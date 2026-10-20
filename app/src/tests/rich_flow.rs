use super::*;
use crate::rich_content::{flow::*, install, runs};
fn rich_fixture() -> Vec<MessageRow> {
    let body = "日😀[笑]本🌸 longunbrokenabcdefghijklmnopqrstuvwxyz0123456789\n二行目のテキスト";
    let mut row = MessageRow {
        id: "rich-emoji".into(),
        body: body.into(),
        ..Default::default()
    };
    row.rich_runs = runs(
        body,
        &[presenter::format::EmojiView {
            span: sen::store::InlineEmoji {
                start_utf16: 3,
                end_utf16: 6,
                start_byte: 7,
                end_byte: 12,
                fallback: "[笑]".into(),
                product_id: "synthetic".into(),
                sticon_id: "synthetic".into(),
                version: None,
                animated: false,
            },
            image_key: "demo-inline".into(),
        }],
    );
    {
        let runs = &row.rich_runs;
        for i in 0..runs.row_count() {
            let mut run = runs.row_data(i).unwrap();
            if run.emoji {
                run.picture = green_image();
                run.has_picture = true;
                runs.set_row_data(i, run);
            }
        }
    }
    vec![MessageRow::default(), row]
}
fn layout(ui: &AppWindow, rows: &ModelRc<RichRun>, width: f32, size: f32) -> ModelRc<RichRun> {
    crate::rich_content::flow::layout(
        ui,
        rows,
        width,
        size,
        &RefCell::new(Measurements::default()),
    )
}

fn watch_layout(ui: &AppWindow) -> Rc<RefCell<(ModelRc<RichRun>, f32)>> {
    let observed = Rc::new(RefCell::new((ModelRc::default(), 0.)));
    let recorded = observed.clone();
    let weak = ui.as_weak();
    let metrics = RefCell::new(Measurements::default());
    ui.global::<ContentState>()
        .on_rich_layout(move |rows, width, size, previous| {
            let output = crate::rich_content::flow::reuse(
                previous,
                crate::rich_content::flow::layout(&weak.unwrap(), &rows, width, size, &metrics),
            );
            *recorded.borrow_mut() = (output.clone(), width);
            output
        });
    observed
}

#[test]
fn missing_emoji_reserves_the_loaded_geometry() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut message = rich_fixture().remove(1);
        rich_content::links::decorate_message(&mut message);
        let before = layout(&ui, &message.rich_runs, 140., 24.);
        {
            let runs = &message.rich_runs;
            for i in 0..runs.row_count() {
                let mut run = runs.row_data(i).unwrap();
                if run.emoji {
                    run.picture = Default::default();
                    run.has_picture = false;
                    runs.set_row_data(i, run);
                }
            }
        }
        let after = layout(&ui, &message.rich_runs, 140., 24.);
        let geometry = |rows: ModelRc<RichRun>| {
            rows.iter()
                .map(|run| (run.x, run.y, run.measured_width, run.measured_height))
                .collect::<Vec<_>>()
        };
        assert_eq!(geometry(before), geometry(after));
    })
    .join()
    .unwrap();
}

#[test]
fn layout_failure_keeps_the_native_body_selectable() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        install(&ui);
        let mut row = MessageRow {
            body: "Before https://example.org/a after".into(),
            outgoing: true,
            ..Default::default()
        };
        rich_content::links::decorate_message(&mut row);
        assert_eq!(row.links.row_count(), 1);
        let oversized = ModelRc::new(VecModel::from(vec![RichRun {
            text: "a".repeat(2050).into(),
            ..Default::default()
        }]));
        assert_eq!(layout(&ui, &oversized, 1., 24.).row_count(), 0);
        ui.global::<ContentState>()
            .on_rich_layout(|_, _, _, _| ModelRc::default());
        ui.global::<ConversationState>()
            .set_messages(ModelRc::new(VecModel::from(vec![row])));
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        let pixels = render(&window, (980, 720));
        let (x, y, _) = pixels
            .enumerate_pixels()
            .find(|(_, _, p)| p.0 == [17, 17, 17])
            .expect("fallback body did not paint");
        let position = slint::LogicalPosition::new(x as f32, y as f32);
        click_at(&ui, position);
        for (text, pressed) in [
            ("\u{11}", true),
            ("a", true),
            ("a", false),
            ("\u{11}", false),
        ] {
            ui.window().dispatch_event(if pressed {
                slint::platform::WindowEvent::KeyPressed { text: text.into() }
            } else {
                slint::platform::WindowEvent::KeyReleased { text: text.into() }
            });
        }
        let selected = render(&window, (980, 720));
        assert!(
            selected.pixels().filter(|p| p.0 == [0, 69, 204]).count() > 100,
            "fallback did not expose native selection"
        );
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn break_tokens_preserve_fallback_and_multiline_copy_source() {
    let body = "日本👩🏽‍💻[笑]\nlongunbrokentext";
    let start = body.find("[笑]").unwrap();
    let end = start + "[笑]".len();
    let span = sen::store::InlineEmoji {
        start_utf16: 0,
        end_utf16: 0,
        start_byte: start,
        end_byte: end,
        fallback: "[笑]".into(),
        product_id: "synthetic".into(),
        sticon_id: "one".into(),
        version: None,
        animated: false,
    };
    let rows = runs(
        body,
        &[presenter::format::EmojiView {
            span,
            image_key: "image-key".into(),
        }],
    );
    let tokens: Vec<_> = (0..rows.row_count())
        .map(|i| rows.row_data(i).unwrap())
        .collect();
    assert_eq!(
        tokens.iter().map(|t| t.text.as_str()).collect::<String>(),
        body
    );
    assert!(tokens.iter().any(|t| t.text == "👩🏽‍💻"));
    assert_eq!(tokens.iter().filter(|t| t.emoji).count(), 1);
    assert_eq!(tokens.iter().filter(|t| t.newline).count(), 1);
}
#[test]
fn maximum_rich_flow_measures_and_reflows_at_narrow_large_text() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();

        install(&ui);
        let observed = watch_layout(&ui);
        let letter = |i: u8| char::from(if i < 26 { b'a' + i } else { b'A' + i - 26 });
        let words = (0..2047)
            .map(|i| format!("{}{} ", letter((i / 52) as u8), letter((i % 52) as u8)))
            .collect::<String>();
        let start = words.len();
        let body = format!("{words}[笑]");
        let end = start + "[笑]".len();
        let span = sen::store::InlineEmoji {
            start_utf16: 0,
            end_utf16: 0,
            start_byte: start,
            end_byte: end,
            fallback: "[笑]".into(),
            product_id: "synthetic".into(),
            sticon_id: "one".into(),
            version: None,
            animated: false,
        };
        let runs = runs(
            &body,
            &[presenter::format::EmojiView {
                span,
                image_key: "synthetic".into(),
            }],
        );
        assert_eq!(runs.row_count(), 2048);
        ui.global::<ConversationState>()
            .set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                id: "stress".into(),
                body: body.into(),
                rich_runs: runs.clone(),
                ..Default::default()
            }])));
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        assert_eq!(
            crate::rich_content::runs(&"日".repeat(2049), &[]).row_count(),
            0
        );
        render(&window, (980, 720));
        let before = observed.borrow().0.iter().last().unwrap().y;
        assert!(before > 0.0, "native run measurements did not initialize");
        ui.global::<SettingsState>().set_text_size(24);
        render(&window, (360, 520));
        let last = observed.borrow().0.iter().last().unwrap();
        assert!(last.y > before, "narrow large text did not reflow");
        for run in observed.borrow().0.iter() {
            assert!(
                run.x + run.measured_width <= observed.borrow().1 + 1.0,
                "rich run overflowed the pane"
            );
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
fn render(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    size: (u32, u32),
) -> image::RgbImage {
    let pixels = snapshot::render(window, size).unwrap();
    image::RgbImage::from_raw(size.0, size.1, pixels.as_bytes().to_vec()).unwrap()
}

fn bubble_bounds(pixels: &image::RgbImage) -> (u32, u32, u32, u32) {
    let bubble: Vec<_> = pixels
        .enumerate_pixels()
        .filter(|(_, _, p)| p.0 == [214, 230, 255])
        .map(|(x, y, _)| (x, y))
        .collect();
    (
        bubble
            .iter()
            .map(|p| p.0)
            .min()
            .expect("bubble did not paint"),
        bubble.iter().map(|p| p.0).max().unwrap(),
        bubble.iter().map(|p| p.1).min().unwrap(),
        bubble.iter().map(|p| p.1).max().unwrap(),
    )
}

#[test]
fn unbroken_bodies_and_inline_links_stay_inside_narrow_bubbles() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        install(&ui);
        let observed = watch_layout(&ui);
        let url = format!("https://example.org/{}", "abcdefghij".repeat(8));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let recorded = opened.clone();
        ui.global::<ContentState>()
            .on_open_link(move |url| recorded.borrow_mut().push(url));
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.global::<SettingsState>().set_text_size(text_size);
                for linked in [false, true] {
                    let mut row = MessageRow {
                        id: "long-run".into(),
                        body: if linked {
                            format!("Before {url} after")
                        } else {
                            format!("https://example.org/{}", "abcdefghij".repeat(10))
                        }
                        .into(),
                        outgoing: true,
                        ..Default::default()
                    };
                    if linked { rich_content::links::decorate_message(&mut row); }
                    ui.global::<ConversationState>().set_messages(ModelRc::new(VecModel::from(vec![row])));
                    let pixels = render(&window, size);
                    let runs = observed.borrow().0.clone();
                    if linked {
                        for run in runs.iter() {
                            assert!(run.measured_height > 0.0);
                            assert!(
                                run.x + run.measured_width <= observed.borrow().1 + 0.25,
                                "inline URL overflowed"
                            );
                            if run.link_start {
                                assert!(run.x + run.link_width <= observed.borrow().1 + 0.25);
                            }
                        }
                        assert!(runs.row_count() < 20);
                        assert_eq!(runs.iter().filter(|r| r.link_first).count(), 1);
                    }
                    let (left, right, top, bottom) = bubble_bounds(&pixels);
                    if linked && size.0 == 980 && text_size == 24 {
                        let link = runs.iter().find(|r| r.link_start && !r.link_first).unwrap();
                        let position = slint::LogicalPosition::new(
                            left as f32 + 12.0 + link.x + link.link_width * 0.8,
                            top as f32 + 8.0 + link.y + link.measured_height / 2.0,
                        );
                        click_at(&ui, position);
                        assert_eq!(
                            opened.borrow().last().map(|value| value.as_str()),
                            Some(url.as_str()),
                            "inline link hit area did not cover its text"
                        );
                    }
                    let ink: Vec<_> = pixels
                        .enumerate_pixels()
                        .filter(|(x, y, p)| (size.0 < 740 || *x > 342) && *y >= top && *y <= bottom && p.0 == [17, 17, 17])
                        .map(|(x, y, _)| (x, y))
                        .collect();
                    assert!(!ink.is_empty());
                    assert!(
                        ink.iter().all(|p| p.0 > left && p.0 < right),
                        "body painted outside the bubble"
                    );
                    let mut lines: Vec<_> = ink.iter().map(|p| p.1).collect();
                    if linked {
                        lines.extend(
                            pixels
                                .enumerate_pixels()
                                .filter(|(x, y, p)| {
                                    *x > left
                                        && *x < right
                                        && *y >= top
                                        && *y <= bottom
                                        && p.0 == [0, 69, 204]
                                })
                                .map(|(_, y, _)| y),
                        );
                    }
                    assert!(
                        lines.iter().max().unwrap() - lines.iter().min().unwrap()
                            > text_size as u32,
                        "long run did not wrap: {size:?} font {text_size} linked {linked}; layout {:?}", runs.iter().map(|r| (r.text, r.x, r.y, r.measured_width, observed.borrow().1)).collect::<Vec<_>>()
                    );
                }
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn emoji_click_keeps_images() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        install(&ui);
        let mut row = rich_fixture().remove(1);
        row.reply_id = "".into();
        let runs = row.rich_runs.clone();
        let model = &runs;
        let emoji_index = runs.iter().position(|r| r.emoji).unwrap();
        let mut emoji = model.row_data(emoji_index).unwrap();
        let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(8, 8);
        buffer.make_mut_slice().fill(slint::Rgb8Pixel {
            r: 227,
            g: 0,
            b: 113,
        });
        emoji.picture = slint::Image::from_rgb8(buffer.clone());
        model.set_row_data(emoji_index, emoji);
        ui.global::<ConversationState>()
            .set_messages(ModelRc::new(VecModel::from(vec![row])));
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.global::<SettingsState>().set_text_size(text_size);
                let before = render(&window, size);
                let red: Vec<_> = before
                    .enumerate_pixels()
                    .filter(|(_, _, p)| p.0 == [227, 0, 113])
                    .map(|(x, y, _)| (x, y))
                    .collect();
                assert!(!red.is_empty(), "emoji did not paint");
                let position = slint::LogicalPosition::new(
                    red[red.len() / 2].0 as f32,
                    red[red.len() / 2].1 as f32,
                );
                click_at(&ui, position);
                let after = render(&window, size);
                assert_eq!(
                    after.pixels().filter(|p| p.0 == [227, 0, 113]).count(),
                    red.len(),
                    "single click entered emoji fallback mode"
                );
            }
        }
        let mut updated = model.row_data(emoji_index).unwrap();
        buffer.make_mut_slice().fill(slint::Rgb8Pixel {
            r: 111,
            g: 0,
            b: 227,
        });
        updated.picture = slint::Image::from_rgb8(buffer);
        model.set_row_data(emoji_index, updated);
        let updated = render(&window, (360, 520));
        assert!(
            updated.pixels().any(|p| p.0 == [111, 0, 227]),
            "asset update did not repaint the measured flow"
        );
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn outgoing_selection_contrasts_in_both_color_schemes() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.global::<ConversationState>()
            .set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                id: "selection".into(),
                body: "Select this message".into(),
                outgoing: true,
                ..Default::default()
            }])));
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.global::<SettingsState>().set_text_size(24);
        ui.show().unwrap();
        for (theme, scheme, ink, selection) in [
            (
                "light",
                slint::language::ColorScheme::Light,
                [17, 17, 17],
                [0, 69, 204],
            ),
            (
                "dark",
                slint::language::ColorScheme::Dark,
                [236, 236, 236],
                [138, 197, 255],
            ),
        ] {
            ui.global::<Palette>().set_color_scheme(scheme);
            for size in [(980, 720), (360, 520)] {
                ui.global::<ConversationState>()
                    .set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                        id: format!("selection-{theme}-{}", size.0).into(),
                        body: "Select this message".into(),
                        outgoing: true,
                        ..Default::default()
                    }])));
                ui.invoke_focus_composer();
                let before = render(&window, size);
                let (x, y, _) = before
                    .enumerate_pixels()
                    .find(|(x, y, p)| *y > 56 && (size.0 < 740 || *x > 342) && p.0 == ink)
                    .expect("outgoing text did not paint");
                let position = slint::LogicalPosition::new(x as f32, y as f32);
                click_at(&ui, position);
                for (text, pressed) in [
                    ("\u{11}", true),
                    ("a", true),
                    ("a", false),
                    ("\u{11}", false),
                ] {
                    ui.window().dispatch_event(if pressed {
                        slint::platform::WindowEvent::KeyPressed { text: text.into() }
                    } else {
                        slint::platform::WindowEvent::KeyReleased { text: text.into() }
                    });
                }
                let selected = render(&window, size);
                assert!(
                    selected.pixels().filter(|p| p.0 == selection).count() > 100,
                    "outgoing selection was not visible in {theme}"
                );
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn failure_marks_stay_beside_the_bubble() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.global::<SettingsState>().set_text_size(text_size);
                ui.global::<ConversationState>()
                    .set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                        id: "failed".into(),
                        body: "Meet at eleven.".into(),
                        outgoing: true,
                        recoverable: true,
                        delivery: DeliveryState::Rejected,
                        time: "12:01".into(),
                        clock: "12:01".into(),
                        ..Default::default()
                    }])));
                ui.global::<ConversationState>()
                    .invoke_show_history(0., false);
                let pixels = render(&window, size);
                let green: Vec<_> = pixels
                    .enumerate_pixels()
                    .filter(|(_, _, p)| p.0 == [214, 230, 255])
                    .map(|(x, y, _)| (x, y))
                    .collect();
                let left = green.iter().map(|p| p.0).min().unwrap();
                let top = green.iter().map(|p| p.1).min().unwrap();
                let bottom = green.iter().map(|p| p.1).max().unwrap();
                assert!(
                    pixels.enumerate_pixels().any(|(x, y, p)| x < left
                        && x + 24 >= left
                        && y >= top
                        && y <= bottom
                        && p.0 == [217, 48, 37]),
                    "failure mark must stay immediately left of an outgoing bubble"
                );
            }
        }
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn link_edges_preserve_closing_punctuation_at_every_width() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        for body in [
            "参照「https://example.org/a」。次は",
            "参照 https://example.org/a、次は",
            "参照 https://example.org/a」次は",
        ] {
            let mut row = MessageRow {
                body: body.into(),
                ..Default::default()
            };
            rich_content::links::decorate_message(&mut row);
            assert!(
                row.rich_runs
                    .iter()
                    .filter(|r| r.text.starts_with(['」', '。', '、']))
                    .all(|r| !r.break_before)
            );
            assert!(row.rich_runs.iter().any(|r| r.text == "https://"));
            for size in [15., 24.] {
                for width in [1, 40, 120, 240, 400, 600] {
                    let lines = layout(&ui, &row.rich_runs, width as f32, size);
                    assert!(lines.row_count() > 0);
                    let mut previous_y = -1.;
                    for run in lines.iter() {
                        if run.y != previous_y {
                            assert!(
                                !run.text.starts_with(['」', '。', '、']),
                                "closing punctuation at width {width}, size {size}: {}",
                                run.text
                            );
                            assert!(!run.text.starts_with(char::is_whitespace));
                        }
                        previous_y = run.y;
                        assert!(run.x + run.measured_width <= width as f32 + 0.01);
                    }
                }
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn oversized_links_share_the_last_line_and_measure_whole_segments() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let mut row = MessageRow {
            body: format!("https://example.org/{} after", "abcdefghij".repeat(8)).into(),
            ..Default::default()
        };
        rich_content::links::decorate_message(&mut row);
        let lines = layout(&ui, &row.rich_runs, 260., 24.);
        let links: Vec<_> = lines.iter().filter(|r| r.link_start).collect();
        assert!(links.len() > 3);
        assert_eq!(links.iter().filter(|r| r.link_first).count(), 1);
        assert!(
            links
                .iter()
                .all(|r| r.link_width == r.measured_width && r.measured_height == 36.)
        );
        let last = links.last().unwrap();
        let after = lines.iter().find(|r| r.text.contains("after")).unwrap();
        assert_eq!(
            last.y, after.y,
            "following text did not share the final link line"
        );
        assert!(after.x > 0.);
        let mut rounded = row.clone();
        rounded.body = "https://a.b.c.d.e.f.g.h.i.j.k.l/m?n=o&p=q。".into();
        rich_content::links::decorate_message(&mut rounded);
        let segments = layout(&ui, &rounded.rich_runs, 1000., 24.);
        let link = segments.iter().find(|r| r.link_start).unwrap();
        let plain = layout(
            &ui,
            &ModelRc::new(VecModel::from(vec![RichRun {
                text: link.text.clone(),
                ..Default::default()
            }])),
            1000.,
            24.,
        );
        assert!((link.measured_width - plain.row_data(0).unwrap().measured_width).abs() <= 1.);
        let punctuation = segments.iter().find(|r| r.text == "。").unwrap();
        assert_eq!(punctuation.x, link.x + link.link_width);
    })
    .join()
    .unwrap();
}

#[test]
fn first_frame_positions_emoji_runs_without_overlap() {
    for (width, height, text_size, emoji) in [
        (980, 720, 15, false),
        (980, 720, 15, true),
        (360, 520, 24, false),
        (360, 520, 24, true),
    ] {
        std::thread::spawn(move || {
            use slint::platform::WindowAdapter;
            let window = snapshot::install().unwrap();
            window.set_size(slint::PhysicalSize::new(width, height));
            let ui = create_app_window().unwrap();
            install(&ui);
            let observed = watch_layout(&ui);
            let mut row = if emoji {
                rich_fixture().remove(1)
            } else {
                MessageRow {
                    body: "Before https://example.org/a after words".into(),
                    ..Default::default()
                }
            };
            row.id = format!("first-frame-{emoji}").into();
            row.outgoing = true;
            rich_content::links::decorate_message(&mut row);
            ui.global::<ConversationState>().set_has_chat(true);
            ui.global::<ShellState>().set_show_conversation(true);
            ui.global::<SettingsState>().set_text_size(text_size);
            ui.global::<ConversationState>()
                .set_messages(ModelRc::new(VecModel::from(vec![row])));
            ui.show().unwrap();
            let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
            assert!(window.draw_if_needed(|renderer| {
                renderer.render(pixels.make_mut_slice(), width as usize);
            }));
            let rows: Vec<_> = observed.borrow().0.iter().filter(|r| !r.newline).collect();
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|r| r.measured_height > 0.));
            for (i, a) in rows.iter().enumerate() {
                for b in rows.iter().skip(i + 1) {
                    assert!(
                        a.x + a.measured_width <= b.x + 0.25
                            || b.x + b.measured_width <= a.x + 0.25
                            || a.y + a.measured_height <= b.y + 0.25
                            || b.y + b.measured_height <= a.y + 0.25,
                        "overlapping runs"
                    );
                }
            }
            let frame =
                image::RgbImage::from_raw(width, height, pixels.as_bytes().to_vec()).unwrap();
            let (left, right, top, bottom) = bubble_bounds(&frame);
            assert!(
                frame.enumerate_pixels().any(|(x, y, p)| x > left
                    && x < right
                    && y >= top
                    && y <= bottom
                    && p.0 == [17, 17, 17]),
                "first frame has no message ink"
            );
            let first = pixels.as_bytes().to_vec();
            slint::platform::update_timers_and_animations();
            window.window().request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(pixels.make_mut_slice(), width as usize);
            });
            assert_eq!(first, pixels.as_bytes(), "first frame moved after layout");
        })
        .join()
        .unwrap();
    }
}

#[test]
fn preview_and_history_measurements_are_independent_of_visit_order() {
    std::thread::spawn(|| {
        let _platform = crate::snapshot::install().unwrap();
        let ui = crate::window::create_app_window().unwrap();
        let rows = runs("hello world", &[]);
        let metrics = RefCell::new(Measurements::default());
        let cold = crate::rich_content::flow::layout(&ui, &rows, 500., 24., &metrics);
        let raw = ui.invoke_measure_rich("hello ".into(), 24.);
        let cached = metrics
            .borrow_mut()
            .get(&("hello ".into(), 24f32.to_bits()))
            .unwrap();
        assert_eq!(cached, (raw.width, raw.height));
        let warmed = RefCell::new(Measurements::default());
        crate::rich_content::preview::layout(&rows, "", 500., 24., |text| {
            crate::rich_content::measure(&ui, &warmed, text, 24.).0
        });
        let warm = crate::rich_content::flow::layout(&ui, &rows, 500., 24., &warmed);
        assert_eq!(
            cold.iter()
                .map(|r| (r.y, r.measured_height))
                .collect::<Vec<_>>(),
            warm.iter()
                .map(|r| (r.y, r.measured_height))
                .collect::<Vec<_>>()
        );
    })
    .join()
    .unwrap();
}

#[test]
fn link_runs_keep_the_body_break_opportunities_and_capped_fallback() {
    let linked = "参照「https://example.org/a」。again https://example.org/a and www.example.jp";
    let words = "Before https://example.org/a?b=c after words 日本語 https://www.example.jp";
    let oversized = format!("https://example.org/{}", "a/".repeat(2049));
    for (body, fallback, links) in [
        (linked, false, 3),
        (words, false, 2),
        (oversized.as_str(), false, 1),
        ("https://example.org/", true, 1),
    ] {
        let mut row = MessageRow {
            body: body.into(),
            rich_fallback: fallback,
            ..Default::default()
        };
        for _ in 0..2 {
            rich_content::links::decorate_message(&mut row);
            assert_eq!(row.body, body);
            assert_eq!(row.links.row_count(), links);
            if fallback || body == oversized {
                assert!(row.rich_fallback);
                assert_eq!(row.rich_runs.row_count(), 0);
            } else {
                let rows: Vec<_> = row.rich_runs.iter().collect();
                assert!(rows.len() < 25);
                assert_eq!(
                    rows.iter().map(|r| r.text.as_str()).collect::<String>(),
                    body
                );
                assert!(
                    row.links
                        .iter()
                        .all(|link| rows.iter().any(|r| r.url == link.url))
                );
                if body == words {
                    for text in ["after ", "words ", "日", "example."] {
                        assert!(rows.iter().any(|r| r.text == text));
                    }
                }
            }
        }
    }
}

#[test]
fn rich_selection_copies_source_breaks_and_whole_emoji() {
    std::thread::spawn(|| {
        let _window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        let source = rich_fixture().remove(1).rich_runs;
        let lines = layout(&ui, &source, 135., 18.);
        let text: String = source.iter().map(|r| r.text.to_string()).collect();
        let point = |run, char| RichPoint { run, char };
        let selection = |anchor, focus| RichSelection { anchor, focus };
        let end = point(lines.row_count() as i32 - 1, i32::MAX);
        assert_eq!(
            rich_content::selection::copy(&source, &lines, selection(point(0, 0), end.clone())),
            text
        );
        for body in ["  before\n\nafter \n", "averylongunbrokenword"] {
            let source = runs(body, &[]);
            let wrapped = layout(&ui, &source, 65., 18.);
            let all = selection(point(0, 0), point(wrapped.row_count() as i32 - 1, i32::MAX));
            assert_eq!(rich_content::selection::copy(&source, &wrapped, all), body);
            if !body.contains(' ') {
                let word = rich_content::selection::word(&source, &wrapped, point(1, 1));
                assert_eq!(rich_content::selection::copy(&source, &wrapped, word), body);
            } else {
                let blank = wrapped.iter().position(|run| run.newline).unwrap();
                let range = selection(
                    point(blank as i32, 0),
                    point(wrapped.row_count() as i32 - 1, i32::MAX),
                );
                assert_eq!(
                    rich_content::selection::copy(&source, &wrapped, range),
                    "\nafter \n"
                );
            }
        }
        let emoji = lines.iter().position(|r| r.emoji).unwrap();
        let run = lines.row_data(emoji).unwrap();
        let left =
            rich_content::selection::hit(&lines, run.x + 1., run.y + 1., false, 135., |_| {
                panic!("emoji must not measure text")
            });
        let right = rich_content::selection::hit(
            &lines,
            run.x + run.measured_width - 1.,
            run.y + 1.,
            false,
            135.,
            |_| panic!("emoji must not measure text"),
        );
        assert_eq!(
            rich_content::selection::copy(&source, &lines, selection(left.clone(), right.clone())),
            "[笑]"
        );
        assert_eq!(
            rich_content::selection::copy(&source, &lines, selection(right, left)),
            "[笑]"
        );
        let across = selection(point(emoji as i32, 0), end);
        assert_eq!(
            rich_content::selection::copy(&source, &lines, across),
            &text[text.find("[笑]").unwrap()..]
        );
        let word_rows = ModelRc::new(VecModel::from(vec![RichRun {
            text: "hello café 日本語".into(),
            ..Default::default()
        }]));
        let word = rich_content::selection::word(&word_rows, &word_rows, point(0, 8));
        assert_eq!(
            rich_content::selection::copy(&word_rows, &word_rows, word),
            "café"
        );
        let word = rich_content::selection::word(&source, &lines, point(emoji as i32, 1));
        assert_eq!(rich_content::selection::copy(&source, &lines, word), "[笑]");
    })
    .join()
    .unwrap();
}

#[test]
fn dragging_link_and_emoji_copies_only_the_range_and_keeps_images() {
    std::thread::spawn(|| {
        use slint::platform::{PointerEventButton, WindowEvent};
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        install(&ui);
        let observed_layout = watch_layout(&ui);
        let body = "https://example.org [笑] after";
        let mut row = rich_fixture().remove(1);
        let mut emoji = row.rich_runs.iter().find(|r| r.emoji).unwrap();
        emoji.text = "[笑]".into();
        row.body = body.into();
        row.rich_runs = ModelRc::new(VecModel::from(vec![
            RichRun {
                text: "https://example.org ".into(),
                ..Default::default()
            },
            emoji,
            RichRun {
                text: " after".into(),
                ..Default::default()
            },
        ]));
        rich_content::links::decorate_message(&mut row);
        let copied = Rc::new(RefCell::new(String::new()));
        let observed = copied.clone();
        ui.global::<ContentState>()
            .on_copy_text(move |text| *observed.borrow_mut() = text.to_string());
        let opened = Rc::new(RefCell::new(String::new()));
        let observed = opened.clone();
        ui.global::<ContentState>()
            .on_open_link(move |url| *observed.borrow_mut() = url.to_string());
        let messages = Rc::new(VecModel::from(vec![row]));
        ui.global::<ConversationState>()
            .set_messages(ModelRc::from(messages.clone()));
        ui.global::<ConversationState>().set_has_chat(true);
        ui.global::<ShellState>().set_show_conversation(true);
        ui.show().unwrap();
        let pixels = render(&window, (980, 720));
        let (ex, ey, _) = pixels
            .enumerate_pixels()
            .find(|(_, _, p)| p.0 == [0, 180, 100])
            .expect("emoji missing");
        let (_, ly, _) = pixels
            .enumerate_pixels()
            .find(|(x, y, p)| *x > 342 && *y > 56 && p.0 == [0, 69, 204])
            .expect("link missing");
        let emoji_x = observed_layout
            .borrow()
            .0
            .iter()
            .find(|run| run.emoji)
            .unwrap()
            .x;
        let start = slint::LogicalPosition::new(ex as f32 - emoji_x + 1., ly as f32);
        let prefix_end = slint::LogicalPosition::new(start.x + 45., start.y);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: prefix_end,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: prefix_end,
            button: PointerEventButton::Left,
        });
        let prefix = render(&window, (980, 720));
        assert!(
            (start.y as u32..start.y as u32 + 15).any(|y| {
                (start.x as u32..prefix_end.x as u32).any(|x| {
                    prefix.get_pixel(x, y).0 == [255, 255, 255]
                        && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                            .iter()
                            .any(|&(x, y)| prefix.get_pixel(x, y).0 == [0, 69, 204])
                })
            }),
            "partial link selection did not paint contrasting text"
        );
        press(&ui, slint::platform::Key::Escape);
        ui.global::<ConversationState>().set_following(true);
        click_at(&ui, start);
        assert_eq!(*opened.borrow(), "https://example.org");
        opened.borrow_mut().clear();
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        let end = slint::LogicalPosition::new(ex as f32 + 23., ey as f32 + 4.);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
        chord(&ui, slint::platform::Key::Control, "c".into());
        assert_eq!(*copied.borrow(), "https://example.org [笑]");
        assert!(opened.borrow().is_empty(), "drag opened a link");
        assert!(ui.global::<ConversationState>().get_history_interacting());
        assert!(!ui.global::<ConversationState>().get_following());
        messages.push(MessageRow {
            id: "arrival".into(),
            body: "new message".into(),
            ..Default::default()
        });
        let selected = render(&window, (980, 720));
        chord(&ui, slint::platform::Key::Control, "c".into());
        assert_eq!(*copied.borrow(), "https://example.org [笑]");
        assert!(
            selected.pixels().any(|p| p.0 == [0, 180, 100]),
            "selection removed emoji"
        );
        press(&ui, slint::platform::Key::Escape);
        assert!(!ui.global::<ConversationState>().get_history_interacting());
        let after = observed_layout
            .borrow()
            .0
            .iter()
            .find(|run| run.text.contains("after"))
            .unwrap();
        let emoji = observed_layout
            .borrow()
            .0
            .iter()
            .find(|run| run.emoji)
            .unwrap();
        let word_position =
            slint::LogicalPosition::new(ex as f32 - emoji.x + after.x + 15., ey as f32 + 4.);
        click_at(&ui, word_position);
        click_at(&ui, word_position);
        chord(&ui, slint::platform::Key::Control, "c".into());
        assert_eq!(*copied.borrow(), "after");
        chord(&ui, slint::platform::Key::Control, "a".into());
        chord(&ui, slint::platform::Key::Control, "c".into());
        assert_eq!(*copied.borrow(), body);
        ui.invoke_focus_composer();
        render(&window, (980, 720));
        assert!(!ui.global::<ConversationState>().get_history_interacting());
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
