//! One native surface. A stale id cannot replace the current one.

use session::Metrics;

use crate::command::{SurfaceId, SurfaceTarget};
use crate::{Presented, Surface};

pub enum Ready {
    Opened {
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
}

impl SurfaceHost {
    pub fn new() -> Self {
        Self { current: None }
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
