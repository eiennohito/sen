use anyhow::Result;
use image::{GrayImage, ImageFormat, Luma};
use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};
use std::io::Cursor;

const SCALE: i32 = 10;
const BORDER: i32 = 4;

fn encode(text: &str) -> Result<QrCode> {
    Ok(QrCode::encode_segments_advanced(
        &QrSegment::make_segments(text),
        QrCodeEcc::Low,
        Version::MIN,
        Version::MAX,
        None,
        false,
    )?)
}

pub(super) fn png(text: &str) -> Result<Vec<u8>> {
    let qr = encode(text)?;
    let size = ((qr.size() + BORDER * 2) * SCALE) as u32;
    let image = GrayImage::from_fn(size, size, |x, y| {
        let dark = qr.get_module(x as i32 / SCALE - BORDER, y as i32 / SCALE - BORDER);
        Luma([if dark { 0 } else { 255 }])
    });
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "line://au/q/synthetic-session?secret=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D&e2eeVersion=1";

    #[test]
    fn pairing_matrix_matches_golden() -> Result<()> {
        let qr = encode(URL)?;
        assert_eq!(qr.version().value(), 5);
        assert_eq!(qr.size(), 37);
        assert_eq!(qr.error_correction_level(), QrCodeEcc::Low);
        let mut matrix = String::new();
        for y in 0..qr.size() {
            for x in 0..qr.size() {
                matrix.push(if qr.get_module(x, y) { '#' } else { '.' });
            }
            matrix.push('\n');
        }
        // Generated once with qrcodegen 1.8.0 from URL above, automatic mask, EC Low.
        assert_eq!(matrix, include_str!("../../fixtures/pairing-qr.txt"));
        Ok(())
    }

    #[test]
    fn png_preserves_modules_and_quiet_zone() -> Result<()> {
        let qr = encode(URL)?;
        let bytes = png(URL)?;
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let image = image::load_from_memory(&bytes)?.to_luma8();
        let width = qr.size() + BORDER * 2;
        assert_eq!(
            image.dimensions(),
            ((width * SCALE) as u32, (width * SCALE) as u32)
        );
        for (x, y, pixel) in image.enumerate_pixels() {
            let dark = qr.get_module(x as i32 / SCALE - BORDER, y as i32 / SCALE - BORDER);
            assert_eq!(pixel.0[0], if dark { 0 } else { 255 });
        }
        Ok(())
    }

    #[test]
    fn oversized_input_returns_an_error() {
        let text = "x".repeat(3000);
        assert!(png(&text).is_err());
    }
}
