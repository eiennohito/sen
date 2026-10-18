//! Opt-in release benchmark; fixture text matches the independent review probe.
use crate::*;
use std::time::Instant;

fn draw(window: &slint::platform::software_renderer::MinimalSoftwareWindow, width: u32) -> f64 {
    let started = Instant::now();
    window.set_size(slint::PhysicalSize::new(width, 720));
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, 720);
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), width as usize);
    });
    started.elapsed().as_secs_f64() * 1000.
}

#[test]
#[ignore = "release performance probe"]
fn distinct_message_performance() {
    std::thread::spawn(|| {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        rich_content::install(&ui, Rc::new(RefCell::new(View::default())));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        window.set_size(slint::PhysicalSize::new(980, 720));
        ui.show().unwrap();
        for words in [20, 60] {
            let rows = (0..12).map(|m| {
                let words = (0..words).map(|i| format!("word{m}x{i}")).collect::<Vec<_>>();
                let mut row = MessageRow {
                    id: format!("r{}-{m}", words.len()).into(),
                    body: format!("{} https://example.org/{m} tail", words.join(" ")).into(),
                    sort_key: format!("{m:020}").into(),
                    ..Default::default()
                };
                links::decorate_message(&mut row, "c");
                row
            }).collect::<Vec<_>>();
            ui.set_messages(ModelRc::new(VecModel::from(rows)));
            let first = draw(&window, 980);
            let resize = draw(&window, 700);
            let resize600 = draw(&window, 600);
            ui.set_text_size(18);
            let text_size = draw(&window, 600);
            ui.set_text_size(15);
            println!("PERF 12x{words}: first={first:.3}ms resize700={resize:.3}ms resize600={resize600:.3}ms text15to18={text_size:.3}ms");
        }
        ui.hide().unwrap();
    }).join().unwrap();
}

#[test]
#[ignore = "release performance probe"]
fn near_cap_distinct_performance() {
    for n in [1000, 1900] {
        std::thread::spawn(move || {
        let window = snapshot::install().unwrap();
        let ui = create_app_window().unwrap();
        rich_content::install(&ui, Rc::new(RefCell::new(View::default())));
        ui.set_has_chat(true);
        ui.set_show_conversation(true);
        window.set_size(slint::PhysicalSize::new(980, 720));
        ui.show().unwrap();
            let words = (0..n).map(|i| format!("w{i}x")).collect::<Vec<_>>();
            let mut row = MessageRow {
                id: format!("d{n}").into(),
                body: format!("{} https://example.org/x", words.join(" ")).into(),
                ..Default::default()
            };
            links::decorate_message(&mut row, "c");
            let runs = row.rich_runs.row_count();
            ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
            let first = draw(&window, 980);
            let resize900 = draw(&window, 900);
            let resize700 = draw(&window, 700);
            let resize701 = draw(&window, 701);
            println!("PERF {n} distinct words ({runs} runs): first={first:.3}ms resize900={resize900:.3}ms resize700={resize700:.3}ms resize701={resize701:.3}ms");
        ui.hide().unwrap();
    }).join().unwrap();
    }
}
