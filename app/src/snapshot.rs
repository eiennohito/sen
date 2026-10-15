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

pub fn save(window: &MinimalSoftwareWindow, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    const WIDTH: u32 = 980;
    const HEIGHT: u32 = 720;
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(WIDTH, HEIGHT);
    if !window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), WIDTH as usize);
    }) {
        return Err("snapshot did not render".into());
    }
    let mut file = File::create(path)?;
    write!(file, "P6\n{WIDTH} {HEIGHT}\n255\n")?;
    file.write_all(pixels.as_bytes())?;
    Ok(())
}
