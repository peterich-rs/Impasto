use crate::Rect;

/// One edge of a window, in scene points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Window metrics in scene points.
///
/// `padding` is the always-on safe area. `view_insets` is the keyboard and other
/// transient overlap. `inset_*` is the per-edge maximum of those two, which is
/// the inset that platform hosts subtract from the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    pub device_pixel_ratio: f32,
    pub inset_left: f32,
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub padding: Insets,
    pub view_insets: Insets,
    /// Regions where system gestures may take precedence over application touch.
    pub system_gesture_insets: Insets,
    /// Local occlusion, for example an undocked keyboard.
    pub keyboard_occlusion: Option<Rect>,
}

impl Metrics {
    pub fn new(width: f32, height: f32, device_pixel_ratio: f32) -> Self {
        Self::from_insets(
            width,
            height,
            device_pixel_ratio,
            Insets::default(),
            Insets::default(),
        )
    }

    /// `inset_*` becomes the per-edge max so the viewport stays the same size
    /// as the single inset rect hosts used to send.
    pub fn from_insets(
        width: f32,
        height: f32,
        device_pixel_ratio: f32,
        padding: Insets,
        view_insets: Insets,
    ) -> Self {
        Self {
            width,
            height,
            device_pixel_ratio,
            inset_left: padding.left.max(view_insets.left),
            inset_top: padding.top.max(view_insets.top),
            inset_right: padding.right.max(view_insets.right),
            inset_bottom: padding.bottom.max(view_insets.bottom),
            padding,
            view_insets,
            system_gesture_insets: Insets::default(),
            keyboard_occlusion: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayFeatureKind {
    Fold,
    Hinge,
    Cutout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldPosture {
    Flat,
    HalfOpened,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayFeature {
    pub bounds: Rect,
    pub kind: DisplayFeatureKind,
    pub posture: Option<FoldPosture>,
    pub separating: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowDisplay {
    pub display_id: Option<i32>,
    pub refresh_rate: Option<f32>,
    pub features: Vec<DisplayFeature>,
}

/// One vsync. Fields are monotonic milliseconds on that platform's clock.
/// Animation samples [`FrameTime::target_ms`].
///
/// [`FrameTime::mono_ms`] is a separate monotonic clock owned by the shell.
/// Timers and `performance.now` sample it. It is not the vsync origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    pub vsync_ms: f64,
    pub target_ms: f64,
    pub interval_ms: f64,
    pub mono_ms: f64,
}

impl FrameTime {
    pub fn at(target_ms: f64) -> Self {
        Self {
            vsync_ms: target_ms,
            target_ms,
            interval_ms: 0.0,
            mono_ms: target_ms,
        }
    }
}
