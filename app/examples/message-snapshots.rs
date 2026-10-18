use std::{path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let output = target.join("ui-messages/review-r5-captures");
    std::fs::create_dir_all(&output)?;
    for case in ["long-url", "mixed", "cjk", "emoji"] {
        for (width, height) in [(360, 520), (980, 720), (1400, 900)] {
            for size in [15, 24] {
                let path = output.join(format!("{case}-{width}-{size}.ppm"));
                let status = Command::new(target.join("release/line-desktop"))
                    .args(["--demo", "--snapshot-case", case, "--snapshot"])
                    .arg(&path)
                    .args([
                        "--snapshot-size",
                        &format!("{width}x{height}"),
                        "--snapshot-text-size",
                        &size.to_string(),
                    ])
                    .status()?;
                if !status.success() {
                    return Err(format!("snapshot failed: {}", path.display()).into());
                }
                let bytes = std::fs::read(&path)?;
                let header = format!("P6\n{width} {height}\n255\n");
                let pixels =
                    image::RgbImage::from_raw(width, height, bytes[header.len()..].to_vec())
                        .ok_or("invalid snapshot pixels")?;
                let png = path.with_extension("png");
                pixels.save(&png)?;
                println!("{}", png.display());
            }
        }
    }
    Ok(())
}
