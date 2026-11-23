//! Optional coarse-level separable Gaussian filter.

use crate::gpu::Gpu;

#[derive(Clone)]
pub(crate) struct ComputeBlur([wgpu::ComputePipeline; 2]);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BlurParams {
    pub size: [u32; 2],
    pub radius: u32,
    pub sigma: f32,
}

impl ComputeBlur {
    pub fn new(gpu: &Gpu) -> Option<Self> {
        let format = wgpu::TextureFormat::Rgba16Float;
        let features = format.guaranteed_format_features(gpu.device.features());
        let limits = gpu.device.limits();
        if std::env::var("IMPASTO_COMPUTE_BLUR").as_deref() != Ok("1")
            || !gpu.caps.has_compute
            || limits.max_compute_workgroup_storage_size < 448 * 16
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
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("impasto compute blur"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("pipelines/compute_blur.wgsl").into(),
                    ),
                });
            Ok(Self([0.0, 1.0].map(|vertical| {
                gpu.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("impasto separable blur"),
                        layout: None,
                        module: &module,
                        entry_point: Some("blur"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &[("vertical", vertical)],
                            ..Default::default()
                        },
                        cache: None,
                    })
            })))
        })
        .ok()
    }

    pub fn pipeline(&self, axis: usize) -> &wgpu::ComputePipeline {
        &self.0[axis]
    }
}
