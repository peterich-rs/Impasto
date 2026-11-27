//! Builds upload-free pictures and installs their geometry on the raster queue.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use engine::scene::{BlendMode, CanvasComposite, Picture};

use super::{
    CompiledPicture, Compiler, Draw, Effect, Group, IDENTITY, PictureBounds, SCOPES_PER_BLOCK,
    Scope, root_state, summarize_bounds,
};
use crate::batch::{GpuMeshBuffer, MeshBuffer};
use crate::gpu::Gpu;
use crate::status::Status;
use crate::textures::Load;

static NEXT_COMPILED_PICTURE: AtomicU64 = AtomicU64::new(1);

/// Inputs used while resolving picture images.
pub struct CompileContext<'a, 'store, 'pixels> {
    pub gpu: &'a Gpu,
    pub device_generation: u64,
    pub resolve: &'a mut dyn FnMut(&str) -> Option<crate::ImagePixels<'pixels>>,
    pub textures: &'a mut crate::textures::TextureTransaction<'store>,
    pub point_scale: f32,
    pub stream_mesh: bool,
}

/// Compiles one picture.
pub fn compile(
    picture: &Picture,
    context: &mut CompileContext<'_, '_, '_>,
    pool: Option<&rayon::ThreadPool>,
) -> Result<CompiledPicture, Status> {
    let prepared = prepare(picture, context.point_scale, pool, &mut |source| {
        context.textures.load(context.gpu, source, context.resolve)
    })?;
    prepared.install(context.gpu, context.device_generation, context.stream_mesh)
}

/// Upload-free geometry prepared on a worker and installed in paint order.
pub struct PreparedPicture {
    compiled: CompiledPicture,
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl PreparedPicture {
    pub fn install(
        mut self,
        gpu: &Gpu,
        device: u64,
        stream_mesh: bool,
    ) -> Result<CompiledPicture, Status> {
        self.compiled.device = device;
        self.compiled.resident_id = NEXT_COMPILED_PICTURE.fetch_add(1, Ordering::Relaxed);
        if !self.indices.is_empty() {
            use crate::arena::FrameArena;
            let mesh = if stream_mesh
                && self.vertices.len() <= FrameArena::vertex_limit::<[f32; 3]>(gpu)
                && self.indices.len() <= FrameArena::vertex_limit::<u32>(gpu)
            {
                MeshBuffer::Streamed {
                    id: self.compiled.resident_id,
                    vertices: self.vertices.into_boxed_slice(),
                    indices: self.indices.into_boxed_slice(),
                    resident: OnceLock::new(),
                }
            } else {
                MeshBuffer::Resident(GpuMeshBuffer::upload(gpu, &self.vertices, &self.indices)?)
            };
            self.compiled
                .mesh_buffer
                .set(mesh)
                .map_err(|_| Status::BadScene)?;
        }
        Ok(self.compiled)
    }
}

pub fn prepare(
    picture: &Picture,
    point_scale: f32,
    pool: Option<&rayon::ThreadPool>,
    load: &mut dyn FnMut(&Arc<str>) -> Load,
) -> Result<PreparedPicture, Status> {
    let origin = picture.origin;
    let mut compiler = Compiler {
        load,
        pool,
        point_scale,
        stack: vec![root_state()],
        scopes: vec![Scope {
            state: root_state(),
            draws: Vec::new(),
            bounds: PictureBounds::Empty,
            has_rects: false,
        }],
        images: Vec::new(),
        waiting: Vec::new(),
        groups: Vec::new(),
        meshes: Arc::new(OnceLock::new()),
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for command in picture.commands().iter() {
        compiler.command(command)?;
    }
    if compiler.stack.len() != 1 {
        // An unbalanced push was rejected at record time. Nothing to do.
        return Err(Status::BadScene);
    }
    let DrawSummary {
        has_glyphs,
        has_shader_effect,
        supports_partial,
    } = summarize_draws(&mut compiler.scopes, compiler.groups.is_empty())?;
    let (bounds, scope_blocks) =
        prepare_bounds(&compiler.scopes, &mut compiler.groups, point_scale);
    let compiled = CompiledPicture {
        resident_id: 0,
        generation: picture.generation,
        has_glyphs,
        has_shader_effect,
        supports_partial,
        device: 0,
        bounds,
        mesh_buffer: compiler.meshes,
        reads_root: compiler.scopes.iter().any(|scope| {
            use engine::scene::shader::BACKDROP_TEXTURE;
            if scope.state.groups.is_empty()
                && scope.draws.iter().any(|draw| {
                    matches!(draw, Draw::ShaderEffect { textures, .. }
                        if textures.iter().any(|source| source.as_ref() == BACKDROP_TEXTURE))
                })
            {
                return true;
            }
            scope.state.groups.first().is_some_and(|&id| {
                matches!(
                    compiler.groups[id].effect,
                    Effect::BackdropBlur
                        | Effect::BackdropMaterial(..)
                        | Effect::Blend(BlendMode::Multiply)
                        | Effect::Canvas(CanvasComposite::Multiply)
                )
            })
        }),
        scale: point_scale,
        origin,
        scopes: compiler.scopes,
        scope_blocks,
        images: compiler.images,
        waiting: compiler.waiting,
        groups: compiler.groups,
    };
    Ok(PreparedPicture {
        compiled,
        vertices: compiler.vertices,
        indices: compiler.indices,
    })
}

struct DrawSummary {
    has_glyphs: bool,
    has_shader_effect: bool,
    supports_partial: bool,
}

fn summarize_draws(scopes: &mut [Scope], ungrouped: bool) -> Result<DrawSummary, Status> {
    let mut has_glyphs = false;
    let mut has_shader_effect = false;
    let mut supports_partial = ungrouped;
    for scope in scopes {
        for draw in &mut scope.draws {
            if let Draw::Rects(run) = draw {
                run.prepare()?;
            }
            if matches!(draw, Draw::Glyphs(_)) {
                has_glyphs = true;
            }
            has_shader_effect |= matches!(draw, Draw::ShaderEffect { .. });
            supports_partial &= matches!(
                draw,
                Draw::Rects(_)
                    | Draw::Images { .. }
                    | Draw::Glyphs(_)
                    | Draw::Fill(_)
                    | Draw::Mesh(_)
            ) && draw.source_over();
        }
    }
    Ok(DrawSummary {
        has_glyphs,
        has_shader_effect,
        supports_partial,
    })
}

fn prepare_bounds(
    scopes: &[Scope],
    groups: &mut [Group],
    point_scale: f32,
) -> (PictureBounds, Vec<PictureBounds>) {
    let scope_blocks = if groups.is_empty() && scopes.len() > SCOPES_PER_BLOCK {
        scopes
            .chunks(SCOPES_PER_BLOCK)
            .map(|scopes| summarize_bounds(scopes, &[], IDENTITY, point_scale).0)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let (bounds, content_by_group) = if scope_blocks.is_empty() {
        summarize_bounds(scopes, groups, IDENTITY, point_scale)
    } else {
        (
            scope_blocks
                .iter()
                .copied()
                .fold(PictureBounds::Empty, PictureBounds::union),
            Vec::new(),
        )
    };
    for (group, content) in groups.iter_mut().zip(content_by_group) {
        group.content = content;
    }
    (bounds, scope_blocks)
}
