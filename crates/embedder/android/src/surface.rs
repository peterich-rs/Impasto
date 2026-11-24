//! Presentation for one Android native window.

use std::ptr::NonNull;
use std::sync::Arc;

use embedder::{OpenSurface, Presented, Surface, SurfaceId};
use raster::{
    FontLibrary, PixelSize, RasterBackend, RasterOptions, Rasterizer, RawSurfaceHandle, Status,
    WgpuSurface,
};

#[derive(Clone, Copy)]
pub(crate) enum Renderer {
    Auto,
    Vulkan,
    Gles,
}

impl Renderer {
    pub fn create_raster(self, fonts: FontLibrary) -> Result<Rasterizer, Status> {
        let backend = match self {
            Self::Auto => None,
            Self::Vulkan => Some(RasterBackend::Vulkan),
            Self::Gles => Some(RasterBackend::Gles),
        };
        Rasterizer::create(
            RasterOptions {
                backend,
                ..Default::default()
            },
            fonts,
        )
    }

    pub fn surfaces(first_frame: impl Fn(SurfaceId) + Send + Sync + 'static) -> OpenSurface {
        let first_frame = Arc::new(first_frame);
        Box::new(move |raster, target| {
            let window = NonNull::new(target.layer.0).ok_or(Status::SurfaceLost)?;
            // SAFETY: the platform retains the native window through the surface-retirement ack.
            let surface =
                unsafe { raster.create_surface(RawSurfaceHandle::android(window), target.pixels) }?;
            let first_frame = Arc::clone(&first_frame);
            Ok(Box::new(WindowSurface {
                surface,
                first_frame: Some(Box::new(move || first_frame(target.id))),
            }))
        })
    }
}

struct WindowSurface {
    surface: WgpuSurface,
    first_frame: Option<Box<dyn FnOnce()>>,
}

impl Surface for WindowSurface {
    fn recover_device(&mut self, raster: &mut Rasterizer, pixels: PixelSize) -> Result<(), Status> {
        raster.recover_surface(&mut self.surface, pixels)
    }

    fn present(&mut self, raster: &mut Rasterizer, pixels: PixelSize) -> Result<Presented, Status> {
        raster.present(&mut self.surface, pixels)?;
        if let Some(notify) = self.first_frame.take() {
            notify();
        }
        Ok(Presented::Continue)
    }
}
