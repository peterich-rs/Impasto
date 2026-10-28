use super::ffi;
use super::*;

impl<'a> crate::painter::Painter<'a> {
    pub(crate) fn draw_image<'pixels>(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: &str,
        mode: ContentMode,
        required: bool,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        if let Some(cached) = self.textures.get_mut(source) {
            cached.used = self.typography.generation();
            let texture = cached.texture;
            let width = cached.width;
            let height = cached.height;
            return self.blit_texture(builder, texture, x, y, w, h, width, height, mode);
        }
        let Some(image) = resolve(source) else {
            if required {
                return false;
            }
            return self.paint_missing_image(builder, x, y, w, h);
        };
        if image.width == 0 || image.height == 0 {
            return true;
        }
        let Some(bytes) = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return false;
        };
        if image.rgba.len() < bytes {
            return false;
        }
        let descriptor = ffi::ImpellerTextureDescriptor {
            pixel_format: ffi::ImpellerPixelFormat::kImpellerPixelFormatRGBA8888,
            size: ffi::ImpellerISize {
                width: image.width as i64,
                height: image.height as i64,
            },
            mip_count: 1,
        };
        let mapping = ffi::ImpellerMapping {
            data: image.rgba.as_ptr(),
            length: bytes as u64,
            on_release: None,
        };
        // SAFETY: `mapping` borrows `image.rgba` until TextureCreate returns. Impeller copies
        // the pixels. The texture handle stays in `textures` until it is evicted or dropped.
        let texture = unsafe {
            ffi::ImpellerTextureCreateWithContentsNew(
                self.impeller,
                &descriptor,
                &mapping,
                ptr::null_mut(),
            )
        };
        if texture.is_null() {
            return false;
        }
        let width = image.width as f32;
        let height = image.height as f32;
        self.textures.insert(
            source.to_owned(),
            crate::textures::CachedTexture {
                texture,
                width,
                height,
                used: self.typography.generation(),
            },
        );
        self.blit_texture(builder, texture, x, y, w, h, width, height, mode)
    }

    /// A visible stand-in for an optional image whose bytes are not available.
    pub(crate) fn paint_missing_image(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> bool {
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return true;
        }
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return false;
        }
        paint_color(paint, [176, 72, 64, 255]);
        unsafe {
            ffi::ImpellerPaintSetDrawStyle(paint, ffi::ImpellerDrawStyle::kImpellerDrawStyleFill);
        }
        self.fill_rect(builder, paint, x, y, w, h, 4.0);
        paint_color(paint, [96, 32, 28, 255]);
        self.set_stroke(paint, 2.0);
        unsafe {
            ffi::ImpellerPaintSetDrawStyle(paint, ffi::ImpellerDrawStyle::kImpellerDrawStyleStroke);
        }
        self.fill_rect(builder, paint, x, y, w, h, 4.0);
        unsafe { ffi::ImpellerPaintRelease(paint) };
        true
    }

    pub(crate) fn blit_texture(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        texture: ffi::ImpellerTexture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        width: f32,
        height: f32,
        mode: ContentMode,
    ) -> bool {
        let Ok(pairs) = slice_rects(x, y, w, h, width, height, mode) else {
            return false;
        };
        for (src, dst) in pairs {
            // SAFETY: `texture` is owned by `self.textures` for at least this submit.
            unsafe {
                ffi::ImpellerDisplayListBuilderDrawTextureRect(
                    builder,
                    texture,
                    &src,
                    &dst,
                    ffi::ImpellerTextureSampling::kImpellerTextureSamplingLinear,
                    ptr::null_mut(),
                );
            }
        }
        true
    }
}
