//! Owned draw commands for a Canvas pass with MSAA resolve and optional stencil.

use super::{Attachment, Frame, Image, PassBinding, PassStamps, PhysicalAttachments, set_scissor};
use crate::Status;
use crate::arena::{DynamicGroup, VertexSlice};
use crate::batch::GpuMeshBuffer;

pub(super) struct CanvasPass {
    resolved: Attachment,
    multisample: Option<Attachment>,
    stencil: Option<Attachment>,
    pub(super) stamps: Option<PassStamps>,
    area: [u32; 4],
    commands: Vec<Command>,
}

enum Command {
    Pipeline(wgpu::RenderPipeline),
    Globals(DynamicGroup),
    Material(PassBinding),
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

    pub(super) fn material(&mut self, group: &PassBinding) {
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

    pub(super) fn resolve(
        &mut self,
        frame: &mut Frame<'_>,
        physical: &PhysicalAttachments,
    ) -> Result<(), Status> {
        for command in &mut self.commands {
            if let Command::Material(group) = command {
                frame.resolve_binding(group, physical)?;
            }
        }
        Ok(())
    }

    pub(super) fn encode(
        self,
        encoder: &mut wgpu::CommandEncoder,
        physical: &PhysicalAttachments,
    ) -> Result<(), Status> {
        let resolved = physical.view(&self.resolved)?;
        let multisample = self
            .multisample
            .as_ref()
            .map(|view| physical.view(view))
            .transpose()?;
        let stencil = self
            .stencil
            .as_ref()
            .map(|view| physical.view(view))
            .transpose()?;
        let timestamp_writes = self.stamps.as_ref().map(PassStamps::render);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas path batch"),
            timestamp_writes,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: multisample.unwrap_or(resolved),
                depth_slice: None,
                resolve_target: multisample.map(|_| resolved),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if self.multisample.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: stencil.map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Discard,
                }),
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
                Command::Material(material) => pass.set_bind_group(1, material.ready(), &[]),
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
        Ok(())
    }
}
