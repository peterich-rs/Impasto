//! Device-owned conversion of decoded sRGB images into linear premultiplied mips.

use crate::gpu::Gpu;

const SOURCE: &str = include_str!("pipelines/upload.wgsl");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageUploadStats {
    pub source_bytes: u64,
    pub temporary_bytes: u64,
    pub resident_bytes: u64,
    pub render_passes: u32,
    pub compute_passes: u32,
}

impl ImageUploadStats {
    pub(crate) fn add(&mut self, other: Self) {
        self.source_bytes += other.source_bytes;
        self.temporary_bytes = self.temporary_bytes.max(other.temporary_bytes);
        self.resident_bytes += other.resident_bytes;
        self.render_passes += other.render_passes;
        self.compute_passes += other.compute_passes;
    }
}

enum Pipeline {
    Render(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline),
}

pub(crate) struct ImageUpload {
    convert: Pipeline,
    downsample: Pipeline,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
}

fn descriptor(
    label: &'static str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    levels: u32,
) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some(label),
        size: crate::compositor::extent(size),
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }
}

pub(crate) fn upload_rgba(gpu: &Gpu, texture: &wgpu::Texture, size: [u32; 2], pixels: &[u8]) {
    gpu.queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 4),
            rows_per_image: Some(size[1]),
        },
        crate::compositor::extent(size),
    );
}

impl ImageUpload {
    pub fn new(gpu: &Gpu, compute: bool) -> Self {
        let source = if compute {
            format!(
                "{SOURCE}\n{}",
                include_str!("pipelines/upload_compute.wgsl")
            )
        } else {
            SOURCE.to_owned()
        };
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("impasto image upload"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        if compute {
            let pipeline = |entry| {
                Pipeline::Compute(gpu.device.create_compute_pipeline(
                    &wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: gpu.pipeline_cache(),
                    },
                ))
            };
            Self {
                convert: pipeline("cs_premultiply"),
                downsample: pipeline("cs_downsample"),
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING,
            }
        } else {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let pipeline = |entry| {
                Pipeline::Render(gpu.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs_upload"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            targets: &[Some(format.into())],
                        }),
                        primitive: Default::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        multiview_mask: None,
                        cache: gpu.pipeline_cache(),
                    },
                ))
            };
            Self {
                convert: pipeline("fs_premultiply"),
                downsample: pipeline("fs_downsample"),
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            }
        }
    }

    pub fn prepare(
        &self,
        gpu: &Gpu,
        size: [u32; 2],
        pixels: &[u8],
    ) -> (wgpu::Texture, ImageUploadStats, wgpu::CommandBuffer) {
        use wgpu::TextureUsages as U;
        let levels = size[0].max(size[1]).ilog2() + 1;
        let source = gpu.device.create_texture(&descriptor(
            "straight sRGB image",
            size,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            U::COPY_DST | U::TEXTURE_BINDING,
            1,
        ));
        let dest = gpu.device.create_texture(&descriptor(
            "premultiplied image mips",
            size,
            self.format,
            self.usage | U::TEXTURE_BINDING,
            levels,
        ));
        upload_rgba(gpu, &source, size, pixels);
        let view = |texture: &wgpu::Texture, level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut source = view(&source, 0);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("premultiply and mip uploaded image"),
            });
        let mut stats = ImageUploadStats {
            source_bytes: pixels.len() as u64,
            temporary_bytes: pixels.len() as u64,
            ..Default::default()
        };
        let bytes_per_pixel = if matches!(self.convert, Pipeline::Compute(_)) {
            8
        } else {
            4
        };
        for level in 0..levels {
            let target = view(&dest, level);
            let pipeline = if level == 0 {
                &self.convert
            } else {
                &self.downsample
            };
            let extent = size.map(|side| (side >> level).max(1));
            pipeline.encode(gpu, &mut encoder, &source, &target, extent);
            stats.resident_bytes += u64::from(extent[0]) * u64::from(extent[1]) * bytes_per_pixel;
            match pipeline {
                Pipeline::Render(_) => stats.render_passes += 1,
                Pipeline::Compute(_) => stats.compute_passes += 1,
            }
            source = target;
        }
        (dest, stats, encoder.finish())
    }
}

impl Pipeline {
    fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        extent: [u32; 2],
    ) {
        use wgpu::BindingResource::TextureView;
        match self {
            Self::Render(pipeline) => {
                let group = crate::pipelines::bind_group(
                    gpu,
                    &pipeline.get_bind_group_layout(0),
                    &[TextureView(source)],
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("impasto image mip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            Self::Compute(pipeline) => {
                let group = crate::pipelines::bind_group(
                    gpu,
                    &pipeline.get_bind_group_layout(0),
                    &[TextureView(source), TextureView(target)],
                );
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("impasto compute image mip"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(extent[0].div_ceil(8), extent[1].div_ceil(8), 1);
            }
        }
    }
}
