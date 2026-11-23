//! Owns one wgpu device generation and its submission queue.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{RasterBackend, RasterOptions, Status};

#[derive(Clone)]
pub(crate) struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub backend: RasterBackend,
    lost: Arc<AtomicBool>,
}

impl Gpu {
    pub fn instance(backend: RasterBackend) -> wgpu::Instance {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backend.backends();
        if cfg!(target_os = "android") && backend == RasterBackend::Vulkan {
            // Some Android emulator Vulkan drivers crash while naming
            // descriptors through VK_EXT_debug_utils. Keep wgpu validation.
            descriptor.flags.remove(wgpu::InstanceFlags::DEBUG);
        }
        wgpu::Instance::new(descriptor)
    }

    pub fn request(
        instance: wgpu::Instance,
        backend: RasterBackend,
        options: RasterOptions,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, Status> {
        let request = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface,
            force_fallback_adapter: options.force_fallback_adapter,
            ..Default::default()
        };
        let adapter = pollster::block_on(instance.request_adapter(&request)).map_err(|error| {
            tracing::debug!(target: "impasto::gpu", ?backend, %error, "adapter unavailable");
            Status::NoDevice(backend)
        })?;
        let info = adapter.get_info();
        if cfg!(target_os = "android")
            && backend == RasterBackend::Vulkan
            && info.name.starts_with("Goldfish GFXStream")
            && info.name.contains("SwiftShader")
        {
            // The emulated Vulkan queue crashes in ResourceTracker::on_vkQueueSubmit.
            // Let auto mode try GLES before creating a device on this adapter.
            tracing::warn!(target: "impasto::gpu", device = %info.name, "unsupported emulated Vulkan driver");
            return Err(Status::NoDevice(backend));
        }
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("impasto device"),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|error| {
            tracing::warn!(target: "impasto::gpu", ?backend, %error, "device request failed");
            Status::NoDevice(backend)
        })?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |reason, message| {
            flag.store(true, Ordering::Release);
            tracing::warn!(target: "impasto::gpu", ?reason, %message, "device lost");
        });
        device.on_uncaptured_error(Arc::new(|error| {
            tracing::error!(target: "impasto::gpu", %error, "wgpu error");
        }));
        tracing::info!(target: "impasto::gpu", ?backend, device = %info.name, "graphics device ready");
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            backend,
            lost,
        })
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    pub fn max_texture_size(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub fn validation<T>(
        &self,
        operation: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        if self.is_lost() {
            return Err(Status::DeviceLost);
        }
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = operation();
        let invalid = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        let exhausted = pollster::block_on(memory.pop());
        if result.is_err() || invalid.is_some() || internal.is_some() || exhausted.is_some() {
            // Encoding can fail before submit advances pending device callbacks.
            let _ = self.device.poll(wgpu::PollType::Poll);
        }
        if self.is_lost() {
            return Err(Status::DeviceLost);
        }
        if let Some(error) = exhausted {
            tracing::warn!(target: "impasto::gpu", %error, "graphics memory exhausted");
            return Err(Status::OutOfMemory);
        }
        if let Some(error) = invalid.or(internal) {
            tracing::error!(target: "impasto::gpu", %error, "invalid render operation");
            return Err(Status::PresentFailed);
        }
        result
    }
}
