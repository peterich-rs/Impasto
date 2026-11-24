//! Time-based layer updates and snapshots exchanged by the UI and raster queues.

use crate::motion::MotionCurve;

use super::{Layer, LayerId, LayerKind};

/// Identity of one animation, changed when its property is retargeted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompositorTrackId(pub u64);

/// Scalar property written by a compositor track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositorProp {
    TranslateX,
    TranslateY,
    Rotate,
    Scale,
    Opacity,
    ScrollX,
    ScrollY,
    /// Packed f32 slot in the effect belonging to the target picture.
    ShaderUniform(u8),
}

impl CompositorProp {
    /// Scroll tracks carry untransformed content offsets in scene points.
    pub fn is_scroll(self) -> bool {
        matches!(self, Self::ScrollX | Self::ScrollY)
    }
}

/// Stored transform channels before the pivot is folded into translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositorTransform {
    pub translate: [f32; 2],
    pub rotate_rad: f32,
    pub scale: [f32; 2],
    pub pivot: Option<[f32; 2]>,
}

impl CompositorTransform {
    pub(crate) fn placed(self) -> [f32; 2] {
        let Some([px, py]) = self.pivot else {
            return self.translate;
        };
        let (sin, cos) = self.rotate_rad.sin_cos();
        let scaled_x = px * self.scale[0];
        let scaled_y = py * self.scale[1];
        [
            self.translate[0] + px - (scaled_x * cos - scaled_y * sin),
            self.translate[1] + py - (scaled_x * sin + scaled_y * cos),
        ]
    }
}

/// Pure data required to evaluate one layer property at an absolute vsync target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositorTrack {
    pub id: CompositorTrackId,
    pub layer: LayerId,
    pub prop: CompositorProp,
    pub started_ms: f64,
    pub curve: MotionCurve,
    /// Exclusive handoff bounds for scroll; renderable channel bounds otherwise.
    pub clamp: Option<[f32; 2]>,
    pub expected_end_ms: f64,
    pub transform: Option<CompositorTransform>,
}

impl CompositorTrack {
    /// Creates a numeric layer track. Transform channels can be placed around the supplied pivot.
    pub fn new(
        id: CompositorTrackId,
        layer: LayerId,
        prop: CompositorProp,
        started_ms: f64,
        curve: MotionCurve,
        clamp: Option<[f32; 2]>,
        transform: Option<CompositorTransform>,
    ) -> Self {
        Self {
            id,
            layer,
            prop,
            started_ms,
            curve,
            clamp,
            expected_end_ms: started_ms + curve.expected_end_ms(),
            transform,
        }
    }

    /// A failed sample hands control back to UI at this timestamp.
    pub fn sample(self, target_ms: f64) -> Option<CompositorSample> {
        if !target_ms.is_finite() || target_ms >= self.expected_end_ms {
            return None;
        }
        let (mut value, done) = self.curve.sample((target_ms - self.started_ms).max(0.0));
        if done || !value.is_finite() {
            return None;
        }
        if let Some([min, max]) = self.clamp {
            if min > max || !min.is_finite() || !max.is_finite() {
                return None;
            }
            if self.prop.is_scroll() && (value <= min || value >= max) {
                return None;
            }
            value = value.clamp(min, max);
        }
        Some(CompositorSample { id: self.id, value })
    }
}

/// Latest presented value of a source motion, in its original scene units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositorSample {
    pub id: CompositorTrackId,
    pub value: f32,
}

/// Values visible when an input or completion returns animation ownership to UI.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositorSnapshot {
    pub target_ms: f64,
    pub samples: Vec<CompositorSample>,
}

/// Writes sorted tracks grouped by target layer. The original transform state avoids
/// accumulating rounding error or losing a sibling channel on repeated vsyncs.
pub fn apply_compositor_samples(
    layers: &mut [Layer],
    tracks: &[CompositorTrack],
    samples: &[CompositorSample],
) -> bool {
    if tracks.len() != samples.len()
        || tracks
            .iter()
            .zip(samples)
            .any(|(track, sample)| track.id != sample.id)
    {
        return false;
    }
    let mut start = 0;
    for group in tracks.chunk_by(|left, right| left.layer == right.layer) {
        let Some(layer) = layers.get_mut(group[0].layer.0 as usize) else {
            return false;
        };
        let end = start + group.len();
        if !apply_layer(&mut layer.kind, group, &samples[start..end]) {
            return false;
        }
        start = end;
    }
    true
}

fn apply_layer(
    kind: &mut LayerKind,
    tracks: &[CompositorTrack],
    samples: &[CompositorSample],
) -> bool {
    match kind {
        LayerKind::Transform {
            translate_x,
            translate_y,
            rotate_rad,
            scale_x,
            scale_y,
        } => {
            let mut transform = tracks[0].transform.unwrap_or(CompositorTransform {
                translate: [*translate_x, *translate_y],
                rotate_rad: *rotate_rad,
                scale: [*scale_x, *scale_y],
                pivot: None,
            });
            transform.scale[1] = transform.scale[0];
            for (track, sample) in tracks.iter().zip(samples) {
                match track.prop {
                    CompositorProp::TranslateX => transform.translate[0] = sample.value,
                    CompositorProp::TranslateY => transform.translate[1] = sample.value,
                    CompositorProp::Rotate => transform.rotate_rad = sample.value,
                    CompositorProp::Scale => transform.scale = [sample.value; 2],
                    _ => return false,
                }
            }
            let placed = transform.placed();
            if !placed.into_iter().all(f32::is_finite) {
                return false;
            }
            [*translate_x, *translate_y] = placed;
            *rotate_rad = transform.rotate_rad;
            [*scale_x, *scale_y] = transform.scale;
        }
        LayerKind::Opacity { alpha, .. }
            if tracks.len() == 1 && tracks[0].prop == CompositorProp::Opacity =>
        {
            *alpha = compositor_alpha(samples[0].value);
        }
        LayerKind::Offset { x, y } => {
            for (track, sample) in tracks.iter().zip(samples) {
                match track.prop {
                    CompositorProp::ScrollX => *x = -sample.value,
                    CompositorProp::ScrollY => *y = -sample.value,
                    _ => return false,
                }
            }
        }
        LayerKind::Picture { .. }
            if tracks.iter().all(|track| {
                matches!(track.prop, CompositorProp::ShaderUniform(index)
                    if usize::from(index) < super::MAX_SHADER_UNIFORM_BYTES / size_of::<f32>())
            }) =>
        {
            // Raster applies scalar samples to the draw uniforms for this picture.
        }
        _ => return false,
    }
    true
}

pub(crate) fn compositor_alpha(value: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the rounded alpha is clamped to the byte range"
    )]
    let alpha = value.round().clamp(0.0, 255.0) as u8;
    alpha
}
