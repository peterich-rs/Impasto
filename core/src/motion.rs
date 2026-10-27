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
}
