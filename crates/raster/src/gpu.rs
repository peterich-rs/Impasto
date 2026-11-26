//! Owns one wgpu device generation and its submission queue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crate::{RasterBackend, RasterOptions, Status, ValidationMode};

pub(crate) const IMAGE_ARRAY_LEN: u32 = 32;
const IMAGE_ARRAY_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_BINDING_ARRAY
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING);

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
    pub max_storage_binding: u64,
    pub msaa4_srgb: bool,
    pub has_vertex_storage: bool,
    pub has_fragment_storage: bool,
    pub has_compute: bool,
    pub paint_storage: bool,
    pub has_timestamp_query: bool,
    pub lcd: bool,
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
            max_storage_binding: limits.max_storage_buffer_binding_size,
            msaa4_srgb: srgb
                .flags
                .contains(T::MULTISAMPLE_X4 | T::MULTISAMPLE_RESOLVE),
            has_vertex_storage: downlevel_flags.contains(D::VERTEX_STORAGE),
            has_fragment_storage: downlevel_flags.contains(D::FRAGMENT_STORAGE),
            has_compute: downlevel_flags.contains(D::COMPUTE_SHADERS),
            paint_storage: downlevel_flags.contains(D::VERTEX_STORAGE | D::FRAGMENT_STORAGE)
                && limits.max_storage_buffers_per_shader_stage > 0
                && limits.max_storage_buffer_binding_size >= crate::pipelines::EXTRA_BYTES as u64
                && std::env::var_os("IMPASTO_PAINT_UNIFORM").is_none(),
            has_timestamp_query: features.contains(F::TIMESTAMP_QUERY),
            lcd: features.contains(F::DUAL_SOURCE_BLENDING),
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
    pub shadow_2d: bool,
    pub hollow_shadows: bool,
    pub text_gamma: bool,
    pub gradient_lut: bool,
    pub dither_output: bool,
    pub dither_gradients: bool,
    lost: Arc<AtomicBool>,
    oom: Arc<AtomicBool>,
    uncaptured_error: Arc<AtomicBool>,
    validate_frames: bool,
    image_upload: Arc<OnceLock<Option<crate::image_upload::ImageUpload>>>,
    pub(crate) inline_image_pipeline: Arc<OnceLock<Option<wgpu::RenderPipeline>>>,
    image_upload_mode: ImageUploadMode,
    pipeline_cache: Option<crate::pipeline_disk_cache::PipelineDiskCache>,
    #[cfg(target_os = "android")]
    pub external_queue: Option<crate::android_vulkan::ExternalQueue>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageUploadMode {
    Cpu,
    Render,
    Compute,
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
        cache_dir: Option<&std::path::Path>,
    ) -> Result<Self, Status> {
        let request = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface,
            force_fallback_adapter: options.force_fallback_adapter,
            ..Default::default()
        };
        let started = Instant::now();
        let adapter = {
            let _trace =
                tracing::debug_span!(target: "impasto::gpu", "gpu_adapter_request").entered();
            pollster::block_on(instance.request_adapter(&request)).map_err(|error| {
                tracing::debug!(target: "impasto::gpu", ?backend, %error, "adapter unavailable");
                Status::NoDevice(backend)
            })?
        };
        let adapter_us = started.elapsed().as_secs_f64() * 1_000_000.0;
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
        let request_device = |adapter: &wgpu::Adapter, required_features: wgpu::Features| {
            let mut limits = limits.clone();
            if required_features.contains(wgpu::Features::IMMEDIATES) {
                limits.max_immediate_size = advertised.max_immediate_size.min(128);
            }
            if required_features.contains(IMAGE_ARRAY_FEATURES) {
                limits.max_binding_array_elements_per_shader_stage = IMAGE_ARRAY_LEN;
                limits.max_sampled_textures_per_shader_stage = limits
                    .max_sampled_textures_per_shader_stage
                    .max(IMAGE_ARRAY_LEN + 1);
            }
            let descriptor = wgpu::DeviceDescriptor {
                label: Some("impasto device"),
                required_features,
                required_limits: limits,
                ..Default::default()
            };
            #[cfg(target_os = "android")]
            if backend == RasterBackend::Vulkan
                && let Some(opened) = crate::android_vulkan::open(adapter, &descriptor)
            {
                return opened;
            }
            pollster::block_on(adapter.request_device(&descriptor))
        };
        let support = adapter.features();
        let timing = if options.enable_gpu_timing && support.contains(timestamps) {
            timestamps
        } else {
            wgpu::Features::empty()
        };
        let lcd = wgpu::Features::DUAL_SOURCE_BLENDING;
        let modern = wgpu::Features::IMMEDIATES | wgpu::Features::SHADER_F16;
        let mut requested = timing | (support & modern);
        if cache_dir.is_some() && backend == RasterBackend::Vulkan {
            requested |= support & wgpu::Features::PIPELINE_CACHE;
        }
        let external_video = wgpu::Features::TEXTURE_FORMAT_NV12;
        if cfg!(target_os = "android")
            && backend == RasterBackend::Vulkan
            && support.contains(external_video)
        {
            requested |= external_video;
        }
        if support.contains(IMAGE_ARRAY_FEATURES)
            && advertised.max_binding_array_elements_per_shader_stage >= IMAGE_ARRAY_LEN
            && advertised.max_sampled_textures_per_shader_stage > IMAGE_ARRAY_LEN
            && std::env::var("IMPASTO_IMAGE_ARRAY").as_deref() != Ok("0")
        {
            requested |= IMAGE_ARRAY_FEATURES;
        }
        if !cfg!(any(target_os = "ios", target_os = "android"))
            && std::env::var("IMPASTO_LCD_RGB").as_deref() == Ok("1")
            && support.contains(lcd)
        {
            requested |= lcd;
        }
        let mut adapter = adapter;
        let device_started = Instant::now();
        let result = loop {
            let result = {
                let _trace =
                    tracing::debug_span!(target: "impasto::gpu", "gpu_device_request").entered();
                request_device(&adapter, requested)
            };
            if result.is_ok() || requested.is_empty() {
                break result;
            }
            tracing::warn!(target: "impasto::gpu", ?backend, ?requested, "retrying with fewer device features");
            if requested.contains(external_video) {
                requested.remove(external_video);
            } else if requested.contains(IMAGE_ARRAY_FEATURES) {
                requested.remove(IMAGE_ARRAY_FEATURES);
            } else if requested.intersects(modern) {
                requested.remove(modern);
            } else if requested.contains(lcd) {
                requested.remove(lcd);
            } else {
                requested = wgpu::Features::empty();
            }
            adapter = pollster::block_on(instance.request_adapter(&request))
                .map_err(|_| Status::NoDevice(backend))?;
            requested &= adapter.features();
        };
        let (device, queue) = result.map_err(|error| {
            tracing::warn!(target: "impasto::gpu", ?backend, %error, "device request failed");
            Status::NoDevice(backend)
        })?;
        let device_us = device_started.elapsed().as_secs_f64() * 1_000_000.0;
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
            caps.max_array_layers, caps.msaa4_srgb, adapter_us, device_us,
            supported = ?caps.adapter_features, enabled = ?device.features(), "graphics device ready");
        let pipeline_cache = cache_dir.and_then(|dir| {
            crate::pipeline_disk_cache::PipelineDiskCache::open(&device, &adapter, dir)
        });
        #[cfg(target_os = "android")]
        let external_queue = crate::android_vulkan::ExternalQueue::new(&device, &adapter);
        Ok(Self {
            #[cfg(target_os = "android")]
            external_queue,
            pipeline_cache,
            instance,
            adapter,
            device,
            queue,
            backend,
            caps,
            shadow_2d: std::env::var("IMPASTO_SHADOW_2D").as_deref() != Ok("0"),
            hollow_shadows: std::env::var("IMPASTO_SHADOW_HOLLOW").as_deref() != Ok("0"),
            text_gamma: std::env::var("IMPASTO_TEXT_GAMMA").as_deref() != Ok("0"),
            gradient_lut: crate::gradient_lut::enabled(),
            dither_output: std::env::var("IMPASTO_DITHER").as_deref() == Ok("1"),
            dither_gradients: std::env::var("IMPASTO_DITHER").as_deref() != Ok("0"),
            lost,
            oom,
            uncaptured_error,
            image_upload: Arc::default(),
            inline_image_pipeline: Arc::default(),
            image_upload_mode: match std::env::var("IMPASTO_IMAGE_UPLOAD").as_deref() {
                Ok("cpu") => ImageUploadMode::Cpu,
                Ok("compute") => ImageUploadMode::Compute,
                _ => ImageUploadMode::Render,
            },
            validate_frames: match options.validation {
                ValidationMode::Auto => {
                    cfg!(test) || std::env::var("IMPASTO_GPU_VALIDATE").as_deref() == Ok("1")
                }
                ValidationMode::Strict => true,
                ValidationMode::AsyncOnly => false,
            },
        })
    }

    pub fn submit(
        &self,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        #[cfg(target_os = "android")]
        let _guard = self
            .external_queue
            .as_ref()
            .map(crate::android_vulkan::ExternalQueue::lock);
        self.queue.submit(commands)
    }

    pub fn present(&self, texture: wgpu::SurfaceTexture) {
        #[cfg(target_os = "android")]
        let _guard = self
            .external_queue
            .as_ref()
            .map(crate::android_vulkan::ExternalQueue::lock);
        self.queue.present(texture);
    }

    pub fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.pipeline_cache.as_ref().map(|cache| &cache.cache)
    }

    pub fn pipelines_updated(&self) {
        if let Some(cache) = &self.pipeline_cache {
            cache.updated();
        }
    }

    pub fn image_array(&self) -> bool {
        self.device.features().contains(IMAGE_ARRAY_FEATURES)
    }

    pub fn shadow_vertices(&self) -> u32 {
        if self.hollow_shadows { 24 } else { 6 }
    }

    pub fn effect_immediates(&self) -> bool {
        self.device.features().contains(wgpu::Features::IMMEDIATES)
            && self.device.limits().max_immediate_size >= crate::pipelines::ParamsUniform::BYTE_LEN
    }

    pub fn image_upload(&self) -> Option<&crate::image_upload::ImageUpload> {
        if self.image_upload_mode == ImageUploadMode::Cpu {
            return None;
        }
        self.image_upload
            .get_or_init(|| {
                use wgpu::{TextureFormatFeatureFlags as F, TextureUsages as U};
                let format = wgpu::TextureFormat::Rgba16Float
                    .guaranteed_format_features(self.device.features());
                let compute = self.image_upload_mode == ImageUploadMode::Compute
                    && self.caps.has_compute
                    && format
                        .allowed_usages
                        .contains(U::TEXTURE_BINDING | U::STORAGE_BINDING)
                    && format.flags.contains(F::FILTERABLE | F::STORAGE_WRITE_ONLY);
                if compute
                    && let Ok(upload) =
                        self.validation(|| Ok(crate::image_upload::ImageUpload::new(self, true)))
                {
                    return Some(upload);
                }
                self.validation(|| Ok(crate::image_upload::ImageUpload::new(self, false)))
                    .ok()
            })
            .as_ref()
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
        if self.validate_frames || result.is_err() {
            let _ = self.device.poll(wgpu::PollType::Poll);
        }
        self.check_async_health()?;
        result
    }

    /// Upload factories can run on IO threads. Leave device-wide asynchronous
    /// errors pending for the owning raster queue, which handles recovery.
    pub fn upload_validation<T>(
        &self,
        operation: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        if self.validate_frames {
            return self.validation(operation);
        }
        if self.is_lost() {
            return Err(Status::DeviceLost);
        }
        let value = operation()?;
        if self.is_lost() {
            return Err(Status::DeviceLost);
        }
        Ok(value)
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
