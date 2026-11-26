//! 显示命令与合成层树。

mod canvas;
mod command;
mod compositor_track;
mod scroll;
pub(crate) use scroll::linear_color_bytes;
pub use scroll::{
    Extrapolate, PiecewiseColorMap, PiecewiseMap, ScrollBinding, ScrollCurve, ScrollSource,
    ScrollTargetProp, ScrollThreshold, ScrollValue,
};
mod corners;
pub mod image;
mod layer;
mod paint;
pub mod shader;
#[cfg(feature = "style-binary")]
pub mod template;
pub mod text;

pub use canvas::{
    CanvasBody, CanvasBrush, CanvasComposite, CanvasImageOp, CanvasImageRef, CanvasIsolation,
    CanvasList, CanvasOp, CanvasPaint, CanvasPath, CanvasTextOp, FillRule, PathVerb,
};
pub use command::{Command, DrawList, RectPaint};
pub use compositor_track::{
    CompositorBlockReason, CompositorEligibility, CompositorProp, CompositorSample,
    CompositorSnapshot, CompositorTrack, CompositorTrackId, CompositorTransform,
    apply_compositor_samples,
};
pub(crate) use compositor_track::{compositor_alpha, list_scroll_prediction};
pub use corners::CornerShape;
pub use image::ContentMode;
pub use layer::{
    Layer, LayerId, LayerKind, LayerTree, PaintSlot, Picture, PictureId, PictureUpdate,
    SELF_PICTURE, SceneChanges, SceneDelta,
};
pub use paint::{
    BackdropMaterial, BlendMode, BoxShadow, ColorMatrix, ColorSpace, ColorStop, Gradient, Shadow,
};
pub use shader::{
    BACKDROP_TEXTURE, MAX_SHADER_SOURCE_BYTES, MAX_SHADER_TEXTURES, MAX_SHADER_UNIFORM_BYTES,
    ShaderClock, ShaderEffect, ShaderError, ShaderProgram, ShaderProgramId,
};
pub use text::{
    DecorationLine, FontFaceId, FontSlant, PositionedGlyph, ShapedParagraph, ShapedRun, SpanPaint,
    TextAlign, TextOverflow, TextSpan, TextStyle,
};
