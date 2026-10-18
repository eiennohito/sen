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

pub fn render(
    window: &MinimalSoftwareWindow,
    (width, height): (u32, u32),
) -> Result<SharedPixelBuffer<Rgb8Pixel>, Box<dyn std::error::Error>> {
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
    Ok(pixels)
}

pub fn save(
    window: &MinimalSoftwareWindow,
    path: &str,
    (width, height): (u32, u32),
) -> Result<(), Box<dyn std::error::Error>> {
    let pixels = render(window, (width, height))?;
    let mut file = File::create(path)?;
    write!(file, "P6\n{width} {height}\n255\n")?;
    file.write_all(pixels.as_bytes())?;
    Ok(())
}
