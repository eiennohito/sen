//! Decode display pixels with the file's orientation, leaving source bytes intact.
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::io::{BufRead, Seek};

pub fn decode<R: BufRead + Seek>(reader: ImageReader<R>) -> image::ImageResult<DynamicImage> {
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exif_orientation_is_applied_before_display_and_attachment_metadata() {
        let bytes = crate::test_support::oriented_jpeg();
        let decoded = decode(
            ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .unwrap(),
        )
        .unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 3));
        // Orientation 6 rotates the red left column to the top row.
        assert!(decoded.to_rgb8().get_pixel(0, 0)[0] > decoded.to_rgb8().get_pixel(0, 0)[2]);
        let root = std::env::temp_dir().join(format!("line-exif-{}.jpg", std::process::id()));
        std::fs::write(&root, &bytes).unwrap();
        let item = crate::attachments::Attachment::prepare(&root).unwrap();
        let photo = item.photo.as_ref().unwrap();
        assert_eq!((photo.width, photo.height), (2, 3));
        assert_eq!((photo.preview_width, photo.preview_height), (256, 384));
        assert_eq!(
            item.read().unwrap(),
            bytes,
            "uploads retain original EXIF and pixels"
        );
        std::fs::remove_file(root).unwrap();
    }
}
