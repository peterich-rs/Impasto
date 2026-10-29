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
}

/// Exponential decay. `tau_ms` is the time constant. Scroll uses 180.
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
        let t = t_ms as f32;
        self.x0 + self.v0 * self.tau_ms * (1.0 - (-t / self.tau_ms).exp())
    }

    fn dx(&self, t_ms: f64) -> f32 {
        if !(self.tau_ms.is_finite() && self.tau_ms > 0.0) {
            return 0.0;
        }
        let t = t_ms as f32;
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
}

/// Damped spring toward `target`. `stiffness` and `damping` are per millisecond.
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
        spring_sample(self, t_ms as f32).0
    }

    fn dx(&self, t_ms: f64) -> f32 {
        spring_sample(self, t_ms as f32).1
    }

    fn done(&self, t_ms: f64) -> bool {
        let (x, v) = spring_sample(self, t_ms as f32);
        (x - self.target).abs() < 0.5 && v.abs() < 0.02
    }

    fn terminal(&self) -> f32 {
        self.target
    }
}

fn spring_sample(spring: &Spring, t: f32) -> (f32, f32) {
    if !(spring.stiffness.is_finite() && spring.stiffness > 0.0) || !t.is_finite() || t <= 0.0 {
        return (spring.x0, spring.v0);
    }
    let omega = spring.stiffness.sqrt();
    let zeta = spring.damping / (2.0 * omega);
    let offset = spring.x0 - spring.target;
    if !zeta.is_finite() {
        return (spring.x0, spring.v0);
    }
    if (zeta - 1.0).abs() < 1.0e-3 {
        let decay = (-omega * t).exp();
        let x = spring.target + (offset + (spring.v0 + omega * offset) * t) * decay;
        let v = (spring.v0 + omega * offset) * decay
            + (offset + (spring.v0 + omega * offset) * t) * (-omega) * decay;
        (x, v)
    } else if zeta < 1.0 {
        let omega_d = omega * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega * t).exp();
        let a = offset;
        let b = (spring.v0 + zeta * omega * a) / omega_d;
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
        let b = (spring.v0 - r1 * offset) / (r2 - r1);
        let a = offset - b;
        let e1 = (r1 * t).exp();
        let e2 = (r2 * t).exp();
        let x = spring.target + a * e1 + b * e2;
        let v = a * r1 * e1 + b * r2 * e2;
        (x, v)
    }
}

/// Scroll friction time constant, in milliseconds.
pub const SCROLL_TAU_MS: f32 = 180.0;

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
        }
    }
}

/// Rubber-band spring that returns overscroll to zero.
pub const OVERSCROLL_STIFFNESS: f32 = 170.0;
pub const OVERSCROLL_DAMPING: f32 = 26.0;
/// Fraction of the distance past the scroll edge that becomes overscroll.
pub const OVERSCROLL_FACTOR: f32 = 0.55;

/// How a drag past the edge maps to overscroll.
///
/// `Linear` multiplies the overflow by a constant factor: the historical
/// behavior. `Curve` is the UIScrollView asymptotic band: the stretch grows
/// with the overflow but its slope shrinks, and the stretch never reaches
/// `dimension / c`. `dimension` is the scroll's own extent on that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Band {
    Linear { factor: f32 },
    Curve { c: f32 },
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
            Self::Curve { c } => {
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
    /// Offset stays on the limit. The leftover times [`OVERSCROLL_FACTOR`] is overscroll.
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

pub(crate) use coded_trinary;

coded_trinary!(ScrollAxis, Both, Vertical, Horizontal);

/// Whether a drag on the declared axis is claimed.
///
/// `Content` needs room in the drag direction. `Always` claims even at the
/// edge, so rubber-band and refresh can run. `Never` leaves the drag for an
/// ancestor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollAccept {
    #[default]
    Content,
    Always,
    Never,
}

coded_trinary!(ScrollAccept, Content, Always, Never);

/// Per-scroll motion parameters. Defaults match the historical constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPhysics {
    /// How a coast decelerates. The default keeps the historical 180ms tau.
    pub decay: Decay,
    /// How a drag past the edge stretches. The default keeps the linear band.
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
        Self {
            decay: Decay::Tau(f64::from(SCROLL_TAU_MS)),
            band: Band::Linear {
                factor: OVERSCROLL_FACTOR,
            },
            overscroll: OverscrollMode::RubberBand,
            overscroll_stiffness: OVERSCROLL_STIFFNESS,
            overscroll_damping: OVERSCROLL_DAMPING,
            overscroll_factor: OVERSCROLL_FACTOR,
            snap_stiffness: OVERSCROLL_STIFFNESS,
            snap_damping: OVERSCROLL_DAMPING,
            snap_x: None,
            snap_y: None,
            accept: ScrollAccept::Content,
        }
    }
}

impl ScrollPhysics {
    /// The UIScrollView profile: a 0.998 deceleration rate, the asymptotic
    /// band, and a firm paging spring. Flutter's `BouncingScrollPhysics`
    /// users recognize the band; iOS users recognize all three.
    pub const IOS: Self = Self {
        decay: Decay::Rate(0.998),
        band: Band::Curve { c: 0.55 },
        overscroll: OverscrollMode::RubberBand,
        overscroll_stiffness: OVERSCROLL_STIFFNESS,
        overscroll_damping: OVERSCROLL_DAMPING,
        overscroll_factor: OVERSCROLL_FACTOR,
        snap_stiffness: 220.0,
        snap_damping: 30.0,
        snap_x: None,
        snap_y: None,
        accept: ScrollAccept::Content,
    };

    /// The tau the coasting friction uses, whichever decay spelling chose it.
    pub fn tau_ms(self) -> f32 {
        self.decay.tau_ms() as f32
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
            stiffness: 0.01,
            damping: 0.2,
        };
        assert!(spring.done(5_000.0));
        assert!((spring.x(5_000.0) - 10.0).abs() < 0.5);
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
