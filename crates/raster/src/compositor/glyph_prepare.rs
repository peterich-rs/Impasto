//! Bounded CPU preparation for visible glyph variants.

use std::ops::{ControlFlow, Range};

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use engine::scene::{LayerId, LayerKind, LayerTree, PictureId};

use super::{Compositor, GlyphTransform};
use crate::atlas::GlyphKey;
use crate::compiler::{self, CompiledPicture, Draw, GlyphDraw, IDENTITY, PictureBounds, Scope};
use crate::gpu::Gpu;

const VISIT_LIMIT: usize = 4096;
const VISIBLE_MASK_SCAN_LIMIT: usize = 4096;
pub(super) const PREPARE_BUDGET: Duration = Duration::from_millis(2);
pub(super) const COLD_MASK_BUDGET: Duration = Duration::from_millis(5);

pub(crate) struct GlyphScene<'scene> {
    pub tree: &'scene LayerTree,
    pub pictures: &'scene HashMap<PictureId, Arc<CompiledPicture>>,
    pub fonts: &'scene crate::FontLibrary,
    pub scale: f32,
    pub notify: Option<&'scene Arc<dyn Fn() + Send + Sync>>,
    pub prepare_deadline: Option<Instant>,
}

pub(crate) enum GlyphChanges<'a> {
    All,
    Rebuilt(&'a HashSet<PictureId>),
    Layers(&'a [LayerId]),
}

#[derive(Clone, Copy)]
struct Visibility {
    transform: compiler::Affine,
    bounds: PictureBounds,
    blur: f32,
}

impl Visibility {
    fn descend(self, kind: &LayerKind) -> Self {
        let blur = match *kind {
            LayerKind::Blur { sigma, .. } => {
                self.blur + 3.0 * sigma.max(0.0) * compiler::effect_scale(self.transform)
            }
            _ => self.blur,
        };
        let clip = match *kind {
            LayerKind::Opacity { alpha: 0, .. } => PictureBounds::Empty,
            LayerKind::Clip { x, y, w, h, .. } => PictureBounds::rect([x, y, w, h]),
            LayerKind::CornerClip { bounds, .. } => PictureBounds::rect(bounds),
            _ => PictureBounds::Unbounded,
        };
        let bounds = match clip.transform(self.transform) {
            PictureBounds::Empty => PictureBounds::Empty,
            PictureBounds::Rect(rect) if blur == 0.0 => self.bounds.clip(rect),
            _ => self.bounds,
        };
        Self {
            transform: compiler::compose(self.transform, compiler::layer_transform(kind)),
            bounds,
            blur,
        }
    }

    fn for_layer(self, layers: &LayerTree, id: LayerId) -> Option<Self> {
        let mut ancestors = Vec::new();
        let mut parent = layers.layers.get(id.0 as usize)?.parent;
        while let Some(id) = parent {
            if ancestors.len() == 128 {
                return None;
            }
            let layer = layers.layers.get(id.0 as usize)?;
            ancestors.push(&layer.kind);
            parent = layer.parent;
        }
        Some(ancestors.into_iter().rev().fold(self, Self::descend))
    }
}

impl Compositor {
    pub(crate) fn prepare_glyphs(
        &mut self,
        gpu: &Gpu,
        scene: GlyphScene<'_>,
        changes: GlyphChanges<'_>,
    ) -> Duration {
        let Some([width, height]) = self.last_viewport else {
            return Duration::ZERO;
        };
        let started = Instant::now();
        let cap = if self.has_submitted {
            PREPARE_BUDGET
        } else {
            COLD_MASK_BUDGET
        };
        let deadline = scene
            .prepare_deadline
            .map_or(started + cap, |outer| outer.min(started + cap));
        if scene.notify.is_some() {
            self.glyph_prefetch_deadline = Some(deadline);
        }
        let aa = 1.0 / scene.scale;
        let viewport = PictureBounds::rect([0.0, 0.0, width as f32 * aa, height as f32 * aa]);
        let root = Visibility {
            transform: IDENTITY,
            bounds: viewport,
            blur: 0.0,
        };
        let context = PrepareContext {
            gpu,
            scene,
            deadline,
        };
        let layers = VisibleLayers::new(context.scene.tree, root, &changes, deadline);
        let mut jobs = GlyphJobs::default();
        for (id, visible) in layers {
            if let GlyphChanges::Rebuilt(rebuilt) = &changes
                && !rebuilt.contains(&id)
            {
                continue;
            }
            let Some(picture) = context.scene.pictures.get(&id) else {
                continue;
            };
            if !picture.has_glyphs {
                continue;
            }
            if jobs
                .collect_picture(self, &context, picture, visible)
                .is_break()
            {
                break;
            }
        }
        let masks = jobs.masks;
        let scene = context.scene;
        self.atlas
            .prepare(scene.fonts, masks, deadline, scene.notify);
        started.elapsed()
    }
}

struct VisibleLayers<'tree> {
    tree: &'tree LayerTree,
    remaining: Vec<(LayerId, Visibility, bool)>,
    visits: Range<usize>,
    deadline: Instant,
}

impl<'tree> VisibleLayers<'tree> {
    fn new(
        tree: &'tree LayerTree,
        root: Visibility,
        changes: &GlyphChanges<'_>,
        deadline: Instant,
    ) -> Self {
        let remaining = match changes {
            GlyphChanges::All | GlyphChanges::Rebuilt(_) => vec![(tree.root, root, true)],
            GlyphChanges::Layers(layers) => layers
                .iter()
                .take(VISIT_LIMIT)
                .take_while(|_| Instant::now() < deadline)
                .filter_map(|&id| {
                    root.for_layer(tree, id)
                        .map(|visibility| (id, visibility, false))
                })
                .collect(),
        };
        Self {
            tree,
            remaining,
            visits: 0..VISIT_LIMIT,
            deadline,
        }
    }
}

impl Iterator for VisibleLayers<'_> {
    type Item = (PictureId, Visibility);

    fn next(&mut self) -> Option<Self::Item> {
        for _ in self.visits.by_ref() {
            let Some((id, visible, siblings)) = self.remaining.pop() else {
                break;
            };
            if Instant::now() >= self.deadline {
                break;
            }
            if visible.bounds == PictureBounds::Empty {
                continue;
            }
            let Some(layer) = self.tree.layers.get(id.0 as usize) else {
                continue;
            };
            if siblings && let Some(next) = layer.next {
                self.remaining.push((next, visible, true));
            }
            if let Some(child) = layer.first_child {
                let child_visible = visible.descend(&layer.kind);
                if child_visible.bounds != PictureBounds::Empty {
                    self.remaining.push((child, child_visible, true));
                }
            }
            if let LayerKind::Picture { picture } = layer.kind {
                return Some((picture, visible));
            }
        }
        None
    }
}

struct PrepareContext<'a, 'scene> {
    gpu: &'a Gpu,
    scene: GlyphScene<'scene>,
    deadline: Instant,
}

#[derive(Default)]
struct GlyphJobs {
    keys: HashSet<GlyphKey>,
    masks: Vec<GlyphKey>,
    glyphs_checked: usize,
}

struct GlyphScope {
    bounds: PictureBounds,
    fringe: f32,
    transform: GlyphTransform,
}

impl GlyphJobs {
    fn collect_picture(
        &mut self,
        compositor: &mut Compositor,
        context: &PrepareContext<'_, '_>,
        picture: &CompiledPicture,
        visible: Visibility,
    ) -> ControlFlow<()> {
        let scale = context.scene.scale;
        let aa = 1.0 / scale;
        let deadline = context.deadline;
        let origin = compiler::compose(
            visible.transform,
            [1.0, 0.0, 0.0, 1.0, picture.origin.0, picture.origin.1],
        );
        let block_culling = !picture.scope_blocks.is_empty()
            && origin[..4] == IDENTITY[..4]
            && scale >= picture.scale
            && visible.blur == 0.0;
        if origin[..4] == IDENTITY[..4]
            && scale >= picture.scale
            && matches!(visible.bounds, PictureBounds::Rect(rect)
                if picture.bounds.transform(origin).expand(aa + visible.blur).clip(rect)
                    == PictureBounds::Empty)
        {
            return ControlFlow::Continue(());
        }
        let block_size = compiler::SCOPES_PER_BLOCK;
        for (block_index, scopes) in picture.scopes.chunks(block_size).enumerate() {
            if block_culling
                && let Some(block_bounds) = picture.scope_blocks.get(block_index)
                && matches!(
                    visible.bounds,
                    PictureBounds::Rect(rect)
                        if block_bounds.transform(origin).expand(aa).clip(rect)
                            == PictureBounds::Empty
                )
            {
                continue;
            }
            for scope in scopes {
                if Instant::now() >= deadline {
                    return ControlFlow::Break(());
                }
                let mut blur = visible.blur;
                if scope.state.groups.iter().any(|&id| {
                    let group = &picture.groups[id];
                    if group.effect == compiler::Effect::Blur {
                        let transform = compiler::compose(origin, group.transform);
                        blur += 3.0 * group.sigma.max(0.0) * compiler::effect_scale(transform);
                    }
                    group.opacity <= 0.0
                }) {
                    continue;
                }
                let fringe = aa + blur;
                let scope_bounds = scope.bounds_in(origin, scale).expand(fringe);
                let bounds = match visible.bounds {
                    PictureBounds::Rect(rect) => scope_bounds.clip(rect),
                    _ => scope_bounds,
                };
                if bounds == PictureBounds::Empty {
                    continue;
                }
                let transform =
                    GlyphTransform::new(compiler::compose(origin, scope.state.transform), scale);
                self.collect_scope(
                    compositor,
                    context,
                    scope,
                    GlyphScope {
                        bounds,
                        fringe,
                        transform,
                    },
                )?;
            }
        }
        ControlFlow::Continue(())
    }

    fn collect_scope(
        &mut self,
        compositor: &mut Compositor,
        context: &PrepareContext<'_, '_>,
        scope: &Scope,
        clip: GlyphScope,
    ) -> ControlFlow<()> {
        for draw in &scope.draws {
            if let Draw::Glyphs(glyphs) = draw {
                self.collect_glyphs(compositor, context, glyphs, &clip)?;
            }
        }
        ControlFlow::Continue(())
    }

    fn collect_glyphs(
        &mut self,
        compositor: &mut Compositor,
        context: &PrepareContext<'_, '_>,
        glyphs: &[GlyphDraw],
        clip: &GlyphScope,
    ) -> ControlFlow<()> {
        let GlyphScope {
            bounds,
            fringe,
            transform,
        } = *clip;
        for glyph in glyphs {
            self.glyphs_checked = self.glyphs_checked.wrapping_add(1);
            if self.glyphs_checked.is_multiple_of(64) && Instant::now() >= context.deadline {
                return ControlFlow::Break(());
            }
            if glyph.decoration.is_some()
                || matches!(bounds, PictureBounds::Rect(rect)
                    if glyph.bounds().transform(transform.matrix)
                        .expand(fringe).clip(rect) == PictureBounds::Empty)
            {
                continue;
            }
            if self.masks.len() >= VISIBLE_MASK_SCAN_LIMIT {
                continue;
            }
            let position = transform.position(glyph);
            let size = glyph.size * transform.pixels;
            if let Some(key) = compositor
                .atlas
                .missing_glyph(context.gpu, glyph, size, position)
                && self.keys.insert(key.clone())
            {
                self.masks.push(key);
            }
        }
        ControlFlow::Continue(())
    }
}
