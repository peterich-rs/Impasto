//! Window attachment, device recovery and RGBA capture.

#[cfg(any(target_vendor = "apple", target_os = "android"))]
use std::{ffi::c_void, ptr::NonNull};

use crate::color::{srgb_to_linear, straight_srgb};
use crate::compositor::extent;
use crate::gpu::Gpu;
use crate::{PixelSize, Rasterizer, Status};
use engine::session::trace::Section;

/// A native surface handle valid for the full life of its wgpu surface.
#[derive(Clone, Copy, Debug)]
pub enum RawSurfaceHandle {
    #[cfg(target_vendor = "apple")]
    CoreAnimationLayer(NonNull<c_void>),
    Window {
        display: Option<raw_window_handle::RawDisplayHandle>,
        window: raw_window_handle::RawWindowHandle,
    },
}

impl RawSurfaceHandle {
    #[cfg(target_os = "android")]
    pub fn android(window: NonNull<c_void>) -> Self {
        Self::Window {
            display: Some(raw_window_handle::AndroidDisplayHandle::new().into()),
            window: raw_window_handle::AndroidNdkWindowHandle::new(window).into(),
        }
    }

    fn into_wgpu(self) -> wgpu::SurfaceTargetUnsafe {
        match self {
            #[cfg(target_vendor = "apple")]
            Self::CoreAnimationLayer(layer) => {
                wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer.as_ptr())
            }
            Self::Window { display, window } => wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: display,
                raw_window_handle: window,
            },
        }
    }
}

/// A drawable and its selected configuration, owned by the raster queue.
pub struct WgpuSurface {
    inner: Option<wgpu::Surface<'static>>,
    handle: RawSurfaceHandle,
    config: wgpu::SurfaceConfiguration,
    device: u64,
    reconfigure: bool,
    first_present_since: Option<std::time::Instant>,
}

impl std::fmt::Debug for WgpuSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WgpuSurface")
            .field("config", &self.config)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Rasterizer {
    /// Opens a surface and selects its device on the first attachment.
    ///
    /// # Safety
    ///
    /// The target must be valid for its platform and retained until the returned
    /// surface is dropped. Configure and present it only on the owning raster queue.
    #[expect(unsafe_code, reason = "the host provides a leased native window")]
    pub unsafe fn create_surface(
        &mut self,
        handle: RawSurfaceHandle,
        pixels: PixelSize,
    ) -> Result<WgpuSurface, Status> {
        if !pixels.is_usable() {
            return Err(Status::NotReady);
        }
        if let Some(device) = self.device.as_mut() {
            if device.gpu.is_lost() {
                return Err(Status::DeviceLost);
            }
            // SAFETY: the caller retains this native handle through surface destruction.
            let raw = unsafe { create_raw(&device.gpu.instance, handle) }?;
            let surface = configure(&device.gpu, device.generation, handle, raw, pixels)?;
            device
                .compositor
                .prewarm_surface(&device.gpu, surface.config.format);
            return Ok(surface);
        }
        let mut last = Status::SurfaceLost;
        for backend in self.backends() {
            let instance = Gpu::instance(backend);
            // SAFETY: the caller retains the native handle throughout all attempts.
            let Ok(raw) = (unsafe { create_raw(&instance, handle) }) else {
                continue;
            };
            let gpu = match Gpu::request(instance, backend, self.options, Some(&raw)) {
                Ok(gpu) => gpu,
                Err(error) => {
                    last = error;
                    continue;
                }
            };
            match Self::new_device(gpu).and_then(|mut device| {
                let surface = configure(&device.gpu, device.generation, handle, raw, pixels)?;
                device
                    .compositor
                    .prewarm_surface(&device.gpu, surface.config.format);
                Ok((surface, device))
            }) {
                Ok((surface, device)) => {
                    self.device = Some(device);
                    return Ok(surface);
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    /// Replaces a lost device and opens the most recent native target again.
    #[expect(
        unsafe_code,
        reason = "the previous surface contract holds through replacement"
    )]
    pub fn recover_surface(
        &mut self,
        surface: &mut WgpuSurface,
        pixels: PixelSize,
    ) -> Result<(), Status> {
        surface.inner = None;
        self.device = None;
        self.frame = None;
        self.invalidate_pictures();
        // SAFETY: the owner keeps the native target present until after this surface is dropped.
        *surface = unsafe { self.create_surface(surface.handle, pixels) }?;
        Ok(())
    }

    #[cfg(target_os = "ios")]
    pub fn prepare_metal_surface(
        &self,
        surface: &mut WgpuSurface,
        pixels: PixelSize,
    ) -> Result<(), Status> {
        let device = self.device.as_ref().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() || surface.device != device.generation {
            return Err(Status::DeviceLost);
        }
        resize_if_needed(&device.gpu, surface, pixels)
    }

    /// Encodes and presents the installed frame.
    #[expect(
        unsafe_code,
        reason = "the previous surface contract holds through replacement"
    )]
    pub fn present(&mut self, surface: &mut WgpuSurface, pixels: PixelSize) -> Result<(), Status> {
        if !pixels.is_usable() {
            return Err(Status::AcquireMiss);
        }
        let state = self.device.as_mut().ok_or(Status::NotReady)?;
        let gpu = &state.gpu;
        if gpu.is_lost() || state.generation != surface.device {
            return Err(Status::DeviceLost);
        }
        if surface.inner.is_none() {
            // SAFETY: the previous surface's native target lease still holds.
            let replacement = unsafe { create_raw(&gpu.instance, surface.handle) }?;
            *surface = configure(gpu, state.generation, surface.handle, replacement, pixels)?;
        }
        resize_if_needed(gpu, surface, pixels)?;
        let raw = surface.inner.as_ref().ok_or(Status::SurfaceLost)?;
        use wgpu::CurrentSurfaceTexture as Frame;
        let started = std::time::Instant::now();
        let acquired = {
            let _trace = Section::begin(c"Surface acquire");
            raw.get_current_texture()
        };
        self.stats.acquire = started.elapsed();
        let texture = match acquired {
            Frame::Success(texture) => texture,
            Frame::Suboptimal(texture) => {
                surface.reconfigure = true;
                texture
            }
            Frame::Timeout => return Err(Status::AcquireMiss),
            Frame::Occluded => return Err(Status::Occluded),
            Frame::Outdated => {
                surface.reconfigure = true;
                return Err(Status::AcquireMiss);
            }
            Frame::Lost if gpu.is_lost() => return Err(Status::DeviceLost),
            Frame::Lost => {
                surface.inner = None;
                // SAFETY: the previous surface's target lease is still held by the host.
                let replacement = unsafe { create_raw(&gpu.instance, surface.handle) }?;
                *surface = configure(gpu, state.generation, surface.handle, replacement, pixels)?;
                return Err(Status::AcquireMiss);
            }
            Frame::Validation => return Err(Status::PresentFailed),
        };
        let view = texture.texture.create_view(&Default::default());
        self.encode_and_submit(&view, surface.config.format, pixels)?;
        let state = self.device.as_ref().ok_or(Status::NotReady)?;
        let started = std::time::Instant::now();
        {
            let _trace = Section::begin(c"Surface present");
            state.gpu.queue.present(texture);
        }
        self.stats.surface_present = started.elapsed();
        if state.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        if let Some(since) = surface.first_present_since.take() {
            tracing::info!(target: "impasto::startup",
                configure_to_present_us = since.elapsed().as_secs_f64() * 1_000_000.0,
                acquire_us = self.stats.acquire.as_secs_f64() * 1_000_000.0,
                present_us = self.stats.surface_present.as_secs_f64() * 1_000_000.0,
                "first surface present returned");
        }
        Ok(())
    }

    /// Renders the installed scene to tightly packed straight-alpha sRGB RGBA.
    pub fn render_rgba(&mut self, pixels: PixelSize) -> Result<Vec<u8>, Status> {
        let gpu = self
            .device
            .as_ref()
            .map(|state| state.gpu.clone())
            .ok_or(Status::NotReady)?;
        if gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        check_size(&gpu, pixels)?;
        let size = extent([pixels.width, pixels.height]);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let target = gpu.validation(|| {
            Ok(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("impasto RGBA capture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }))
        })?;
        self.encode_and_submit(&target.create_view(&Default::default()), format, pixels)?;
        read_rgba(&gpu, &target, pixels)
    }
}

pub(crate) fn read_rgba(
    gpu: &Gpu,
    target: &wgpu::Texture,
    pixels: PixelSize,
) -> Result<Vec<u8>, Status> {
    let size = extent([pixels.width, pixels.height]);
    let row = pixels.width.checked_mul(4).ok_or(Status::BadScene)?;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let stride = row
        .div_ceil(alignment)
        .checked_mul(alignment)
        .ok_or(Status::BadScene)?;
    let buffer_size = u64::from(stride) * u64::from(pixels.height);
    if buffer_size > gpu.device.limits().max_buffer_size {
        return Err(Status::BadScene);
    }
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("impasto readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut command = gpu.device.create_command_encoder(&Default::default());
    command.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        size,
    );
    let submitted = gpu.queue.submit([command.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    let timeout = std::time::Duration::from_secs(5);
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(timeout),
        })
        .map_err(|_| {
            if gpu.is_lost() {
                Status::DeviceLost
            } else {
                Status::PresentFailed
            }
        })?;
    receive
        .recv_timeout(timeout)
        .map_err(|_| Status::PresentFailed)?
        .map_err(|_| Status::PresentFailed)?;
    let bytes = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|_| Status::PresentFailed)?;
    let mut output = Vec::with_capacity(row as usize * pixels.height as usize);
    for padded in bytes.chunks_exact(stride as usize) {
        for rgba in padded[..row as usize].as_chunks::<4>().0 {
            if rgba[3] == 255 {
                output.extend_from_slice(rgba);
            } else {
                let color = [
                    srgb_to_linear(rgba[0]),
                    srgb_to_linear(rgba[1]),
                    srgb_to_linear(rgba[2]),
                    f32::from(rgba[3]) / 255.0,
                ];
                output.extend(straight_srgb(color));
            }
        }
    }
    Ok(output)
}

#[expect(
    unsafe_code,
    reason = "wgpu exposes platform surface creation through its unsafe boundary"
)]
unsafe fn create_raw(
    instance: &wgpu::Instance,
    handle: RawSurfaceHandle,
) -> Result<wgpu::Surface<'static>, Status> {
    // SAFETY: callers keep the native window and display alive through the resulting surface.
    unsafe { instance.create_surface_unsafe(handle.into_wgpu()) }.map_err(|error| {
        tracing::debug!(target: "impasto::gpu", %error, "could not create surface");
        Status::SurfaceLost
    })
}

fn configure(
    gpu: &Gpu,
    device: u64,
    handle: RawSurfaceHandle,
    raw: wgpu::Surface<'static>,
    pixels: PixelSize,
) -> Result<WgpuSurface, Status> {
    let caps = raw.get_capabilities(&gpu.adapter);
    let Some(mut config) = raw.get_default_config(&gpu.adapter, pixels.width, pixels.height) else {
        return Err(Status::NoDevice(gpu.backend));
    };
    if let Some(format) = caps.formats.iter().find(|format| format.is_srgb()) {
        config.format = *format;
    }
    if caps
        .color_spaces(config.format)
        .contains(wgpu::SurfaceColorSpaces::SRGB)
    {
        config.color_space = wgpu::SurfaceColorSpace::Srgb;
    }
    config.desired_maximum_frame_latency = std::env::var("IMPASTO_SURFACE_LATENCY")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (1..=3).contains(value))
        .unwrap_or(2);
    let present = std::env::var("IMPASTO_PRESENT_MODE").ok();
    config.present_mode = present_mode(present.as_deref(), &caps.present_modes);
    let alpha = match std::env::var("IMPASTO_SURFACE_ALPHA").as_deref() {
        Ok("opaque") => Some(wgpu::CompositeAlphaMode::Opaque),
        Ok("premultiplied") => Some(wgpu::CompositeAlphaMode::PreMultiplied),
        Ok("postmultiplied") => Some(wgpu::CompositeAlphaMode::PostMultiplied),
        _ => None,
    };
    if let Some(alpha) = alpha.filter(|alpha| caps.alpha_modes.contains(alpha)) {
        config.alpha_mode = alpha;
    }
    resize(gpu, &raw, &mut config, pixels)?;
    tracing::info!(
        target: "impasto::surface", ?present, requested_alpha = ?alpha,
        present = ?config.present_mode, alpha = ?config.alpha_mode, format = ?config.format,
        color_space = ?config.color_space, latency = config.desired_maximum_frame_latency,
        "surface configured"
    );
    Ok(WgpuSurface {
        inner: Some(raw),
        handle,
        config,
        device,
        reconfigure: false,
        first_present_since: Some(std::time::Instant::now()),
    })
}

fn present_mode(selected: Option<&str>, available: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    use wgpu::PresentMode as Mode;
    let requested = match selected {
        Some("mailbox") => Mode::Mailbox,
        Some("fifo-relaxed") => Mode::FifoRelaxed,
        _ => Mode::Fifo,
    };
    if available.contains(&requested) {
        requested
    } else {
        Mode::Fifo
    }
}

fn resize_if_needed(gpu: &Gpu, surface: &mut WgpuSurface, pixels: PixelSize) -> Result<(), Status> {
    if surface.reconfigure
        || [surface.config.width, surface.config.height] != [pixels.width, pixels.height]
    {
        let raw = surface.inner.as_ref().ok_or(Status::SurfaceLost)?;
        resize(gpu, raw, &mut surface.config, pixels)?;
        surface.reconfigure = false;
    }
    Ok(())
}

fn resize(
    gpu: &Gpu,
    surface: &wgpu::Surface<'_>,
    config: &mut wgpu::SurfaceConfiguration,
    pixels: PixelSize,
) -> Result<(), Status> {
    check_size(gpu, pixels)?;
    config.width = pixels.width;
    config.height = pixels.height;
    gpu.validation(|| {
        surface.configure(&gpu.device, config);
        Ok(())
    })
}

pub(crate) fn check_size(gpu: &Gpu, pixels: PixelSize) -> Result<(), Status> {
    if !pixels.is_usable() {
        return Err(Status::NotReady);
    }
    if pixels.width.max(pixels.height) > gpu.max_texture_size() {
        return Err(Status::BadScene);
    }
    Ok(())
}
