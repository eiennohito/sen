use anyhow::Result;
use image::{Rgba, RgbaImage};
use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};

const SCALE: i32 = 10;
const BORDER: i32 = 4;

pub(super) fn rgba(text: &str) -> Result<RgbaImage> {
    let qr = QrCode::encode_segments_advanced(
        &QrSegment::make_segments(text),
        QrCodeEcc::Low,
        Version::MIN,
        Version::MAX,
        None,
        false,
    )?;
    let size = ((qr.size() + BORDER * 2) * SCALE) as u32;
    Ok(RgbaImage::from_fn(size, size, |x, y| {
        let dark = qr.get_module(x as i32 / SCALE - BORDER, y as i32 / SCALE - BORDER);
        let value = if dark { 0 } else { 255 };
        Rgba([value, value, value, 255])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_matrix_matches_golden() -> Result<()> {
        let image = rgba(
            "line://au/q/synthetic-session?secret=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D&e2eeVersion=1",
        )?;
        let mut matrix = String::new();
        for y in 0..37 {
            for x in 0..37 {
                matrix.push(if image.get_pixel((x + 4) * 10, (y + 4) * 10)[0] == 0 {
                    '#'
                } else {
                    '.'
                });
            }
            matrix.push('\n');
        }
        assert_eq!(matrix, include_str!("../../fixtures/pairing-qr.txt"));
        Ok(())
    }
    #[test]
    fn module_count_and_quiet_zone() -> Result<()> {
        let image = rgba(
            "line://au/q/synthetic-session?secret=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D&e2eeVersion=1",
        )?;
        assert_eq!(image.dimensions(), (450, 450)); // 37 modules + four on each edge.
        for (x, y, pixel) in image.enumerate_pixels() {
            if !(40..410).contains(&x) || !(40..410).contains(&y) {
                assert_eq!(*pixel, Rgba([255; 4]));
            }
        }
        assert_eq!(*image.get_pixel(40, 40), Rgba([0, 0, 0, 255]));
        Ok(())
    }
}
