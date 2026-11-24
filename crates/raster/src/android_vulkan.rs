//! Android HardwareBuffer import and foreign queue-family ownership transfers.

#![expect(
    unsafe_code,
    reason = "Vulkan and Android share the producer's native allocation"
)]

mod yuv;

use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Sender},
    },
};

use ash::{android::external_memory_android_hardware_buffer as ahb, ext::queue_family_foreign, vk};
use wgpu::hal::{self, api::Vulkan, vulkan::TextureMemory};

use crate::{GpuImage, PixelSize, Rasterizer, gpu::Gpu};

fn sampler_conversion_feature(hal: &hal::vulkan::Adapter) -> bool {
    let instance = hal.shared_instance();
    if instance.instance_api_version() < vk::API_VERSION_1_1 {
        return false;
    }
    let physical = hal.raw_physical_device();
    let raw = instance.raw_instance();
    // SAFETY: this physical device belongs to the live Vulkan instance.
    if unsafe { raw.get_physical_device_properties(physical) }.api_version < vk::API_VERSION_1_1 {
        return false;
    }
    let mut feature = vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default();
    let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut feature);
    // SAFETY: the Vulkan 1.1 instance supports the query and the output chain is writable.
    unsafe { raw.get_physical_device_features2(physical, &mut features) };
    feature.sampler_ycbcr_conversion == vk::TRUE
}

pub fn open(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
) -> Option<Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>> {
    // SAFETY: the borrowed HAL adapter must be the Vulkan adapter associated with wgpu.
    let hal = unsafe { adapter.as_hal::<Vulkan>() }?;
    let extensions = [ahb::NAME, queue_family_foreign::NAME];
    if !extensions
        .iter()
        .all(|name| hal.physical_device_capabilities().supports_extension(name))
    {
        return None;
    }
    let mut sampler_conversion =
        vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default().sampler_ycbcr_conversion(true);
    let conversion = sampler_conversion_feature(&hal).then_some(&mut sampler_conversion);
    // SAFETY: the adapter advertised both extensions. The callback extends the
    // normal HAL request and leaves the required features and queues intact.
    let opened = unsafe {
        hal.open_with_callback(
            desc.required_features,
            &desc.required_limits,
            &desc.memory_hints,
            Some(Box::new(move |args| {
                // wgpu 30 exposes manual plane sampling and leaves this feature disabled.
                if let Some(conversion) = conversion {
                    *args.create_info = args.create_info.push_next(conversion);
                }
                for name in extensions {
                    if !args.extensions.contains(&name) {
                        args.extensions.push(name);
                    }
                }
            })),
        )
    }
    .inspect_err(|error| {
        tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer device request failed");
    })
    .ok()?;
    drop(hal);
    // SAFETY: opened came from this adapter and was created for this descriptor.
    Some(unsafe { adapter.create_device_from_hal(opened, desc) })
}

#[derive(Clone)]
pub struct ExternalQueue {
    gate: Arc<Mutex<()>>,
    retired: Sender<Retired>,
    yuv: Option<Arc<Mutex<yuv::Cache>>>,
    imported: Arc<AtomicU8>,
}

impl ExternalQueue {
    pub fn new(device: &wgpu::Device, adapter: &wgpu::Adapter) -> Option<Self> {
        // SAFETY: the guard keeps the raw Vulkan device valid for this read.
        let hal = unsafe { device.as_hal::<Vulkan>() }?;
        if !hal.enabled_device_extensions().contains(&ahb::NAME) {
            return None;
        }
        // SAFETY: this Vulkan adapter backs the AHB-capable device opened above.
        let adapter = unsafe { adapter.as_hal::<Vulkan>() }?;
        let yuv = sampler_conversion_feature(&adapter)
            .then(|| Arc::new(Mutex::new(yuv::Cache::new(&adapter))));
        tracing::debug!(target: "impasto::gpu", ycbcr = yuv.is_some(), "Vulkan HardwareBuffer extensions enabled");
        let gate = Arc::new(Mutex::new(()));
        let (retired, releases) = mpsc::channel::<Retired>();
        let queue = Arc::clone(&gate);
        std::thread::Builder::new()
            .name("impasto Android texture release".into())
            .spawn(move || {
                while let Ok(item) = releases.recv() {
                    item.release(&queue);
                }
            })
            .ok()?;
        Some(Self {
            gate,
            retired,
            yuv,
            imported: Arc::default(),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, ()> {
        self.gate.lock().unwrap_or_else(|error| error.into_inner())
    }
}

struct ImageAllocation {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    pool: vk::CommandPool,
    queue: vk::Queue,
    family: u32,
}

impl Drop for ImageAllocation {
    fn drop(&mut self) {
        // SAFETY: the transfer fence or device teardown has retired submissions
        // using these objects. The retained wgpu device outlives this allocation.
        unsafe {
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

enum Retired {
    Direct {
        allocation: ImageAllocation,
        device: wgpu::Device,
        lease: Box<dyn FnOnce() + Send + Sync>,
    },
    Converted(yuv::SubmittedFrame),
}

impl Retired {
    fn release(self, gate: &Mutex<()>) {
        match self {
            Self::Direct {
                allocation,
                device,
                lease,
            } => {
                // The Vulkan image is unused by wgpu when its HAL callback runs.
                let _ = allocation.transfer(gate, false);
                drop(allocation);
                lease();
                drop(device);
            }
            Self::Converted(frame) => frame.release(),
        }
    }
}

impl ImageAllocation {
    fn begin_commands(&self) -> Result<vk::CommandBuffer, vk::Result> {
        // SAFETY: the sole owner records commands in this private command pool.
        unsafe {
            let command = self.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            self.device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            Ok(command)
        }
    }

    fn record_transfer(&self, command: vk::CommandBuffer, acquire: bool) {
        let graphics = (
            self.family,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );
        let foreign = (
            vk::QUEUE_FAMILY_FOREIGN_EXT,
            vk::ImageLayout::GENERAL,
            vk::AccessFlags::empty(),
            if acquire {
                vk::PipelineStageFlags::TOP_OF_PIPE
            } else {
                vk::PipelineStageFlags::BOTTOM_OF_PIPE
            },
        );
        let (from, to) = if acquire {
            (foreign, graphics)
        } else {
            (graphics, foreign)
        };
        let barrier = vk::ImageMemoryBarrier::default()
            .src_queue_family_index(from.0)
            .dst_queue_family_index(to.0)
            .old_layout(from.1)
            .new_layout(to.1)
            .src_access_mask(from.2)
            .dst_access_mask(to.2)
            .image(self.image)
            .subresource_range(color_subresource());
        // SAFETY: the command buffer is recording, the image is bound, and
        // the frame owns its producer lease throughout the transfer.
        unsafe {
            self.device.cmd_pipeline_barrier(
                command,
                from.3,
                to.3,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    fn submit(
        &self,
        command: vk::CommandBuffer,
        gate: &Mutex<()>,
    ) -> Result<vk::Fence, vk::Result> {
        let device = &self.device;
        // SAFETY: the pool and all referenced resources are retained until the
        // fence signals. This gate serializes raw and wgpu uses of the queue.
        unsafe {
            device.end_command_buffer(command)?;
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let submitted = {
                let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
                device.queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default().command_buffers(&[command])],
                    fence,
                )
            };
            if let Err(error) = submitted {
                device.destroy_fence(fence, None);
                return Err(error);
            }
            Ok(fence)
        }
    }

    fn transfer(&self, gate: &Mutex<()>, acquire: bool) -> Result<(), vk::Result> {
        let command = self.begin_commands()?;
        self.record_transfer(command, acquire);
        let fence = self.submit(command, gate)?;
        // SAFETY: the fence belongs to this transfer. Once it signals, the
        // command is complete; a device loss also terminates its execution.
        unsafe {
            let result = self.device.wait_for_fences(&[fence], true, u64::MAX);
            self.device.destroy_fence(fence, None);
            self.device.free_command_buffers(self.pool, &[command]);
            result
        }
    }
}

fn color_subresource() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}

impl Rasterizer {
    /// Imports a ready Android hardware buffer on a Vulkan device.
    ///
    /// # Safety
    ///
    /// The buffer must remain live, GPU-sampleable and exclusively leased
    /// from the producer. Its producer fence must have completed. The caller
    /// must keep this lease alive until the supplied callback is dropped or
    /// invoked. The size must match the native buffer's full, uncropped image.
    pub unsafe fn import_android_texture(
        &self,
        buffer: NonNull<c_void>,
        size: PixelSize,
        lease: impl FnOnce() + Send + Sync + 'static,
    ) -> Result<Option<GpuImage>, crate::Status> {
        let Some(state) = self.device.as_ref() else {
            return Ok(None);
        };
        crate::surface::check_size(&state.gpu, size)?;
        if state.gpu.is_lost() {
            return Err(crate::Status::DeviceLost);
        }
        state.gpu.validation(|| {
            // SAFETY: the caller supplies the buffer, synchronization and lease.
            Ok(unsafe { import(&state.gpu, state.generation, buffer, size, Box::new(lease)) })
        })
    }
}

unsafe fn import(
    gpu: &Gpu,
    generation: u64,
    buffer: NonNull<c_void>,
    size: PixelSize,
    lease: Box<dyn FnOnce() + Send + Sync>,
) -> Option<GpuImage> {
    let external = gpu.external_queue.as_ref()?;
    // SAFETY: both HAL borrows use the same Vulkan adapter/device and only
    // query them while holding wgpu's lifetime guards.
    let hal_adapter = unsafe { gpu.adapter.as_hal::<Vulkan>() }?;
    // SAFETY: this device is from the adapter above and its guard keeps it live.
    let hal_device = unsafe { gpu.device.as_hal::<Vulkan>() }?;
    let device = hal_device.raw_device();
    let extension = ahb::Device::new(hal_adapter.shared_instance().raw_instance(), device);
    let mut format_properties = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
    let mut properties =
        vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format_properties);
    let native_buffer = buffer.as_ptr().cast();
    // SAFETY: the producer lease keeps the AHardwareBuffer live through import.
    unsafe { extension.get_android_hardware_buffer_properties(native_buffer, &mut properties) }
        .inspect_err(|error| {
            tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer property query failed");
        })
        .ok()?;
    let allocation_size = properties.allocation_size;
    let memory_type_bits = properties.memory_type_bits;
    let format = match format_properties.format {
        vk::Format::R8G8B8A8_UNORM => Some(wgpu::TextureFormat::Rgba8Unorm),
        vk::Format::B8G8R8A8_UNORM => Some(wgpu::TextureFormat::Bgra8Unorm),
        vk::Format::G8_B8R8_2PLANE_420_UNORM
            if gpu
                .device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
                && size.width.is_multiple_of(2)
                && size.height.is_multiple_of(2) =>
        {
            Some(wgpu::TextureFormat::NV12)
        }
        vk::Format::UNDEFINED
            if format_properties.external_format != 0 && external.yuv.is_some() =>
        {
            None
        }
        format => {
            tracing::trace!(target: "impasto::gpu", ?format, "unsupported Android HardwareBuffer format");
            return None;
        }
    };
    if memory_type_bits == 0
        || !format_properties
            .format_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
    {
        return None;
    }
    let mut external_image = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
    let mut external_format =
        vk::ExternalFormatANDROID::default().external_format(format_properties.external_format);
    let mut image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format_properties.format)
        .extent(vk::Extent3D {
            width: size.width,
            height: size.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .flags(if format == Some(wgpu::TextureFormat::NV12) {
            vk::ImageCreateFlags::MUTABLE_FORMAT
        } else {
            vk::ImageCreateFlags::empty()
        })
        .push_next(&mut external_image);
    if format.is_none() {
        image_info = image_info.push_next(&mut external_format);
    }
    // SAFETY: requested base format, extents, usage and tiling match the
    // AHardwareBuffer and the queried Vulkan properties.
    let image = unsafe { device.create_image(&image_info, None) }
        .inspect_err(|error| {
            tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer image creation failed");
        })
        .ok()?;
    let family = hal_device.queue_family_index();
    let mut allocation = ImageAllocation {
        device: device.clone(),
        image,
        memory: vk::DeviceMemory::null(),
        pool: vk::CommandPool::null(),
        queue: hal_device.raw_queue(),
        family,
    };
    let mut import = vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(native_buffer);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let info = vk::MemoryAllocateInfo::default()
        .allocation_size(allocation_size)
        .memory_type_index(memory_type_bits.trailing_zeros())
        .push_next(&mut import)
        .push_next(&mut dedicated);
    // SAFETY: dedicated allocation imports this live buffer using its supported
    // memory type. A successful allocation retains the hardware buffer.
    unsafe {
        allocation.memory = device.allocate_memory(&info, None)
            .inspect_err(|error| tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer allocation failed")).ok()?;
        device.bind_image_memory(image, allocation.memory, 0)
            .inspect_err(|error| tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer memory bind failed")).ok()?;
        allocation.pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
            .ok()?;
    }
    let Some(format) = format else {
        drop(hal_device);
        drop(hal_adapter);
        let texture = yuv::convert(gpu, external, allocation, size, &format_properties, lease)
            .inspect_err(|error| tracing::debug!(target: "impasto::gpu", ?error, "Vulkan external YUV conversion failed"))
            .ok()?;
        if external.imported.fetch_or(4, Ordering::Relaxed) & 4 == 0 {
            tracing::info!(target: "impasto::gpu", external_format = format_properties.external_format, "Vulkan HardwareBuffer external YUV conversion active");
        }
        return GpuImage::from_android(gpu, texture, size, generation, (0.0, 0.0), false, [0.0; 2])
            .ok();
    };
    allocation.transfer(&external.gate, true)
        .inspect_err(|error| tracing::debug!(target: "impasto::gpu", ?error, "Vulkan HardwareBuffer acquire failed"))
        .ok()?;
    let descriptor = wgpu::TextureDescriptor {
        label: Some("impasto Android hardware buffer"),
        size: crate::compositor::extent([size.width, size.height]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let raw_descriptor = hal::TextureDescriptor {
        label: descriptor.label,
        size: descriptor.size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: descriptor.dimension,
        format,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let send = external.retired.clone();
    let guard = Retired::Direct {
        allocation,
        device: gpu.device.clone(),
        lease,
    };
    // SAFETY: this exact VkDevice allocated the accepted image, whose layout
    // and family were set by the completed acquire. The retire queue owns the
    // image and its allocation until the final GPU use is complete.
    let raw = unsafe {
        hal_device.texture_from_raw(
            image,
            &raw_descriptor,
            Some(Box::new(move || {
                let _ = send.send(guard);
            })),
            TextureMemory::External,
        )
    };
    drop(hal_device);
    drop(hal_adapter);
    // SAFETY: the raw wrapper matches descriptor and was initialized as a sampled texture.
    let texture = unsafe {
        gpu.device
            .create_texture_from_hal::<Vulkan>(raw, &descriptor, wgpu::TextureUses::RESOURCE)
    };
    let weights = match format_properties.suggested_ycbcr_model {
        vk::SamplerYcbcrModelConversion::YCBCR_709 => (0.2126, 0.0722),
        vk::SamplerYcbcrModelConversion::YCBCR_2020 => (0.2627, 0.0593),
        _ => (0.299, 0.114),
    };
    let chroma_offset = [
        f32::from(format_properties.suggested_x_chroma_offset == vk::ChromaLocation::COSITED_EVEN)
            * 0.5
            / size.width as f32,
        f32::from(format_properties.suggested_y_chroma_offset == vk::ChromaLocation::COSITED_EVEN)
            * 0.5
            / size.height as f32,
    ];
    let image = GpuImage::from_android(
        gpu,
        texture,
        size,
        generation,
        weights,
        format_properties.suggested_ycbcr_range == vk::SamplerYcbcrRange::ITU_NARROW,
        chroma_offset,
    )
    .ok()?;
    let nv12 = format == wgpu::TextureFormat::NV12;
    let bit = if nv12 { 2 } else { 1 };
    if external.imported.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        tracing::info!(target: "impasto::gpu", nv12, "Vulkan HardwareBuffer import active");
    }
    Some(image)
}
