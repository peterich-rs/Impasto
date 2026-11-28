//! Time-based motion. Samples use milliseconds, not a frame count.

/// Scroll coast and control springs share this.
///
/// `t_ms` is milliseconds since the motion started. A 60 Hz clock and a 120 Hz
/// clock that visit the same timestamps produce the same positions.
pub trait Simulation {
    fn x(&self, t_ms: f64) -> f32;
    fn dx(&self, t_ms: f64) -> f32;
    fn done(&self, t_ms: f64) -> bool;
    /// Value the motion approaches. A spring returns its target. Friction returns
    /// `x0 + v0 * tau`.
    fn terminal(&self) -> f32;

    /// Copies the pure parameters when another queue can sample this simulation.
    fn compositor_curve(&self) -> Option<MotionCurve> {
        None
    }
}

/// Exponential decay. `tau_ms` is the time constant. Scroll uses 500.
#[derive(Clone, Copy, Debug)]
pub struct Friction {
    pub x0: f32,
    pub v0: f32,
    pub tau_ms: f32,
}

impl Simulation for Friction {
    fn x(&self, t_ms: f64) -> f32 {
        if !(self.tau_ms.is_finite() && self.tau_ms > 0.0) {
            return self.x0;
        }
        let t = crate::cast::f32_from_f64(t_ms);
        self.x0 + self.v0 * self.tau_ms * (1.0 - (-t / self.tau_ms).exp())
    }

    fn dx(&self, t_ms: f64) -> f32 {
        if !(self.tau_ms.is_finite() && self.tau_ms > 0.0) {
            return 0.0;
        }
        let t = crate::cast::f32_from_f64(t_ms);
        self.v0 * (-t / self.tau_ms).exp()
    }

    fn done(&self, t_ms: f64) -> bool {
        self.dx(t_ms).abs() < 0.02
    }

    fn terminal(&self) -> f32 {
        if !(self.tau_ms.is_finite() && self.tau_ms > 0.0) {
            return self.x0;
        }
        self.x0 + self.v0 * self.tau_ms
    }

    fn compositor_curve(&self) -> Option<MotionCurve> {
        (self.x0.is_finite() && self.v0.is_finite() && self.tau_ms.is_finite() && self.tau_ms > 0.0)
            .then_some(MotionCurve::Friction {
                x0: self.x0,
                v0: self.v0,
                tau_ms: self.tau_ms,
            })
    }
}

/// Damped spring toward `target`. `stiffness` is per second squared and
/// `damping` is per second. `v0` and [`Simulation::dx`] stay in scene points
/// per millisecond, matching [`Friction`].
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub x0: f32,
    pub v0: f32,
    pub target: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Simulation for Spring {
    fn x(&self, t_ms: f64) -> f32 {
        spring_sample(self, crate::cast::f32_from_f64(t_ms)).0
    }

    fn dx(&self, t_ms: f64) -> f32 {
        spring_sample(self, crate::cast::f32_from_f64(t_ms)).1
    }

    fn done(&self, t_ms: f64) -> bool {
        let (x, v) = spring_sample(self, crate::cast::f32_from_f64(t_ms));
        (x - self.target).abs() < 0.5 && v.abs() < 0.02
    }

    fn terminal(&self) -> f32 {
        self.target
    }

    fn compositor_curve(&self) -> Option<MotionCurve> {
        ([self.x0, self.v0, self.target, self.stiffness, self.damping]
            .into_iter()
            .all(f32::is_finite)
            && self.stiffness > 0.0
            && self.damping > 0.0)
            .then_some(MotionCurve::Spring {
                x0: self.x0,
                v0: self.v0,
                target: self.target,
                stiffness: self.stiffness,
                damping: self.damping,
            })
    }
}

mod curve;

pub use curve::{Easing, MotionCurve, Tween};

fn spring_sample(spring: &Spring, t_ms: f32) -> (f32, f32) {
    if !(spring.stiffness.is_finite() && spring.stiffness > 0.0) || !t_ms.is_finite() || t_ms <= 0.0
    {
        return (spring.x0, spring.v0);
    }
    let t = t_ms / 1000.0;
    let v0 = spring.v0 * 1000.0;
    let (x, v_per_sec) = spring_sample_seconds(spring, t, v0);
    (x, v_per_sec / 1000.0)
}

fn spring_sample_seconds(spring: &Spring, t: f32, v0: f32) -> (f32, f32) {
    let omega = spring.stiffness.sqrt();
    let zeta = spring.damping / (2.0 * omega);
    let offset = spring.x0 - spring.target;
    if !zeta.is_finite() {
        return (spring.x0, v0);
    }
    if (zeta - 1.0).abs() < 1.0e-3 {
        let decay = (-omega * t).exp();
        let x = spring.target + (offset + (v0 + omega * offset) * t) * decay;
        let v =
            (v0 + omega * offset) * decay + (offset + (v0 + omega * offset) * t) * (-omega) * decay;
        (x, v)
    } else if zeta < 1.0 {
        let omega_d = omega * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega * t).exp();
        let a = offset;
        let b = (v0 + zeta * omega * a) / omega_d;
        let cos = (omega_d * t).cos();
        let sin = (omega_d * t).sin();
        let x = spring.target + decay * (a * cos + b * sin);
        let v = -zeta * omega * decay * (a * cos + b * sin)
            + decay * (-a * omega_d * sin + b * omega_d * cos);
        (x, v)
    } else {
        let disc = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega * (zeta - disc);
        let r2 = -omega * (zeta + disc);
        let b = (v0 - r1 * offset) / (r2 - r1);
        let a = offset - b;
        let e1 = (r1 * t).exp();
        let e2 = (r2 * t).exp();
        let x = spring.target + a * e1 + b * e2;
        let v = a * r1 * e1 + b * r2 * e2;
        (x, v)
    }
}

/// Scroll friction time constant, in milliseconds.
pub const SCROLL_TAU_MS: f32 = 500.0;

/// Property a motion writes on each sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimProp {
    ScrollX,
    ScrollY,
    Opacity,
    TranslateX,
    TranslateY,
    Rotate,
    Scale,
    OverscrollX,
    OverscrollY,
    /// Indexed runtime effect uniform in the shader's own units.
    ShaderUniform(u8),
}

impl AnimProp {
    /// Script name used by `onMotionEnd`. Matches `guard.ts`.
    pub fn script_name(self) -> &'static str {
        match self {
            Self::ScrollX => "scrollX",
            Self::ScrollY => "scrollY",
            Self::Opacity => "opacity",
            Self::TranslateX => "translateX",
            Self::TranslateY => "translateY",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
            Self::OverscrollX => "overscrollX",
            Self::OverscrollY => "overscrollY",
            Self::ShaderUniform(_) => "shaderUniform",
        }
    }
}

/// Critical damping at ω = 22 retraces 95% of a resting stretch in about 216ms.
pub const OVERSCROLL_STIFFNESS: f32 = 484.0;
pub const OVERSCROLL_DAMPING: f32 = 44.0;
/// Paging and programmatic scroll spring, per second.
pub const SNAP_STIFFNESS: f32 = 220.0;
pub const SNAP_DAMPING: f32 = 30.0;
/// Fraction of the distance past the scroll edge that becomes overscroll.
pub const OVERSCROLL_FACTOR: f32 = 0.55;

/// How a drag past the edge maps to overscroll.
///
/// `Linear` multiplies the overflow by a constant factor: the historical
/// behavior. For `Curve`, the stretch grows with the overflow, its slope
/// shrinks, and the stretch approaches the viewport extent or a smaller cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Band {
    Linear { factor: f32 },
    Curve { c: f32, max_dimension: Option<f32> },
}

impl Band {
    /// Overscroll for `over` points past the edge of a `dimension`-sized scroll.
    pub fn stretch(self, over: f32, dimension: f32) -> f32 {
        // The sign travels with the overflow: pulling past the top edge
        // reports negative overscroll, past the bottom edge positive.
        let sign = if over < 0.0 { -1.0 } else { 1.0 };
        let over = over.abs();
        if over == 0.0 || !(over.is_finite() && dimension.is_finite()) {
            return 0.0;
        }
        match self {
            Self::Linear { factor } => sign * factor * over,
            Self::Curve { c, max_dimension } => {
                let dimension = dimension.min(max_dimension.unwrap_or(dimension));
                if !(c.is_finite() && c > 0.0) || dimension <= 0.0 {
                    return 0.0;
                }
                // d(x) = D (1 - 1 / (c x / D + 1)); at x = D/c the stretch is
                // half the ceiling and the slope keeps falling.
                let ratio = c * over / dimension + 1.0;
                sign * dimension * (1.0 - 1.0 / ratio)
            }
        }
    }

    /// Derivative of [`Band::stretch`] with respect to overflow.
    ///
    /// `Linear` is the constant factor. `Curve` is `c D² / (c|over| + D)²`,
    /// which is `c` at the edge and falls as the pull deepens.
    pub fn slope(self, over: f32, dimension: f32) -> f32 {
        if !over.is_finite() || !dimension.is_finite() || dimension <= 0.0 {
            return 0.0;
        }
        match self {
            Self::Linear { factor } => {
                if factor.is_finite() && factor > 0.0 {
                    factor
                } else {
                    0.0
                }
            }
            Self::Curve { c, max_dimension } => {
                let dimension = dimension.min(max_dimension.unwrap_or(dimension));
                if !(c.is_finite() && c > 0.0) || dimension <= 0.0 {
                    return 0.0;
                }
                let ratio = dimension / (c * over.abs() + dimension);
                c * ratio * ratio
            }
        }
    }

    /// Derivative evaluated at the already stretched viewport position.
    pub(crate) fn slope_at_stretch(self, stretch: f32, dimension: f32) -> f32 {
        if !stretch.is_finite() {
            return 0.0;
        }
        let edge_slope = self.slope(0.0, dimension);
        if let Self::Curve { max_dimension, .. } = self
            && edge_slope > 0.0
        {
            let dimension = dimension.min(max_dimension.unwrap_or(dimension));
            // The fraction of the viewport still covered is D / (c|over| + D).
            let remaining = (1.0 - stretch.abs() / dimension).max(0.0);
            edge_slope * remaining * remaining
        } else {
            edge_slope
        }
    }

    /// Inverse of [`Band::stretch`]: overflow past the edge for a stretched distance.
    pub fn unstretch(self, stretch: f32, dimension: f32) -> f32 {
        if !stretch.is_finite() || !dimension.is_finite() || stretch == 0.0 {
            return 0.0;
        }
        let sign = stretch.signum();
        let abs = stretch.abs();
        match self {
            Self::Linear { factor } => {
                if factor.is_finite() && factor > 0.0 {
                    sign * abs / factor
                } else {
                    0.0
                }
            }
            Self::Curve { c, max_dimension } => {
                let dimension = dimension.min(max_dimension.unwrap_or(dimension));
                if !(c.is_finite() && c > 0.0) || dimension <= 0.0 {
                    return 0.0;
                }
                let clamped = abs.min(dimension * 0.999);
                sign * dimension * clamped / (c * (dimension - clamped))
            }
        }
    }
}

/// How a coast loses speed.
///
/// `Tau` is the historical time constant. `Rate` is `decelerationRate`
/// semantics: speed multiplies by `rate` each millisecond. `Rate(0.998)` is
/// the iOS normal profile and equals a tau of about 500ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decay {
    Tau(f64),
    Rate(f64),
}

impl Decay {
    /// The time constant this decay is equivalent to, for `Friction`.
    pub fn tau_ms(self) -> f64 {
        match self {
            Self::Tau(tau) => tau,
            Self::Rate(rate) => {
                if rate.is_finite() && (0.0..1.0).contains(&rate) {
                    -1.0 / rate.ln()
                } else {
                    0.0
                }
            }
        }
    }
}

/// How a scroll treats a drag past its limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverscrollMode {
    /// Offset stays on the limit. The band maps the overflow to overscroll.
    RubberBand,
    /// Offset stays on the limit and overscroll stays zero.
    Clamp,
}

/// Which edge of a snap interval lines up with the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapAlign {
    Start,
    Center,
    End,
}

/// One axis of periodic snap targets, in scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapGrid {
    pub interval: f32,
    pub align: SnapAlign,
}

/// Which axes a scroll is allowed to move.
///
/// `Both` is the historical default: the larger pointer delta picks the axis.
/// A declared axis never moves the other component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
    #[default]
    Both,
}

impl ScrollAxis {
    pub fn allows_x(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn allows_y(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    /// Scroll and overscroll motions. Other props are not scroll axes.
    pub fn allows(self, prop: AnimProp) -> bool {
        match prop {
            AnimProp::ScrollX | AnimProp::OverscrollX => self.allows_x(),
            AnimProp::ScrollY | AnimProp::OverscrollY => self.allows_y(),
            _ => false,
        }
    }

    /// Drops the component this axis does not scroll.
    pub fn filter(self, x: f32, y: f32) -> (f32, f32) {
        (
            if self.allows_x() { x } else { 0.0 },
            if self.allows_y() { y } else { 0.0 },
        )
    }
}

macro_rules! coded_trinary {
    ($ty:ident, $default:ident, $one:ident, $two:ident) => {
        impl $ty {
            pub fn from_code(code: i32) -> Self {
                match code {
                    1 => Self::$one,
                    2 => Self::$two,
                    _ => Self::$default,
                }
            }

            pub fn code(self) -> i32 {
                match self {
                    Self::$default => 0,
                    Self::$one => 1,
                    Self::$two => 2,
                }
            }
        }
    };
}

coded_trinary!(ScrollAxis, Both, Vertical, Horizontal);

/// Whether a drag on the declared axis is claimed.
///
/// `Content` needs room in the drag direction. `Always` claims even at the
/// edge, so rubber-band and refresh can run. `Never` leaves the drag for an
/// ancestor. `Overflow` allows edge capture on axes with a scrollable range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollAccept {
    #[default]
    Content,
    Always,
    Never,
    Overflow,
}

impl ScrollAccept {
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Always,
            2 => Self::Never,
            3 => Self::Overflow,
            _ => Self::Content,
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Self::Content => 0,
            Self::Always => 1,
            Self::Never => 2,
            Self::Overflow => 3,
        }
    }
}

/// Which distance arms a refresh. The threshold lives on the scroll node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PullMode {
    #[default]
    Off,
    /// `max(0, -overscroll.y)` while dragging. Commit when the phase returns to idle.
    Overscroll,
    /// Sum of top-edge clamp loss. Commit on drag end.
    Edge,
}

coded_trinary!(PullMode, Off, Overscroll, Edge);

/// Per-scroll motion parameters. [`Default`] is [`ScrollPhysics::BOUNCING`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPhysics {
    /// How a coast decelerates. The default is a 500ms time constant.
    pub decay: Decay,
    /// How a drag past the edge stretches. The default is the asymptotic band.
    pub band: Band,
    pub overscroll: OverscrollMode,
    pub overscroll_stiffness: f32,
    pub overscroll_damping: f32,
    /// Only the `Linear` band reads this; `Curve` derives its slope from `c`.
    pub overscroll_factor: f32,
    /// Spring that pulls a snapped or programmatic scroll to rest. Separate
    /// from the overscroll spring so paging can feel firmer than the band.
    pub snap_stiffness: f32,
    pub snap_damping: f32,
    pub snap_x: Option<SnapGrid>,
    pub snap_y: Option<SnapGrid>,
    pub accept: ScrollAccept,
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        Self::BOUNCING
    }
}

impl ScrollPhysics {
    /// Asymptotic rubber band, 500ms coast, and a critically damped return.
    /// A behavior preset, not a platform. `accept` stays [`ScrollAccept::Content`].
    pub const BOUNCING: Self = Self {
        decay: Decay::Tau(500.0),
        band: Band::Curve {
            c: 0.55,
            max_dimension: None,
        },
        overscroll: OverscrollMode::RubberBand,
        overscroll_stiffness: OVERSCROLL_STIFFNESS,
        overscroll_damping: OVERSCROLL_DAMPING,
        overscroll_factor: OVERSCROLL_FACTOR,
        snap_stiffness: SNAP_STIFFNESS,
        snap_damping: SNAP_DAMPING,
        snap_x: None,
        snap_y: None,
        accept: ScrollAccept::Content,
    };

    /// Phased wheel input uses a tighter default band and return spring.
    pub(crate) fn for_trackpad(mut self) -> Self {
        if self.band == Self::BOUNCING.band {
            self.band = Band::Curve {
                c: 0.35,
                max_dimension: Some(260.0),
            };
        }
        if [self.overscroll_stiffness, self.overscroll_damping]
            == [OVERSCROLL_STIFFNESS, OVERSCROLL_DAMPING]
        {
            self.overscroll_stiffness = 576.0;
            self.overscroll_damping = 48.0;
        }
        self
    }

    /// Offset stops at the limit. Overscroll stays zero.
    pub const CLAMPING: Self = Self {
        overscroll: OverscrollMode::Clamp,
        ..Self::BOUNCING
    };

    /// The tau the coasting friction uses, whichever decay spelling chose it.
    pub fn tau_ms(self) -> f32 {
        crate::cast::f32_from_f64(self.decay.tau_ms())
    }

    /// Overscroll for `over` points past the edge of a `dimension`-sized scroll.
    pub fn stretch(self, over: f32, dimension: f32) -> f32 {
        match self.overscroll {
            OverscrollMode::Clamp => 0.0,
            OverscrollMode::RubberBand => self.band.stretch(over, dimension),
        }
    }
}

/// Coast, drag, and settle. Emitted only when the value changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollPhase {
    #[default]
    Idle,
    Dragging,
    Coasting,
    Settling,
}

impl ScrollPhase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Dragging => "dragging",
            Self::Coasting => "coasting",
            Self::Settling => "settling",
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Self::Idle => 0,
            Self::Dragging => 1,
            Self::Coasting => 2,
            Self::Settling => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_timestamps_match_at_60_and_120_hertz() {
        let motion = Friction {
            x0: 0.0,
            v0: 2.0,
            tau_ms: SCROLL_TAU_MS,
        };
        let slow = [0.0, 16.0, 32.0];
        let fast = [0.0, 8.0, 16.0, 24.0, 32.0];
        let slow_end = motion.x(*slow.last().unwrap());
        let fast_end = motion.x(*fast.last().unwrap());
        assert!((slow_end - fast_end).abs() < 1.0e-4);
        assert!((motion.x(16.0) - motion.x(fast[2])).abs() < 1.0e-4);
    }

    #[test]
    fn a_spring_settles_on_its_target() {
        let spring = Spring {
            x0: 0.0,
            v0: 0.0,
            target: 10.0,
            stiffness: OVERSCROLL_STIFFNESS,
            damping: OVERSCROLL_DAMPING,
        };
        assert!(
            (spring.x(16.0) - 10.0).abs() > 1.0,
            "16ms should still be in flight, got {}",
            spring.x(16.0)
        );
        assert!(spring.done(400.0));
        assert!((spring.x(400.0) - 10.0).abs() < 0.5);
    }

    #[test]
    fn release_slope_matches_the_drag_at_its_displayed_stretch() {
        let band = ScrollPhysics::BOUNCING.band;
        let dimension = 400.0;
        for overflow in [-600.0, -40.0, 0.0, 40.0, 600.0] {
            let stretch = band.stretch(overflow, dimension);
            let drag_slope =
                band.stretch(overflow + 0.5, dimension) - band.stretch(overflow - 0.5, dimension);
            assert!(
                (band.slope_at_stretch(stretch, dimension) - drag_slope).abs() < 0.001,
                "release velocity did not match drag velocity at overflow {overflow}"
            );
        }
        let linear = Band::Linear { factor: 0.5 };
        assert_eq!(linear.slope_at_stretch(-40.0, dimension), 0.5);
    }

    #[test]
    fn unstretch_inverts_stretch() {
        let curve = ScrollPhysics::BOUNCING.band;
        let dim = 400.0;
        for overflow in [-600.0, -40.0, 0.0, 40.0, 600.0] {
            let stretch = curve.stretch(overflow, dim);
            let recovered = curve.unstretch(stretch, dim);
            assert!(
                (recovered - overflow).abs() < 1.0e-2,
                "curve overflow {overflow} -> {stretch} -> {recovered}"
            );
        }
        let linear = Band::Linear { factor: 0.55 };
        for overflow in [-120.0, 0.0, 80.0] {
            let recovered = linear.unstretch(linear.stretch(overflow, dim), dim);
            assert!((recovered - overflow).abs() < 1.0e-4);
        }
    }

    #[test]
    fn bouncing_is_the_default_and_clamping_drops_the_band() {
        assert_eq!(ScrollPhysics::default(), ScrollPhysics::BOUNCING);
        assert_eq!(ScrollPhysics::CLAMPING.overscroll, OverscrollMode::Clamp);
        assert_eq!(ScrollPhysics::CLAMPING.stretch(40.0, 80.0), 0.0);
        let band = ScrollPhysics::BOUNCING.band;
        assert!((band.slope(0.0, 80.0) - 0.55).abs() < 1.0e-4);
    }

    #[test]
    fn script_name_matches_the_guard_keys() {
        assert_eq!(AnimProp::ScrollX.script_name(), "scrollX");
        assert_eq!(AnimProp::ScrollY.script_name(), "scrollY");
        assert_eq!(AnimProp::Opacity.script_name(), "opacity");
        assert_eq!(AnimProp::TranslateX.script_name(), "translateX");
        assert_eq!(AnimProp::TranslateY.script_name(), "translateY");
        assert_eq!(AnimProp::Rotate.script_name(), "rotate");
        assert_eq!(AnimProp::Scale.script_name(), "scale");
        assert_eq!(AnimProp::OverscrollX.script_name(), "overscrollX");
        assert_eq!(AnimProp::OverscrollY.script_name(), "overscrollY");
    }
}
