//! Picture compilation: commands into instance batches and canvas meshes.
//!
//! A compiled picture is a flat list of scopes. Each scope carries the
//! transform, clip, and opacity its draws run under, so a scroll offset or
//! an animation transform changes only the scope matrix, never the batches.
//! Compiled pictures are cached by `PictureId + generation` and reused
//! until the picture leaves the tree or its generation changes.

use std::sync::Arc;

use engine::scene::{
    BlendMode, BoxShadow, CanvasBrush, CanvasComposite, CanvasIsolation, CanvasList, CanvasOp,
    CanvasPaint, CanvasPath, Command, ContentMode, Gradient, Picture, Shadow, ShapedParagraph,
    ShapedRun, SpanPaint,
};

use crate::batch::{
    Brush, ImageInstance, Mesh, PackedGradient, RECT_FILL, RECT_FILL_STROKE, RECT_GRADIENT,
    RECT_SHADOW, RECT_STROKE, RectInstance, image_slices, lyon_fill_rule, pack_canvas_gradient,
    pack_gradient, tessellation_path,
};
use crate::color::linear_premultiplied;
use crate::gpu::Gpu;
use crate::status::Status;

/// Column-major 3x3 affine in scene points: `[a, b, c, d, e, f]` as
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
pub type Affine = [f32; 6];

pub const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Clip state of one scope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    /// Intersection of axis-aligned clips in scene points, or `None`.
    pub scissor: Option<[f32; 4]>,
    /// Rounded or transformed clips, each a rounded rect in the scene space
    /// of the scope that pushed it. The compositor renders these into a
    /// coverage mask when the scope is entered.
    pub shapes: Vec<ClipShape>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipShape {
    pub rect: [f32; 4],
    pub radius: f32,
    /// Transform from the clip's space into the current scope's space.
    pub transform: Affine,
    /// Tessellated path for a canvas clip, in the clip's space.
    pub mesh: Option<Arc<Mesh>>,
}

/// State every draw in a scope runs under.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeState {
    /// Picture space into the parent layer's space.
    pub transform: Affine,
    pub clip: Clip,
    /// Product of every enclosing opacity group.
    pub opacity: f32,
    /// Offscreen group this scope renders into. Scopes between a group's
    /// open and close share the id. `None` draws straight into the target.
    pub groups: Vec<usize>,
}

/// An offscreen group the compositor renders into a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Opacity,
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
}

/// Draw batches inside one scope. Adjacent draws of the same kind merge.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Rects(Vec<RectInstance>, Option<Arc<[[f32; 4]]>>),
    Fill([f32; 4]),
    /// Glyph quads. Atlas pages are resolved when the frame is encoded, so
    /// a cache hit never re-rasterizes.
    Glyphs(Vec<GlyphDraw>),
    Images {
        source: Arc<str>,
        instances: Vec<ImageInstance>,
    },
    Mesh(Mesh),
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
    pub coords: Arc<[i16]>,
    pub embolden: bool,
    pub skew: Option<f32>,
    /// Solid brush, or white for a gradient.
    pub color: [f32; 4],
    pub gradient: Option<Arc<PackedGradient>>,
    /// Span box for gradient coordinates, in scene points.
    pub decoration: Option<[f32; 4]>,
}

/// A picture compiled for one point scale.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledPicture {
    pub generation: u64,
    pub scale: f32,
    /// Picture origin in the parent layer's space.
    pub origin: (f32, f32),
    pub scopes: Vec<Scope>,
    /// Image sources this picture references.
    pub images: Vec<(Arc<str>, bool)>,
    /// Images that were missing at compile time. The picture is rebuilt
    /// once any of them is cached.
    pub waiting: Vec<Arc<str>>,
    pub groups: Vec<Group>,
}

/// Inputs used while resolving picture images.
pub struct CompileContext<'a, 'pixels> {
    pub gpu: &'a Gpu,
    pub resolve: &'a mut dyn FnMut(&str) -> Option<crate::ImagePixels<'pixels>>,
    pub textures: &'a mut crate::textures::TextureStore,
    pub point_scale: f32,
}

/// Compiles one picture.
pub fn compile(
    picture: &Picture,
    context: &mut CompileContext<'_, '_>,
) -> Result<CompiledPicture, Status> {
    let point_scale = context.point_scale;
    let origin = picture.origin;
    let mut compiler = Compiler {
        context,
        stack: vec![root_state()],
        scopes: vec![Scope {
            state: root_state(),
            draws: Vec::new(),
        }],
        images: Vec::new(),
        waiting: Vec::new(),
        groups: Vec::new(),
    };
    for command in picture.commands().iter() {
        compiler.command(command)?;
    }
    if compiler.stack.len() != 1 {
        // An unbalanced push was rejected at record time. Nothing to do.
        return Err(Status::BadScene);
    }
    Ok(CompiledPicture {
        generation: picture.generation,
        scale: point_scale,
        origin,
        scopes: compiler.scopes,
        images: compiler.images,
        waiting: compiler.waiting,
        groups: compiler.groups,
    })
}

struct Compiler<'c, 'a, 'pixels> {
    context: &'c mut CompileContext<'a, 'pixels>,
    stack: Vec<ScopeState>,
    scopes: Vec<Scope>,
    images: Vec<(Arc<str>, bool)>,
    waiting: Vec<Arc<str>>,
    groups: Vec<Group>,
}

impl Compiler<'_, '_, '_> {
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
            });
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.draws.push(draw);
        }
    }

    /// Pushes a scope boundary: the state changed, so the next draw opens
    /// a new scope even if nothing is drawn in between.
    fn boundary(&mut self) {
        let state = self.current().clone();
        self.scopes.push(Scope {
            state,
            draws: Vec::new(),
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
            } => self.push_clip(*x, *y, *w, *h, *corner_radius),
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

    fn clip_fields(&self) -> ([f32; 4], [f32; 4]) {
        let state = self.current();
        let scissor = state.clip.scissor.unwrap_or([0.0; 4]);
        let mask = [0.0; 4];
        (scissor, mask)
    }

    fn base(&self) -> (f32, [f32; 4], [f32; 4]) {
        let (scissor, mask) = self.clip_fields();
        (self.current().opacity, scissor, mask)
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
    ) -> Result<(), Status> {
        if !positive_box(x, y, w, h) {
            return Ok(());
        }
        let (opacity, scissor, mask) = self.base();
        let mut instances = Vec::new();
        for shadow in box_shadows {
            if let Some(instance) = box_shadow(x, y, w, h, corner_radius, shadow, opacity) {
                instances.push(instance);
            }
        }
        if let Some(shadow) = shadow
            && shadow.elevation.is_finite()
            && shadow.elevation > 0.0
            && let Some(instance) = material_shadow(x, y, w, h, corner_radius, shadow, opacity)
        {
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
                    self.push_draw(Draw::Rects(instances, None));
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
        instances.push(RectInstance {
            rect: [x, y, w, h],
            radius: clamped_radius(corner_radius, w, h),
            stroke_width: if stroke { stroke_width } else { 0.0 },
            kind,
            flags,
            color: scale_color(fill_color, opacity),
            stroke_color: scale_color(stroke_color, opacity),
            gradient_a,
            gradient_b: [gradient_b[0], gradient_b[1], gradient_b[2], opacity],
            shadow: [0.0; 4],
            clip_rect: scissor,
            clip_mask: mask,
            pad: [0.0; 4],
        });
        self.push_draw(Draw::Rects(instances, stops.map(Arc::from)));
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
                coords: Arc::from([]),
                embolden: false,
                skew: None,
                color: paint.solid,
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
        let (opacity, scissor, mask) = self.base();
        match self
            .context
            .textures
            .load(self.context.gpu, source, self.context.resolve)
        {
            crate::textures::Load::Ready { width, height, .. } => {
                let slices =
                    image_slices(mode, x, y, w, h, width, height).ok_or(Status::BadScene)?;
                let instances: Vec<ImageInstance> = slices
                    .into_iter()
                    .map(|(rect, uv)| ImageInstance {
                        rect,
                        uv,
                        color: [opacity; 4],
                        flags: 0,
                        clip_rect: scissor,
                        clip_mask: mask,
                    })
                    .collect();
                self.remember_image(source, required);
                self.push_draw(Draw::Images {
                    source: Arc::clone(source),
                    instances,
                });
            }
            crate::textures::Load::Missing => {
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
            crate::textures::Load::Empty => {
                self.waiting.push(Arc::clone(source));
                self.remember_image(source, required);
            }
            crate::textures::Load::Failed => return Err(Status::BadScene),
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

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, corner_radius: f32) {
        let mut state = self.current().clone();
        let transform = state.transform;
        let radius = clamped_radius(corner_radius, w, h);
        if radius <= 0.0 && is_axis_aligned(transform) {
            // Axis-aligned and untransformed: fold into the scissor.
            let (x0, y0) = apply(transform, x, y);
            let (x1, y1) = apply(transform, x + w, y + h);
            let rect = [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()];
            state.clip.scissor = Some(match state.clip.scissor {
                Some(existing) => intersect(existing, rect),
                None => rect,
            });
        } else {
            state.clip.shapes.push(ClipShape {
                rect: [x, y, w, h],
                radius,
                transform,
                mesh: None,
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
        let scale = self.context.point_scale * a.hypot(b).max(c.hypot(d));
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
            self.push_effect(Effect::Opacity, 1.0, 0.0, None);
        }
        self.push_clip(x, y, width, height, 0.0);
        self.push_transform(x, y, 0.0, 1.0, 1.0);
        let floor = self.stack.len();
        for op in list.ops() {
            self.canvas_op(op, floor)?;
        }
        self.stack.truncate(entry);
        self.boundary();
        Ok(())
    }

    fn canvas_op(&mut self, op: &CanvasOp, floor: usize) -> Result<(), Status> {
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
                    self.current_mut().clip.scissor = Some([0.0; 4]);
                } else {
                    let mesh = tessellate(path, self.tessellation_tolerance())?;
                    self.current_mut().clip.shapes.push(ClipShape {
                        rect: mesh_bounds(&mesh),
                        radius: 0.0,
                        transform,
                        mesh: Some(Arc::new(mesh)),
                    });
                }
                self.boundary();
            }
            CanvasOp::Fill { path, paint } => {
                let mesh = fill_mesh(path, paint, self.tessellation_tolerance())?;
                let isolated = self.push_canvas_composite(paint.composite);
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
                let load = self.context.textures.load(
                    self.context.gpu,
                    &image.source,
                    self.context.resolve,
                );
                match load {
                    crate::textures::Load::Ready {
                        width: tex_w,
                        height: tex_h,
                        ..
                    } => {
                        let Some((rect, uv)) = canvas_image_sample(image, tex_w, tex_h) else {
                            return Ok(());
                        };
                        let isolated = self.push_canvas_composite(image.composite);
                        let mut state = self.current().clone();
                        state.transform = compose(state.transform, image.ctm);
                        self.stack.push(state);
                        let (opacity, scissor, mask) = self.base();
                        let alpha = crate::color::coverage_alpha(image.alpha, opacity);
                        self.remember_image(&image.source, false);
                        self.push_draw(Draw::Images {
                            source: Arc::clone(&image.source),
                            instances: vec![ImageInstance {
                                rect,
                                uv,
                                color: [alpha, alpha, alpha, alpha],
                                flags: u32::from(!image.smoothing),
                                clip_rect: scissor,
                                clip_mask: mask,
                            }],
                        });
                        self.pop();
                        if isolated {
                            self.pop();
                        }
                    }
                    crate::textures::Load::Missing => {
                        if !image
                            .source
                            .starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX)
                        {
                            self.waiting.push(Arc::clone(&image.source));
                        }
                    }
                    crate::textures::Load::Empty => self.waiting.push(Arc::clone(&image.source)),
                    crate::textures::Load::Failed => return Err(Status::BadScene),
                }
            }
            CanvasOp::Clear { x, y, w, h, ctm } => {
                self.push_effect(
                    Effect::Canvas(CanvasComposite::DestinationOut),
                    1.0,
                    0.0,
                    None,
                );
                let mut state = self.current().clone();
                state.transform = compose(state.transform, *ctm);
                self.stack.push(state);
                self.push_draw(Draw::Rects(
                    vec![solid_rect([*x, *y, *w, *h], [1.0; 4])],
                    None,
                ));
                self.pop();
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
        stroke_width: 0.0,
        kind: RECT_FILL,
        flags: 0,
        color,
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [0.0; 4],
        clip_rect: [0.0; 4],
        clip_mask: [0.0; 4],
        pad: [0.0; 4],
    }
}

fn append_glyphs(out: &mut Vec<GlyphDraw>, run: &ShapedRun, x: f32, y: f32, paint: &RunPaint) {
    let coords = Arc::<[i16]>::from(run.normalized_coords.as_ref());
    out.extend(run.glyphs.iter().map(|glyph| GlyphDraw {
        face: Arc::clone(&run.face),
        glyph: glyph.id,
        x: x + glyph.x,
        y: y + glyph.y,
        size: run.font_size,
        coords: Arc::clone(&coords),
        embolden: run.embolden,
        skew: run.skew_degrees,
        color: paint.solid,
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

fn fill_mesh(path: &CanvasPath, paint: &CanvasPaint, tolerance: f32) -> Result<Mesh, Status> {
    let (brush, _composite) = match &paint.brush {
        CanvasBrush::Color(color) => (
            Brush::Solid(linear_premultiplied(*color, paint.alpha).unwrap_or([0.0; 4])),
            paint.composite,
        ),
        gradient => (
            Brush::Gradient(pack_canvas_gradient(gradient, paint.alpha).ok_or(Status::BadScene)?),
            paint.composite,
        ),
    };
    let mut mesh = tessellate(path, tolerance)?;
    mesh.brush = brush;
    mesh.composite = paint.composite;
    mesh.alpha = paint.alpha;
    Ok(mesh)
}

/// Tessellates a path in canvas points into position+coverage vertices.
fn tessellate(canvas_path: &CanvasPath, tolerance: f32) -> Result<Mesh, Status> {
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
    Ok(Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
        brush: Brush::Solid([0.0; 4]),
        composite: CanvasComposite::SourceOver,
        alpha: 255,
    })
}

fn mesh_bounds(mesh: &Mesh) -> [f32; 4] {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for vertex in &mesh.vertices {
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
    opacity: f32,
) -> Option<RectInstance> {
    if !shadow.blur_sigma.is_finite()
        || shadow.blur_sigma < 0.0
        || !shadow.spread.is_finite()
        || !shadow.offset[0].is_finite()
        || !shadow.offset[1].is_finite()
    {
        return None;
    }
    let (opacity, scissor, mask) = (opacity, [0.0; 4], [0.0; 4]);
    let _ = (scissor, mask);
    let margin = shadow.blur_sigma * 3.0;
    let spread = shadow.spread;
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
        stroke_width: 0.0,
        kind: RECT_SHADOW,
        flags: 0,
        color: scale_color(linear_premultiplied(shadow.color, 255)?, opacity),
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [
            shadow.offset[0],
            shadow.offset[1],
            spread,
            shadow.blur_sigma,
        ],
        clip_rect: [0.0; 4],
        clip_mask: [0.0; 4],
        pad: [0.0; 4],
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
    opacity: f32,
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
        stroke_width: 0.0,
        kind: RECT_SHADOW,
        flags: 0,
        color: scale_color(linear_premultiplied(shadow.color, 255)?, opacity),
        stroke_color: [0.0; 4],
        gradient_a: [0.0; 4],
        gradient_b: [0.0, 0.0, 0.0, 1.0],
        shadow: [0.0, elevation * 0.25, 0.0, sigma],
        clip_rect: [0.0; 4],
        clip_mask: [0.0; 4],
        pad: [0.0; 4],
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
        .and_then(|stop| linear_premultiplied(stop.color, 255))
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

fn is_axis_aligned(transform: Affine) -> bool {
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
        clip: Clip {
            scissor: None,
            shapes: Vec::new(),
        },
        opacity: 1.0,
        groups: Vec::new(),
    }
}

static ROOT: std::sync::OnceLock<ScopeState> = std::sync::OnceLock::new();
