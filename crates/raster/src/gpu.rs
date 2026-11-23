//! Owns one wgpu device generation and its submission queue.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{RasterBackend, RasterOptions, Status, ValidationMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GpuTier {
    Tier0Baseline,
    Tier1Standard,
    Tier2Native,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GpuCaps {
    pub tier: GpuTier,
    pub adapter_features: wgpu::Features,
    pub downlevel_flags: wgpu::DownlevelFlags,
    pub min_uniform_align: u32,
    pub max_texture_2d: u32,
    pub max_array_layers: u32,
    pub max_buffer_size: u64,
    pub msaa4_srgb: bool,
    pub has_vertex_storage: bool,
    pub has_fragment_storage: bool,
    pub has_compute: bool,
    pub has_timestamp_query: bool,
}

impl GpuCaps {
    fn detect(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Self {
        use wgpu::{DownlevelFlags as D, Features as F, TextureFormatFeatureFlags as T};
        let adapter_features = adapter.features();
        let features = device.features();
        let downlevel_flags = adapter.get_downlevel_capabilities().flags;
        let limits = device.limits();
        // Query the capabilities available to this device without optional format features.
        let srgb =
            wgpu::TextureFormat::Rgba8UnormSrgb.guaranteed_format_features(device.features());
        let mut caps = Self {
            tier: GpuTier::Tier0Baseline,
            adapter_features,
            downlevel_flags,
            min_uniform_align: limits.min_uniform_buffer_offset_alignment,
            max_texture_2d: limits.max_texture_dimension_2d,
            max_array_layers: limits.max_texture_array_layers,
            max_buffer_size: limits.max_buffer_size,
            msaa4_srgb: srgb
                .flags
                .contains(T::MULTISAMPLE_X4 | T::MULTISAMPLE_RESOLVE),
            has_vertex_storage: downlevel_flags.contains(D::VERTEX_STORAGE),
            has_fragment_storage: downlevel_flags.contains(D::FRAGMENT_STORAGE),
            has_compute: downlevel_flags.contains(D::COMPUTE_SHADERS),
            has_timestamp_query: features.contains(F::TIMESTAMP_QUERY),
        };
        if caps.has_compute
            && caps.has_vertex_storage
            && caps.has_fragment_storage
            && caps.downlevel_flags.contains(D::READ_ONLY_DEPTH_STENCIL)
        {
            caps.tier = if adapter_features.intersects(
                F::IMMEDIATES
                    | F::PIPELINE_CACHE
                    | F::TIMESTAMP_QUERY
                    | F::DUAL_SOURCE_BLENDING
                    | F::SHADER_F16
                    | F::TEXTURE_BINDING_ARRAY,
            ) {
                GpuTier::Tier2Native
            } else {
                GpuTier::Tier1Standard
            };
        }
        caps
    }
}

#[derive(Clone)]
pub(crate) struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub backend: RasterBackend,
    pub caps: GpuCaps,
    lost: Arc<AtomicBool>,
    oom: Arc<AtomicBool>,
    uncaptured_error: Arc<AtomicBool>,
    validate_frames: bool,
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
        let advertised = adapter.limits();
        let baseline = |defaults: wgpu::Limits| wgpu::Limits {
            max_buffer_size: advertised.max_buffer_size,
            max_texture_array_layers: advertised.max_texture_array_layers,
            max_uniform_buffer_binding_size: advertised.max_uniform_buffer_binding_size,
            ..defaults
                .using_resolution(advertised.clone())
                .using_alignment(advertised.clone())
        };
        let standard = baseline(wgpu::Limits::downlevel_defaults());
        let limits = if standard.check_limits(&advertised) {
            standard
        } else {
            // GLES 3.0 may advertise zero compute and storage limits.
            baseline(wgpu::Limits::downlevel_webgl2_defaults())
        };
        let timestamps = wgpu::Features::TIMESTAMP_QUERY;
        let request_device = |adapter: &wgpu::Adapter, required_features| {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("impasto device"),
                required_features,
                required_limits: limits.clone(),
                ..Default::default()
            }))
        };
        let (adapter, result) = if options.enable_gpu_timing
            && adapter.features().contains(timestamps)
        {
            match request_device(&adapter, timestamps) {
                Ok(device) => (adapter, Ok(device)),
                Err(error) => {
                    tracing::warn!(target: "impasto::gpu", ?backend, %error, "retrying without timestamp queries");
                    let adapter = pollster::block_on(instance.request_adapter(&request))
                        .map_err(|_| Status::NoDevice(backend))?;
                    let result = request_device(&adapter, wgpu::Features::empty());
                    (adapter, result)
                }
            }
        } else {
            let result = request_device(&adapter, wgpu::Features::empty());
            (adapter, result)
        };
        let (device, queue) = result.map_err(|error| {
            tracing::warn!(target: "impasto::gpu", ?backend, %error, "device request failed");
            Status::NoDevice(backend)
        })?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |reason, message| {
            flag.store(true, Ordering::Release);
            tracing::warn!(target: "impasto::gpu", ?reason, %message, "device lost");
        });
        let oom = Arc::new(AtomicBool::new(false));
        let uncaptured_error = Arc::new(AtomicBool::new(false));
        let memory = Arc::clone(&oom);
        let invalid = Arc::clone(&uncaptured_error);
        device.on_uncaptured_error(Arc::new(move |error| match error {
            wgpu::Error::OutOfMemory { .. } => {
                memory.store(true, Ordering::Release);
                tracing::warn!(target: "impasto::gpu", %error, "graphics memory exhausted");
            }
            _ => {
                invalid.store(true, Ordering::Release);
                tracing::error!(target: "impasto::gpu", %error, "wgpu error");
            }
        }));
        let caps = GpuCaps::detect(&adapter, &device);
        tracing::info!(target: "impasto::gpu", ?backend, device = %info.name,
            tier = ?caps.tier, caps.min_uniform_align, caps.max_texture_2d,
            caps.max_array_layers, caps.msaa4_srgb, supported = ?caps.adapter_features, enabled = ?device.features(), "graphics device ready");
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            backend,
            caps,
            lost,
            oom,
            uncaptured_error,
            validate_frames: match options.validation {
                ValidationMode::Auto => {
                    cfg!(any(test, debug_assertions))
                        || std::env::var("IMPASTO_GPU_VALIDATE").as_deref() == Ok("1")
                }
                ValidationMode::Strict => true,
                ValidationMode::AsyncOnly => false,
            },
        })
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    pub fn max_texture_size(&self) -> u32 {
        self.caps.max_texture_2d
    }

    pub fn check_async_health(&self) -> Result<(), Status> {
        if self.is_lost() {
            Err(Status::DeviceLost)
        } else if self.oom.swap(false, Ordering::AcqRel) {
            Err(Status::OutOfMemory)
        } else if self.uncaptured_error.swap(false, Ordering::AcqRel) {
            Err(Status::PresentFailed)
        } else {
            Ok(())
        }
    }

    pub fn hot_validation<T>(
        &self,
        operation: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        self.check_async_health()?;
        let result = if self.validate_frames {
            self.validation(operation)
        } else {
            operation()
        };
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.check_async_health()?;
        result
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
