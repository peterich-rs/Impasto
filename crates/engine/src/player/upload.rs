//! Texture upload. The graphics context can create textures off the render thread.
//! `on_release` stays null so Impeller copies the pixels before the call returns.

use std::ptr;
#[cfg(target_os = "android")]
use std::sync::Arc;

#[cfg(target_os = "android")]
use super::vulkan::VulkanProcs;

/// One retained Impeller context for the `io` thread.
///
/// Metal and Vulkan contexts have no thread restriction for texture creation.
/// This retain outlives the engine's own context reference.
pub struct TextureFactory {
    context: impeller::ImpellerContext,
    #[cfg(target_os = "android")]
    vulkan_procs: Option<Arc<VulkanProcs>>,
}

// SAFETY: Impeller documents non-OpenGL contexts as usable from other threads
// for texture creation. The factory only calls `ImpellerTextureCreateWithContentsNew`.
unsafe impl Send for TextureFactory {}

impl Drop for TextureFactory {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        let _guard = self.vulkan_procs.as_deref().map(VulkanProcs::current);
        // SAFETY: `context` is the extra retain from `Engine::texture_factory`.
        unsafe { impeller::ImpellerContextRelease(self.context) };
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
        let descriptor = impeller::ImpellerTextureDescriptor {
            pixel_format: impeller::ImpellerPixelFormat::kImpellerPixelFormatRGBA8888,
            size: impeller::ImpellerISize {
                width: width as i64,
                height: height as i64,
            },
            mip_count: 1,
        };
        let mapping = impeller::ImpellerMapping {
            data: rgba.as_ptr(),
            length: bytes as u64,
            on_release: None,
        };
        #[cfg(target_os = "android")]
        let _guard = self.vulkan_procs.as_deref().map(VulkanProcs::current);
        // SAFETY: `mapping` borrows `rgba` until create returns. Impeller copies.
        // The returned texture is owned by `GpuImage`.
        let texture = unsafe {
            impeller::ImpellerTextureCreateWithContentsNew(
                self.context,
                &descriptor,
                &mapping,
                ptr::null_mut(),
            )
        };
        if texture.is_null() {
            return None;
        }
        // SAFETY: the factory still owns a live context. The image balances
        // this retain after releasing its texture, even if it outlives the factory.
        unsafe { impeller::ImpellerContextRetain(self.context) };
        Some(GpuImage {
            texture,
            context: self.context,
            #[cfg(target_os = "android")]
            vulkan_procs: self.vulkan_procs.clone(),
            width,
            height,
        })
    }
}

/// A texture created on `io`. The render thread retains it into the frame cache.
pub struct GpuImage {
    pub(crate) texture: impeller::ImpellerTexture,
    context: impeller::ImpellerContext,
    #[cfg(target_os = "android")]
    vulkan_procs: Option<Arc<VulkanProcs>>,
    pub width: u32,
    pub height: u32,
}

// SAFETY: the texture and retained Metal or Vulkan context can be released
// from another thread. The Vulkan loader stays alive through both releases.
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
        #[cfg(target_os = "android")]
        let _guard = self.vulkan_procs.as_deref().map(VulkanProcs::current);
        // SAFETY: both handles are owned by this image. Release the texture
        // before dropping its retain of the context.
        unsafe {
            impeller::ImpellerTextureRelease(self.texture);
            impeller::ImpellerContextRelease(self.context);
        }
        self.texture = ptr::null_mut();
    }
}

impl super::Engine {
    /// Extra context retain for background texture upload. Null context is `None`.
    pub fn texture_factory(&self) -> Option<TextureFactory> {
        if self.context.is_null() {
            return None;
        }
        #[cfg(target_os = "android")]
        if self.is_gles() {
            return None;
        }
        // SAFETY: `context` is the engine's live context. Retain balances the
        // factory's drop. Release is a no-op on null, but we already checked.
        unsafe { impeller::ImpellerContextRetain(self.context) };
        Some(TextureFactory {
            context: self.context,
            #[cfg(target_os = "android")]
            vulkan_procs: self.vulkan_procs.clone(),
        })
    }

    /// Puts an already-uploaded texture where submit will blit it.
    /// A cache hit does not retain again.
    pub fn install_uploaded(&mut self, source: &str, image: &GpuImage) {
        if self.textures.contains(source)
            || image.texture.is_null()
            || image.context != self.context
        {
            return;
        }
        // SAFETY: `image.texture` is live. The store's copy is released on evict.
        unsafe { impeller::ImpellerTextureRetain(image.texture) };
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

#[cfg(test)]
mod tests {
    use super::super::Engine;

    #[test]
    fn upload_factory_and_texture_can_outlive_the_engine() {
        #[cfg(target_os = "android")]
        let engine = Engine::create_vulkan().expect("Vulkan context");
        #[cfg(not(target_os = "android"))]
        let Ok(engine) = Engine::create_metal() else {
            return;
        };
        #[cfg(target_os = "android")]
        let procs = std::sync::Arc::downgrade(engine.vulkan_procs.as_ref().expect("Vulkan loader"));
        let factory = engine.texture_factory().expect("live context");
        drop(engine);
        std::thread::spawn(move || {
            let image = factory.upload_rgba(1, 1, &[0, 0, 0, 255]).expect("upload");
            drop(factory);
            #[cfg(target_os = "android")]
            assert_eq!(procs.strong_count(), 1);
            drop(image);
            #[cfg(target_os = "android")]
            assert_eq!(procs.strong_count(), 0);
        })
        .join()
        .expect("texture releases on the upload thread");
    }
}
