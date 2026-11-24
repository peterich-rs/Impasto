//! An animated 3D reference producer for the Gallery GPU lab.

use raster::wgpu;
use raster::{GpuContext, GpuViewError, GpuViewProducer, PixelSize};

#[derive(Default)]
pub(crate) struct GalleryGpuView {
    state: Option<State>,
    started: Option<f64>,
}

struct State {
    generation: u64,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bindings: wgpu::BindGroup,
}

impl State {
    fn new(ctx: &GpuContext<'_>) -> Self {
        let device = ctx.device();
        let label = Some("impasto Gallery GPU view");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label,
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_view_gallery.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            generation: ctx.device_generation(),
            format: ctx.format(),
            pipeline,
            uniform,
            bindings,
        }
    }
}

impl GpuViewProducer for GalleryGpuView {
    fn prepare_or_render(
        &mut self,
        ctx: &GpuContext<'_>,
        target: &wgpu::TextureView,
        size: PixelSize,
        time_ms: f64,
    ) -> Result<wgpu::CommandBuffer, GpuViewError> {
        let started = *self.started.get_or_insert(time_ms);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the angle is bounded to one turn before packing the WGSL uniform"
        )]
        let angle = ((time_ms - started) * 0.0007).rem_euclid(std::f64::consts::TAU) as f32;
        if self.state.as_ref().is_some_and(|state| {
            state.generation != ctx.device_generation() || state.format != ctx.format()
        }) {
            self.state = None;
        }
        let state = self.state.get_or_insert_with(|| State::new(ctx));
        let info = [size.width as f32, size.height as f32, angle, 0.0];
        let mut bytes = [0; 16];
        let (words, _) = bytes.as_chunks_mut::<4>();
        for (chunk, value) in words.iter_mut().zip(info) {
            chunk.copy_from_slice(&value.to_ne_bytes());
        }
        ctx.queue().write_buffer(&state.uniform, 0, &bytes);
        let mut encoder = ctx
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("impasto Gallery GPU view"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impasto Gallery cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(encoder.finish())
    }

    fn on_device_lost(&mut self) {
        self.state = None;
    }
}
