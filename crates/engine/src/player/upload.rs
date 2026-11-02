//! Texture upload. The graphics context can create textures off the render thread.
//! `on_release` stays null so Impeller copies the pixels before the call returns.

use std::ptr;

use super::ffi;

/// One retained Impeller context for the `io` thread.
///
/// Metal and Vulkan contexts have no thread restriction for texture creation.
/// This retain outlives the engine's own context reference.
pub struct TextureFactory {
    context: ffi::ImpellerContext,
}

// SAFETY: Impeller documents non-OpenGL contexts as usable from other threads
// for texture creation. The factory only calls `ImpellerTextureCreateWithContentsNew`.
unsafe impl Send for TextureFactory {}

impl Drop for TextureFactory {
    fn drop(&mut self) {
        // SAFETY: `context` is the extra retain from `Engine::texture_factory`.
        unsafe { ffi::ImpellerContextRelease(self.context) };
        self.context = ptr::null_mut();
    }
}

impl TextureFactory {
    /// Copies `rgba` into a new texture. The slice must live until this returns.
    pub fn upload_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> Option<GpuImage> {
        if self.context.is_null() || width == 0 || height == 0 {
            return None;
        }
        let bytes = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if rgba.len() < bytes {
            return None;
        }
        let descriptor = ffi::ImpellerTextureDescriptor {
            pixel_format: ffi::ImpellerPixelFormat::kImpellerPixelFormatRGBA8888,
            size: ffi::ImpellerISize {
                width: width as i64,
                height: height as i64,
            },
            mip_count: 1,
        };
        let mapping = ffi::ImpellerMapping {
            data: rgba.as_ptr(),
            length: bytes as u64,
            on_release: None,
        };
        // SAFETY: `mapping` borrows `rgba` until create returns. Impeller copies.
        // The returned texture is owned by `GpuImage`.
        let texture = unsafe {
            ffi::ImpellerTextureCreateWithContentsNew(
                self.context,
                &descriptor,
                &mapping,
                ptr::null_mut(),
            )
        };
        if texture.is_null() {
            return None;
        }
        Some(GpuImage {
            texture,
            width,
            height,
        })
    }
}

/// A texture created on `io`. The render thread retains it into the frame cache.
pub struct GpuImage {
    pub(crate) texture: ffi::ImpellerTexture,
    pub width: u32,
    pub height: u32,
}

// SAFETY: Impeller textures are documented thread-safe. Drop releases one reference.
unsafe impl Send for GpuImage {}

impl std::fmt::Debug for GpuImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GpuImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Drop for GpuImage {
    fn drop(&mut self) {
        // SAFETY: `texture` came from `ImpellerTextureCreateWithContentsNew`.
        unsafe { ffi::ImpellerTextureRelease(self.texture) };
        self.texture = ptr::null_mut();
    }
}

impl super::Engine {
    /// Extra context retain for background texture upload. Null context is `None`.
    pub fn texture_factory(&self) -> Option<TextureFactory> {
        if self.context.is_null() {
            return None;
        }
        // SAFETY: `context` is the engine's live context. Retain balances the
        // factory's drop. Release is a no-op on null, but we already checked.
        unsafe { ffi::ImpellerContextRetain(self.context) };
        Some(TextureFactory {
            context: self.context,
        })
    }

    /// Puts an already-uploaded texture where submit will blit it.
    /// A cache hit does not retain again.
    pub fn install_uploaded(&mut self, source: &str, image: &GpuImage) {
        if self.textures.contains(source) || image.texture.is_null() {
            return;
        }
        // SAFETY: `image.texture` is live. The store's copy is released on evict.
        unsafe { ffi::ImpellerTextureRetain(image.texture) };
        self.textures.insert(
            source.to_owned(),
            crate::textures::CachedTexture {
                texture: image.texture,
                width: image.width as f32,
                height: image.height as f32,
                used: 0,
            },
        );
    }
}
