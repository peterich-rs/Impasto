//! Device-owned conversion of decoded sRGB images into linear premultiplied mips.

use crate::gpu::Gpu;

const SOURCE: &str = include_str!("pipelines/upload.wgsl");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageUploadStats {
    pub source_bytes: u64,
    pub temporary_bytes: u64,
    pub resident_bytes: u64,
    pub render_passes: u32,
}

impl ImageUploadStats {
    pub(crate) fn add(&mut self, other: Self) {
        self.source_bytes += other.source_bytes;
        self.temporary_bytes = self.temporary_bytes.max(other.temporary_bytes);
        self.resident_bytes += other.resident_bytes;
        self.render_passes += other.render_passes;
    }
}

struct UploadPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

pub(crate) struct ImageUpload {
    convert: UploadPipeline,
    downsample: UploadPipeline,
    sampler: wgpu::Sampler,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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
    pub fn new(gpu: &Gpu) -> Self {
        let source = format!("{SOURCE}\n{}", include_str!("pipelines/upload_render.wgsl"));
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("impasto image upload"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
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
                        targets: &[Some(FORMAT.into())],
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
            convert: pipeline("fs_premultiply"),
            downsample: pipeline("fs_downsample"),
            sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("impasto image downsample"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    pub fn prepare(
        &self,
        gpu: &Gpu,
        size: [u32; 2],
        pixels: &[u8],
    ) -> (wgpu::TextureView, ImageUploadStats, wgpu::CommandBuffer) {
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
            FORMAT,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC,
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
        let mut source = view(&source, 0);
        for level in 0..levels {
            let target = view(&dest, level);
            let (pipeline, sampler) = if level == 0 {
                (&self.convert, None)
            } else {
                (&self.downsample, Some(&self.sampler))
            };
            pipeline.encode(gpu, &mut encoder, &source, &target, sampler);
            source = target;
        }
        stats.render_passes = levels;
        stats.resident_bytes = (0..levels)
            .map(|level| {
                let [w, h] = size.map(|side| (side >> level).max(1));
                u64::from(w) * u64::from(h) * 4
            })
            .sum();
        (
            dest.create_view(&Default::default()),
            stats,
            encoder.finish(),
        )
    }
}

impl UploadPipeline {
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
