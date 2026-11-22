//! Canvas 2D recording. Geometry is prepared here; the player only fills paths.

mod paint;
mod path;
mod prepare;
mod record;
mod text;

#[cfg(test)]
mod tests;

pub use paint::{ColorStop, Gradient, PaintSource};
pub use path::{FillRule, Path2D};
pub use prepare::prepare;
pub(crate) use prepare::verb_of as path_verb;
pub use record::{
    AuthorState, CanvasSize, Deadline, LineCap, LineJoin, PaintEnv, Recorder, Recording, TextAlign,
    TextBaseline, TextDirection,
};
pub use text::{CanvasFont, TextMetrics, measure_font};

/// Last-frame content box of one canvas node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSurface {
    pub width: f32,
    pub height: f32,
    /// This node or an ancestor has `display: none`.
    pub hidden: bool,
    /// Inherited layout direction. `inherit` walks to the style environment.
    pub rtl: bool,
}

use std::cell::{Cell, RefCell};
use std::sync::Weak;

use thiserror::Error;

/// Session cap for recorder, path, gradient, and retained canvas lists.
pub const SESSION_BYTES: usize = 32 * 1024 * 1024;

pub const MAX_OPS: usize = 65_536;
pub const MAX_SEGMENTS: usize = 262_144;
pub const MAX_SAVES: usize = 256;
pub const MAX_STOPS: usize = 256;
/// Stored dash length after an odd pattern is repeated. Checked before the copy.
pub const MAX_DASH: usize = 256;
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

/// Curve error in physical pixels. Scene tolerance is this divided by `pixel_ratio`.
pub const CURVE_PIXELS: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasErrorCode {
    InvalidArgument,
    LimitExceeded,
    ContextClosed,
    DrawSideEffect,
    AsyncDraw,
    TextUnavailable,
    Unsupported,
    Interrupted,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct CanvasError {
    pub code: CanvasErrorCode,
    pub api: Option<&'static str>,
    pub message: String,
}

impl CanvasError {
    pub fn new(code: CanvasErrorCode, api: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            api: Some(api),
            message: message.into(),
        }
    }

    pub fn argument(api: &'static str, message: impl Into<String>) -> Self {
        Self::new(CanvasErrorCode::InvalidArgument, api, message)
    }

    pub fn limit(api: &'static str) -> Self {
        Self::new(CanvasErrorCode::LimitExceeded, api, "canvas limit exceeded")
    }

    pub fn interrupted() -> Self {
        Self {
            code: CanvasErrorCode::Interrupted,
            api: None,
            message: "canvas draw exceeded its time budget".into(),
        }
    }
}

#[derive(Debug)]
struct LiveBody {
    weak: Weak<crate::scene::CanvasBody>,
    bytes: usize,
}

/// Counts Rust canvas bytes. Shared immutable lists are charged once, and a
/// list still held by a picture is not refunded.
#[derive(Debug)]
pub struct Budget {
    limit: usize,
    temporary: Cell<usize>,
    live: RefCell<Vec<LiveBody>>,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            temporary: Cell::new(0),
            live: RefCell::new(Vec::new()),
        }
    }

    pub fn session() -> Self {
        Self::new(SESSION_BYTES)
    }

    pub fn used(&self) -> usize {
        self.reap();
        self.temporary
            .get()
            .saturating_add(self.live.borrow().iter().map(|slot| slot.bytes).sum())
    }

    pub fn reserve(&self, bytes: usize) -> Result<(), CanvasError> {
        self.reap();
        if self.used().saturating_add(bytes) > self.limit {
            return Err(CanvasError::limit("budget"));
        }
        self.temporary
            .set(self.temporary.get().saturating_add(bytes));
        Ok(())
    }

    pub fn refund(&self, bytes: usize) {
        self.temporary
            .set(self.temporary.get().saturating_sub(bytes));
    }

    /// Charge `list` until its last `Arc` drops.
    pub fn track(&self, list: &crate::scene::CanvasList) -> Result<(), CanvasError> {
        self.reap();
        let ptr = ArcPtr(std::sync::Arc::as_ptr(&list.body) as usize);
        if self
            .live
            .borrow()
            .iter()
            .any(|slot| slot.weak.as_ptr() as usize == ptr.0)
        {
            return Ok(());
        }
        let bytes = list.bytes();
        if self.used().saturating_add(bytes) > self.limit {
            return Err(CanvasError::limit("budget"));
        }
        self.live.borrow_mut().push(LiveBody {
            weak: std::sync::Arc::downgrade(&list.body),
            bytes,
        });
        Ok(())
    }

    fn reap(&self) {
        self.live
            .borrow_mut()
            .retain(|slot| slot.weak.strong_count() > 0);
    }
}

struct ArcPtr(usize);

/// Refunds a temporary reservation unless [`Hold::keep`] runs.
pub struct Hold<'a> {
    budget: &'a Budget,
    bytes: usize,
    kept: bool,
}

impl<'a> Hold<'a> {
    pub fn new(budget: &'a Budget, bytes: usize) -> Result<Self, CanvasError> {
        budget.reserve(bytes)?;
        Ok(Self {
            budget,
            bytes,
            kept: false,
        })
    }

    pub fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        if !self.kept {
            self.budget.refund(self.bytes);
        }
    }
}

pub(crate) fn finite(value: f64) -> bool {
    value.is_finite()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "canvas geometry is computed in f64 and stored as scene-point f32 after a finite check"
)]
pub(crate) fn narrow(value: f64) -> Option<f32> {
    if value.is_finite() {
        let narrowed = value as f32;
        narrowed.is_finite().then_some(narrowed)
    } else {
        None
    }
}
