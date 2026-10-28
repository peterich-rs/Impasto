//! One native surface. A stale id cannot replace the current one.

use session::Metrics;

use crate::command::{SurfaceId, SurfaceTarget};
use crate::{Presented, Surface};

/// Failed `NotReady` opens before the shell stops arming another try.
pub(crate) const SURFACE_OPEN_ATTEMPTS: u32 = 120;

pub(crate) enum OpenRetry {
    Again,
    GaveUp,
}

pub(crate) enum PendingMetrics {
    Absent,
    Updated,
    Rearm,
}

struct PendingOpen {
    target: SurfaceTarget,
    attempts: u32,
    gave_up: bool,
}

pub enum Ready {
    Opened {
        #[allow(dead_code)]
        id: SurfaceId,
        replaced: Option<SurfaceId>,
    },
    Failed(engine::Status),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changed {
    Staged,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destroyed {
    Dropped,
    Ignored,
}

struct Open {
    id: SurfaceId,
    surface: Box<dyn Surface>,
    metrics: Metrics,
}

pub struct SurfaceHost {
    current: Option<Open>,
    pending: Option<PendingOpen>,
}

impl SurfaceHost {
    pub fn new() -> Self {
        Self {
            current: None,
            pending: None,
        }
    }

    /// Attempts already spent on `id`, if a retry is still armed.
    /// A new ready replaces any other pending target.
    pub fn take_prior_attempts(&mut self, id: SurfaceId) -> u32 {
        match self.pending.take() {
            Some(pending) if pending.target.id == id && !pending.gave_up => pending.attempts,
            _ => 0,
        }
    }

    /// The next open try. A target that already gave up stays stored.
    pub fn take_attempt(&mut self) -> Option<(SurfaceTarget, u32)> {
        if self.pending.as_ref().is_some_and(|pending| pending.gave_up) {
            return None;
        }
        self.pending
            .take()
            .map(|pending| (pending.target, pending.attempts))
    }

    pub fn fail_attempt(&mut self, target: SurfaceTarget, attempts: u32) -> OpenRetry {
        let attempts = attempts.saturating_add(1);
        let gave_up = attempts >= SURFACE_OPEN_ATTEMPTS;
        self.pending = Some(PendingOpen {
            target,
            attempts,
            gave_up,
        });
        if gave_up {
            OpenRetry::GaveUp
        } else {
            OpenRetry::Again
        }
    }

    /// `Resumed` after the cap starts another round of attempts.
    pub fn rearm_pending(&mut self) -> bool {
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        if !pending.gave_up {
            return false;
        }
        pending.attempts = 0;
        pending.gave_up = false;
        true
    }

    pub fn drop_pending(&mut self, id: SurfaceId) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.target.id == id)
        {
            self.pending = None;
        }
    }

    /// Writes metrics onto a pending open. A gave-up target starts another round.
    pub fn retarget(&mut self, id: SurfaceId, metrics: Metrics) -> PendingMetrics {
        let Some(pending) = self.pending.as_mut() else {
            return PendingMetrics::Absent;
        };
        if pending.target.id != id {
            return PendingMetrics::Absent;
        }
        pending.target.metrics = metrics;
        if pending.gave_up {
            pending.attempts = 0;
            pending.gave_up = false;
            PendingMetrics::Rearm
        } else {
            PendingMetrics::Updated
        }
    }

    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    #[allow(dead_code)]
    pub fn id(&self) -> Option<SurfaceId> {
        self.current.as_ref().map(|open| open.id)
    }

    pub fn ready(
        &mut self,
        target: SurfaceTarget,
        engine: &mut engine::Engine,
        open: &mut dyn FnMut(
            &mut engine::Engine,
            SurfaceTarget,
        ) -> Result<Box<dyn Surface>, engine::Status>,
    ) -> Ready {
        let id = target.id;
        let metrics = target.metrics;
        match open(engine, target) {
            Ok(surface) => {
                let replaced = self.current.as_ref().map(|open| open.id);
                self.current = Some(Open {
                    id,
                    surface,
                    metrics,
                });
                Ready::Opened { id, replaced }
            }
            Err(status) => Ready::Failed(status),
        }
    }

    pub fn changed(&mut self, id: SurfaceId, metrics: Metrics) -> Changed {
        let Some(open) = self.current.as_mut() else {
            return Changed::Ignored;
        };
        if open.id != id {
            return Changed::Ignored;
        }
        open.metrics = metrics;
        Changed::Staged
    }

    #[allow(dead_code)]
    pub fn metrics(&self) -> Option<Metrics> {
        self.current.as_ref().map(|open| open.metrics)
    }

    /// Drops the surface only when `id` is current. The caller acks `id` either way.
    pub fn destroyed(&mut self, id: SurfaceId) -> Destroyed {
        if self.current.as_ref().is_some_and(|open| open.id == id) {
            self.current = None;
            Destroyed::Dropped
        } else {
            Destroyed::Ignored
        }
    }

    pub fn present(&mut self, engine: &mut engine::Engine) -> Result<Presented, engine::Status> {
        let Some(open) = self.current.as_mut() else {
            return Err(engine::Status::NotReady);
        };
        open.surface.present(engine)
    }

    pub fn clear(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use session::Metrics;

    use super::{OpenRetry, PendingMetrics, SURFACE_OPEN_ATTEMPTS, SurfaceHost};
    use crate::command::{SurfaceId, SurfaceTarget};
    use crate::ptr::NativePtr;

    fn target(id: u64) -> SurfaceTarget {
        SurfaceTarget {
            id: SurfaceId(id),
            layer: NativePtr(std::ptr::null_mut()),
            metrics: Metrics::new(10.0, 20.0, 2.0),
        }
    }

    #[test]
    fn a_failed_open_retries_until_the_cap_and_resume_starts_again() {
        let mut host = SurfaceHost::new();
        let id = SurfaceId(7);
        let mut attempts = 0;
        for _ in 0..SURFACE_OPEN_ATTEMPTS - 1 {
            let retry = host.fail_attempt(target(id.0), attempts);
            assert!(matches!(retry, OpenRetry::Again));
            let (kept, spent) = host.take_attempt().expect("retry");
            assert_eq!(kept.id, id);
            attempts = spent;
        }
        assert!(matches!(
            host.fail_attempt(target(id.0), attempts),
            OpenRetry::GaveUp
        ));
        assert!(host.take_attempt().is_none());
        assert!(host.rearm_pending());
        assert!(host.take_attempt().is_some());
    }

    #[test]
    fn destroy_drops_a_pending_open_and_a_size_change_rearms_after_the_cap() {
        let mut host = SurfaceHost::new();
        let id = SurfaceId(3);
        assert!(matches!(
            host.fail_attempt(target(id.0), SURFACE_OPEN_ATTEMPTS - 1),
            OpenRetry::GaveUp
        ));
        host.drop_pending(id);
        assert!(!host.rearm_pending());

        assert!(matches!(
            host.fail_attempt(target(id.0), SURFACE_OPEN_ATTEMPTS - 1),
            OpenRetry::GaveUp
        ));
        let metrics = Metrics::new(11.0, 20.0, 2.0);
        assert!(matches!(host.retarget(id, metrics), PendingMetrics::Rearm));
        let (kept, attempts) = host.take_attempt().expect("rearmed");
        assert_eq!(attempts, 0);
        assert_eq!(kept.metrics.width, 11.0);
        assert!(matches!(
            host.retarget(SurfaceId(9), metrics),
            PendingMetrics::Absent
        ));
    }
}
