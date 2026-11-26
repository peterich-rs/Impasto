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

struct UploadPipeline<P> {
    pipeline: P,
    layout: wgpu::BindGroupLayout,
}

type Render = UploadPipeline<wgpu::RenderPipeline>;
type Compute = UploadPipeline<wgpu::ComputePipeline>;

enum Pipelines {
    Render {
        convert: Render,
        downsample: Render,
        sampler: wgpu::Sampler,
    },
    Compute {
        convert: Compute,
        downsample: Compute,
        tiled: Option<[Compute; 2]>,
    },
}

pub(crate) struct ImageUpload {
    pipelines: Pipelines,
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
            format!("{SOURCE}\n{}", include_str!("pipelines/upload_render.wgsl"))
        };
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("impasto image upload"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        if compute {
            let pipeline = |entry| {
                let pipeline =
                    gpu.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: None,
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            cache: gpu.pipeline_cache(),
                        });
                let layout = pipeline.get_bind_group_layout(0);
                UploadPipeline { pipeline, layout }
            };
            let limits = gpu.device.limits();
            Self {
                pipelines: Pipelines::Compute {
                    convert: pipeline("cs_premultiply"),
                    downsample: pipeline("cs_downsample"),
                    tiled: (limits.max_storage_textures_per_shader_stage >= 4
                        && limits.max_compute_workgroup_storage_size >= 1024)
                        .then(|| {
                            [
                                pipeline("cs_premultiply_mips"),
                                pipeline("cs_downsample_mips"),
                            ]
                        }),
                },
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING,
            }
        } else {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let pipeline = |entry| {
                let pipeline = gpu
                    .device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    });
                let layout = pipeline.get_bind_group_layout(0);
                UploadPipeline { pipeline, layout }
            };
            Self {
                pipelines: Pipelines::Render {
                    convert: pipeline("fs_premultiply"),
                    downsample: pipeline("fs_downsample"),
                    sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("impasto image downsample"),
                        mag_filter: wgpu::FilterMode::Linear,
                        min_filter: wgpu::FilterMode::Linear,
                        ..Default::default()
                    }),
                },
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
            self.usage
                | U::TEXTURE_BINDING
                | if matches!(self.pipelines, Pipelines::Render { .. }) {
                    U::COPY_SRC
                } else {
                    U::empty()
                },
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
        let source = view(&source, 0);
        let targets: Vec<_> = (0..levels).map(|level| view(&dest, level)).collect();
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
        let bytes_per_pixel = match &self.pipelines {
            Pipelines::Render {
                convert,
                downsample,
                sampler,
            } => {
                let mut source = &source;
                for (level, target) in targets.iter().enumerate() {
                    let (pipeline, sampler) = if level == 0 {
                        (convert, None)
                    } else {
                        (downsample, Some(sampler))
                    };
                    pipeline.encode(gpu, &mut encoder, source, target, sampler);
                    source = target;
                }
                stats.render_passes = levels;
                4
            }
            Pipelines::Compute {
                convert,
                downsample,
                tiled,
            } => {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("impasto compute image mips"),
                    timestamp_writes: None,
                });
                let mut source = &source;
                let mut level = 0;
                while level < levels {
                    let extent = size.map(|side| (side >> level).max(1));
                    let remaining = &targets[level as usize..];
                    let base = level == 0;
                    let (pipeline, count) = match tiled {
                        Some([convert, downsample])
                            if remaining.len() >= 4
                                && extent.into_iter().all(|side| side.is_multiple_of(8)) =>
                        {
                            (if base { convert } else { downsample }, 4)
                        }
                        _ => (if base { convert } else { downsample }, 1),
                    };
                    let outputs = &remaining[..count];
                    pipeline.dispatch(gpu, &mut pass, source, outputs, extent);
                    source = &outputs[count - 1];
                    level += if count == 4 { 4 } else { 1 };
                }
                stats.compute_passes = 1;
                8
            }
        };
        stats.resident_bytes = (0..levels)
            .map(|level| {
                let [w, h] = size.map(|side| (side >> level).max(1));
                u64::from(w) * u64::from(h) * bytes_per_pixel
            })
            .sum();
        (dest, stats, encoder.finish())
    }
}

impl Render {
    fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        sampler: Option<&wgpu::Sampler>,
    ) {
        use wgpu::BindingResource::{Sampler, TextureView};
        let bindings = [
            TextureView(source),
            sampler.map_or(TextureView(source), Sampler),
        ];
        let group = crate::pipelines::bind_group(
            gpu,
            &self.layout,
            &bindings[..1 + usize::from(sampler.is_some())],
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Compute {
    fn dispatch(
        &self,
        gpu: &Gpu,
        pass: &mut wgpu::ComputePass<'_>,
        source: &wgpu::TextureView,
        targets: &[wgpu::TextureView],
        extent: [u32; 2],
    ) {
        use wgpu::BindingResource::TextureView;
        let mut bindings: [_; 5] = std::array::from_fn(|_| TextureView(source));
        for (resource, target) in bindings[1..].iter_mut().zip(targets) {
            *resource = TextureView(target);
        }
        let group = crate::pipelines::bind_group(gpu, &self.layout, &bindings[..1 + targets.len()]);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(extent[0].div_ceil(8), extent[1].div_ceil(8), 1);
    }
}
