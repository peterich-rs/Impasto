//! Conservative frame-to-frame damage in physical pixels.

use std::collections::HashMap;

use engine::scene::{BlendMode, LayerKind, LayerTree, PictureId, image::EXTERNAL_TEXTURE_PREFIX};

use crate::compiler::{self, Affine, IDENTITY, PictureBounds};
use crate::textures::TextureTransaction;
use crate::{Frame, LayerId, PixelSize};

/// Pixels potentially modified by an installed scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DamageRect {
    Empty,
    /// The minimum inclusive X/Y and maximum exclusive X/Y.
    Rect([i32; 4]),
    #[default]
    Full,
}

impl DamageRect {
    /// Computes the fraction of the target touched by the damage.
    pub fn fraction(self, pixels: PixelSize) -> f64 {
        match self {
            Self::Empty => 0.0,
            Self::Full => 1.0,
            Self::Rect([x0, y0, x1, y1]) if pixels.is_usable() => {
                let [w, h] = [i64::from(pixels.width), i64::from(pixels.height)];
                let width = (i64::from(x1).min(w) - i64::from(x0).max(0)).max(0);
                let height = (i64::from(y1).min(h) - i64::from(y0).max(0)).max(0);
                (width as f64 * height as f64) / (w as f64 * h as f64)
            }
            Self::Rect(_) => 0.0,
        }
    }

    pub(crate) fn pixels(self, size: [u32; 2]) -> Option<[u32; 4]> {
        let Self::Rect([x0, y0, x1, y1]) = self else {
            return None;
        };
        let coordinate = |value, max| u32::try_from(value).unwrap_or(0).min(max);
        let [x0, x1] = [coordinate(x0, size[0]), coordinate(x1, size[0])];
        let [y0, y1] = [coordinate(y0, size[1]), coordinate(y1, size[1])];
        (x0 < x1 && y0 < y1).then(|| [x0, y0, x1 - x0, y1 - y0])
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "damage coordinates are rounded outwards and checked before conversion"
    )]
    fn from_bounds(bounds: PictureBounds, scale: f32) -> Self {
        match bounds {
            PictureBounds::Empty => Self::Empty,
            PictureBounds::Unbounded => Self::Full,
            PictureBounds::Rect([x, y, w, h]) => {
                let bounds = [
                    (x * scale).floor() - 1.0,
                    (y * scale).floor() - 1.0,
                    ((x + w) * scale).ceil() + 1.0,
                    ((y + h) * scale).ceil() + 1.0,
                ];
                if bounds.iter().all(|&value| {
                    value.is_finite() && value >= i32::MIN as f32 && value < i32::MAX as f32
                }) {
                    Self::Rect(bounds.map(|value| value as i32))
                } else {
                    Self::Full
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PaintState {
    transform: Affine,
    blur: f32,
    clip: PictureBounds,
    affected: bool,
}

impl Default for PaintState {
    fn default() -> Self {
        Self {
            transform: IDENTITY,
            blur: 0.0,
            clip: PictureBounds::Unbounded,
            affected: false,
        }
    }
}

impl PaintState {
    fn child(self, kind: &LayerKind, affected: bool) -> Self {
        let mut state = self;
        match *kind {
            LayerKind::Blur { sigma, .. } => {
                state.blur += 3.0 * sigma.max(0.0) * compiler::effect_scale(self.transform);
            }
            LayerKind::Clip { x, y, w, h, .. } if self.blur == 0.0 => {
                state.clip =
                    self.clipped(PictureBounds::rect([x, y, w, h]).transform(self.transform));
            }
            LayerKind::CornerClip { bounds, .. } if self.blur == 0.0 => {
                state.clip = self.clipped(PictureBounds::rect(bounds).transform(self.transform));
            }
            _ => {}
        }
        state.transform = compiler::compose(self.transform, compiler::layer_transform(kind));
        state.affected |= affected;
        state
    }

    fn clipped(self, bounds: PictureBounds) -> PictureBounds {
        match self.clip {
            PictureBounds::Unbounded => bounds,
            PictureBounds::Rect(clip) => bounds.clip(clip),
            PictureBounds::Empty => PictureBounds::Empty,
        }
    }

    fn bounds(self, picture: &crate::CompiledPicture, scale: f32) -> PictureBounds {
        let (x, y) = picture.origin;
        let origin = compiler::compose(self.transform, [1.0, 0.0, 0.0, 1.0, x, y]);
        self.clipped(picture.bounds_in(origin, scale).0.expand(self.blur))
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Readback {
    source: PictureBounds,
    output: PictureBounds,
    material: Option<(
        engine::scene::BackdropMaterial,
        Option<engine::scene::CornerShape>,
    )>,
}

impl Readback {
    fn for_layer(kind: &LayerKind, state: PaintState, scale: f32) -> Option<Self> {
        let (rect, sigma, material) = match *kind {
            LayerKind::BackdropBlur { x, y, w, h, sigma } if sigma > 0.0 => {
                ([x, y, w, h], sigma, None)
            }
            LayerKind::BackdropMaterial {
                bounds,
                corners,
                ref material,
            } => (bounds, material.sigma, Some((**material, corners))),
            LayerKind::Blend {
                x,
                y,
                w,
                h,
                mode: BlendMode::Multiply,
            } => ([x, y, w, h], 0.0, None),
            _ => return None,
        };
        let rect = PictureBounds::rect(rect).transform(state.transform);
        // Match the compositor's read area and keep fractional pixel rounding conservative.
        let output = state.clipped(rect).expand(state.blur + 1.0 / scale);
        let source = output.expand(3.0 * sigma * compiler::effect_scale(state.transform));
        Some(Self {
            source,
            output,
            material,
        })
    }
}

struct Drawing {
    picture: PictureId,
    layer: LayerId,
    state: PaintState,
    readbacks_before: usize,
}

#[derive(Default)]
struct SceneDamage {
    drawings: Vec<Drawing>,
    readbacks: Vec<Readback>,
    fills: Vec<(LayerId, PictureBounds, bool)>,
    dynamic: bool,
    unknown_readback: bool,
}

impl SceneDamage {
    fn collect(frame: &Frame, affected: impl Fn(LayerId) -> bool) -> Self {
        let mut damage = Self::default();
        let mut states = vec![PaintState::default(); frame.tree.layers.len()];
        for &id in &frame.order {
            let index = id.0 as usize;
            let layer = &frame.tree.layers[index];
            let parent = layer
                .parent
                .map_or_else(PaintState::default, |id| states[id.0 as usize]);
            let state = parent.child(&layer.kind, affected(id));
            states[index] = state;
            if let Some(readback) = Readback::for_layer(&layer.kind, state, frame.scale) {
                damage.readbacks.push(readback);
            }
            match layer.kind {
                LayerKind::Picture { picture } => {
                    let compiled = &frame.pictures[&picture];
                    damage.dynamic |= compiled.has_shader_effect;
                    damage.unknown_readback |= compiled.reads_root;
                    damage.drawings.push(Drawing {
                        picture,
                        layer: id,
                        state,
                        readbacks_before: damage.readbacks.len(),
                    });
                }
                LayerKind::Fill { bounds, .. } => {
                    let bounds = state.clipped(
                        PictureBounds::rect(bounds)
                            .transform(state.transform)
                            .expand(state.blur),
                    );
                    damage.fills.push((id, bounds, state.affected));
                }
                LayerKind::PlatformView { .. } => damage.dynamic = true,
                _ => {}
            }
        }
        damage
    }

    fn affected_bounds(&self, frame: &Frame) -> PictureBounds {
        self.drawings
            .iter()
            .filter(|draw| draw.state.affected)
            .map(|draw| {
                draw.state
                    .bounds(&frame.pictures[&draw.picture], frame.scale)
            })
            .chain(
                self.fills
                    .iter()
                    .filter_map(|(_, bounds, affected)| affected.then_some(*bounds)),
            )
            .fold(PictureBounds::Empty, PictureBounds::union)
    }
}

fn ancestors_match(old: &LayerTree, old_id: LayerId, new: &LayerTree, new_id: LayerId) -> bool {
    let mut before = old.layers[old_id.0 as usize].parent;
    let mut after = new.layers[new_id.0 as usize].parent;
    while let (Some(a), Some(b)) = (before, after) {
        let a = &old.layers[a.0 as usize];
        let b = &new.layers[b.0 as usize];
        if a.kind != b.kind {
            return false;
        }
        before = a.parent;
        after = b.parent;
    }
    before.is_none() && after.is_none()
}

fn overlaps(a: PictureBounds, b: PictureBounds) -> bool {
    match (a, b) {
        (PictureBounds::Empty, _) | (_, PictureBounds::Empty) => false,
        (PictureBounds::Rect(a), b) => b.clip(a) != PictureBounds::Empty,
        _ => true,
    }
}

fn resolve(
    mut bounds: PictureBounds,
    before: &SceneDamage,
    after: &SceneDamage,
    scale: f32,
) -> DamageRect {
    if before.dynamic || after.dynamic {
        return DamageRect::Full;
    }
    if before.readbacks != after.readbacks {
        // A changed or removed backdrop can expose content from its old output.
        for effect in before.readbacks.iter().chain(&after.readbacks) {
            bounds = bounds.union(effect.output);
        }
    }
    if bounds != PictureBounds::Empty && (before.unknown_readback || after.unknown_readback) {
        return DamageRect::Full;
    }
    for effect in &after.readbacks {
        if overlaps(bounds, effect.source) {
            bounds = bounds.union(effect.output);
        }
    }
    DamageRect::from_bounds(bounds, scale)
}

pub(crate) fn between(
    old: Option<&Frame>,
    new: &Frame,
    textures: &TextureTransaction<'_>,
) -> DamageRect {
    let Some(old) = old else {
        return DamageRect::Full;
    };
    if new.scale != old.scale
        || new.tree.shader_programs != old.tree.shader_programs
        || !new.gpu_view_requests.is_empty()
    {
        return DamageRect::Full;
    }
    let before = SceneDamage::collect(old, |_| false);
    let after = SceneDamage::collect(new, |_| false);
    let mut previous = HashMap::with_capacity(before.drawings.len());
    for (index, draw) in before.drawings.iter().enumerate() {
        if previous.insert(draw.picture, index).is_some() {
            // The public scene format permits instancing the same PictureId.
            return DamageRect::Full;
        }
    }
    let mut last_match = None;
    let mut bounds = PictureBounds::Empty;
    for (id, fill, _) in before.fills.iter().chain(&after.fills) {
        if old.tree.layers.get(id.0 as usize).map(|layer| &layer.kind)
            != new.tree.layers.get(id.0 as usize).map(|layer| &layer.kind)
            || old.tree.layers.get(id.0 as usize).is_none()
            || new.tree.layers.get(id.0 as usize).is_none()
            || !ancestors_match(&old.tree, *id, &new.tree, *id)
        {
            bounds = bounds.union(*fill);
        }
    }
    for draw in &after.drawings {
        let picture = &new.pictures[&draw.picture];
        let Some(index) = previous.remove(&draw.picture) else {
            bounds = bounds.union(draw.state.bounds(picture, new.scale));
            continue;
        };
        if last_match.is_some_and(|last| index < last) {
            return DamageRect::Full;
        }
        last_match = Some(index);
        let past = &before.drawings[index];
        let old_picture = &old.pictures[&past.picture];
        let changed = picture.resident_id != old_picture.resident_id
            || new.shader_uniforms.get(&draw.layer) != old.shader_uniforms.get(&past.layer)
            || draw.readbacks_before != past.readbacks_before
            || !ancestors_match(&old.tree, past.layer, &new.tree, draw.layer)
            || picture.images.iter().any(|(source, _)| {
                source.starts_with(EXTERNAL_TEXTURE_PREFIX) || !textures.same_revision(source)
            });
        if changed {
            bounds = bounds
                .union(draw.state.bounds(picture, new.scale))
                .union(past.state.bounds(old_picture, old.scale));
        }
    }
    for index in previous.into_values() {
        let past = &before.drawings[index];
        bounds = bounds.union(past.state.bounds(&old.pictures[&past.picture], old.scale));
    }
    resolve(bounds, &before, &after, new.scale)
}

pub(crate) fn sample(
    frame: &mut Frame,
    samples: &[engine::scene::CompositorSample],
) -> Option<DamageRect> {
    let tracks = std::sync::Arc::clone(&frame.tree.tracks);
    let linked: Vec<LayerId> = tracks
        .iter()
        .flat_map(|track| track.scroll_bindings.iter().map(|binding| binding.layer))
        .collect();
    let tracked = |id: LayerId| {
        tracks
            .binary_search_by_key(&id.0, |track| track.layer.0)
            .is_ok()
            || linked.contains(&id)
    };
    let before = SceneDamage::collect(frame, tracked);
    let previous = before.affected_bounds(frame);
    let tree = std::sync::Arc::make_mut(&mut frame.tree);
    if !engine::scene::apply_compositor_samples(&mut tree.layers, &tree.tracks, samples) {
        return None;
    }
    let after = SceneDamage::collect(frame, tracked);
    let current = after.affected_bounds(frame);
    Some(resolve(
        previous.union(current),
        &before,
        &after,
        frame.scale,
    ))
}
