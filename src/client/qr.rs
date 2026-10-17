use anyhow::Result;
use image::{GrayImage, ImageFormat, Luma};
use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};
use std::io::{Cursor, Write};

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

pub(super) fn write_text(text: &str, out: &mut impl Write, colour: bool) -> Result<()> {
    let qr = encode(text)?;
    for y in (-BORDER..qr.size() + BORDER).step_by(2) {
        if colour {
            // True-colour foreground/background keep modules black and the entire
            // quiet zone white, independently of the terminal's theme palette.
            out.write_all(b"\x1b[38;2;0;0;0;48;2;255;255;255m")?;
        }
        for x in -BORDER..qr.size() + BORDER {
            // Plain output follows qrencode UTF8's dark-terminal polarity:
            // foreground blocks represent white space, including the quiet zone.
            let cell = match (
                qr.get_module(x, y) == colour,
                qr.get_module(x, y + 1) == colour,
            ) {
                (false, false) => " ",
                (true, false) => "▀",
                (false, true) => "▄",
                (true, true) => "█",
            };
            out.write_all(cell.as_bytes())?;
        }
        if colour {
            out.write_all(b"\x1b[0m")?;
        }
        writeln!(out)?;
    }
    out.flush()?;
    Ok(())
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
    fn text_has_positive_polarity_and_four_module_margin() -> Result<()> {
        let qr = encode(URL)?;
        let width = (qr.size() + 8) as usize;
        for colour in [false, true] {
            let mut text = Vec::new();
            write_text(URL, &mut text, colour)?;
            let text = String::from_utf8(text)?;
            assert_eq!(text.lines().count(), width.div_ceil(2));
            for (y, row) in text.lines().enumerate() {
                let row = if colour {
                    row.strip_prefix("\x1b[38;2;0;0;0;48;2;255;255;255m")
                        .unwrap()
                        .strip_suffix("\x1b[0m")
                        .unwrap()
                } else {
                    assert!(!row.contains('\x1b'));
                    row
                };
                let white = if colour { ' ' } else { '█' };
                assert_eq!(row.chars().count(), width);
                assert!(row.chars().take(4).all(|cell| cell == white));
                assert!(row.chars().skip(width - 4).all(|cell| cell == white));
                if y < 2 {
                    assert!(row.chars().all(|cell| cell == white));
                }
                for (x, cell) in row.chars().enumerate() {
                    let foreground = match cell {
                        ' ' => [false, false],
                        '▀' => [true, false],
                        '▄' => [false, true],
                        '█' => [true, true],
                        _ => panic!("unexpected QR character"),
                    };
                    for (offset, foreground) in foreground.into_iter().enumerate() {
                        let dark = foreground == colour;
                        let module_y = (y * 2 + offset) as i32 - 4;
                        assert_eq!(dark, qr.get_module(x as i32 - 4, module_y));
                        if module_y >= qr.size() {
                            assert!(!dark, "bottom quiet zone must be white");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn oversized_input_returns_an_error() {
        let text = "x".repeat(3000);
        assert!(png(&text).is_err());
        for colour in [false, true] {
            assert!(write_text(&text, &mut Vec::new(), colour).is_err());
        }
    }
}
