//! Picture compilation: commands into instance batches and canvas meshes.
//!
//! A compiled picture is a flat list of scopes. Each scope carries the
//! transform, clip, and opacity its draws run under, so a scroll offset or
//! an animation transform changes only the scope matrix, never the batches.
//! Compiled pictures are cached by `PictureId + generation` and reused
//! until the picture leaves the tree or its generation changes.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use engine::scene::{
    BlendMode, BoxShadow, CanvasBrush, CanvasComposite, CanvasIsolation, CanvasList, CanvasOp,
    CanvasPath, Command, ContentMode, Gradient, Picture, Shadow, ShapedParagraph, ShapedRun,
    SpanPaint,
};
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use crate::batch::{
    self, Brush, GpuMeshBuffer, ImageInstance, Mesh, PackedGradient, RECT_FILL, RECT_FILL_STROKE,
    RECT_GRADIENT, RECT_SHADOW, RECT_STROKE, RectInstance, image_slices, lyon_fill_rule,
    pack_canvas_gradient, pack_gradient, tessellation_path,
};
use crate::color::linear_premultiplied;
use crate::gpu::Gpu;
use crate::pipelines::{self, BlendKey};
use crate::status::Status;
use crate::textures::Load;

/// Column-major 3x3 affine in scene points: `[a, b, c, d, e, f]` as
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
pub type Affine = [f32; 6];

static NEXT_COMPILED_PICTURE: AtomicU64 = AtomicU64::new(1);

pub const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Conservative draw support in scene points. Unknown geometry can fill the target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PictureBounds {
    #[default]
    Empty,
    Rect([f32; 4]),
    Unbounded,
}

impl PictureBounds {
    pub fn rect(rect: [f32; 4]) -> Self {
        if !rect.iter().all(|value| value.is_finite()) {
            Self::Unbounded
        } else if rect[2] <= 0.0 || rect[3] <= 0.0 {
            Self::Empty
        } else {
            Self::Rect(rect)
        }
    }

    pub fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unbounded, _) | (_, Self::Unbounded) => Self::Unbounded,
            (Self::Empty, bounds) | (bounds, Self::Empty) => bounds,
            (Self::Rect(a), Self::Rect(b)) => {
                let x = a[0].min(b[0]);
                let y = a[1].min(b[1]);
                Self::rect([
                    x,
                    y,
                    (a[0] + a[2]).max(b[0] + b[2]) - x,
                    (a[1] + a[3]).max(b[1] + b[3]) - y,
                ])
            }
        }
    }

    pub fn transform(self, transform: Affine) -> Self {
        let Self::Rect([x, y, w, h]) = self else {
            return self;
        };
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for (x, y) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            let (x, y) = apply(transform, x, y);
            if !x.is_finite() || !y.is_finite() {
                return Self::Unbounded;
            }
            min = [min[0].min(x), min[1].min(y)];
            max = [max[0].max(x), max[1].max(y)];
        }
        Self::rect([min[0], min[1], max[0] - min[0], max[1] - min[1]])
    }

    pub fn clip(self, rect: [f32; 4]) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Unbounded => Self::rect(rect),
            Self::Rect(bounds) => Self::rect(intersect(bounds, rect)),
        }
    }

    pub fn expand(self, margin: f32) -> Self {
        match self {
            Self::Rect([x, y, w, h]) => {
                Self::rect([x - margin, y - margin, w + 2.0 * margin, h + 2.0 * margin])
            }
            bounds => bounds,
        }
    }
}

/// Clip state of one scope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    /// Intersection of axis-aligned clips in scene points, or `None`.
    pub scissor: Option<[f32; 4]>,
    /// Exact rectangular clips. Rounded AABBs can tighten the scissor but
    /// should not be rasterized again when the picture moves under a rotation.
    pub hard_scissor: Option<[f32; 4]>,
    /// Rounded or transformed clips, each a rounded rect in the scene space
    /// of the scope that pushed it. The compositor renders these into a
    /// coverage mask when the scope is entered.
    pub shapes: Vec<ClipShape>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipShape {
    pub rect: [f32; 4],
    pub radius: f32,
    pub corners: Option<engine::scene::CornerShape>,
    /// Transform from the clip's space into the current scope's space.
    pub transform: Affine,
    /// Tessellated path for a canvas clip, in the clip's space.
    pub mesh: Option<Arc<Mesh>>,
    pub path: Option<CanvasPath>,
}

/// State every draw in a scope runs under.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeState {
    /// Picture space into the parent layer's space.
    pub transform: Affine,
    pub clip: Clip,
    /// Offscreen group this scope renders into. Scopes between a group's
    /// open and close share the id. `None` draws straight into the target.
    pub groups: Vec<usize>,
}

/// An offscreen group the compositor renders into a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Opacity,
    Isolation,
    Blur,
    BackdropBlur,
    Blend(BlendMode),
    Canvas(CanvasComposite),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub effect: Effect,
    pub sigma: f32,
    pub opacity: f32,
    pub bounds: Option<[f32; 4]>,
    pub content: PictureBounds,
    pub transform: Affine,
}

/// One run of draws that share a transform, clip, opacity, and group.
///
/// Scopes are flat and in draw order. The compiler starts a new scope
/// whenever the state changes, so a scroll or animation transform changes
/// only the scope matrix and the compositor replays the same batches.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub state: ScopeState,
    pub draws: Vec<Draw>,
    pub bounds: PictureBounds,
    has_rects: bool,
    pub resident_candidates: Vec<bool>,
}

impl Scope {
    pub fn bounds_in(&self, origin: Affine, scale: f32) -> PictureBounds {
        let transform = compose(origin, self.state.transform);
        let margin = if self.has_rects {
            // The shader limits its geometry to stroke/2 plus this margin.
            // Anisotropy can stretch the AA fringe along the larger axis.
            (primitive_scale(transform, scale).max(0.001).recip() - 1.0).max(0.0)
        } else {
            0.0
        };
        let bounds = self.bounds.expand(margin).transform(transform);
        match self
            .state
            .clip
            .scissor
            .map(|clip| PictureBounds::rect(clip).transform(origin))
        {
            Some(PictureBounds::Empty) => PictureBounds::Empty,
            Some(PictureBounds::Rect(clip)) => bounds.clip(clip),
            _ => bounds,
        }
    }
}

/// Draw batches inside one scope. Adjacent draws of the same kind merge.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Rects(RectRun),
    Fill([f32; 4]),
    /// Glyph quads. Atlas pages are resolved when the frame is encoded, so
    /// a cache hit never re-rasterizes.
    Glyphs(Vec<GlyphDraw>),
    Images {
        source: Arc<str>,
        instances: Vec<ImageInstance>,
        blend: BlendKey,
    },
    Mesh(Mesh),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectRun {
    pub rects: Vec<RectInstance>,
    pub draws: u32,
    pub stops: Option<Arc<[[f32; 4]]>>,
    pub blend: BlendKey,
    pub vertices: Vec<batch::RectVertex>,
    pub paints: Vec<batch::PaintExtra>,
}

impl RectRun {
    fn prepare(&mut self) -> Result<(), Status> {
        use std::collections::{HashMap, hash_map::Entry};
        let mut indices = HashMap::<[u32; size_of::<batch::PaintExtra>() / 4], u32>::new();
        self.vertices.reserve(self.rects.len());
        for rect in &self.rects {
            let index = if let Some(paint) = rect.extra(0.0, 0) {
                match indices.entry(bytemuck::cast(paint)) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => {
                        let index =
                            u32::try_from(self.paints.len()).map_err(|_| Status::OutOfMemory)?;
                        self.paints.push(paint);
                        *entry.insert(index)
                    }
                }
            } else {
                u32::MAX
            };
            self.vertices.push(rect.vertex(index));
        }
        self.draws = u32::try_from(batch::rect_pipeline_runs(&self.vertices).count())
            .map_err(|_| Status::OutOfMemory)?;
        Ok(())
    }

    pub fn stops(&self) -> &[[f32; 4]] {
        self.stops.as_deref().unwrap_or_default()
    }
}

fn resident_candidate(draw: &Draw) -> bool {
    match draw {
        Draw::Rects(run) => !run.rects.is_empty(),
        Draw::Images {
            source, instances, ..
        } => {
            !source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX)
                && instances.first().is_some_and(|first| {
                    instances
                        .iter()
                        .all(|instance| (instance.flags ^ first.flags) & batch::IMAGE_NEAREST == 0)
                })
        }
        Draw::Glyphs(glyphs) => {
            !glyphs.is_empty()
                && glyphs
                    .iter()
                    .all(|glyph| glyph.gradient.is_none() && glyph.decoration.is_none())
        }
        Draw::Fill(_) | Draw::Mesh(_) => false,
    }
}

impl Draw {
    pub fn source_over(&self) -> bool {
        match self {
            Self::Rects(run) => run.blend == BlendKey::SrcOver,
            Self::Images { blend, .. } => *blend == BlendKey::SrcOver,
            Self::Mesh(mesh) => mesh.composite == CanvasComposite::SourceOver,
            Self::Fill(_) | Self::Glyphs(_) => true,
        }
    }
}

impl Draw {
    fn bounds(&self) -> PictureBounds {
        use PictureBounds as B;
        let rect = B::rect;
        match self {
            Self::Rects(run) => run.rects.iter().fold(B::Empty, |bounds, draw| {
                bounds.union(rect(draw.rect).expand(draw.stroke_width * 0.5 + 1.0))
            }),
            Self::Fill(_) => B::Unbounded,
            Self::Glyphs(glyphs) => glyphs.iter().fold(B::Empty, |bounds, glyph| {
                // Covers the font box, overhang, synthetic paint and subpixel placement.
                let pad = glyph.size * 2.0 + 1.0;
                bounds.union(rect(glyph.decoration.unwrap_or([
                    glyph.x - pad,
                    glyph.y - pad,
                    2.0 * pad,
                    2.0 * pad,
                ])))
            }),
            Self::Images { instances, .. } => instances
                .iter()
                .fold(B::Empty, |bounds, image| bounds.union(rect(image.rect))),
            Self::Mesh(mesh) => rect(mesh.bounds),
        }
    }
}

/// One glyph quad, before its atlas page is known.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphDraw {
    pub face: Arc<engine::scene::FontFaceId>,
    pub glyph: u16,
    /// Baseline position in scene points, after the scope transform.
    pub x: f32,
    pub y: f32,
    /// Font size in scene points.
    pub size: f32,
    pub coords: Option<Arc<[i16]>>,
    pub embolden: bool,
    pub skew: Option<f32>,
    /// Solid brush, or white for a gradient.
    pub color: [u16; 4],
    pub gradient: Option<Arc<PackedGradient>>,
    /// Span box for gradient coordinates, in scene points.
    pub decoration: Option<[f32; 4]>,
}

/// A picture compiled for one point scale.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledPicture {
    pub resident_id: u64,
    pub generation: u64,
    pub scale: f32,
    pub device: u64,
    pub bounds: PictureBounds,
    pub mesh_buffer: Arc<OnceLock<GpuMeshBuffer>>,
    pub reads_root: bool,
    pub has_resident_candidate: bool,
    pub has_glyphs: bool,
    pub cache_safe: bool,
    pub cache_expensive: bool,
    pub supports_partial: bool,
    /// Picture origin in the parent layer's space.
    pub origin: (f32, f32),
    pub scopes: Vec<Scope>,
    /// Bounds of consecutive scopes in picture space, used for translated replay.
    pub scope_blocks: Vec<PictureBounds>,
    /// Image sources this picture references.
    pub images: Vec<(Arc<str>, bool)>,
    /// Images that were missing at compile time. The picture is rebuilt
    /// once any of them is cached.
    pub waiting: Vec<Arc<str>>,
    pub groups: Vec<Group>,
}

pub const SCOPES_PER_BLOCK: usize = 32;

impl CompiledPicture {
    pub fn bounds_in(
        &self,
        origin: Affine,
        scale: f32,
    ) -> (PictureBounds, Option<Vec<PictureBounds>>) {
        if origin[..4] == IDENTITY[..4] && scale >= self.scale {
            return (self.bounds.transform(origin), None);
        }
        let (bounds, groups) = summarize_bounds(&self.scopes, &self.groups, origin, scale);
        (bounds, Some(groups))
    }
}

fn summarize_bounds(
    scopes: &[Scope],
    groups: &[Group],
    origin: Affine,
    scale: f32,
) -> (PictureBounds, Vec<PictureBounds>) {
    let mut bounds = PictureBounds::Empty;
    let mut content_by_group = vec![PictureBounds::Empty; groups.len()];
    for scope in scopes {
        let mut content = scope.bounds_in(origin, scale);
        for &id in scope.state.groups.iter().rev() {
            content_by_group[id] = content_by_group[id].union(content);
            content = group_output(content, groups[id], origin);
        }
        bounds = bounds.union(content);
    }
    (bounds, content_by_group)
}

/// Inputs used while resolving picture images.
pub struct CompileContext<'a, 'store, 'pixels> {
    pub gpu: &'a Gpu,
    pub device_generation: u64,
    pub resolve: &'a mut dyn FnMut(&str) -> Option<crate::ImagePixels<'pixels>>,
    pub textures: &'a mut crate::textures::TextureTransaction<'store>,
    pub point_scale: f32,
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
    prepared.install(context.gpu, context.device_generation)
}

type Tessellation = lyon_tessellation::VertexBuffers<[f32; 3], u32>;

/// Upload-free geometry prepared on a worker and installed in paint order.
pub struct PreparedPicture {
    compiled: CompiledPicture,
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl PreparedPicture {
    pub fn install(mut self, gpu: &Gpu, device: u64) -> Result<CompiledPicture, Status> {
        if !self.indices.is_empty() {
            let mesh = gpu.validation(|| {
                Ok(GpuMeshBuffer {
                    vertices: pipelines::buffer_from(
                        gpu,
                        "picture mesh vertices",
                        bytemuck::cast_slice(&self.vertices),
                        wgpu::BufferUsages::VERTEX,
                    )?,
                    indices: pipelines::buffer_from(
                        gpu,
                        "picture mesh indices",
                        bytemuck::cast_slice(&self.indices),
                        wgpu::BufferUsages::INDEX,
                    )?,
                })
            })?;
            self.compiled
                .mesh_buffer
                .set(mesh)
                .map_err(|_| Status::BadScene)?;
        }
        self.compiled.device = device;
        self.compiled.resident_id = NEXT_COMPILED_PICTURE.fetch_add(1, Ordering::Relaxed);
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
            resident_candidates: Vec::new(),
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
    let mut has_resident_candidate = false;
    let mut has_glyphs = false;
    let mut supports_partial = compiler.groups.is_empty();
    let mut cache_safe = true;
    let mut shadows = 0usize;
    let mut paths = 0usize;
    for scope in &mut compiler.scopes {
        for draw in &mut scope.draws {
            if let Draw::Rects(run) = draw {
                run.prepare()?;
                shadows += run
                    .rects
                    .iter()
                    .filter(|rect| rect.kind == RECT_SHADOW)
                    .count();
            }
            has_glyphs |= matches!(draw, Draw::Glyphs(_));
            supports_partial &= matches!(
                draw,
                Draw::Rects(_) | Draw::Images { .. } | Draw::Glyphs(_) | Draw::Fill(_)
            ) && draw.source_over();
            paths += usize::from(matches!(draw, Draw::Mesh(_)));
            cache_safe &= matches!(draw, Draw::Rects(_) | Draw::Mesh(_))
                && (draw.source_over() || !scope.state.groups.is_empty());
        }
        if compiler.groups.is_empty() {
            let candidates: Vec<_> = scope.draws.iter().map(resident_candidate).collect();
            if candidates.contains(&true) {
                scope.resident_candidates = candidates;
                has_resident_candidate = true;
            }
        }
    }
    let scope_blocks = if compiler.groups.is_empty() && compiler.scopes.len() > SCOPES_PER_BLOCK {
        compiler
            .scopes
            .chunks(SCOPES_PER_BLOCK)
            .map(|scopes| summarize_bounds(scopes, &[], IDENTITY, point_scale).0)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let (bounds, content_by_group) = if scope_blocks.is_empty() {
        summarize_bounds(&compiler.scopes, &compiler.groups, IDENTITY, point_scale)
    } else {
        (
            scope_blocks
                .iter()
                .copied()
                .fold(PictureBounds::Empty, PictureBounds::union),
            Vec::new(),
        )
    };
    for (group, content) in compiler.groups.iter_mut().zip(content_by_group) {
        group.content = content;
    }
    cache_safe &= compiler
        .groups
        .iter()
        .all(|group| group.effect != Effect::BackdropBlur)
        && compiler.scopes.iter().all(|scope| {
            scope.state.groups.first().is_none_or(|id| {
                matches!(
                    compiler.groups[*id].effect,
                    Effect::Opacity
                        | Effect::Isolation
                        | Effect::Blur
                        | Effect::Blend(BlendMode::SrcOver)
                )
            })
        });
    let cache_expensive = paths >= 32
        || shadows >= 2
        || compiler.groups.iter().any(|group| {
            matches!(
                group.effect,
                Effect::Blur | Effect::Opacity | Effect::Blend(_)
            )
        });
    let compiled = CompiledPicture {
        resident_id: 0,
        generation: picture.generation,
        has_resident_candidate,
        has_glyphs,
        cache_safe,
        cache_expensive,
        supports_partial,
        device: 0,
        bounds,
        mesh_buffer: compiler.meshes,
        reads_root: compiler.scopes.iter().any(|scope| {
            scope.state.groups.first().is_some_and(|&id| {
                matches!(
                    compiler.groups[id].effect,
                    Effect::BackdropBlur
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

pub fn effect_scale(transform: Affine) -> f32 {
    transform[0]
        .hypot(transform[1])
        .max(transform[2].hypot(transform[3]))
}

pub fn primitive_scale(transform: Affine, scale: f32) -> f32 {
    scale
        * transform[0]
            .hypot(transform[1])
            .min(transform[2].hypot(transform[3]))
            .max(0.001)
}

fn group_output(content: PictureBounds, group: Group, origin: Affine) -> PictureBounds {
    let transform = compose(origin, group.transform);
    match group.effect {
        Effect::Blur => content.expand(3.0 * group.sigma.max(0.0) * effect_scale(transform)),
        Effect::BackdropBlur => {
            content.union(group.bounds.map_or(PictureBounds::Empty, |bounds| {
                PictureBounds::rect(bounds).transform(transform)
            }))
        }
        _ => content,
    }
}

struct Compiler<'a> {
    load: &'a mut dyn FnMut(&Arc<str>) -> Load,
    pool: Option<&'a rayon::ThreadPool>,
    point_scale: f32,
    stack: Vec<ScopeState>,
    scopes: Vec<Scope>,
    images: Vec<(Arc<str>, bool)>,
    waiting: Vec<Arc<str>>,
    groups: Vec<Group>,
    meshes: Arc<OnceLock<GpuMeshBuffer>>,
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Compiler<'_> {
    fn current(&self) -> &ScopeState {
        self.stack
            .last()
            .unwrap_or_else(|| ROOT.get_or_init(root_state))
    }

    fn current_mut(&mut self) -> &mut ScopeState {
        let count = self.stack.len().saturating_sub(1);
        &mut self.stack[count]
    }

    fn push_draw(&mut self, draw: Draw) {
        let state = self.current().clone();
        // Start a new scope when the state differs from the open one.
        let needs_scope = self.scopes.last().is_none_or(|scope| scope.state != state);
        if needs_scope {
            self.scopes.push(Scope {
                state,
                draws: Vec::new(),
                bounds: PictureBounds::Empty,
                has_rects: false,
                resident_candidates: Vec::new(),
            });
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.bounds = scope.bounds.union(draw.bounds());
            scope.has_rects |= matches!(draw, Draw::Rects(..));
            scope.draws.push(draw);
        }
    }

    fn push_rects(
        &mut self,
        rects: impl IntoIterator<Item = RectInstance>,
        stops: Option<Arc<[[f32; 4]]>>,
        blend: BlendKey,
    ) {
        let state = self.current();
        if let Some(scope) = self.scopes.last()
            && &scope.state == state
            && let Some(Draw::Rects(run)) = scope.draws.last()
            && run.blend == blend
            && run.stops == stops
            && let Some(scope) = self.scopes.last_mut()
            && let Some(Draw::Rects(run)) = scope.draws.last_mut()
        {
            for rect in rects {
                scope.bounds = scope
                    .bounds
                    .union(PictureBounds::rect(rect.rect).expand(rect.stroke_width * 0.5 + 1.0));
                run.rects.push(rect);
            }
        } else {
            self.push_draw(Draw::Rects(RectRun {
                rects: rects.into_iter().collect(),
                draws: 0,
                stops,
                blend,
                vertices: Vec::new(),
                paints: Vec::new(),
            }));
        }
    }

    /// Pushes a scope boundary: the state changed, so the next draw opens
    /// a new scope even if nothing is drawn in between.
    fn boundary(&mut self) {
        let state = self.current().clone();
        self.scopes.push(Scope {
            state,
            draws: Vec::new(),
            bounds: PictureBounds::Empty,
            has_rects: false,
            resident_candidates: Vec::new(),
        });
    }

    fn command(&mut self, command: &Command) -> Result<(), Status> {
        match command {
            Command::Fill { color } => self.fill(*color),
            Command::Rect {
                x,
                y,
                w,
                h,
                corner_radius,
                color,
                stroke_width,
                stroke_color,
                gradient,
                shadow,
                box_shadows,
            } => {
                self.rect(
                    *x,
                    *y,
                    *w,
                    *h,
                    *corner_radius,
                    color,
                    *stroke_width,
                    stroke_color,
                    gradient,
                    shadow,
                    box_shadows,
                    None,
                )?;
            }
            Command::ShapedRect {
                bounds: [x, y, w, h],
                corners,
                paint,
            } => {
                self.rect(
                    *x,
                    *y,
                    *w,
                    *h,
                    0.0,
                    &paint.color,
                    paint.stroke_width,
                    &paint.stroke_color,
                    &paint.gradient,
                    &paint.shadow,
                    &paint.box_shadows,
                    Some(*corners),
                )?;
            }
            Command::Text {
                x,
                y,
                shaped,
                color,
                span_paints,
            } => self.text(*x, *y, shaped.as_deref(), color, span_paints)?,
            Command::Image {
                x,
                y,
                w,
                h,
                source,
                mode,
                required,
            } => self.image(*x, *y, *w, *h, source, *mode, *required)?,
            Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius,
            } => self.push_clip(*x, *y, *w, *h, *corner_radius, None),
            Command::PushCornerClip {
                bounds: [x, y, w, h],
                corners,
            } => {
                self.push_clip(*x, *y, *w, *h, 0.0, Some(*corners));
            }
            Command::PushOpacity { alpha, .. } => {
                self.push_effect(Effect::Opacity, f32::from(*alpha) / 255.0, 0.0, None)
            }
            Command::PushTransform {
                translate_x,
                translate_y,
                rotate_rad,
                scale_x,
                scale_y,
            } => self.push_transform(*translate_x, *translate_y, *rotate_rad, *scale_x, *scale_y),
            Command::PushBlur { sigma, .. } => self.push_effect(Effect::Blur, 1.0, *sigma, None),
            Command::PushBackdropBlur { x, y, w, h, sigma } => {
                self.push_effect(Effect::BackdropBlur, 1.0, *sigma, Some([*x, *y, *w, *h]));
            }
            Command::PushBlend { mode, .. } => {
                self.push_effect(Effect::Blend(*mode), 1.0, 0.0, None)
            }
            Command::Pop => {
                if self.stack.len() <= 1 {
                    return Err(Status::BadScene);
                }
                self.pop();
            }
            Command::Canvas {
                x,
                y,
                width,
                height,
                list,
            } => self.canvas(*x, *y, *width, *height, list)?,
        }
        Ok(())
    }

    fn fill(&mut self, color: [u8; 4]) {
        let Some(paint) = linear_premultiplied(color, 255) else {
            return;
        };
        self.push_draw(Draw::Fill(paint));
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one rect command carries its fill, stroke, gradient, and shadows"
    )]
    fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
        color: &[u8; 4],
        stroke_width: f32,
        stroke_color: &[u8; 4],
        gradient: &Option<Gradient>,
        shadow: &Option<Shadow>,
        box_shadows: &[BoxShadow],
        corners: Option<engine::scene::CornerShape>,
    ) -> Result<(), Status> {
        if !positive_box(x, y, w, h) {
            return Ok(());
        }
        let corners = corners.map(|shape| shape.fitted(w, h));
        let mut instances = Vec::new();
        for shadow in box_shadows {
            if let Some(mut instance) = box_shadow(x, y, w, h, corner_radius, shadow) {
                instance.corners = corners.map(|mut shape| {
                    shape.radii = shape.radii.map(|radius| (radius + shadow.spread).max(0.0));
                    shape.fitted(w + 2.0 * shadow.spread, h + 2.0 * shadow.spread)
                });
                instances.push(instance);
            }
        }
        if let Some(shadow) = shadow
            && shadow.elevation.is_finite()
            && shadow.elevation > 0.0
            && let Some(mut instance) = material_shadow(x, y, w, h, corner_radius, shadow)
        {
            instance.corners = corners;
            instances.push(instance);
        }
        let stroke = stroke_width.is_finite() && stroke_width > 0.0;
        let fill = color[3] > 0 || gradient.is_some();
        let kind = match (fill, stroke) {
            (true, true) => RECT_FILL_STROKE,
            (false, true) => RECT_STROKE,
            (true, false) => RECT_FILL,
            (false, false) => {
                if !instances.is_empty() {
                    self.push_rects(instances, None, BlendKey::SrcOver);
                }
                return Ok(());
            }
        };
        let (flags, gradient_a, gradient_b, stops) = match gradient {
            Some(gradient) => {
                let Some(packed) = pack_gradient(gradient, x, y, w, h, 255) else {
                    return Ok(());
                };
                (
                    RECT_GRADIENT,
                    packed.a,
                    [
                        packed.kind as f32,
                        0.0,
                        packed.stops.len() as f32 / 2.0,
                        1.0,
                    ],
                    Some(packed.stops),
                )
            }
            None => (0, [0.0; 4], [0.0, 0.0, 0.0, 1.0], None),
        };
        let fill_color = match gradient {
            Some(gradient) => gradient_solid(gradient).unwrap_or([0.0; 4]),
            None => linear_premultiplied(*color, 255).unwrap_or([0.0; 4]),
        };
        let stroke_color = linear_premultiplied(*stroke_color, 255).unwrap_or([0.0; 4]);
        // Gradient stops are shared per draw batch; store them on the
        // compositor's stop buffer at encode time.
        let fill = RectInstance {
            rect: [x, y, w, h],
            radius: clamped_radius(corner_radius, w, h),
            corners,
            stroke_width: if stroke { stroke_width } else { 0.0 },
            kind,
            flags,
            color: fill_color,
            stroke_color,
            gradient_a,
            gradient_b,
            shadow: [0.0; 4],
        };
        self.push_rects(
            instances.into_iter().chain([fill]),
            stops.map(Arc::from),
            BlendKey::SrcOver,
        );
        Ok(())
    }

    fn text(
        &mut self,
        x: f32,
        y: f32,
        shaped: Option<&ShapedParagraph>,
        color: &[u8; 4],
        span_paints: &[SpanPaint],
    ) -> Result<(), Status> {
        let Some(shaped) = shaped else {
            return Ok(());
        };
        if shaped.is_empty() {
            return Ok(());
        }
        let mut glyphs = Vec::new();
        for run in &shaped.runs {
            let Some(paint) = run_paint(run, color, span_paints, x, y) else {
                continue;
            };
            append_glyphs(&mut glyphs, run, x, y, &paint);
        }
        for line in &shaped.decorations {
            let Some(paint) = decoration_paint(line, color, span_paints, x, y) else {
                continue;
            };
            let rect = [
                x + line.rect[0],
                y + line.rect[1],
                line.rect[2],
                line.rect[3],
            ];
            glyphs.push(GlyphDraw {
                face: Arc::new(engine::scene::FontFaceId(u64::MAX)),
                glyph: 0,
                x: rect[0],
                y: rect[1],
                size: 0.0,
                coords: None,
                embolden: false,
                skew: None,
                color: pack_glyph_color(paint.solid),
                gradient: paint.gradient,
                decoration: Some(rect),
            });
        }
        if !glyphs.is_empty() {
            self.push_draw(Draw::Glyphs(glyphs));
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one image command carries its destination, source and content mode"
    )]
    fn image(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: &Arc<str>,
        mode: ContentMode,
        required: bool,
    ) -> Result<(), Status> {
        match (self.load)(source) {
            Load::Ready { width, height, .. } => {
                let slices =
                    image_slices(mode, x, y, w, h, width, height).ok_or(Status::BadScene)?;
                let instances: Vec<ImageInstance> = slices
                    .into_iter()
                    .map(|(rect, uv)| ImageInstance {
                        rect,
                        uv,
                        color: [1.0; 4],
                        flags: if matches!(mode, ContentMode::NineSlice { .. }) {
                            batch::IMAGE_BASE_LEVEL
                        } else {
                            0
                        },
                    })
                    .collect();
                self.remember_image(source, required);
                self.push_draw(Draw::Images {
                    source: Arc::clone(source),
                    instances,
                    blend: BlendKey::SrcOver,
                });
            }
            Load::Missing => {
                if required {
                    return Err(Status::NotReady);
                }
                if !source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX) {
                    self.waiting.push(Arc::clone(source));
                    self.remember_image(source, false);
                } else {
                    self.remember_image(source, false);
                }
            }
            Load::Empty => {
                self.waiting.push(Arc::clone(source));
                self.remember_image(source, required);
            }
            Load::Failed => return Err(Status::BadScene),
        }
        Ok(())
    }

    fn remember_image(&mut self, source: &Arc<str>, required: bool) {
        if let Some((_, flag)) = self.images.iter_mut().find(|(known, _)| known == source) {
            *flag |= required;
        } else {
            self.images.push((Arc::clone(source), required));
        }
    }

    fn push_clip(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
        corners: Option<engine::scene::CornerShape>,
    ) {
        let mut state = self.current().clone();
        let transform = state.transform;
        let radius = clamped_radius(corner_radius, w, h);
        if is_axis_aligned(transform) {
            // Axis-aligned and untransformed: fold into the scissor.
            let (x0, y0) = apply(transform, x, y);
            let (x1, y1) = apply(transform, x + w, y + h);
            let rect = [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()];
            state.clip.scissor = Some(match state.clip.scissor {
                Some(existing) => intersect(existing, rect),
                None => rect,
            });
            if radius <= 0.0 && corners.is_none() {
                state.clip.hard_scissor = Some(
                    state
                        .clip
                        .hard_scissor
                        .map_or(rect, |old| intersect(old, rect)),
                );
            }
        }
        if radius > 0.0 || corners.is_some() || !is_axis_aligned(transform) {
            state.clip.shapes.push(ClipShape {
                rect: [x, y, w, h],
                radius,
                corners: corners.map(|shape| shape.fitted(w, h)),
                transform,
                mesh: None,
                path: None,
            });
        }
        self.stack.push(state);
        self.boundary();
    }

    fn push_transform(
        &mut self,
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    ) {
        let mut state = self.current().clone();
        // Translate, then rotate clockwise, then scale, in the parent's space.
        let (sin, cos) = rotate_rad.sin_cos();
        let local: Affine = [
            cos * scale_x,
            sin * scale_x,
            -sin * scale_y,
            cos * scale_y,
            translate_x,
            translate_y,
        ];
        state.transform = compose(state.transform, local);
        self.stack.push(state);
        self.boundary();
    }

    fn tessellation_tolerance(&self) -> f32 {
        let [a, b, c, d, ..] = self.current().transform;
        let scale = self.point_scale * a.hypot(b).max(c.hypot(d));
        0.2 / scale.clamp(0.25, 128.0)
    }

    fn push_effect(&mut self, effect: Effect, opacity: f32, sigma: f32, bounds: Option<[f32; 4]>) {
        let mut state = self.current().clone();
        let id = self.groups.len();
        self.groups.push(Group {
            effect,
            sigma,
            opacity,
            bounds,
            content: PictureBounds::Empty,
            transform: state.transform,
        });
        state.groups.push(id);
        self.stack.push(state);
        self.boundary();
    }

    fn push_canvas_composite(&mut self, mode: CanvasComposite) -> bool {
        if mode == CanvasComposite::SourceOver {
            return false;
        }
        self.push_effect(Effect::Canvas(mode), 1.0, 0.0, None);
        true
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.boundary();
        }
    }

    fn canvas(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        list: &CanvasList,
    ) -> Result<(), Status> {
        if list.ops().is_empty() || !positive_box(x, y, width, height) {
            return Ok(());
        }
        let entry = self.stack.len();
        let isolated = list.isolation() == CanvasIsolation::Isolated;
        if isolated {
            self.push_effect(Effect::Isolation, 1.0, 0.0, None);
        }
        self.push_clip(x, y, width, height, 0.0, None);
        self.push_transform(x, y, 0.0, 1.0, 1.0);
        let floor = self.stack.len();
        let ops = list.ops();
        if let Some(pool) = self.pool.filter(|_| crate::prepare::path_count(ops) >= 16) {
            // Canvas paths already contain their local transforms. Save/restore
            // and the remaining ops leave the enclosing scale unchanged.
            let tolerance = self.tessellation_tolerance();
            let prepared: Vec<_> = pool.install(|| {
                ops.par_iter()
                    .map(|op| match op {
                        CanvasOp::Clip { path } if path.verbs.is_empty() => Ok(None),
                        CanvasOp::Fill { path, .. } | CanvasOp::Clip { path } => {
                            tessellate(path, tolerance).map(Some)
                        }
                        _ => Ok(None),
                    })
                    .collect()
            });
            for (op, buffers) in ops.iter().zip(prepared) {
                self.canvas_op(op, floor, buffers?)?;
            }
        } else {
            for op in ops {
                self.canvas_op(op, floor, None)?;
            }
        }
        self.stack.truncate(entry);
        self.boundary();
        Ok(())
    }

    fn mesh(
        &mut self,
        path: &CanvasPath,
        brush: Brush,
        composite: CanvasComposite,
        buffers: Option<Tessellation>,
    ) -> Result<Mesh, Status> {
        let buffers = match buffers {
            Some(buffers) => buffers,
            None => tessellate(path, self.tessellation_tolerance())?,
        };
        let base = u32::try_from(self.vertices.len()).map_err(|_| Status::BadScene)?;
        let start = u32::try_from(self.indices.len()).map_err(|_| Status::BadScene)?;
        let bounds = mesh_bounds(&buffers.vertices);
        for index in buffers.indices {
            self.indices
                .push(base.checked_add(index).ok_or(Status::BadScene)?);
        }
        let end = u32::try_from(self.indices.len()).map_err(|_| Status::BadScene)?;
        self.vertices.extend(buffers.vertices);
        Ok(Mesh {
            buffer: Arc::clone(&self.meshes),
            indices: start..end,
            bounds,
            brush,
            composite,
        })
    }

    fn canvas_op(
        &mut self,
        op: &CanvasOp,
        floor: usize,
        buffers: Option<Tessellation>,
    ) -> Result<(), Status> {
        match op {
            CanvasOp::Save => {
                self.stack.push(self.current().clone());
                self.boundary();
            }
            CanvasOp::Restore => {
                if self.stack.len() > floor {
                    self.pop();
                }
            }
            CanvasOp::Clip { path } => {
                let transform = self.current().transform;
                if path.verbs.is_empty() {
                    let clip = &mut self.current_mut().clip;
                    clip.scissor = Some([0.0; 4]);
                    clip.hard_scissor = Some([0.0; 4]);
                } else {
                    let mesh = self.mesh(
                        path,
                        Brush::Solid([1.0; 4]),
                        CanvasComposite::SourceOver,
                        buffers,
                    )?;
                    self.current_mut().clip.shapes.push(ClipShape {
                        rect: mesh.bounds,
                        radius: 0.0,
                        corners: None,
                        transform,
                        mesh: Some(Arc::new(mesh)),
                        path: Some(path.clone()),
                    });
                }
                self.boundary();
            }
            CanvasOp::Fill { path, paint } => {
                let brush = match &paint.brush {
                    CanvasBrush::Color(color) => {
                        Brush::Solid(linear_premultiplied(*color, paint.alpha).unwrap_or([0.0; 4]))
                    }
                    gradient => Brush::Gradient(
                        pack_canvas_gradient(gradient, paint.alpha).ok_or(Status::BadScene)?,
                    ),
                };
                let mesh = self.mesh(path, brush, paint.composite, buffers)?;
                let isolated = paint.composite == CanvasComposite::Multiply
                    && self.push_canvas_composite(paint.composite);
                self.push_draw(Draw::Mesh(mesh));
                if isolated {
                    self.pop();
                }
            }
            CanvasOp::Text(text) => {
                if let Some(shaped) = &text.shaped {
                    let Some(paint) = canvas_brush(&text.paint.brush, text.paint.alpha, 1.0) else {
                        return Ok(());
                    };
                    let isolated = self.push_canvas_composite(text.paint.composite);
                    let mut state = self.current().clone();
                    let squeeze = [
                        text.compress,
                        0.0,
                        0.0,
                        1.0,
                        text.anchor_x * (1.0 - text.compress),
                        0.0,
                    ];
                    state.transform = compose(state.transform, compose(text.ctm, squeeze));
                    self.stack.push(state);
                    let mut glyphs = Vec::new();
                    for run in &shaped.runs {
                        append_glyphs(&mut glyphs, run, text.x, text.y, &paint);
                    }
                    if !glyphs.is_empty() {
                        self.push_draw(Draw::Glyphs(glyphs));
                    }
                    self.pop();
                    if isolated {
                        self.pop();
                    }
                }
            }
            CanvasOp::Image(image) => {
                let load = (self.load)(&image.source);
                match load {
                    Load::Ready {
                        width: tex_w,
                        height: tex_h,
                        ..
                    } => {
                        let Some((rect, uv)) = canvas_image_sample(image, tex_w, tex_h) else {
                            return Ok(());
                        };
                        let blend = BlendKey::from_mode(batch::canvas_composite(image.composite));
                        let isolated =
                            blend.is_none() && self.push_canvas_composite(image.composite);
                        let mut state = self.current().clone();
                        state.transform = compose(state.transform, image.ctm);
                        self.stack.push(state);
                        let alpha = f32::from(image.alpha) / 255.0;
                        self.remember_image(&image.source, false);
                        self.push_draw(Draw::Images {
                            source: Arc::clone(&image.source),
                            blend: blend.unwrap_or_default(),
                            instances: vec![ImageInstance {
                                rect,
                                uv,
                                color: [alpha, alpha, alpha, alpha],
                                flags: batch::IMAGE_BASE_LEVEL
                                    | if image.smoothing {
                                        0
                                    } else {
                                        batch::IMAGE_NEAREST
                                    },
                            }],
                        });
                        self.pop();
                        if isolated {
                            self.pop();
                        }
                    }
                    Load::Missing => {
                        if !image
                            .source
                            .starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX)
                        {
                            self.waiting.push(Arc::clone(&image.source));
                        }
                    }
                    Load::Empty => self.waiting.push(Arc::clone(&image.source)),
                    Load::Failed => return Err(Status::BadScene),
                }
            }
            CanvasOp::Clear { x, y, w, h, ctm } => {
                let mut state = self.current().clone();
                state.transform = compose(state.transform, *ctm);
                self.stack.push(state);
                self.push_rects(
                    [solid_rect([*x, *y, *w, *h], [1.0; 4])],
                    None,
                    BlendKey::DstOut,
                );
                self.pop();
            }
        }
        Ok(())
    }
}

pub fn solid_rect(rect: [f32; 4], color: [f32; 4]) -> RectInstance {
    RectInstance {
        rect,
        radius: 0.0,
        corners: None,
        stroke_width: 0.0,
        kind: RECT_FILL,
        flags: 0,
        color,
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [0.0; 4],
    }
}

fn pack_glyph_color(color: [f32; 4]) -> [u16; 4] {
    color.map(|component| half::f16::from_f32(component).to_bits())
}

fn append_glyphs(out: &mut Vec<GlyphDraw>, run: &ShapedRun, x: f32, y: f32, paint: &RunPaint) {
    let coords =
        (!run.normalized_coords.is_empty()).then(|| Arc::from(run.normalized_coords.as_ref()));
    let color = pack_glyph_color(paint.solid);
    out.extend(run.glyphs.iter().map(|glyph| GlyphDraw {
        face: Arc::clone(&run.face),
        glyph: glyph.id,
        x: x + glyph.x,
        y: y + glyph.y,
        size: run.font_size,
        coords: coords.clone(),
        embolden: run.embolden,
        skew: run.skew_degrees,
        color,
        gradient: paint.gradient.clone(),
        decoration: None,
    }));
}

/// Paint of a run: a solid premultiplied color and an optional gradient
/// whose geometry is in paragraph space offset by the text origin.
struct RunPaint {
    solid: [f32; 4],
    gradient: Option<Arc<PackedGradient>>,
}

fn run_paint(
    run: &ShapedRun,
    color: &[u8; 4],
    span_paints: &[SpanPaint],
    x: f32,
    y: f32,
) -> Option<RunPaint> {
    let span = match run.span_index {
        Some(index) => span_paints.get(index as usize)?,
        None => &SpanPaint::Color(*color),
    };
    let bounds = [
        x + run.span_bounds[0],
        y + run.span_bounds[1],
        run.span_bounds[2],
        run.span_bounds[3],
    ];
    match span {
        SpanPaint::Color(color) => Some(RunPaint {
            solid: linear_premultiplied(*color, 255)?,
            gradient: None,
        }),
        SpanPaint::Gradient(gradient) => {
            let packed = pack_gradient(gradient, bounds[0], bounds[1], bounds[2], bounds[3], 255)?;
            Some(RunPaint {
                solid: [1.0; 4],
                gradient: Some(Arc::new(packed)),
            })
        }
    }
}

fn decoration_paint(
    decoration: &engine::scene::DecorationLine,
    color: &[u8; 4],
    span_paints: &[SpanPaint],
    x: f32,
    y: f32,
) -> Option<RunPaint> {
    let span = match decoration.span_index {
        Some(index) => span_paints.get(index as usize)?,
        None => &SpanPaint::Color(*color),
    };
    let mut bounds = decoration.span_bounds;
    bounds[0] += x;
    bounds[1] += y;
    match span {
        SpanPaint::Color(color) => Some(RunPaint {
            solid: linear_premultiplied(*color, 255)?,
            gradient: None,
        }),
        SpanPaint::Gradient(gradient) => {
            let packed = pack_gradient(gradient, bounds[0], bounds[1], bounds[2], bounds[3], 255)?;
            Some(RunPaint {
                solid: [1.0; 4],
                gradient: Some(Arc::new(packed)),
            })
        }
    }
}

fn canvas_brush(brush: &CanvasBrush, alpha: u8, opacity: f32) -> Option<RunPaint> {
    let coverage = crate::color::coverage_alpha(alpha, opacity);
    match brush {
        CanvasBrush::Color(color) => Some(RunPaint {
            solid: linear_premultiplied(*color, alpha)?,
            gradient: None,
        })
        .map(|mut paint| {
            paint.solid = scale_color(paint.solid, opacity);
            paint
        }),
        gradient => {
            let packed = pack_canvas_gradient(gradient, alpha)?;
            let solid = [0.0, 0.0, 0.0, coverage];
            Some(RunPaint {
                solid,
                gradient: Some(Arc::new(packed)),
            })
        }
    }
}

fn canvas_image_sample(
    image: &engine::scene::CanvasImageOp,
    width: f32,
    height: f32,
) -> Option<([f32; 4], [f32; 4])> {
    let [x, y, w, h] = image.src.unwrap_or([0.0, 0.0, width, height]);
    let [dw, dh] = [image.w.unwrap_or(w), image.h.unwrap_or(h)];
    if !positive_box(x, y, w, h) || !positive_box(image.x, image.y, dw, dh) {
        return None;
    }
    let left = x.max(0.0);
    let top = y.max(0.0);
    let right = (x + w).min(width);
    let bottom = (y + h).min(height);
    if right <= left || bottom <= top {
        return None;
    }
    let rect = [
        image.x + (left - x) * dw / w,
        image.y + (top - y) * dh / h,
        (right - left) * dw / w,
        (bottom - top) * dh / h,
    ];
    let uv = [left / width, top / height, right / width, bottom / height];
    Some((rect, uv))
}

/// Tessellates a path in canvas points into position+coverage vertices.
pub fn tessellate(
    canvas_path: &CanvasPath,
    tolerance: f32,
) -> Result<lyon_tessellation::VertexBuffers<[f32; 3], u32>, Status> {
    let mut tessellator = lyon_tessellation::FillTessellator::new();
    let mut buffers: lyon_tessellation::VertexBuffers<[f32; 3], u32> =
        lyon_tessellation::VertexBuffers::new();
    let path = tessellation_path(&canvas_path.verbs);
    let result = tessellator.tessellate_path(
        &path,
        &lyon_tessellation::FillOptions::tolerance(tolerance)
            .with_fill_rule(lyon_fill_rule(canvas_path.fill)),
        &mut lyon_tessellation::BuffersBuilder::new(
            &mut buffers,
            |vertex: lyon_tessellation::FillVertex| {
                let position = vertex.position();
                [position.x, position.y, 1.0]
            },
        ),
    );
    result.map_err(|error| {
        tracing::warn!(target: "impasto::canvas", %error, "path tessellation failed");
        Status::BadScene
    })?;
    Ok(buffers)
}

pub fn mesh_bounds(vertices: &[[f32; 3]]) -> [f32; 4] {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for vertex in vertices {
        min[0] = min[0].min(vertex[0]);
        min[1] = min[1].min(vertex[1]);
        max[0] = max[0].max(vertex[0]);
        max[1] = max[1].max(vertex[1]);
    }
    if min[0].is_infinite() {
        return [0.0; 4];
    }
    [min[0], min[1], max[0] - min[0], max[1] - min[1]]
}

/// Analytic box shadow: one quad grown by the blur margin.
fn box_shadow(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    corner_radius: f32,
    shadow: &BoxShadow,
) -> Option<RectInstance> {
    if !shadow.blur_sigma.is_finite()
        || shadow.blur_sigma < 0.0
        || !shadow.spread.is_finite()
        || !shadow.offset[0].is_finite()
        || !shadow.offset[1].is_finite()
    {
        return None;
    }
    let margin = shadow.blur_sigma * 3.0;
    let spread = shadow.spread;
    if w + 2.0 * spread <= 0.0 || h + 2.0 * spread <= 0.0 {
        return None;
    }
    // The quad is the spread rect grown by the margin, shifted by the
    // offset. The shader recovers the spread box from the margin.
    let rect = [
        x + shadow.offset[0] - spread - margin,
        y + shadow.offset[1] - spread - margin,
        w + (spread + margin) * 2.0,
        h + (spread + margin) * 2.0,
    ];
    Some(RectInstance {
        rect,
        radius: corner_radius,
        corners: None,
        stroke_width: 0.0,
        kind: RECT_SHADOW,
        flags: 0,
        color: linear_premultiplied(shadow.color, 255)?,
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [
            shadow.offset[0],
            shadow.offset[1],
            spread,
            shadow.blur_sigma,
        ],
    })
}

/// Material shadow: a tight shadow under the box, scaled by elevation.
fn material_shadow(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    corner_radius: f32,
    shadow: &Shadow,
) -> Option<RectInstance> {
    let elevation = shadow.elevation;
    let sigma = elevation * 0.5;
    let margin = sigma * 3.0;
    let rect = [
        x - margin,
        y - margin + elevation * 0.25,
        w + margin * 2.0,
        h + margin * 2.0,
    ];
    Some(RectInstance {
        rect,
        radius: corner_radius,
        corners: None,
        stroke_width: 0.0,
        kind: RECT_SHADOW,
        flags: 0,
        color: linear_premultiplied(shadow.color, 255)?,
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [0.0, elevation * 0.25, 0.0, sigma],
    })
}

fn gradient_solid(gradient: &Gradient) -> Option<[f32; 4]> {
    let stops = match gradient {
        Gradient::Linear { stops, .. }
        | Gradient::Radial { stops, .. }
        | Gradient::Sweep { stops, .. } => stops,
    };
    stops
        .first()
        .map(|stop| crate::color::gradient_stop(stop, 255))
}

fn scale_color(color: [f32; 4], opacity: f32) -> [f32; 4] {
    [
        color[0] * opacity,
        color[1] * opacity,
        color[2] * opacity,
        color[3] * opacity,
    ]
}

fn clamped_radius(radius: f32, w: f32, h: f32) -> f32 {
    if !radius.is_finite() || radius <= 0.0 {
        0.0
    } else {
        radius.min(w.min(h) * 0.5)
    }
}

pub fn is_axis_aligned(transform: Affine) -> bool {
    transform[1] == 0.0 && transform[2] == 0.0
}

pub fn apply(transform: Affine, x: f32, y: f32) -> (f32, f32) {
    (
        transform[0] * x + transform[2] * y + transform[4],
        transform[1] * x + transform[3] * y + transform[5],
    )
}

/// `outer ∘ inner`: apply `inner` first, then `outer`.
pub fn layer_transform(kind: &engine::scene::LayerKind) -> Affine {
    use engine::scene::LayerKind;
    match *kind {
        LayerKind::Offset { x, y } => [1.0, 0.0, 0.0, 1.0, x, y],
        LayerKind::Transform {
            translate_x,
            translate_y,
            rotate_rad,
            scale_x,
            scale_y,
        } => {
            let (sin, cos) = rotate_rad.sin_cos();
            [
                cos * scale_x,
                sin * scale_x,
                -sin * scale_y,
                cos * scale_y,
                translate_x,
                translate_y,
            ]
        }
        _ => IDENTITY,
    }
}

pub fn compose(outer: Affine, inner: Affine) -> Affine {
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        outer[0] * inner[4] + outer[2] * inner[5] + outer[4],
        outer[1] * inner[4] + outer[3] * inner[5] + outer[5],
    ]
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

fn positive_box(x: f32, y: f32, w: f32, h: f32) -> bool {
    x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0
}

/// Whether a layer node needs an offscreen target.
/// The effect and its sigma for a layer node, when it needs a group.
pub fn root_state() -> ScopeState {
    ScopeState {
        transform: IDENTITY,
        clip: Clip::default(),
        groups: Vec::new(),
    }
}

static ROOT: OnceLock<ScopeState> = OnceLock::new();
