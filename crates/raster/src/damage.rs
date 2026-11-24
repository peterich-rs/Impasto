//! Conservative frame-to-frame damage in physical pixels.

use engine::scene::{LayerKind, image::EXTERNAL_TEXTURE_PREFIX};

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
}

impl Default for PaintState {
    fn default() -> Self {
        Self {
            transform: IDENTITY,
            blur: 0.0,
        }
    }
}

impl PaintState {
    fn child(self, kind: &LayerKind) -> Self {
        let blur = match kind {
            LayerKind::Blur { sigma, .. } => {
                self.blur + 3.0 * sigma.max(0.0) * compiler::effect_scale(self.transform)
            }
            _ => self.blur,
        };
        Self {
            transform: compiler::compose(self.transform, compiler::layer_transform(kind)),
            blur,
        }
    }

    fn bounds(self, picture: &crate::CompiledPicture, scale: f32) -> PictureBounds {
        let (x, y) = picture.origin;
        let origin = compiler::compose(self.transform, [1.0, 0.0, 0.0, 1.0, x, y]);
        picture.bounds_in(origin, scale).0.expand(self.blur)
    }
}

#[derive(Clone, Copy, Default)]
struct DiffState {
    old: PaintState,
    new: PaintState,
    changed: bool,
}

pub(crate) fn between(
    old: Option<&Frame>,
    new: &Frame,
    order: &[LayerId],
    textures: &TextureTransaction<'_>,
) -> DamageRect {
    let Some(old) = old else {
        return DamageRect::Full;
    };
    if new.scale != old.scale
        || new.tree.root != old.tree.root
        || new.tree.layers.len() != old.tree.layers.len()
        || (!new.tree.tracks.is_empty() && new.tree.tracks != old.tree.tracks)
    {
        return DamageRect::Full;
    }
    let mut states = vec![DiffState::default(); order.len()];
    let mut changed_bounds = PictureBounds::Empty;
    let mut reads_surroundings = false;
    for &id in order {
        let index = id.0 as usize;
        let before = &old.tree.layers[index];
        let after = &new.tree.layers[index];
        if before.parent != after.parent
            || before.first_child != after.first_child
            || before.next != after.next
            || std::mem::discriminant(&before.kind) != std::mem::discriminant(&after.kind)
        {
            return DamageRect::Full;
        }
        let parent = after
            .parent
            .map_or_else(DiffState::default, |id| states[id.0 as usize]);
        let changed = parent.changed || before.kind != after.kind;
        if changed
            && matches!(
                after.kind,
                LayerKind::BackdropBlur { .. } | LayerKind::PlatformView { .. }
            )
        {
            return DamageRect::Full;
        }
        states[index] = DiffState {
            old: parent.old.child(&before.kind),
            new: parent.new.child(&after.kind),
            changed,
        };
        if let (LayerKind::Picture { picture: old_id }, LayerKind::Picture { picture: new_id }) =
            (&before.kind, &after.kind)
        {
            let Some(old_picture) = old.pictures.get(old_id) else {
                return DamageRect::Full;
            };
            let Some(new_picture) = new.pictures.get(new_id) else {
                return DamageRect::Full;
            };
            let resource_damage = new_picture.images.iter().any(|(source, _)| {
                source.starts_with(EXTERNAL_TEXTURE_PREFIX) || !textures.same_revision(source)
            });
            if changed || old_picture.resident_id != new_picture.resident_id || resource_damage {
                if old_picture.reads_root || new_picture.reads_root {
                    return DamageRect::Full;
                }
                changed_bounds = changed_bounds
                    .union(parent.old.bounds(old_picture, old.scale))
                    .union(parent.new.bounds(new_picture, new.scale));
            }
            reads_surroundings |= old_picture.reads_root || new_picture.reads_root;
        }
        reads_surroundings |= matches!(after.kind, LayerKind::BackdropBlur { .. });
    }
    if reads_surroundings && changed_bounds != PictureBounds::Empty {
        DamageRect::Full
    } else {
        DamageRect::from_bounds(changed_bounds, new.scale)
    }
}
