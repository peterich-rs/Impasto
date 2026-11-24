//! 显示命令与合成层树。

mod canvas;
mod command;
mod compositor_track;
mod corners;
pub mod image;
mod layer;
mod paint;
pub mod shader;
pub mod text;

pub use canvas::{
    CanvasBody, CanvasBrush, CanvasComposite, CanvasImageOp, CanvasImageRef, CanvasIsolation,
    CanvasList, CanvasOp, CanvasPaint, CanvasPath, CanvasTextOp, FillRule, PathVerb,
};
pub use command::{Command, DrawList, RectPaint};
pub(crate) use compositor_track::compositor_alpha;
pub use compositor_track::{
    CompositorProp, CompositorSample, CompositorSnapshot, CompositorTrack, CompositorTrackId,
    CompositorTransform, apply_compositor_samples,
};
pub use corners::CornerShape;
pub use image::ContentMode;
pub use layer::{Layer, LayerId, LayerKind, LayerTree, Picture, PictureId, SELF_PICTURE};
pub use paint::{BlendMode, BoxShadow, ColorSpace, ColorStop, Gradient, Shadow};
pub use shader::{
    BACKDROP_TEXTURE, MAX_SHADER_SOURCE_BYTES, MAX_SHADER_TEXTURES, MAX_SHADER_UNIFORM_BYTES,
    ShaderClock, ShaderEffect, ShaderError, ShaderProgram, ShaderProgramId,
};
pub use text::{
    DecorationLine, FontFaceId, FontSlant, PositionedGlyph, ShapedParagraph, ShapedRun, SpanPaint,
    TextAlign, TextOverflow, TextSpan, TextStyle,
};
