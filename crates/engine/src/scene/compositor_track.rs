//! Time-based layer updates and snapshots exchanged by the UI and raster queues.

use crate::motion::MotionCurve;

use super::{Layer, LayerId, LayerKind, ScrollBinding, ScrollValue};
use std::{collections::HashMap, sync::Arc};

/// Why an active motion must be updated by the UI queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositorBlockReason {
    ScrollCallback { node: u64 },
    GlobalFrameHook,
    CanvasVisibilityObserver,
    PlatformViewPresent,
    PendingUiWork,
    ListWindowNotSettled { node: u64 },
    UnsupportedTrack { node: u64 },
    Disabled,
    SharedTopology,
}

impl CompositorBlockReason {
    pub fn name(self) -> &'static str {
        match self {
            Self::ScrollCallback { .. } => "scroll_callback",
            Self::GlobalFrameHook => "global_frame_hook",
            Self::CanvasVisibilityObserver => "canvas_visibility_observer",
            Self::PlatformViewPresent => "platform_view_present",
            Self::PendingUiWork => "pending_ui_work",
            Self::ListWindowNotSettled { .. } => "list_window_not_settled",
            Self::UnsupportedTrack { .. } => "unsupported_track",
            Self::Disabled => "disabled",
            Self::SharedTopology => "shared_topology",
        }
    }

    pub fn node(self) -> Option<u64> {
        match self {
            Self::ScrollCallback { node }
            | Self::ListWindowNotSettled { node }
            | Self::UnsupportedTrack { node } => Some(node),
            _ => None,
        }
    }
}

pub type CompositorEligibility = Result<(), CompositorBlockReason>;

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
    OverscrollX,
    OverscrollY,
    /// Packed f32 slot in the effect belonging to the target picture.
    ShaderUniform(u8),
}

impl CompositorProp {
    /// Scroll tracks carry untransformed content offsets in scene points.
    pub fn is_scroll(self) -> bool {
        matches!(
            self,
            Self::ScrollX | Self::ScrollY | Self::OverscrollX | Self::OverscrollY
        )
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
#[derive(Clone, Debug, PartialEq)]
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
    /// Allow a raster snapshot of the subtree while this transition is active.
    pub freeze_subtree: bool,
    /// Absolute scroll base when the sampled channel is rubber-band displacement.
    pub overscroll_base: f32,
    /// Exclusive limits before the mounted list or its pinned row needs updating.
    pub scroll_window: Option<[f32; 2]>,
    /// Half-open bounds that preserve every discrete scroll observation.
    pub threshold_window: Option<[f32; 2]>,
    /// Mounted-window edges that can expand before a compositor handoff.
    pub list_prefetch_edges: [Option<f32>; 2],
    /// Visual properties that follow this scroll track.
    pub scroll_bindings: Arc<[ScrollBinding]>,
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
            freeze_subtree: false,
            overscroll_base: 0.0,
            scroll_window: None,
            threshold_window: None,
            list_prefetch_edges: [None; 2],
            scroll_bindings: Arc::default(),
        }
    }

    /// Returns the current speed in the track's units per second.
    pub fn speed(&self, target_ms: f64) -> f32 {
        const SAMPLE_MS: f64 = 4.0;
        let time = (target_ms - self.started_ms).max(0.0);
        let (here, _) = self.curve.sample(time);
        let (next, _) = self.curve.sample(time + SAMPLE_MS);
        (next - here).abs() * 250.0
    }

    /// Requests a UI update while the installed window still covers this frame.
    pub fn needs_list_prefetch(&self, target_ms: f64, interval_ms: f64) -> bool {
        let [low, high] = self.list_prefetch_edges;
        if low.is_none() && high.is_none() {
            return false;
        }
        let lookahead = (2.0 * interval_ms).clamp(8.0, 40.0);
        let future = self
            .curve
            .sample((target_ms + lookahead - self.started_ms).max(0.0))
            .0;
        let future = match self.clamp {
            Some([lo, hi]) if lo <= hi => future.clamp(lo, hi),
            Some(_) => return false,
            None => future,
        };
        low.is_some_and(|edge| future < edge) || high.is_some_and(|edge| future > edge)
    }

    /// Whether this vsync would leave a previously stable threshold state.
    pub fn crosses_scroll_threshold(&self, target_ms: f64) -> bool {
        let Some([low, high]) = self.threshold_window else {
            return false;
        };
        let (value, _) = self.curve.sample((target_ms - self.started_ms).max(0.0));
        let value = match self.clamp {
            Some([min, max]) if min <= max => value.clamp(min, max),
            Some(_) => return false,
            None => value,
        };
        value < low || value >= high
    }

    /// A failed sample hands control back to UI at this timestamp.
    pub fn sample(&self, target_ms: f64) -> Option<CompositorSample> {
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
            if self.prop.is_scroll()
                && (matches!(self.curve, MotionCurve::Friction { .. })
                    && (value <= min || value >= max)
                    || value < min - 0.5
                    || value > max + 0.5)
            {
                return None;
            }
            value = value.clamp(min, max);
        }
        if let Some([min, max]) = self.scroll_window
            && !(min < value && value < max)
        {
            return None;
        }
        if let Some([min, max]) = self.threshold_window
            && !(min <= value && value < max)
        {
            return None;
        }
        if matches!(
            self.prop,
            CompositorProp::OverscrollX | CompositorProp::OverscrollY
        ) && let MotionCurve::Spring { x0, v0, .. } = self.curve
        {
            // At an edge, inertia can start the spring at zero with a nonzero derivative.
            let side = if x0 == 0.0 { v0 } else { x0 };
            if side * value < 0.0 {
                return None;
            }
        }
        Some(CompositorSample { id: self.id, value })
    }
}

pub(crate) fn list_scroll_prediction(curve: MotionCurve, started_ms: f64, target_ms: f64) -> f32 {
    // Leave about two 60 Hz frame intervals for the UI to mount and measure rows.
    const PREFETCH_MS: f64 = 32.0;
    curve
        .sample((target_ms + PREFETCH_MS - started_ms).max(0.0))
        .0
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
    let mut transforms = HashMap::new();
    for (track, sample) in tracks.iter().zip(samples) {
        for binding in track.scroll_bindings.iter() {
            let value = binding.prop.evaluate(&binding.curve, sample.value);
            let Some(layer) = layers.get_mut(binding.layer.0 as usize) else {
                return false;
            };
            if let (Some(base), ScrollValue::Scalar(value)) = (binding.transform, value) {
                let transform = transforms.entry(binding.layer).or_insert(base);
                binding.prop.apply_transform(transform, value);
            } else if !binding.apply(&mut layer.kind, value) {
                return false;
            }
        }
    }
    for (id, transform) in transforms {
        if !write_transform(&mut layers[id.0 as usize].kind, transform) {
            return false;
        }
    }
    true
}

fn write_transform(kind: &mut LayerKind, transform: CompositorTransform) -> bool {
    let LayerKind::Transform {
        translate_x,
        translate_y,
        rotate_rad,
        scale_x,
        scale_y,
    } = kind
    else {
        return false;
    };
    let placed = transform.placed();
    if !placed.into_iter().all(f32::is_finite) {
        return false;
    }
    [*translate_x, *translate_y] = placed;
    *rotate_rad = transform.rotate_rad;
    [*scale_x, *scale_y] = transform.scale;
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
            for (track, sample) in tracks.iter().zip(samples) {
                match track.prop {
                    CompositorProp::TranslateX => transform.translate[0] = sample.value,
                    CompositorProp::TranslateY => transform.translate[1] = sample.value,
                    CompositorProp::Rotate => transform.rotate_rad = sample.value,
                    CompositorProp::Scale => transform.scale = [sample.value; 2],
                    _ => return false,
                }
            }
            return write_transform(kind, transform);
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
                    CompositorProp::OverscrollX => *x = -(track.overscroll_base + sample.value),
                    CompositorProp::OverscrollY => *y = -(track.overscroll_base + sample.value),
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
