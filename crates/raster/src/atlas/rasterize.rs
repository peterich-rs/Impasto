//! CPU glyph bitmap generation shared by synchronous replay and preparation workers.

use std::collections::HashMap;

use engine::scene::FontFaceId;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Angle, Fill, Format, Style, Transform, Vector};
use swash::{CacheKey, FontRef, GlyphId};

use super::{GlyphKey, SUBPIXEL_BUCKETS};
use crate::fonts::FontLibrary;

#[derive(Default)]
pub(super) struct GlyphRasterizer {
    context: ScaleContext,
    font_keys: HashMap<FontFaceId, CacheKey>,
}

impl GlyphRasterizer {
    pub(super) fn clear_font_keys(&mut self) {
        self.font_keys.clear();
    }

    pub(super) fn glyph(&mut self, library: &FontLibrary, key: &GlyphKey) -> Option<Bitmap> {
        library.with_face(&key.face, |bytes, index| self.rasterize(bytes, index, key))?
    }

    pub(super) fn rasterize(&mut self, bytes: &[u8], index: u32, key: &GlyphKey) -> Option<Bitmap> {
        let mut font = FontRef::from_index(bytes, index as usize)?;
        if self.font_keys.len() >= 1024 {
            self.font_keys.clear();
        }
        font.key = *self.font_keys.entry(*key.face).or_insert(font.key);
        let ppem = key.variant.size as f32 / 64.0;
        let mut scaler = self
            .context
            .builder(font)
            .size(ppem)
            .hint(key.variant.hint)
            .normalized_coords(key.coords.as_deref().unwrap_or_default().iter().copied())
            .build();
        let offset = Vector::new(key.variant.subpixel as f32 / SUBPIXEL_BUCKETS as f32, 0.0);
        let id = GlyphId::from(key.glyph);
        // Color sources first: emoji and COLR glyphs keep their own pixels.
        let mut image = swash::scale::image::Image::new();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            Source::Bitmap(swash::scale::StrikeWith::ExactSize),
        ]);
        render.format(Format::Alpha).offset(offset);
        if render.render_into(&mut scaler, id, &mut image)
            && image.content == swash::scale::image::Content::Color
        {
            return Some(Bitmap::Color(image));
        }
        let mut render = Render::new(&[
            Source::Outline,
            Source::Bitmap(swash::scale::StrikeWith::ExactSize),
        ]);
        render
            .format(Format::Alpha)
            .style(Style::Fill(Fill::NonZero))
            .offset(offset);
        if key.embolden {
            render.embolden(ppem / 24.0);
        } else if key.variant.darken_stem {
            render.embolden(0.12);
        }
        if let Some(skew) = key.skew.map(f32::from_bits) {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(-skew),
                Angle::ZERO,
            )));
        }
        if render.render_into(&mut scaler, id, &mut image) {
            return Some(Bitmap::Mask(image));
        }
        // No outline: whitespace or a missing glyph. Cache an empty entry.
        Some(Bitmap::Empty)
    }
}

pub(super) enum Bitmap {
    Empty,
    Mask(swash::scale::image::Image),
    Color(swash::scale::image::Image),
}

impl Bitmap {
    pub(super) fn bytes(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Mask(image) | Self::Color(image) => image.data.len(),
        }
    }
}
