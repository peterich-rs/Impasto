//! 显示命令与合成层树。

mod canvas;
mod command;
pub mod image;
mod layer;
mod paint;
pub mod text;

pub use canvas::{
    CanvasBody, CanvasBrush, CanvasComposite, CanvasImageOp, CanvasImageRef, CanvasIsolation,
    CanvasList, CanvasOp, CanvasPaint, CanvasPath, CanvasTextOp, FillRule, PathVerb,
};
pub use command::{Command, DrawList};
pub use image::ContentMode;
pub use layer::{Layer, LayerId, LayerKind, LayerTree, Picture, PictureId, SELF_PICTURE};
pub use paint::{BlendMode, BoxShadow, ColorSpace, ColorStop, Gradient, Shadow};
pub use text::{
    DecorationLine, FontFaceId, FontSlant, PositionedGlyph, ShapedParagraph, ShapedRun, SpanPaint,
    TextAlign, TextOverflow, TextSpan, TextStyle,
};
