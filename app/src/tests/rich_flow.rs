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
            span: line::store::InlineEmoji {
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
    if let Some(runs) = row.rich_runs.as_any().downcast_ref::<VecModel<RichRun>>() {
        for i in 0..runs.row_count() {
            let mut run = runs.row_data(i).unwrap();
            if run.emoji {
                run.picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &[0, 180, 100, 255],
                    1,
                    1,
                ));
                run.has_picture = true;
                runs.set_row_data(i, run);
            }
        }
    }
    vec![MessageRow::default(), row]
}
slint::slint! {
    import { TextMetrics } from "../../ui/text-metrics.slint";
    export component MetricsProbe inherits Window {
        in-out property <string> sample;
        in-out property <float> size: 15;
        in-out property <float> measured-width;
        out property <float> plain-width: plain.preferred-width / 1px;
        background: #ffffff;
        metrics := TextMetrics {}
        public function measure() { root.measured-width = metrics.measure(root.sample, root.size).width; }
        public function measure-part(content: string) -> float { return metrics.measure(content, root.size).width; }
        plain := Text { x: 20px; y: 20px; width: parent.width - 40px; height: 40px; text: root.sample; font-size: root.size * 1px; color: #008c52; wrap: no-wrap; }
        Text { x: 20px; y: 100px; width: root.measured-width * 1px; height: 40px; text: root.sample; font-size: root.size * 1px; color: #008c52; wrap: no-wrap; overflow: clip; }
    }
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

fn watch_layout(ui: &AppWindow) -> Rc<RefCell<ModelRc<RichRun>>> {
    let observed = Rc::new(RefCell::new(ModelRc::default()));
    let recorded = observed.clone();
    let weak = ui.as_weak();
    let metrics = RefCell::new(Measurements::default());
    ui.global::<State>()
        .on_rich_layout(move |rows, width, size, previous| {
            let output = reuse(
                previous,
                crate::rich_content::flow::layout(&weak.unwrap(), &rows, width, size, &metrics),
            );
            *recorded.borrow_mut() = output.clone();
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
        if let Some(runs) = message
            .rich_runs
            .as_any()
            .downcast_ref::<VecModel<RichRun>>()
        {
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
fn measurement_lru_bounds_text_and_entries_and_separates_font_sizes() {
    let mut cache = Measurements::default();
    for i in 0..METRIC_ENTRIES {
        cache.insert(
            (format!("word{i}").into(), 15_f32.to_bits()),
            (i as f32, 20.),
        );
    }
    let oldest = ("word0".into(), 15_f32.to_bits());
    assert_eq!(cache.get(&oldest), Some((0., 20.)));
    cache.insert(("new".into(), 15_f32.to_bits()), (1., 20.));
    assert!(cache.get(&oldest).is_some());
    assert!(cache.get(&("word1".into(), 15_f32.to_bits())).is_none());
    assert!(cache.get(&("word0".into(), 18_f32.to_bits())).is_none());
    assert_eq!(cache.values.len(), METRIC_ENTRIES);
    assert_eq!(cache.recent.len(), METRIC_ENTRIES);
    cache.insert(
        ("x".repeat(METRIC_TEXT_BYTES).into(), 15_f32.to_bits()),
        (1., 20.),
    );
    assert_eq!(cache.text_bytes, METRIC_TEXT_BYTES);
    assert_eq!(cache.values.len(), 1);
    cache.insert(
        (
            "too large".repeat(METRIC_TEXT_BYTES).into(),
            15_f32.to_bits(),
        ),
        (1., 20.),
    );
    assert_eq!(cache.text_bytes, METRIC_TEXT_BYTES);
    assert_eq!(cache.values.len(), 1);
}

#[test]
fn public_scaled_metrics_match_painted_segments() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let probe = MetricsProbe::new().unwrap();
        probe.show().unwrap();
        let mut max_rounding = 0.0_f32;
        let mut max_ink = 0;
        let mut max_sum = 0.0_f32;
        let mut blank = 0;
        for sample in [
            "AVATAR office ffi", "To Wa fi fl ffi", "affinity waffle", "https://www.example.jp",
            "日本語の文字列", "漢字かなカナ。", "参照「リンク」、次は", "👩🏽‍💻", "👨‍👩‍👧‍👦", "🏳️‍🌈", "A👩🏽‍💻Z", "A👨‍👩‍👧‍👦Z", "A🏳️‍🌈Z",
            "한글 메시지", "한국어 테스트", "ภาษาไทยทดสอบ", "ข้อความภาษาไทย",
        ] {
            probe.set_sample(sample.into());
            let mut row = MessageRow { body: sample.into(), ..Default::default() };
            rich_content::links::decorate_message(&mut row);
            let pieces = if row.rich_runs.row_count() > 0 { row.rich_runs } else { runs(sample, &[]) };
            for size in [12., 15., 18., 22., 24.] {
                probe.set_size(size);
                probe.invoke_measure();
                let measured = probe.get_measured_width();
                let rounded = probe.get_plain_width();
                let error = rounded - measured;
                assert!((-0.001..=1.0).contains(&error), "{sample} at {size}: measured {measured}, public width {rounded}");
                max_rounding = max_rounding.max(error);
                let summed = pieces.iter().map(|run| probe.invoke_measure_part(run.text)).sum::<f32>();
                let delta = (summed - measured).abs();
                assert!(delta <= 1., "{sample} at {size}: summed widths differ by {delta}px");
                max_sum = max_sum.max(delta);
                probe.set_measured_width(summed);
                let pixels = render(&window, (980, 180));
                let ink = |top| {
                    let xs: Vec<_> = pixels.enumerate_pixels().filter(|(_, y, p)| *y >= top && *y < top + 40 && p.0 != [255, 255, 255]).map(|(x, _, _)| x).collect();
                    xs.iter().min().zip(xs.iter().max()).map(|(left, right)| (*left, *right))
                };
                let plain = ink(20);
                let clipped = ink(100);
                if plain.is_none() && clipped.is_none() { blank += 1; continue; }
                let (plain_left, plain_right) = plain.expect("native reference did not paint");
                let (clipped_left, clipped_right) = clipped.expect("measured segment lost all painted ink");
                let delta = (plain_right - plain_left).abs_diff(clipped_right - clipped_left);
                assert!(delta <= 1, "{sample} at {size}: measured segment clipped painted ink by {delta}px");
                max_ink = max_ink.max(delta);
            }
        }
        eprintln!("public64 metrics: max preferred-width rounding {max_rounding}px; max summed-width difference {max_sum}px; max painted ink difference {max_ink}px; {blank} native blank samples (17 strings × 5 sizes)");
        probe.hide().unwrap();
    }).join().unwrap();
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
        ui.global::<State>()
            .on_rich_layout(|_, _, _, _| ModelRc::default());
        ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.show().unwrap();
        let pixels = render(&window, (980, 720));
        let (x, y, _) = pixels
            .enumerate_pixels()
            .find(|(_, _, p)| p.0 == [17, 17, 17])
            .expect("fallback body did not paint");
        let position = slint::LogicalPosition::new(x as f32, y as f32);
        for pressed in [true, false] {
            ui.window().dispatch_event(if pressed {
                slint::platform::WindowEvent::PointerPressed {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                }
            } else {
                slint::platform::WindowEvent::PointerReleased {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                }
            });
        }
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
            selected.pixels().filter(|p| p.0 == [0, 135, 60]).count() > 100,
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
    let span = line::store::InlineEmoji {
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
    assert_eq!(runs(&"日".repeat(2049), &[]).row_count(), 0);
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
        let span = line::store::InlineEmoji {
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
        ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
            id: "stress".into(),
            body: body.into(),
            rich_runs: runs.clone(),
            ..Default::default()
        }])));
        ui.set_show_conversation(true);
        ui.show().unwrap();
        let started = std::time::Instant::now();
        render(&window, (980, 720));
        let cold = started.elapsed();
        let before = observed.borrow().iter().last().unwrap().y;
        let resized = std::time::Instant::now();
        assert!(before > 0.0, "native run measurements did not initialize");
        ui.set_text_size(24);
        render(&window, (360, 520));
        let last = observed.borrow().iter().last().unwrap();
        assert!(last.y > before, "narrow large text did not reflow");
        for run in observed.borrow().iter() {
            assert!(
                run.x + run.measured_width <= run.available + 1.0,
                "rich run overflowed the pane"
            );
        }
        eprintln!(
            "synthetic2048run cold: {cold:?}; resize: {:?}",
            resized.elapsed()
        );
        ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
fn render(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    size: (u32, u32),
) -> image::RgbImage {
    use slint::platform::WindowAdapter;
    window.set_size(slint::PhysicalSize::new(size.0, size.1));
    window.window().request_redraw();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.0, size.1);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), size.0 as usize);
        });
    }
    image::RgbImage::from_raw(size.0, size.1, pixels.as_bytes().to_vec()).unwrap()
}

#[test]
fn unbroken_bodies_and_inline_links_stay_inside_narrow_bubbles() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        install(&ui);
        let observed = watch_layout(&ui);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let recorded = opened.clone();
        ui.global::<State>()
            .on_open_link(move |url| recorded.borrow_mut().push(url));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.set_text_size(text_size);
                for linked in [false, true] {
                    let mut row = MessageRow {
                        id: "long-run".into(),
                        body: if linked {
                            format!(
                                "Before https://example.org/{} after",
                                "abcdefghij".repeat(8)
                            )
                        } else {
                            "abcdefghij".repeat(10)
                        }
                        .into(),
                        outgoing: true,
                        ..Default::default()
                    };
                    rich_content::links::decorate_message(&mut row);
                    ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
                    let pixels = render(&window, size);
                    let runs = observed.borrow().clone();
                    if linked {
                        for run in runs.iter() {
                            assert!(run.measured_height > 0.0);
                            assert!(
                                run.x + run.measured_width <= run.available + 0.25,
                                "inline URL overflowed"
                            );
                            if run.link_start {
                                assert!(run.x + run.link_width <= run.available + 0.25);
                            }
                        }
                        assert!(runs.row_count() < 20);
                        assert_eq!(runs.iter().filter(|r| r.link_first).count(), 1);
                    }
                    let green: Vec<_> = pixels
                        .enumerate_pixels()
                        .filter(|(_, _, p)| p.0 == [199, 242, 164])
                        .map(|(x, y, _)| (x, y))
                        .collect();
                    assert!(!green.is_empty());
                    let left = green.iter().map(|p| p.0).min().unwrap();
                    let right = green.iter().map(|p| p.0).max().unwrap();
                    if linked && size.0 == 980 && text_size == 24 {
                        let link = runs.iter().find(|r| r.link_start).unwrap();
                        let top = green.iter().map(|p| p.1).min().unwrap();
                        let position = slint::LogicalPosition::new(
                            left as f32 + 12.0 + link.x + link.link_width * 0.8,
                            top as f32 + 8.0 + link.y + link.measured_height / 2.0,
                        );
                        ui.window().dispatch_event(
                            slint::platform::WindowEvent::PointerPressed {
                                position,
                                button: slint::platform::PointerEventButton::Left,
                            },
                        );
                        ui.window().dispatch_event(
                            slint::platform::WindowEvent::PointerReleased {
                                position,
                                button: slint::platform::PointerEventButton::Left,
                            },
                        );
                        assert_eq!(
                            opened.borrow().last(),
                            Some(&link.url),
                            "inline link hit area did not cover its text"
                        );
                    }
                    let bubble_top = green.iter().map(|p| p.1).min().unwrap();
                    let bubble_bottom = green.iter().map(|p| p.1).max().unwrap();
                    let ink: Vec<_> = pixels
                        .enumerate_pixels()
                        .filter(|(x, y, p)| (size.0 < 740 || *x > 342) && *y >= bubble_top && *y <= bubble_bottom && p.0 == [17, 17, 17])
                        .map(|(x, y, _)| (x, y))
                        .collect();
                    assert!(!ink.is_empty());
                    assert!(
                        ink.iter().all(|p| p.0 > left && p.0 < right),
                        "body painted outside the bubble"
                    );
                    let top = green.iter().map(|p| p.1).min().unwrap();
                    let bottom = green.iter().map(|p| p.1).max().unwrap();
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
                                        && p.0 == [0, 135, 60]
                                })
                                .map(|(_, y, _)| y),
                        );
                    }
                    assert!(
                        lines.iter().max().unwrap() - lines.iter().min().unwrap()
                            > text_size as u32,
                        "long run did not wrap: {size:?} font {text_size} linked {linked}; layout {:?}", runs.iter().map(|r| (r.text, r.x, r.y, r.measured_width, r.available)).collect::<Vec<_>>()
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
        let model = runs.as_any().downcast_ref::<VecModel<RichRun>>().unwrap();
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
        ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.set_text_size(text_size);
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
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    });
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
        ui.set_demo(true);
        ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
            id: "selection".into(),
            body: "Select this message".into(),
            outgoing: true,
            ..Default::default()
        }])));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.set_text_size(24);
        ui.show().unwrap();
        for (theme, scheme, ink, selection) in [
            (
                "light",
                slint::language::ColorScheme::Light,
                [17, 17, 17],
                [0, 135, 60],
            ),
            (
                "dark",
                slint::language::ColorScheme::Dark,
                [236, 236, 236],
                [61, 220, 132],
            ),
        ] {
            ui.invoke_preview_dark(scheme == slint::language::ColorScheme::Dark);
            for size in [(980, 720), (360, 520)] {
                ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
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
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                        position,
                        button: slint::platform::PointerEventButton::Left,
                    });
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
fn inline_links_replace_matching_text_without_changing_the_body() {
    let mut row = MessageRow {
        body: "参照「https://example.org/a」。again https://example.org/a and www.example.jp"
            .into(),
        ..Default::default()
    };
    rich_content::links::decorate_message(&mut row);
    assert_eq!(
        row.rich_runs
            .iter()
            .map(|r| r.text.to_string())
            .collect::<String>(),
        row.body.as_str()
    );
    assert_eq!(row.links.row_count(), 3);
    for link in row.links.iter() {
        assert!(row.rich_runs.iter().any(|r| r.url == link.url));
    }
}

#[test]
fn failure_marks_stay_beside_the_bubble() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        ui.set_demo(true);
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        ui.show().unwrap();
        for size in [(980, 720), (360, 520)] {
            for text_size in [15, 24] {
                ui.set_text_size(text_size);
                ui.set_messages(ModelRc::new(VecModel::from(vec![MessageRow {
                    id: "failed".into(),
                    body: "Meet at eleven.".into(),
                    outgoing: true,
                    recoverable: true,
                    delivery: DeliveryState::Rejected,
                    time: "12:01".into(),
                    clock: "12:01".into(),
                    ..Default::default()
                }])));
                ui.invoke_show_history(0., false);
                let pixels = render(&window, size);
                let green: Vec<_> = pixels
                    .enumerate_pixels()
                    .filter(|(_, _, p)| p.0 == [199, 242, 164])
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
fn coarse_runs_keep_words_and_cjk_break_opportunities() {
    let body = "Before https://example.org/a?b=c after words 日本語 https://www.example.jp";
    let mut row = MessageRow {
        body: body.into(),
        ..Default::default()
    };
    rich_content::links::decorate_message(&mut row);
    let rows: Vec<_> = row.rich_runs.iter().collect();
    assert!(rows.len() < 25);
    assert!(rows.iter().any(|r| r.text == "after "));
    assert!(rows.iter().any(|r| r.text == "words "));
    assert!(rows.iter().any(|r| r.text == "日"));
    assert!(rows.iter().any(|r| r.text == "example."));
    assert_eq!(
        rows.iter().map(|r| r.text.as_str()).collect::<String>(),
        body
    );
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
fn pathological_link_bodies_keep_the_original_and_menu_links() {
    let body = format!("https://example.org/{}", "a/".repeat(2049));
    let mut row = MessageRow {
        body: body.clone().into(),
        ..Default::default()
    };
    rich_content::links::decorate_message(&mut row);
    assert_eq!(row.rich_runs.row_count(), 0);
    assert!(row.rich_fallback);
    assert_eq!(row.body, body);
    assert_eq!(row.links.row_count(), 1);
    rich_content::links::decorate_message(&mut row);
    assert_eq!(row.rich_runs.row_count(), 0);
    let mut row = MessageRow {
        body: "https://example.org/".into(),
        rich_fallback: true,
        ..Default::default()
    };
    rich_content::links::decorate_message(&mut row);
    assert_eq!(
        row.rich_runs.row_count(),
        0,
        "emoji fallback was rebuilt as an uncapped link flow"
    );
    assert_eq!(row.links.row_count(), 1);
}

#[test]
fn first_frame_never_paints_unpositioned_or_overlapping_runs() {
    for size in [(980, 720, 15), (360, 520, 24)] {
        for emoji in [false, true] {
            std::thread::spawn(move || {
                use slint::platform::WindowAdapter;
                let window = snapshot::install().unwrap();
                window.set_size(slint::PhysicalSize::new(size.0, size.1));
                let ui = create_app_window().unwrap();
                install(&ui);
                ui.set_has_chat(true);
                ui.set_show_conversation(true);
                ui.set_text_size(size.2);
                let mut row = if emoji {
                    rich_fixture().remove(1)
                } else {
                    MessageRow {
                        body: "Before https://example.org/a after words".into(),
                        ..Default::default()
                    }
                };
                row.outgoing = true;
                row.reply_id = "".into();
                rich_content::links::decorate_message(&mut row);
                let observed = watch_layout(&ui);
                ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
                ui.show().unwrap();
                let mut first_ink = Vec::new();
                for frame in 0..2 {
                    if frame > 0 {
                        slint::platform::update_timers_and_animations();
                    }
                    window.window().request_redraw();
                    let mut pixels =
                        slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.0, size.1);
                    window.draw_if_needed(|renderer| {
                        renderer.render(pixels.make_mut_slice(), size.0 as usize);
                    });
                    let ink: Vec<_> = pixels
                        .as_slice()
                        .iter()
                        .enumerate()
                        .filter(|(i, p)| {
                            i / size.0 as usize > 56
                                && (size.0 < 740 || i % size.0 as usize > 342)
                                && p.r == 17
                                && p.g == 17
                                && p.b == 17
                        })
                        .map(|(i, _)| i)
                        .collect();
                    assert!(!ink.is_empty(), "first frame has no message ink");
                    if frame == 0 {
                        first_ink = ink;
                    } else {
                        assert_eq!(ink, first_ink, "message ink moved after the first frame");
                    }
                    let rows: Vec<_> = observed.borrow().iter().collect();
                    let positioned = !rows.is_empty()
                        && rows.iter().all(|r| r.positioned && r.measured_height > 0.0);
                    if !positioned {
                        assert!(
                            !pixels
                                .as_slice()
                                .iter()
                                .any(|p| p.r == 17 && p.g == 17 && p.b == 17),
                            "first frame painted stacked runs"
                        );
                    } else {
                        for (i, a) in rows.iter().enumerate().filter(|(_, r)| !r.newline) {
                            for b in rows.iter().skip(i + 1).filter(|r| !r.newline) {
                                assert!(
                                    a.x + a.measured_width <= b.x + 0.25
                                        || b.x + b.measured_width <= a.x + 0.25
                                        || a.y + a.measured_height <= b.y + 0.25
                                        || b.y + b.measured_height <= a.y + 0.25,
                                    "first frame contains overlapping runs"
                                );
                            }
                        }
                    }
                    eprintln!(
                        "{}px emoji={emoji} frame {frame}: positioned={positioned}",
                        size.0
                    );
                    if !emoji || frame == 1 {
                        assert!(positioned, "layout added another unpositioned frame");
                    }
                }
                ui.hide().unwrap();
            })
            .join()
            .unwrap();
        }
    }
}

#[test]
fn metadata_labels_are_explicit() {
    assert_eq!(duration(125000), "2:05");
    assert_eq!(file_size(1024), "1.0 KB");
    assert_eq!(file_size(12), "12 B");
}
