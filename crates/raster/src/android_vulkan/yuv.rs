//! Samples an Android external format into a regular sRGB texture.

use std::{collections::HashMap, io::Cursor, sync::Arc};

use ash::{util::read_spv, vk};
use wgpu::hal::{self, api::Vulkan, vulkan::TextureMemory};

use super::{ExternalQueue, ImageAllocation, Retired, color_subresource};
use crate::{PixelSize, gpu::Gpu};

const SAMPLED_RGBA: vk::Format = vk::Format::R8G8B8A8_SRGB;
// The conversion sampler returns encoded channels. Write their bytes through
// a compatible UNORM view and let the sRGB sampling view decode them.
const COLOR_ATTACHMENT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const PIPELINE_LIMIT: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    format: u64,
    model: vk::SamplerYcbcrModelConversion,
    range: vk::SamplerYcbcrRange,
    x_chroma: vk::ChromaLocation,
    y_chroma: vk::ChromaLocation,
    filter: vk::Filter,
}

pub(super) struct Cache {
    pipelines: HashMap<Key, Arc<Pipeline>>,
    memory: vk::PhysicalDeviceMemoryProperties,
    max_descriptors: u32,
}

impl Cache {
    pub(super) fn new(adapter: &hal::vulkan::Adapter) -> Self {
        let instance = adapter.shared_instance().raw_instance();
        let physical = adapter.raw_physical_device();
        // SAFETY: both queries use the live physical device from this instance.
        let (memory, limits) = unsafe {
            (
                instance.get_physical_device_memory_properties(physical),
                instance.get_physical_device_properties(physical).limits,
            )
        };
        Self {
            pipelines: HashMap::new(),
            memory,
            max_descriptors: limits
                .max_per_stage_descriptor_samplers
                .min(limits.max_per_stage_descriptor_sampled_images),
        }
    }

    fn pipeline(
        &mut self,
        gpu: &Gpu,
        raw: &ash::Device,
        props: &vk::AndroidHardwareBufferFormatPropertiesANDROID<'_>,
    ) -> Result<Arc<Pipeline>, vk::Result> {
        let linear = props.format_features.contains(
            vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER,
        );
        let key = Key {
            format: props.external_format,
            model: props.suggested_ycbcr_model,
            range: props.suggested_ycbcr_range,
            x_chroma: props.suggested_x_chroma_offset,
            y_chroma: props.suggested_y_chroma_offset,
            filter: if linear {
                vk::Filter::LINEAR
            } else {
                vk::Filter::NEAREST
            },
        };
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Ok(Arc::clone(pipeline));
        }
        let pipeline = Arc::new(Pipeline::new(gpu, raw, key)?);
        if self.pipelines.len() >= PIPELINE_LIMIT
            && let Some(old) = self.pipelines.keys().next().copied()
        {
            self.pipelines.remove(&old);
        }
        self.pipelines.insert(key, Arc::clone(&pipeline));
        Ok(pipeline)
    }
}

struct Pipeline {
    raw: ash::Device,
    _device: wgpu::Device,
    conversion: vk::SamplerYcbcrConversion,
    sampler: vk::Sampler,
    descriptor_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pass: vk::RenderPass,
    pipeline: vk::Pipeline,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: the frame retains an Arc until its fence retires all uses
        // of the pipeline, immutable sampler and conversion.
        unsafe {
            self.raw.destroy_pipeline(self.pipeline, None);
            self.raw.destroy_render_pass(self.pass, None);
            self.raw.destroy_pipeline_layout(self.layout, None);
            self.raw
                .destroy_descriptor_set_layout(self.descriptor_layout, None);
            self.raw.destroy_sampler(self.sampler, None);
            self.raw
                .destroy_sampler_ycbcr_conversion(self.conversion, None);
        }
    }
}

impl Pipeline {
    fn new(gpu: &Gpu, raw: &ash::Device, key: Key) -> Result<Self, vk::Result> {
        let mut result = Self {
            raw: raw.clone(),
            _device: gpu.device.clone(),
            conversion: vk::SamplerYcbcrConversion::null(),
            sampler: vk::Sampler::null(),
            descriptor_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pass: vk::RenderPass::null(),
            pipeline: vk::Pipeline::null(),
        };
        let mut format = vk::ExternalFormatANDROID::default().external_format(key.format);
        let attachments = [vk::AttachmentDescription::default()
            .format(COLOR_ATTACHMENT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let color = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color)];
        let dependencies = [vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)];
        // SAFETY: the queried Android format and its suggested conversion values
        // are passed to the enabled feature. The sampler and view share that
        // conversion. Every object is owned by result, including on early return.
        unsafe {
            result.conversion = raw.create_sampler_ycbcr_conversion(
                &vk::SamplerYcbcrConversionCreateInfo::default()
                    .format(vk::Format::UNDEFINED)
                    .ycbcr_model(key.model)
                    .ycbcr_range(key.range)
                    .x_chroma_offset(key.x_chroma)
                    .y_chroma_offset(key.y_chroma)
                    .chroma_filter(key.filter)
                    .push_next(&mut format),
                None,
            )?;
            let mut conversion =
                vk::SamplerYcbcrConversionInfo::default().conversion(result.conversion);
            result.sampler = raw.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(key.filter)
                    .min_filter(key.filter)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .push_next(&mut conversion),
                None,
            )?;
            let samplers = [result.sampler];
            let bindings = [vk::DescriptorSetLayoutBinding::default()
                .descriptor_count(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .immutable_samplers(&samplers)];
            result.descriptor_layout = raw.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            result.layout = raw.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[result.descriptor_layout]),
                None,
            )?;
            result.pass = raw.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies),
                None,
            )?;
        }
        result.pipeline = result.graphics_pipeline()?;
        Ok(result)
    }

    fn graphics_pipeline(&self) -> Result<vk::Pipeline, vk::Result> {
        let vertex = Shader::new(&self.raw, include_bytes!("yuv.vert.spv"))?;
        let fragment = Shader::new(&self.raw, include_bytes!("yuv.frag.spv"))?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex.handle)
                .name(c"main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment.handle)
                .name(c"main"),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0);
        let samples = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let colors = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&colors);
        let dynamic = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&samples)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(self.layout)
            .render_pass(self.pass);
        // SAFETY: shader interfaces match the immutable combined sampler and
        // the only color attachment. All create-info references are live.
        unsafe {
            self.raw
                .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
                .map(|pipelines| pipelines[0])
                .map_err(|(pipelines, error)| {
                    for pipeline in pipelines {
                        self.raw.destroy_pipeline(pipeline, None);
                    }
                    error
                })
        }
    }
}

struct Shader<'a> {
    device: &'a ash::Device,
    handle: vk::ShaderModule,
}

impl<'a> Shader<'a> {
    fn new(device: &'a ash::Device, code: &[u8]) -> Result<Self, vk::Result> {
        let words = read_spv(&mut Cursor::new(code))
            .map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;
        // SAFETY: the embedded SPIR-V was built for Vulkan from the adjacent GLSL source.
        let handle = unsafe {
            device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
        }?;
        Ok(Self { device, handle })
    }
}

impl Drop for Shader<'_> {
    fn drop(&mut self) {
        // SAFETY: pipeline creation has returned and the device is borrowed.
        unsafe { self.device.destroy_shader_module(self.handle, None) };
    }
}

struct Output {
    raw: ash::Device,
    _device: wgpu::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

impl Output {
    fn new(
        gpu: &Gpu,
        raw: &ash::Device,
        size: PixelSize,
        properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<Self, vk::Result> {
        let mut output = Self {
            raw: raw.clone(),
            _device: gpu.device.clone(),
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
        };
        // SAFETY: RGBA8 supports color attachments and sampling on this device.
        // The dedicated memory type must be compatible with the created image.
        unsafe {
            output.image = raw.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(SAMPLED_RGBA)
                    .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
                    .extent(vk::Extent3D {
                        width: size.width,
                        height: size.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED),
                None,
            )?;
            let requirements = raw.get_image_memory_requirements(output.image);
            let memory_type = (0..properties.memory_type_count)
                .filter(|&i| requirements.memory_type_bits & (1 << i) != 0)
                .min_by_key(|&i| {
                    !properties.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                })
                .ok_or(vk::Result::ERROR_FEATURE_NOT_PRESENT)?;
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(output.image);
            output.memory = raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type)
                    .push_next(&mut dedicated),
                None,
            )?;
            raw.bind_image_memory(output.image, output.memory, 0)?;
        }
        Ok(output)
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: the conversion fence and wgpu's HAL drop callback each retain
        // an Arc throughout their respective use of the image.
        unsafe {
            self.raw.destroy_image(self.image, None);
            self.raw.free_memory(self.memory, None);
        }
    }
}

struct Frame {
    input: ImageAllocation,
    output: Arc<Output>,
    pipeline: Arc<Pipeline>,
    input_view: vk::ImageView,
    output_view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    descriptors: vk::DescriptorPool,
}

impl Drop for Frame {
    fn drop(&mut self) {
        let raw = &self.input.device;
        // SAFETY: a submitted frame waits for completion before dropping these
        // objects. The source, output and pipeline outlive their dependents.
        unsafe {
            raw.destroy_descriptor_pool(self.descriptors, None);
            raw.destroy_framebuffer(self.framebuffer, None);
            raw.destroy_image_view(self.output_view, None);
            raw.destroy_image_view(self.input_view, None);
        }
    }
}

impl Frame {
    fn allocate_set(&mut self, count: u32) -> Result<vk::DescriptorSet, vk::Result> {
        let raw = &self.input.device;
        let sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: count,
        }];
        // SAFETY: the pool is private to this frame and unused on entry. The
        // immutable sampler's layout lives in the retained pipeline.
        unsafe {
            raw.destroy_descriptor_pool(self.descriptors, None);
            self.descriptors = vk::DescriptorPool::null();
            self.descriptors = raw.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&sizes),
                None,
            )?;
            Ok(raw.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.descriptors)
                    .set_layouts(&[self.pipeline.descriptor_layout]),
            )?[0])
        }
    }

    fn record(
        &mut self,
        size: PixelSize,
        max_descriptors: u32,
    ) -> Result<vk::CommandBuffer, vk::Result> {
        // External YUV usually consumes up to four hardware descriptors. The
        // pipeline layout is limited by max_descriptors even on other drivers.
        let set = match self.allocate_set(max_descriptors.min(4)) {
            Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY) if max_descriptors > 4 => {
                self.allocate_set(max_descriptors)?
            }
            result => result?,
        };
        let raw = &self.input.device;
        let pipeline = &self.pipeline;
        let mut conversion =
            vk::SamplerYcbcrConversionInfo::default().conversion(pipeline.conversion);
        let view = |image, format| {
            vk::ImageViewCreateInfo::default()
                .image(image)
                .format(format)
                .view_type(vk::ImageViewType::TYPE_2D)
                .subresource_range(color_subresource())
        };
        let area = vk::Rect2D::default().extent(vk::Extent2D {
            width: size.width,
            height: size.height,
        });
        // SAFETY: both images are fully bound. The external image view uses
        // the identical conversion as the immutable sampler in this set.
        unsafe {
            self.input_view = raw.create_image_view(
                &view(self.input.image, vk::Format::UNDEFINED).push_next(&mut conversion),
                None,
            )?;
            self.output_view =
                raw.create_image_view(&view(self.output.image, COLOR_ATTACHMENT), None)?;
            self.framebuffer = raw.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(pipeline.pass)
                    .attachments(&[self.output_view])
                    .width(size.width)
                    .height(size.height)
                    .layers(1),
                None,
            )?;
            let descriptor = [vk::DescriptorImageInfo::default()
                .image_view(self.input_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&descriptor)],
                &[],
            );
        }
        let command = self.input.begin_commands()?;
        self.input.record_transfer(command, true);
        // SAFETY: the command buffer is recording and the input acquire precedes
        // sampling. Viewport and scissor cover the whole initialized attachment.
        unsafe {
            raw.cmd_begin_render_pass(
                command,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(pipeline.pass)
                    .framebuffer(self.framebuffer)
                    .render_area(area),
                vk::SubpassContents::INLINE,
            );
            raw.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, pipeline.pipeline);
            raw.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout,
                0,
                &[set],
                &[],
            );
            raw.cmd_set_viewport(
                command,
                0,
                &[vk::Viewport::default()
                    .width(size.width as f32)
                    .height(size.height as f32)
                    .max_depth(1.0)],
            );
            raw.cmd_set_scissor(command, 0, &[area]);
            raw.cmd_draw(command, 3, 1, 0, 0);
            raw.cmd_end_render_pass(command);
        }
        self.input.record_transfer(command, false);
        Ok(command)
    }
}

pub(super) struct SubmittedFrame {
    frame: Frame,
    fence: vk::Fence,
    lease: Box<dyn FnOnce() + Send + Sync>,
}

impl SubmittedFrame {
    pub(super) fn release(self) {
        let Self {
            frame,
            fence,
            lease,
        } = self;
        let raw = &frame.input.device;
        // SAFETY: the frame owns the live fence and all GPU resources. Wait
        // off the raster thread before dropping them and returning the lease.
        unsafe {
            loop {
                match raw.wait_for_fences(&[fence], true, u64::MAX) {
                    Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST) => break,
                    Err(_) => std::thread::yield_now(),
                }
            }
            raw.destroy_fence(fence, None);
        }
        drop(frame);
        lease();
    }
}

pub(super) fn convert(
    gpu: &Gpu,
    queue: &ExternalQueue,
    input: ImageAllocation,
    size: PixelSize,
    props: &vk::AndroidHardwareBufferFormatPropertiesANDROID<'_>,
    lease: Box<dyn FnOnce() + Send + Sync>,
) -> Result<wgpu::Texture, vk::Result> {
    let cache = queue
        .yuv
        .as_ref()
        .ok_or(vk::Result::ERROR_FEATURE_NOT_PRESENT)?;
    let (pipeline, memory, max_descriptors) = {
        let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
        let pipeline = cache.pipeline(gpu, &input.device, props)?;
        (pipeline, cache.memory, cache.max_descriptors)
    };
    let output = Arc::new(Output::new(gpu, &input.device, size, &memory)?);
    let mut frame = Frame {
        input,
        output: Arc::clone(&output),
        pipeline,
        input_view: vk::ImageView::null(),
        output_view: vk::ImageView::null(),
        framebuffer: vk::Framebuffer::null(),
        descriptors: vk::DescriptorPool::null(),
    };
    // SAFETY: this HAL guard belongs to the same device that created the images.
    let hal = unsafe { gpu.device.as_hal::<Vulkan>() }.ok_or(vk::Result::ERROR_DEVICE_LOST)?;
    let command = frame.record(size, max_descriptors)?;
    let fence = frame.input.submit(command, &queue.gate)?;
    if let Err(error) = queue.retired.send(Retired::Converted(SubmittedFrame {
        frame,
        fence,
        lease,
    })) {
        error.0.release(&queue.gate);
    }
    let desc = wgpu::TextureDescriptor {
        label: Some("impasto converted Android YUV"),
        size: crate::compositor::extent([size.width, size.height]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let raw_desc = hal::TextureDescriptor {
        label: desc.label,
        size: desc.size,
        mip_level_count: desc.mip_level_count,
        sample_count: desc.sample_count,
        dimension: desc.dimension,
        format: desc.format,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    // SAFETY: the queued pass initializes this exact image and leaves it in
    // SHADER_READ_ONLY_OPTIMAL. Subsequent wgpu submissions use the same queue
    // and depend on the pass's outgoing barrier. The drop hook retains the image.
    let raw = unsafe {
        hal.texture_from_raw(
            output.image,
            &raw_desc,
            Some(Box::new(move || drop(output))),
            TextureMemory::External,
        )
    };
    drop(hal);
    // SAFETY: the raw texture has the descriptor and post-conversion state above.
    Ok(unsafe {
        gpu.device
            .create_texture_from_hal::<Vulkan>(raw, &desc, wgpu::TextureUses::RESOURCE)
    })
}
