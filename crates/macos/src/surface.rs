//! Metal drawable present. The layer pointer stays owned by the main thread.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;

use anyhow::Context;
use engine::Engine;
use hostshell::{NativePtr, Presented, Surface};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLOrigin, MTLRegion, MTLSize, MTLTexture};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use crate::pixels::write_bgra_png;

pub struct MetalSurface {
    layer: NativePtr,
    dump: Option<PathBuf>,
    on_stop: Arc<dyn Fn(i32) + Send + Sync>,
}

impl MetalSurface {
    pub fn new(
        layer: NativePtr,
        dump: Option<PathBuf>,
        on_stop: Arc<dyn Fn(i32) + Send + Sync>,
    ) -> Self {
        Self {
            layer,
            dump,
            on_stop,
        }
    }

    fn present_drawable(&mut self, engine: &mut Engine) -> Result<Presented, engine::Status> {
        // SAFETY: the main thread retains this layer until shutdown; this is the render thread's only use.
        let layer = unsafe { &*self.layer.0.cast::<CAMetalLayer>() };
        let Some(drawable) = layer.nextDrawable() else {
            return Err(engine::Status::PresentFailed);
        };
        let drawable_ptr = Retained::as_ptr(&drawable) as *mut c_void;
        // SAFETY: `drawable` stays retained until this function returns.
        engine.draw_metal(drawable_ptr)?;
        let Some(path) = &self.dump else {
            return Ok(Presented::Continue);
        };
        if let Err(error) = read_drawable_png(&drawable, path) {
            eprintln!("failed to write {}: {error:#}", path.display());
            (self.on_stop)(1);
            return Ok(Presented::Stop);
        }
        println!("frame dumped");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        (self.on_stop)(0);
        Ok(Presented::Stop)
    }
}

impl Surface for MetalSurface {
    fn present(&mut self, engine: &mut Engine) -> Result<Presented, engine::Status> {
        objc2::rc::autoreleasepool(|_| self.present_drawable(engine))
    }
}

fn read_drawable_png(
    drawable: &ProtocolObject<dyn CAMetalDrawable>,
    path: &PathBuf,
) -> anyhow::Result<()> {
    let texture = drawable.texture();
    let width = texture.width();
    let height = texture.height();
    if width == 0 || height == 0 {
        anyhow::bail!("drawable has no pixels");
    }
    let row = width.checked_mul(4).context("texture row is too wide")?;
    let len = row.checked_mul(height).context("texture is too large")?;
    let mut pixels = vec![0u8; len];
    let Some(bytes) = NonNull::new(pixels.as_mut_ptr().cast()) else {
        anyhow::bail!("texture pixel buffer is null");
    };
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
    };
    // `present` returns before the drawable texture can be read. Poll until the
    // GPU has written a pixel, or until the wait expires for an empty frame.
    let start = std::time::Instant::now();
    loop {
        // SAFETY: `pixels` has `len` bytes, `row` is the stride, and the texture is not framebuffer-only.
        unsafe { texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(bytes, row, region, 0) };
        let painted = pixels
            .chunks(4)
            .any(|pixel| pixel.iter().any(|byte| *byte != 0));
        if painted || start.elapsed() >= std::time::Duration::from_millis(500) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    write_bgra_png(path, width, height, &pixels)
}
