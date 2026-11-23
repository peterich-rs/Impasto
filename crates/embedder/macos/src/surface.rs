//! Presentation for the window's Core Animation layer.

use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use embedder::{Presented, Surface, SurfaceTarget};
use raster::{PixelSize, Rasterizer, RawSurfaceHandle, Status, WgpuSurface};

use crate::pixels::write_rgba_png;

pub struct MetalSurface {
    surface: WgpuSurface,
    dump: Option<PathBuf>,
    exit_code: Arc<AtomicI32>,
}

impl MetalSurface {
    pub fn open(
        raster: &mut Rasterizer,
        target: SurfaceTarget,
        dump: Option<PathBuf>,
        exit_code: Arc<AtomicI32>,
    ) -> Result<Self, Status> {
        let layer = NonNull::new(target.layer.0).ok_or(Status::SurfaceLost)?;
        // SAFETY: the main thread keeps the layer alive until raster shutdown.
        let surface = unsafe {
            raster.create_surface(RawSurfaceHandle::CoreAnimationLayer(layer), target.pixels)
        }?;
        Ok(Self {
            surface,
            dump,
            exit_code,
        })
    }
}

impl Surface for MetalSurface {
    fn recover_device(&mut self, raster: &mut Rasterizer, pixels: PixelSize) -> Result<(), Status> {
        raster.recover_surface(&mut self.surface, pixels)
    }

    fn present(&mut self, raster: &mut Rasterizer, pixels: PixelSize) -> Result<Presented, Status> {
        objc2::rc::autoreleasepool(|_| {
            if let Some(path) = &self.dump {
                let rgba = raster.render_rgba(pixels)?;
                if let Err(error) =
                    write_rgba_png(path, pixels.width as usize, pixels.height as usize, &rgba)
                {
                    tracing::error!(target: "impasto::shell", %error, path = %path.display(), "dump write failed");
                    self.exit_code.store(1, Ordering::SeqCst);
                } else {
                    tracing::info!(target: "impasto::shell", "frame dumped");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                return Ok(Presented::Stop);
            }
            raster.present(&mut self.surface, pixels)?;
            Ok(Presented::Continue)
        })
    }
}
