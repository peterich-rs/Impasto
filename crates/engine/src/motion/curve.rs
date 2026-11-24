//! Motion parameters that can cross between the UI and raster queues.

use super::{Friction, Simulation, Spring};

/// Easing for a fixed-duration motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseInOut,
}

/// A motion with an exact end time and an explicit easing function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub duration_ms: f64,
    pub easing: Easing,
}

impl Tween {
    fn progress(self, t_ms: f64) -> f32 {
        if self.duration_ms > 0.0 {
            crate::cast::f32_from_f64((t_ms / self.duration_ms).clamp(0.0, 1.0))
        } else {
            1.0
        }
    }
}

impl Simulation for Tween {
    fn x(&self, t_ms: f64) -> f32 {
        let t = self.progress(t_ms);
        let eased = match self.easing {
            Easing::Linear => t,
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
        };
        if t == 1.0 {
            self.to
        } else {
            self.from + (self.to - self.from) * eased
        }
    }

    fn dx(&self, t_ms: f64) -> f32 {
        if t_ms < 0.0 || self.done(t_ms) {
            return 0.0;
        }
        let t = self.progress(t_ms);
        let rate = match self.easing {
            Easing::Linear => 1.0,
            Easing::EaseInOut => 6.0 * t * (1.0 - t),
        };
        (self.to - self.from) * rate / crate::cast::f32_from_f64(self.duration_ms)
    }

    fn done(&self, t_ms: f64) -> bool {
        t_ms >= self.duration_ms
    }

    fn terminal(&self) -> f32 {
        self.to
    }

    fn compositor_curve(&self) -> Option<MotionCurve> {
        (self.from.is_finite()
            && self.to.is_finite()
            && self.duration_ms.is_finite()
            && self.duration_ms > 0.0)
            .then_some(MotionCurve::Tween {
                from: self.from,
                to: self.to,
                duration_ms: self.duration_ms,
                easing: self.easing,
            })
    }
}

/// Pure parameters for an analytic motion sampled by either queue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MotionCurve {
    Friction {
        x0: f32,
        v0: f32,
        tau_ms: f32,
    },
    Spring {
        x0: f32,
        v0: f32,
        target: f32,
        stiffness: f32,
        damping: f32,
    },
    Tween {
        from: f32,
        to: f32,
        duration_ms: f64,
        easing: Easing,
    },
}

impl MotionCurve {
    /// Samples position and stop state with the same thresholds as the UI tree.
    pub fn sample(self, elapsed_ms: f64) -> (f32, bool) {
        match self {
            Self::Friction { x0, v0, tau_ms } => {
                let curve = Friction { x0, v0, tau_ms };
                (curve.x(elapsed_ms), curve.done(elapsed_ms))
            }
            Self::Spring {
                x0,
                v0,
                target,
                stiffness,
                damping,
            } => {
                let spring = Spring {
                    x0,
                    v0,
                    target,
                    stiffness,
                    damping,
                };
                let (x, v) = super::spring_sample(&spring, crate::cast::f32_from_f64(elapsed_ms));
                (x, (x - target).abs() < 0.5 && v.abs() < 0.02)
            }
            Self::Tween {
                from,
                to,
                duration_ms,
                easing,
            } => {
                let curve = Tween {
                    from,
                    to,
                    duration_ms,
                    easing,
                };
                (curve.x(elapsed_ms), curve.done(elapsed_ms))
            }
        }
    }

    /// Latest autonomous sampling time. Springs can settle earlier at a sampled vsync.
    pub(crate) fn expected_end_ms(self) -> f64 {
        match self {
            Self::Friction { v0, tau_ms, .. } => {
                // Friction's stop test is strict and operates in f32.
                f64::from(tau_ms) * f64::from(v0.abs().max(0.02) / 0.02).ln() + 1.0
            }
            Self::Spring { .. } => 30_000.0,
            Self::Tween { duration_ms, .. } => duration_ms,
        }
    }
}
