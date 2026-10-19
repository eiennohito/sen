// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use core::num::NonZeroU16;

use alloc::vec::Vec;
use skrifa::{MetadataProvider, raw::TableProvider};

use crate::PhysicalLength;
use crate::fixed::Fixed;
use i_slint_common::sharedfontique::fontique;
use i_slint_core::lengths::PhysicalPx;
use i_slint_core::textlayout::{Glyph, TextShaper};

use super::{RenderableVectorGlyph, VectorGlyphImage};

// A length in font design space.
struct FontUnit;
type FontLength = euclid::Length<i32, FontUnit>;
type FontScaleFactor = euclid::Scale<f32, FontUnit, PhysicalPx>;

/// Number of horizontal sub-pixel positions a glyph can be placed at. The
/// shaper produces sub-pixel accurate pen positions, but glyph bitmaps live on
/// the integer pixel grid; rendering each glyph at the nearest 1/N pixel bin
/// (instead of snapping the pen to a whole pixel) keeps inter-glyph spacing
/// even. 4 bins (quarter-pixel) is enough to remove the visible unevenness at
/// UI text sizes while keeping the glyph cache small.
pub(crate) const SUBPIXEL_BIN_COUNT: i32 = 4;

/// Cache key includes blob id, font index, pixel size, glyph id, a hash of normalized
/// variation coordinates (so different variable font instances produce distinct cache
/// entries), the horizontal sub-pixel bin, and the COLR foreground color.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    /// Font blob id.
    font_blob_id: u64,
    /// Font index within the blob.
    font_index: u32,
    /// Rendered pixel size.
    pixel_size: PhysicalLength,
    /// Glyph id.
    glyph_id: core::num::NonZeroU16,
    /// Hash of the normalized variation coordinates.
    coords_hash: u64,
    /// Horizontal sub-pixel bin.
    subpixel_bin: u8,
    foreground: [u8; 3],
}

struct RenderableGlyphWeightScale;

impl clru::WeightScale<GlyphCacheKey, RenderableVectorGlyph> for RenderableGlyphWeightScale {
    fn weight(&self, _: &GlyphCacheKey, value: &RenderableVectorGlyph) -> usize {
        match &value.image {
            VectorGlyphImage::AlphaMap(data) => data.len(),
            VectorGlyphImage::Rgba(image) => image.width() as usize * image.height() as usize * 4,
        }
    }
}

type GlyphCache = clru::CLruCache<
    GlyphCacheKey,
    RenderableVectorGlyph,
    std::collections::hash_map::RandomState,
    RenderableGlyphWeightScale,
>;

i_slint_core::thread_local!(static GLYPH_CACHE: core::cell::RefCell<GlyphCache>  =
    core::cell::RefCell::new(
        clru::CLruCache::with_config(
            clru::CLruCacheConfig::new(core::num::NonZeroUsize::new(1024 * 1024).unwrap())
                .with_scale(RenderableGlyphWeightScale)
        )
    )
);

pub struct VectorFont {
    font_index: u32,
    font_blob: fontique::Blob<u8>,
    swash_key: swash::CacheKey,
    swash_offset: u32,
    ascender: PhysicalLength,
    descender: PhysicalLength,
    height: PhysicalLength,
    pixel_size: PhysicalLength,
    x_height: PhysicalLength,
    cap_height: PhysicalLength,
    /// Normalized variation coordinates (F2Dot14, fvar axis order) for variable font rendering.
    normalized_coords: Vec<i16>,
    /// Hash of normalized_coords for use in the glyph cache key.
    coords_hash: u64,
    has_color_bitmaps: bool,
    has_colr: bool,
}

fn hash_coords(coords: &[i16]) -> u64 {
    use core::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    coords.hash(&mut hasher);
    hasher.finish()
}

impl VectorFont {
    fn swash_font_ref(&self) -> swash::FontRef<'_> {
        swash::FontRef {
            data: self.font_blob.data(),
            offset: self.swash_offset,
            key: self.swash_key,
        }
    }

    pub fn new(
        font: fontique::QueryFont,
        swash_key: swash::CacheKey,
        swash_offset: u32,
        pixel_size: PhysicalLength,
    ) -> Self {
        Self::new_from_blob_and_index(font.blob, font.index, swash_key, swash_offset, pixel_size)
    }

    pub fn new_from_blob_and_index(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        swash_key: swash::CacheKey,
        swash_offset: u32,
        pixel_size: PhysicalLength,
    ) -> Self {
        Self::new_from_blob_and_index_with_coords(
            font_blob,
            font_index,
            swash_key,
            swash_offset,
            pixel_size,
            &[],
        )
    }

    pub fn new_from_blob_and_index_with_coords(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        swash_key: swash::CacheKey,
        swash_offset: u32,
        pixel_size: PhysicalLength,
        normalized_coords: &[i16],
    ) -> Self {
        let face = skrifa::FontRef::from_index(font_blob.data(), font_index).unwrap();

        let skrifa_coords: Vec<skrifa::instance::NormalizedCoord> = normalized_coords
            .iter()
            .map(|&c| skrifa::instance::NormalizedCoord::from_bits(c))
            .collect();
        let location = skrifa::instance::LocationRef::new(&skrifa_coords);

        let metrics = face.metrics(skrifa::instance::Size::unscaled(), location);

        let ascender = FontLength::new(metrics.ascent as _);
        let descender = FontLength::new(metrics.descent as _);
        let height = FontLength::new((metrics.ascent - metrics.descent) as _);
        let x_height = FontLength::new(metrics.x_height.unwrap_or_default() as _);
        let cap_height = FontLength::new(metrics.cap_height.unwrap_or_default() as _);
        let units_per_em = metrics.units_per_em;
        let scale = FontScaleFactor::new(pixel_size.get() as f32 / units_per_em as f32);
        let coords_hash = hash_coords(normalized_coords);
        let swash_font =
            swash::FontRef { data: font_blob.data(), offset: swash_offset, key: swash_key };
        let has_color_bitmaps = swash_font.color_strikes().next().is_some();
        let has_colr = face.colr().is_ok();
        Self {
            font_index,
            font_blob,
            swash_key,
            swash_offset,
            ascender: (ascender.cast() * scale).cast(),
            descender: (descender.cast() * scale).cast(),
            height: (height.cast() * scale).cast(),
            pixel_size,
            x_height: (x_height.cast() * scale).cast(),
            cap_height: (cap_height.cast() * scale).cast(),
            normalized_coords: normalized_coords.to_vec(),
            coords_hash,
            has_color_bitmaps,
            has_colr,
        }
    }

    pub fn render_vector_glyph(
        &self,
        glyph_id: core::num::NonZeroU16,
        subpixel_bin: u8,
        foreground: [u8; 4],
        slint_context: &i_slint_core::SlintContext,
    ) -> Option<RenderableVectorGlyph> {
        // Bitmap strikes ignore the subpixel offset.
        let subpixel_bin = if self.has_color_bitmaps { 0 } else { subpixel_bin };
        GLYPH_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();

            let cache_key = GlyphCacheKey {
                font_blob_id: self.font_blob.id(),
                font_index: self.font_index,
                pixel_size: self.pixel_size,
                glyph_id,
                coords_hash: self.coords_hash,
                subpixel_bin,
                foreground: if self.has_colr {
                    [foreground[0], foreground[1], foreground[2]]
                } else {
                    [0; 3]
                },
            };

            if let Some(entry) = cache.get(&cache_key) {
                return Some(entry.clone());
            }

            let subpixel_offset_x = subpixel_bin as f32 / SUBPIXEL_BIN_COUNT as f32;

            let glyph = {
                let font_ref = self.swash_font_ref();
                let mut ctx = slint_context.swash_scale_context().borrow_mut();
                let mut scaler = ctx
                    .builder(font_ref)
                    .size(self.pixel_size.get() as f32)
                    .normalized_coords(&self.normalized_coords)
                    .build();
                let mut image = swash::scale::Render::new(&[
                    swash::scale::Source::ColorOutline(0),
                    swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
                    swash::scale::Source::Outline,
                ])
                .default_color(foreground)
                .format(swash::zeno::Format::Alpha)
                .offset(swash::zeno::Vector::new(subpixel_offset_x, 0.0))
                .render(&mut scaler, glyph_id.get())?;

                let placement = image.placement;
                let image = if image.content == swash::scale::image::Content::Color {
                    // Swash composites COLR layers premultiplied; decoded PNG bitmaps
                    // have straight alpha and need premultiplication for the texture path.
                    if matches!(image.source, swash::scale::Source::ColorBitmap(_)) {
                        for pixel in image.data.as_chunks_mut::<4>().0 {
                            for channel in 0..3 {
                                pixel[channel] =
                                    ((pixel[channel] as u16 * pixel[3] as u16 + 127) / 255) as u8;
                            }
                        }
                    }
                    VectorGlyphImage::Rgba(
                        i_slint_core::graphics::SharedImageBuffer::RGBA8Premultiplied(
                            i_slint_core::graphics::SharedPixelBuffer::clone_from_slice(
                                &image.data,
                                placement.width,
                                placement.height,
                            ),
                        ),
                    )
                } else {
                    VectorGlyphImage::AlphaMap(image.data.into())
                };

                Some(RenderableVectorGlyph {
                    x: Fixed::from_integer(placement.left),
                    y: Fixed::from_integer(placement.top - placement.height as i32),
                    width: PhysicalLength::new(placement.width.try_into().unwrap()),
                    height: PhysicalLength::new(placement.height.try_into().unwrap()),
                    image,
                    pixel_stride: placement.width.try_into().unwrap(),
                    glyph_origin_x: placement.left as f32,
                })
            };

            if let Some(ref glyph) = glyph {
                cache.put_with_weight(cache_key, glyph.clone()).ok();
            }
            glyph
        })
    }
}

impl TextShaper for VectorFont {
    type LengthPrimitive = i16;
    type Length = PhysicalLength;
    fn shape_text<GlyphStorage: core::iter::Extend<Glyph<PhysicalLength>>>(
        &self,
        text: &str,
        glyphs: &mut GlyphStorage,
    ) {
        let font_ref = self.swash_font_ref();
        let charmap = font_ref.charmap();
        let gm = font_ref.glyph_metrics(&[]);
        let metrics = font_ref.metrics(&[]);
        let scale = self.pixel_size.get() as f32 / metrics.units_per_em as f32;

        glyphs.extend(text.char_indices().map(|(byte_offset, char)| {
            let glyph_id = NonZeroU16::try_from(charmap.map(char)).ok();
            let x_advance = glyph_id.map_or_else(
                || self.pixel_size.get(),
                |id| (gm.advance_width(id.get()) * scale) as _,
            );

            Glyph {
                glyph_id,
                advance: PhysicalLength::new(x_advance),
                text_byte_offset: byte_offset,
                ..Default::default()
            }
        }));
    }

    fn glyph_for_char(&self, ch: char) -> Option<Glyph<PhysicalLength>> {
        let font_ref = self.swash_font_ref();
        let charmap = font_ref.charmap();
        let gm = font_ref.glyph_metrics(&[]);
        let metrics = font_ref.metrics(&[]);
        let scale = self.pixel_size.get() as f32 / metrics.units_per_em as f32;

        NonZeroU16::try_from(charmap.map(ch)).ok().map(|glyph_id| Glyph {
            glyph_id: Some(glyph_id),
            advance: PhysicalLength::new((gm.advance_width(glyph_id.get()) * scale) as _),
            ..Default::default()
        })
    }
}

impl i_slint_core::textlayout::FontMetrics<PhysicalLength> for VectorFont {
    fn ascent(&self) -> PhysicalLength {
        self.ascender
    }

    fn height(&self) -> PhysicalLength {
        self.height
    }

    fn descent(&self) -> PhysicalLength {
        self.descender
    }

    fn x_height(&self) -> PhysicalLength {
        self.x_height
    }

    fn cap_height(&self) -> PhysicalLength {
        self.cap_height
    }
}

impl super::GlyphRenderer for VectorFont {
    fn render_glyph(
        &self,
        glyph_id: core::num::NonZeroU16,
        slint_context: &i_slint_core::SlintContext,
    ) -> Option<super::RenderableGlyph> {
        // This alpha-only path does not support color fonts; draw_glyph_run handles them.
        if self.has_colr || self.has_color_bitmaps {
            return None;
        }
        self.render_vector_glyph(glyph_id, 0, [0, 0, 0, 255], slint_context).and_then(|glyph| {
            let VectorGlyphImage::AlphaMap(alpha_map) = glyph.image else {
                return None;
            };
            Some(super::RenderableGlyph {
                x: glyph.x,
                y: glyph.y,
                width: glyph.width,
                height: glyph.height,
                alpha_map: alpha_map.into(),
                pixel_stride: glyph.pixel_stride,
                sdf: false,
            })
        })
    }

    fn scale_delta(&self) -> super::Fixed<u16, 8> {
        super::Fixed::from_integer(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Platform;
    impl i_slint_core::platform::Platform for Platform {
        fn create_window_adapter(
            &self,
        ) -> Result<
            alloc::rc::Rc<dyn i_slint_core::platform::WindowAdapter>,
            i_slint_core::api::PlatformError,
        > {
            unreachable!("glyph rasterization does not create windows")
        }
    }

    #[test]
    fn bitmap_edges_are_premultiplied_and_subpixel_bins_share_one_cache_entry() {
        let context = i_slint_core::SlintContext::new(alloc::boxed::Box::new(Platform));
        let blob = fontique::Blob::new(std::sync::Arc::new(
            include_bytes!("../tests/fonts/sbix.ttf").to_vec(),
        ));
        let font_ref = swash::FontRef::from_index(blob.data(), 0).unwrap();
        let font = VectorFont::new_from_blob_and_index(
            blob.clone(),
            0,
            font_ref.key,
            font_ref.offset,
            PhysicalLength::new(20),
        );
        let id = NonZeroU16::new(font_ref.charmap().map('😀')).unwrap();
        let glyph = font.render_vector_glyph(id, 0, [0, 0, 0, 255], &context).unwrap();
        let entries = GLYPH_CACHE.with(|cache| cache.borrow().len());
        font.render_vector_glyph(id, 3, [0, 0, 0, 255], &context).unwrap();
        assert_eq!(GLYPH_CACHE.with(|cache| cache.borrow().len()), entries);
        let VectorGlyphImage::Rgba(i_slint_core::graphics::SharedImageBuffer::RGBA8Premultiplied(
            image,
        )) = glyph.image
        else {
            panic!("expected color bitmap")
        };
        assert!(image.as_bytes().as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255));
        for &[r, g, b, a] in image.as_bytes().as_chunks::<4>().0 {
            assert!(r <= a && g <= a && b <= a, "non-premultiplied pixel: {:?}", [r, g, b, a]);
        }
    }

    #[test]
    fn colr_layers_are_premultiplied_and_use_the_text_foreground() {
        let context = i_slint_core::SlintContext::new(alloc::boxed::Box::new(Platform));
        let blob = fontique::Blob::new(std::sync::Arc::new(
            include_bytes!("../tests/fonts/colr.ttf").to_vec(),
        ));
        let font_ref = swash::FontRef::from_index(blob.data(), 0).unwrap();
        let font = VectorFont::new_from_blob_and_index(
            blob.clone(),
            0,
            font_ref.key,
            font_ref.offset,
            PhysicalLength::new(20),
        );
        let id = NonZeroU16::new(font_ref.charmap().map('😀')).unwrap();
        for foreground in [[0, 0, 255, 255], [0, 255, 0, 255]] {
            let glyph = font.render_vector_glyph(id, 0, foreground, &context).unwrap();
            let VectorGlyphImage::Rgba(
                i_slint_core::graphics::SharedImageBuffer::RGBA8Premultiplied(image),
            ) = glyph.image
            else {
                panic!("COLR glyph must be a premultiplied RGBA image");
            };
            let pixels = image.as_bytes();
            let red = &pixels[(5 * image.width() as usize + 5) * 4..][..4];
            assert!(
                red[0] > 100 && red[0] <= red[3] && red[3] < 140,
                "red layer: {red:?}, size={}x{}",
                image.width(),
                image.height()
            );
            assert_eq!(&red[1..3], &[0, 0]);
            let text = &pixels[(5 * image.width() as usize + 15) * 4..][..4];
            for (actual, expected) in text.iter().zip(foreground) {
                assert!((i16::from(*actual) - i16::from(expected)).abs() <= 2);
            }
        }
    }
}
