//! Optional coarse-level separable Gaussian filter.

use crate::gpu::Gpu;

#[derive(Clone)]
pub(crate) struct ComputeBlur {
    pipelines: [wgpu::ComputePipeline; 2],
    pub layout: wgpu::BindGroupLayout,
    pub format: wgpu::TextureFormat,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BlurParams {
    pub size: [u32; 2],
    pub radius: u32,
    pub sigma: f32,
}

impl ComputeBlur {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Option<Self> {
        let storage_format = format.remove_srgb_suffix();
        let features = storage_format.guaranteed_format_features(gpu.device.features());
        let limits = gpu.device.limits();
        let f16_tile = gpu.device.features().contains(wgpu::Features::SHADER_F16);
        if std::env::var("IMPASTO_COMPUTE_BLUR").as_deref() != Ok("1")
            || !gpu.caps.has_compute
            || format.is_srgb()
                && !gpu
                    .caps
                    .downlevel_flags
                    .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
            || limits.max_compute_invocations_per_workgroup < 128
            || limits.max_compute_workgroup_size_x < 16
            || limits.max_compute_workgroup_size_y < 8
            || limits.max_compute_workgroup_storage_size
                < 512 * if f16_tile { 8 } else { 16 } + 25 * 4
            || !features.allowed_usages.contains(
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            || !features.flags.contains(
                wgpu::TextureFormatFeatureFlags::FILTERABLE
                    | wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY,
            )
        {
            return None;
        }
        gpu.validation(|| {
            let base = include_str!("pipelines/compute_blur.wgsl");
            let base = if format.is_srgb() {
                std::borrow::Cow::Owned(base.replace("rgba16float, write", "rgba8unorm, write"))
            } else {
                std::borrow::Cow::Borrowed(base)
            };
            let source = if f16_tile {
                std::borrow::Cow::Owned(format!(
                    "enable f16;
{}",
                    base.replace(
                        "alias TilePixel = vec4<f32>;",
                        "alias TilePixel = vec4<f16>;"
                    )
                ))
            } else {
                base
            };
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("impasto compute blur"),
                    source: wgpu::ShaderSource::Wgsl(source),
                });
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            };
            let layout = gpu
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("impasto compute blur resources"),
                    entries: &[
                        entry(
                            0,
                            wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                        ),
                        entry(
                            1,
                            wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: storage_format,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                        ),
                        entry(
                            2,
                            wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: true,
                                min_binding_size: wgpu::BufferSize::new(
                                    size_of::<BlurParams>() as u64
                                ),
                            },
                        ),
                    ],
                });
            let pipeline_layout =
                gpu.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("impasto compute blur layout"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipelines = [0.0, 1.0].map(|vertical| {
                gpu.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("impasto separable blur"),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some("blur"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &[
                                ("vertical", vertical),
                                ("output_srgb", f64::from(format.is_srgb())),
                            ],
                            ..Default::default()
                        },
                        cache: gpu.pipeline_cache(),
                    })
            });
            Ok(Self {
                pipelines,
                layout,
                format,
            })
        })
        .ok()
    }

    pub fn pipeline(&self, axis: usize) -> &wgpu::ComputePipeline {
        &self.pipelines[axis]
    }
}
