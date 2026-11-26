//! Local drawing commands for one [`crate::scene::Command::Canvas`].
//!
//! Coordinates are scene points in the canvas content box. The list does not
//! carry a JavaScript value, a node id, or a font handle.

use std::sync::Arc;

use crate::scene::{ColorStop, ShapedParagraph};

/// Whether the player must composite this list off the parent picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasIsolation {
    /// Source-over draws can share the parent picture, clipped to the content box.
    Shared,
    /// `clearRect` or a destination-sensitive blend draws into a transparent layer.
    Isolated,
}

/// Fill rule for a path or a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// One segment of a path already mapped into canvas space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathVerb {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A path plus the rule used when it is filled or clipped.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasPath {
    pub verbs: Arc<[PathVerb]>,
    pub fill: FillRule,
}

/// Porter-Duff and separable blends the screen canvas implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CanvasComposite {
    SourceOver,
    DestinationOver,
    DestinationOut,
    Multiply,
    Screen,
    Lighter,
    Xor,
}

impl CanvasComposite {
    /// Blends that read the destination, plus callers that also clear.
    pub const fn reads_destination(self) -> bool {
        !matches!(self, Self::SourceOver)
    }
}

/// A paint resolved into canvas space.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasBrush {
    Color([u8; 4]),
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        stops: Arc<[ColorStop]>,
    },
    /// Two-circle radial. The player maps this to a conical gradient.
    Radial {
        start: [f32; 2],
        start_radius: f32,
        end: [f32; 2],
        end_radius: f32,
        stops: Arc<[ColorStop]>,
    },
    /// Sweep around `center`, starting at `angle` radians.
    Conic {
        center: [f32; 2],
        angle: f32,
        stops: Arc<[ColorStop]>,
    },
}

/// Brush, coverage, and blend for one drawing operation.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasPaint {
    pub brush: CanvasBrush,
    pub alpha: u8,
    pub composite: CanvasComposite,
}

/// One image draw. Source pixels and destination scene points are separate.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasImageOp {
    pub source: Arc<str>,
    /// Source rect in image pixels. `None` uses the whole image.
    pub src: Option<[f32; 4]>,
    pub x: f32,
    pub y: f32,
    /// `None` maps the source's pixel size to scene points at draw time.
    pub w: Option<f32>,
    pub h: Option<f32>,
    pub smoothing: bool,
    pub alpha: u8,
    pub composite: CanvasComposite,
    /// Maps the destination rect from user space into the canvas content box.
    pub ctm: [f32; 6],
}

/// One text run. `(x, y)` is the top-left of the unscaled line box in user space.
/// `ctm` maps that point into the canvas content box. `anchor_x` stays fixed
/// when `compress` squeezes the run.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasTextOp {
    pub x: f32,
    pub y: f32,
    pub anchor_x: f32,
    /// Source text, kept for tests and diagnostics. Drawing uses `shaped`.
    pub utf8: Arc<str>,
    /// Horizontal scale about `anchor_x`. `1` draws the measured advances.
    pub compress: f32,
    /// `[a, b, c, d, e, f]` matches canvas `setTransform`.
    pub ctm: [f32; 6],
    /// Frozen glyphs relative to `(x, y)`. `None` draws nothing.
    pub shaped: Option<Arc<ShapedParagraph>>,
    pub paint: CanvasPaint,
}

/// One local operation. A list never contains another list.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasOp {
    Save,
    Restore,
    Clip {
        path: CanvasPath,
    },
    Fill {
        path: CanvasPath,
        paint: CanvasPaint,
    },
    /// Solid source-over rectangle in canvas space, optionally with circular corners.
    /// A reusable vector asset with both canonical and mapped geometry.
    Vector {
        path: CanvasPath,
        asset: CanvasPath,
        transform: [f32; 6],
        paint: CanvasPaint,
    },
    SolidRect {
        bounds: [f32; 4],
        radius: f32,
        color: [u8; 4],
        alpha: u8,
    },
    Text(CanvasTextOp),
    Image(CanvasImageOp),
    /// Punch a hole. Ignores fill, alpha, and the blend mode.
    /// The rect is in user space; `ctm` maps it into the canvas content box.
    Clear {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        ctm: [f32; 6],
    },
}

/// An image this list may draw. Loading and texture lifetime use this table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasImageRef {
    pub source: Arc<str>,
}

/// Immutable body shared by every copy of a [`CanvasList`].
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasBody {
    pub ops: Box<[CanvasOp]>,
    pub images: Box<[CanvasImageRef]>,
    pub isolation: CanvasIsolation,
    /// Bytes counted once for every live `Arc` of this body.
    pub bytes: usize,
}

/// One installed canvas picture. Cloning shares the body.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasList {
    pub body: Arc<CanvasBody>,
}

impl CanvasList {
    pub fn empty() -> Self {
        Self {
            body: Arc::new(CanvasBody {
                ops: Box::new([]),
                images: Box::new([]),
                isolation: CanvasIsolation::Shared,
                bytes: 0,
            }),
        }
    }

    pub fn ops(&self) -> &[CanvasOp] {
        &self.body.ops
    }

    pub fn images(&self) -> &[CanvasImageRef] {
        &self.body.images
    }

    pub fn isolation(&self) -> CanvasIsolation {
        self.body.isolation
    }

    pub fn bytes(&self) -> usize {
        self.body.bytes
    }
}
