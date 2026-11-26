//! Owned draw commands for a Canvas pass with MSAA resolve and optional stencil.

use super::{Image, PassStamps, set_scissor};
use crate::arena::{DynamicGroup, VertexSlice};
use crate::batch::GpuMeshBuffer;

pub(super) struct CanvasPass {
    resolved: wgpu::TextureView,
    multisample: Option<wgpu::TextureView>,
    stencil: Option<wgpu::TextureView>,
    pub(super) stamps: Option<PassStamps>,
    area: [u32; 4],
    commands: Vec<Command>,
}

enum Command {
    Pipeline(wgpu::RenderPipeline),
    Globals(DynamicGroup),
    Material(wgpu::BindGroup),
    Geometry(GpuMeshBuffer),
    Paint(VertexSlice),
    Stencil(u32),
    Draw(std::ops::Range<u32>, u32),
}

impl CanvasPass {
    pub(super) fn new(
        resolved: &Image,
        multisample: Option<&Image>,
        stencil: Option<&Image>,
        [_, _, width, height]: [u32; 4],
    ) -> Self {
        Self {
            resolved: resolved.view.clone(),
            multisample: multisample.map(|image| image.view.clone()),
            stencil: stencil.map(|image| image.view.clone()),
            stamps: None,
            area: [0, 0, width, height],
            commands: Vec::new(),
        }
    }

    pub(super) fn pipeline(&mut self, pipeline: &wgpu::RenderPipeline) {
        self.commands.push(Command::Pipeline(pipeline.clone()));
    }

    pub(super) fn globals(&mut self, globals: &DynamicGroup) {
        self.commands.push(Command::Globals(globals.clone()));
    }

    pub(super) fn material(&mut self, group: &wgpu::BindGroup) {
        self.commands.push(Command::Material(group.clone()));
    }

    pub(super) fn geometry(&mut self, geometry: &GpuMeshBuffer) {
        self.commands.push(Command::Geometry(geometry.clone()));
    }

    pub(super) fn paint(&mut self, vertices: &VertexSlice) {
        self.commands.push(Command::Paint(vertices.clone()));
    }

    pub(super) fn stencil(&mut self, value: u32) {
        self.commands.push(Command::Stencil(value));
    }

    pub(super) fn draw(&mut self, indices: std::ops::Range<u32>, instance: u32) {
        self.commands.push(Command::Draw(indices, instance));
    }

    pub(super) fn encode(self, encoder: &mut wgpu::CommandEncoder) {
        let timestamp_writes = self.stamps.as_ref().map(PassStamps::render);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas path batch"),
            timestamp_writes,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.multisample.as_ref().unwrap_or(&self.resolved),
                depth_slice: None,
                resolve_target: self.multisample.as_ref().map(|_| &self.resolved),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if self.multisample.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: self.stencil.as_ref().map(|view| {
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }
            }),
            ..Default::default()
        });
        set_scissor(&mut pass, Some(self.area));
        for command in &self.commands {
            match command {
                Command::Pipeline(pipeline) => pass.set_pipeline(pipeline),
                Command::Globals(globals) => {
                    pass.set_bind_group(0, &globals.group, globals.offsets())
                }
                Command::Material(material) => pass.set_bind_group(1, material, &[]),
                Command::Geometry(geometry) => {
                    pass.set_vertex_buffer(0, geometry.vertices.slice());
                    pass.set_index_buffer(geometry.indices.slice(), wgpu::IndexFormat::Uint32);
                }
                Command::Paint(vertices) => pass.set_vertex_buffer(1, vertices.slice()),
                Command::Stencil(reference) => pass.set_stencil_reference(*reference),
                Command::Draw(indices, instance) => {
                    pass.draw_indexed(indices.clone(), 0, *instance..*instance + 1)
                }
            }
        }
    }
}
