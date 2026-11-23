//! Metal interop for UIKit transactions and producer-owned textures.

use std::ffi::c_void;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLCommandBuffer, MTLCommandBufferStatus, MTLPixelFormat, MTLResource, MTLTexture,
    MTLTextureType, MTLTextureUsage,
};
use objc2_quartz_core::CAMetalDrawable;
use wgpu::hal::api::Metal;

use crate::{GpuImage, PixelSize, Rasterizer, Status, compositor::extent, surface::check_size};

type Texture = ProtocolObject<dyn MTLTexture>;
type Drawable = ProtocolObject<dyn CAMetalDrawable>;

/// A set of drawable encodings owned by the raster queue until submission.
pub struct MetalSliceBatch {
    generation: u64,
    frame: u64,
    started: Instant,
    commands: Vec<wgpu::CommandBuffer>,
}

impl Rasterizer {
    pub fn begin_metal_slices(&mut self) -> Result<MetalSliceBatch, Status> {
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        if self.frame.is_none() {
            return Err(Status::NotReady);
        }
        device.compositor.begin_frame();
        Ok(MetalSliceBatch {
            generation: device.generation,
            frame: self.frame_number,
            started: Instant::now(),
            commands: Vec::new(),
        })
    }

    /// Records a single drawable without presenting it.
    ///
    /// # Safety
    ///
    /// `drawable` must point to a live CAMetalDrawable retained by the host
    /// through submission and the UIKit transaction.
    #[expect(
        unsafe_code,
        reason = "CAMetalDrawable crosses the retained native surface boundary"
    )]
    pub unsafe fn encode_metal_slice(
        &mut self,
        batch: &mut MetalSliceBatch,
        slice: Option<usize>,
        drawable: *mut c_void,
        pixels: PixelSize,
    ) -> Result<(), Status> {
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        let frame = self.frame.as_ref().ok_or(Status::NotReady)?;
        if batch.generation != device.generation || batch.frame != self.frame_number {
            return Err(Status::NotReady);
        }
        // SAFETY: the host guarantees this is a retained CAMetalDrawable.
        let drawable =
            unsafe { drawable.cast::<Drawable>().as_ref() }.ok_or(Status::AcquireMiss)?;
        check_size(&device.gpu, pixels)?;
        let gpu = &device.gpu;
        let subset = slice
            .map(|index| frame.slices.get(index).ok_or(Status::BadScene))
            .transpose()?;
        let texture = drawable.texture();
        let format = match texture.pixelFormat() {
            MTLPixelFormat::BGRA8Unorm_sRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
            MTLPixelFormat::BGRA8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            _ => return Err(Status::PresentFailed),
        };
        if !matches_texture(&gpu.device, &texture, pixels)
            || !texture.usage().contains(MTLTextureUsage::RenderTarget)
        {
            return Err(Status::AcquireMiss);
        }
        let descriptor = texture_descriptor(pixels, format, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let command = gpu.validation(|| {
            let texture = wrap(
                &gpu.device,
                texture,
                &descriptor,
                wgpu::TextureUses::UNINITIALIZED,
                None,
            );
            device.compositor.encode(
                gpu,
                frame.scene(&self.fonts, subset),
                &texture.create_view(&Default::default()),
                format,
                [pixels.width, pixels.height],
            )
        })?;
        batch.commands.push(command);
        Ok(())
    }

    /// Submits the whole batch and waits for Metal to schedule its tail.
    #[expect(
        unsafe_code,
        reason = "wgpu exposes Metal scheduling through its HAL encoder"
    )]
    pub fn wait_for_metal_schedule(&mut self, batch: MetalSliceBatch) -> Result<(), Status> {
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if batch.generation != device.generation
            || batch.frame != self.frame_number
            || batch.commands.is_empty()
        {
            return Err(Status::NotReady);
        }
        let gpu = &device.gpu;
        let mut tail = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("impasto UIKit scheduling tail"),
            });
        // SAFETY: this encoder uses only HAL recording. Retain the raw buffer
        // without committing it or ending it. wgpu owns finish and submission.
        let scheduled = unsafe {
            tail.as_hal_mut::<Metal, _, _>(|encoder| {
                encoder
                    .and_then(|encoder| encoder.raw_command_buffer())
                    .and_then(|buffer| Retained::retain(std::ptr::from_ref(buffer).cast_mut()))
            })
        }
        .ok_or(Status::PresentFailed)?;
        self.stats.atlas = gpu.validation(|| {
            let atlas = device.compositor.flush(gpu);
            gpu.queue
                .submit(batch.commands.into_iter().chain([tail.finish()]));
            Ok(atlas)
        })?;
        scheduled.waitUntilScheduled();
        if gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        if !matches!(
            scheduled.status(),
            MTLCommandBufferStatus::Scheduled | MTLCommandBufferStatus::Completed
        ) {
            return Err(Status::PresentFailed);
        }
        self.stats.render = device.compositor.render_stats();
        self.stats.compose = batch.started.elapsed();
        Ok(())
    }

    /// Imports a ready BGRA8 Core Video frame. The release callback stays
    /// attached to the underlying HAL texture through GPU use.
    ///
    /// # Safety
    ///
    /// `texture` must reference a live Metal texture. The producer must finish
    /// writing it before import and keep its backing storage stable until release.
    #[expect(
        unsafe_code,
        reason = "the iOS producer hands over a live Metal texture"
    )]
    pub unsafe fn import_metal_texture(
        &self,
        texture: *const c_void,
        size: [u32; 2],
        release: impl FnOnce() + Send + Sync + 'static,
    ) -> Option<GpuImage> {
        let device = self.device.as_ref()?;
        let gpu = &device.gpu;
        let pixels = PixelSize::new(size[0], size[1]);
        check_size(gpu, pixels).ok()?;
        if gpu.is_lost() {
            return None;
        }
        // SAFETY: the caller retains the texture through this call. Acquire our own retain.
        let texture = unsafe { Retained::retain(texture.cast::<Texture>().cast_mut()) }?;
        if !matches_texture(&gpu.device, &texture, pixels)
            || texture.pixelFormat() != MTLPixelFormat::BGRA8Unorm
            || !texture.usage().contains(MTLTextureUsage::ShaderRead)
            || texture.isFramebufferOnly()
        {
            return None;
        }
        let texture = wrap(
            &gpu.device,
            texture,
            &texture_descriptor(
                pixels,
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            wgpu::TextureUses::RESOURCE,
            Some(Box::new(release)),
        );
        Some(GpuImage::from_metal(texture, pixels, device.generation))
    }
}

#[expect(
    unsafe_code,
    reason = "the native texture must belong to this wgpu Metal device"
)]
fn matches_texture(device: &wgpu::Device, texture: &Texture, size: PixelSize) -> bool {
    // SAFETY: inspect the live HAL device under the guard without changing it.
    let Some(hal) = (unsafe { device.as_hal::<Metal>() }) else {
        return false;
    };
    let texture_device = texture.device();
    std::ptr::eq(&*texture_device, &**hal.raw_device())
        && texture.textureType() == MTLTextureType::Type2D
        && texture.width() == size.width as usize
        && texture.height() == size.height as usize
        && texture.depth() == 1
        && texture.sampleCount() == 1
        && texture.arrayLength() == 1
        && texture.mipmapLevelCount() == 1
}

fn texture_descriptor(
    size: PixelSize,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("impasto imported Metal texture"),
        size: extent([size.width, size.height]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }
}

/// Callers verify that the texture and descriptor match and keep the producer synchronized.
#[expect(
    unsafe_code,
    reason = "validated Metal textures enter wgpu through HAL"
)]
fn wrap(
    device: &wgpu::Device,
    texture: Retained<Texture>,
    descriptor: &wgpu::TextureDescriptor<'_>,
    initial_state: wgpu::TextureUses,
    release: Option<wgpu::hal::DropCallback>,
) -> wgpu::Texture {
    // SAFETY: both callers verify the Metal device, descriptor and usage. UIKit
    // drawables are cleared by the first wgpu pass, and frame producers are ready to read.
    unsafe {
        let hal = wgpu::hal::metal::Device::texture_from_raw(
            texture,
            descriptor.format,
            MTLTextureType::Type2D,
            1,
            1,
            descriptor.size.into(),
            release,
        );
        device.create_texture_from_hal::<Metal>(hal, descriptor, initial_state)
    }
}
