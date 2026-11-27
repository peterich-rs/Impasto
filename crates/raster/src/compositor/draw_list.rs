//! Painter-ordered commands and the per-pass tables they reference.

use indexmap::IndexSet;

use super::{CompositeDraw, DrawPipeline, set_scissor};
use crate::Status;
use crate::arena::{DynamicGroup, VertexSlice};
use crate::batch::GpuMeshBuffer;
use crate::pipelines::PipelineCache;

const NO_BINDING: u16 = u16::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Bindings {
    pipeline: u16,
    globals: u16,
    material: u16,
    mask: u16,
    scissor: u16,
}

#[derive(Clone, Copy)]
enum DrawCmd {
    Instanced {
        bindings: Bindings,
        vertices: u16,
        vertex_count: u32,
        instances: u32,
    },
    Indexed {
        bindings: Bindings,
        mesh: u16,
        paint: u16,
        first: u32,
        count: u32,
    },
    Composite(u16),
}

const _: () = assert!(size_of::<DrawCmd>() <= 24);

#[derive(Default)]
pub(super) struct DrawList {
    pipelines: IndexSet<DrawPipeline>,
    globals: IndexSet<DynamicGroup>,
    materials: IndexSet<wgpu::BindGroup>,
    scissors: IndexSet<[u32; 4]>,
    vertices: Vec<VertexSlice>,
    meshes: Vec<GpuMeshBuffer>,
    composites: Vec<CompositeDraw>,
    commands: Vec<DrawCmd>,
    overflow: Vec<DrawList>,
}

fn intern<T: Clone + Eq + std::hash::Hash>(
    table: &mut IndexSet<T>,
    value: &T,
) -> Result<u16, Status> {
    if let Some(index) = table.get_index_of(value) {
        return slot(index);
    }
    slot(table.insert_full(value.clone()).0)
}

fn slot(index: usize) -> Result<u16, Status> {
    u16::try_from(index)
        .ok()
        .filter(|&index| index != NO_BINDING)
        .ok_or(Status::OutOfMemory)
}

impl DrawList {
    fn reserve_command(&mut self) {
        let limit = usize::from(NO_BINDING) - 1;
        if self
            .materials
            .len()
            .max(self.vertices.len())
            .max(self.composites.len())
            >= limit
        {
            // Large scenes use a fresh table inside the same render pass.
            let mut overflow = std::mem::take(&mut self.overflow);
            overflow.push(std::mem::take(self));
            self.overflow = overflow;
        }
    }

    fn bindings(
        &mut self,
        pipeline: &DrawPipeline,
        globals: &DynamicGroup,
        material: &wgpu::BindGroup,
        mask: Option<&wgpu::BindGroup>,
        scissor: [u32; 4],
    ) -> Result<Bindings, Status> {
        Ok(Bindings {
            pipeline: intern(&mut self.pipelines, pipeline)?,
            globals: intern(&mut self.globals, globals)?,
            material: intern(&mut self.materials, material)?,
            mask: mask.map_or(Ok(NO_BINDING), |mask| intern(&mut self.materials, mask))?,
            scissor: intern(&mut self.scissors, &scissor)?,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one instanced draw and its render state"
    )]
    pub(super) fn instanced(
        &mut self,
        pipeline: &DrawPipeline,
        globals: &DynamicGroup,
        material: &wgpu::BindGroup,
        mask: Option<&wgpu::BindGroup>,
        scissor: [u32; 4],
        slice: VertexSlice,
        vertex_count: u32,
        instances: u32,
    ) -> Result<bool, Status> {
        self.reserve_command();
        let bindings = self.bindings(pipeline, globals, material, mask, scissor)?;
        if let Some(DrawCmd::Instanced {
            bindings: previous,
            vertices,
            vertex_count: previous_count,
            instances: previous_instances,
        }) = self.commands.last_mut()
            && *previous == bindings
            && *previous_count == vertex_count
            && let Some(combined) = previous_instances.checked_add(instances)
            && self.vertices[usize::from(*vertices)].append(&slice)
        {
            *previous_instances = combined;
            return Ok(false);
        }

        let vertices = slot(self.vertices.len())?;
        self.vertices.push(slice);
        self.commands.push(DrawCmd::Instanced {
            bindings,
            vertices,
            vertex_count,
            instances,
        });
        Ok(true)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one indexed mesh draw and its render state"
    )]
    pub(super) fn indexed(
        &mut self,
        pipeline: &DrawPipeline,
        globals: &DynamicGroup,
        material: &wgpu::BindGroup,
        scissor: [u32; 4],
        geometry: GpuMeshBuffer,
        paint: VertexSlice,
        indices: std::ops::Range<u32>,
    ) -> Result<(), Status> {
        self.reserve_command();
        let bindings = self.bindings(pipeline, globals, material, None, scissor)?;
        let mesh = slot(self.meshes.len())?;
        let paint_index = slot(self.vertices.len())?;
        self.meshes.push(geometry);
        self.vertices.push(paint);
        self.commands.push(DrawCmd::Indexed {
            bindings,
            mesh,
            paint: paint_index,
            first: indices.start,
            count: indices.end - indices.start,
        });
        Ok(())
    }

    pub(super) fn composite(&mut self, draw: CompositeDraw) -> Result<(), Status> {
        self.reserve_command();
        self.commands
            .push(DrawCmd::Composite(slot(self.composites.len())?));
        self.composites.push(draw);
        Ok(())
    }

    pub(super) fn clear(&mut self) {
        self.overflow.clear();
        self.commands.clear();
        self.pipelines.clear();
        self.globals.clear();
        self.materials.clear();
        self.scissors.clear();
        self.vertices.clear();
        self.meshes.clear();
        self.composites.clear();
    }

    pub(super) fn encode(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target: &wgpu::TextureView,
        size: [u32; 2],
        pipelines: &PipelineCache,
    ) {
        for list in &self.overflow {
            list.encode_table(pass, target, size, pipelines);
        }
        self.encode_table(pass, target, size, pipelines);
    }

    fn encode_table(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target: &wgpu::TextureView,
        size: [u32; 2],
        pipelines: &PipelineCache,
    ) {
        let mut last = None::<Bindings>;
        set_scissor(pass, Some([0, 0, size[0], size[1]]));
        for cmd in &self.commands {
            let bindings = match *cmd {
                DrawCmd::Instanced { bindings, .. } | DrawCmd::Indexed { bindings, .. } => bindings,
                DrawCmd::Composite(index) => {
                    set_scissor(pass, Some([0, 0, size[0], size[1]]));
                    self.composites[usize::from(index)].encode(pass, pipelines);
                    pass.set_viewport(
                        0.0,
                        0.0,
                        target.texture().width() as f32,
                        target.texture().height() as f32,
                        0.0,
                        1.0,
                    );
                    last = None;
                    continue;
                }
            };
            let same_pipeline = last.is_some_and(|last| last.pipeline == bindings.pipeline);
            if !same_pipeline {
                pass.set_pipeline(self.pipelines[usize::from(bindings.pipeline)].get(pipelines));
            }
            if !same_pipeline || last.is_none_or(|last| last.globals != bindings.globals) {
                let globals = &self.globals[usize::from(bindings.globals)];
                pass.set_bind_group(0, &globals.group, globals.offsets());
            }
            if !same_pipeline || last.is_none_or(|last| last.material != bindings.material) {
                pass.set_bind_group(1, &self.materials[usize::from(bindings.material)], &[]);
            }
            if bindings.mask != NO_BINDING
                && (!same_pipeline || last.is_none_or(|last| last.mask != bindings.mask))
            {
                pass.set_bind_group(2, &self.materials[usize::from(bindings.mask)], &[]);
            }
            if last.is_none_or(|last| last.scissor != bindings.scissor) {
                set_scissor(pass, Some(self.scissors[usize::from(bindings.scissor)]));
            }
            last = Some(bindings);
            match *cmd {
                DrawCmd::Instanced {
                    vertices,
                    vertex_count,
                    instances,
                    ..
                } => {
                    pass.set_vertex_buffer(0, self.vertices[usize::from(vertices)].slice());
                    pass.draw(0..vertex_count, 0..instances);
                }
                DrawCmd::Indexed {
                    mesh,
                    paint,
                    first,
                    count,
                    ..
                } => {
                    let mesh = &self.meshes[usize::from(mesh)];
                    pass.set_vertex_buffer(0, mesh.vertices.slice());
                    pass.set_vertex_buffer(1, self.vertices[usize::from(paint)].slice());
                    pass.set_index_buffer(mesh.indices.slice(), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(first..first + count, 0, 0..1);
                }
                DrawCmd::Composite(_) => {}
            }
        }
    }
}
