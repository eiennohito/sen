//! Headless software rendering for reviewing the sketch without a desktop.
use slint::{
    PhysicalSize, Rgb8Pixel, SharedPixelBuffer,
    platform::{
        Platform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{fs::File, io::Write, rc::Rc};

struct SnapshotPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for SnapshotPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

pub fn install() -> Result<Rc<MinimalSoftwareWindow>, Box<dyn std::error::Error>> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(SnapshotPlatform(window.clone())))?;
    Ok(window)
}

pub fn save(
    window: &MinimalSoftwareWindow,
    path: &str,
    (width, height): (u32, u32),
) -> Result<(), Box<dyn std::error::Error>> {
    window.set_size(PhysicalSize::new(width, height));
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(width, height);
    // Let one-shot scroll/layout restoration settle just as it does on the
    // desktop; keep the final buffer even if the last pass needs no repaint.
    let mut drawn = false;
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        drawn |= window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), width as usize);
        });
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if !drawn {
        return Err("snapshot did not render".into());
    }
    let mut file = File::create(path)?;
    write!(file, "P6\n{width} {height}\n255\n")?;
    file.write_all(pixels.as_bytes())?;
    Ok(())
}

pub(crate) fn message_case(
    name: &str,
) -> Result<Vec<crate::MessageRow>, Box<dyn std::error::Error>> {
    if name == "emoji" {
        let mut row = crate::demo::rich_fixture().remove(1);
        row.reply_id = "".into();
        return Ok(vec![row]);
    }
    let body = match name {
        "long-url" => format!("https://example.org/{}", "averylongunbrokenpath".repeat(8)),
        "mixed" => "Before https://www.example.jp after ordinary words. Next: https://example.org/a?b=c&name=one_two#part after.".into(),
        "cjk" => "日本語の前置き https://www.example.jp。続きの日本語と https://example.org/長い/道?名前=値。終わり".into(),
        _ => return Err(format!("unknown message snapshot case {name}").into()),
    };
    Ok(vec![crate::MessageRow {
        id: format!("snapshot-{name}").into(),
        body: body.into(),
        outgoing: true,
        time: "2026/10/17 12:00".into(),
        sort_key: crate::present::order_key(1_792_206_000_000).into(),
        ..Default::default()
    }])
}
