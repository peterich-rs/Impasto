//! Plays one [`protocol::Command::Canvas`] into the current picture.
//!
//! Scene points stay in the canvas content box. `point_scale` is already on the
//! builder transform; canvas ops do not multiply it again.

use std::ptr;
use std::sync::Arc;

use protocol::{
    CanvasBrush, CanvasComposite, CanvasImageOp, CanvasIsolation, CanvasList, CanvasOp,
    CanvasPaint, CanvasPath, CanvasTextOp, ColorStop, FillRule, PathVerb, TextStyle,
};

use super::HeldParagraph;

use super::images::TextureLoad;
use crate::ImagePixels;
use crate::player::{impeller_color, paint_color};

const TEXT_WIDTH: f32 = 1_000_000.0;

impl<'a> crate::painter::Painter<'a> {
    #[allow(
        clippy::too_many_arguments,
        reason = "one canvas command is the content box, the list, and the picture builder"
    )]
    pub(crate) fn play_canvas<'pixels>(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        list: &CanvasList,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        if list.ops().is_empty() || !positive_box(x, y, width, height) {
            return true;
        }
        // SAFETY: `builder` is the picture builder for this submit. The count is a
        // plain integer; nothing is retained.
        let entry = unsafe { impeller::ImpellerDisplayListBuilderGetSaveCount(builder) };
        let Some(layer) = open_canvas(builder, list.isolation(), x, y, width, height) else {
            return false;
        };
        let played = self.play_ops(builder, paint, width, height, list.ops(), resolve);
        // SAFETY: `entry` was captured before this canvas pushed. RestoreToCount
        // drops the isolation save and any author save left open, including when
        // an op failed halfway through.
        unsafe { impeller::ImpellerDisplayListBuilderRestoreToCount(builder, entry) };
        if let CanvasLayer::Isolated(layer_paint) = layer {
            // SAFETY: the layer paint was retained by this canvas until Restore.
            // The display list keeps its own reference.
            unsafe { impeller::ImpellerPaintRelease(layer_paint) };
        }
        played
    }

    fn play_ops<'pixels>(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        width: f32,
        height: f32,
        ops: &[CanvasOp],
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        // SAFETY: the count is the isolation save plus the builder's previous stack.
        let floor = unsafe { impeller::ImpellerDisplayListBuilderGetSaveCount(builder) };
        for op in ops {
            let drawn = match op {
                CanvasOp::Save => {
                    // SAFETY: paired with an author Restore, or with RestoreToCount
                    // when the author leaves the save open.
                    unsafe { impeller::ImpellerDisplayListBuilderSave(builder) };
                    true
                }
                CanvasOp::Restore => {
                    // SAFETY: only pops an author save. The isolation save stays
                    // until the canvas entry count is restored.
                    unsafe {
                        if impeller::ImpellerDisplayListBuilderGetSaveCount(builder) > floor {
                            impeller::ImpellerDisplayListBuilderRestore(builder);
                        }
                    }
                    true
                }
                CanvasOp::Clip { path } => clip_path(builder, path),
                CanvasOp::Fill { path, paint } => fill_path(builder, path, paint),
                CanvasOp::Text(text) => self.draw_text_op(builder, paint, width, height, text),
                CanvasOp::Image(image) => {
                    self.draw_image_op(builder, width, height, image, resolve)
                }
                CanvasOp::Clear { x, y, w, h, ctm } => clear_rect(builder, *x, *y, *w, *h, *ctm),
            };
            if !drawn {
                return false;
            }
        }
        true
    }

    fn draw_text_op(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        width: f32,
        height: f32,
        text: &CanvasTextOp,
    ) -> bool {
        if text.utf8.is_empty() || !text.font_size.is_finite() || text.font_size <= 0.0 {
            return true;
        }
        if !ctm_finite(text.ctm) {
            return true;
        }
        let Ok(layer) = open_composite(builder, text.paint.composite, width, height) else {
            return false;
        };
        let drawn = self.draw_text_transformed(builder, paint, text);
        close_composite(builder, layer);
        drawn
    }

    fn draw_text_transformed(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        text: &CanvasTextOp,
    ) -> bool {
        let compress = text_compress(text);
        let transformed = compress != 1.0 || !ctm_identity(text.ctm);
        if !transformed {
            return self.draw_text_run(builder, paint, text);
        }
        // SAFETY: the save is restored on every path. Scale about `anchor_x` is
        // closest to the glyph, then the user matrix maps into the canvas.
        unsafe { impeller::ImpellerDisplayListBuilderSave(builder) };
        apply_ctm(builder, text.ctm);
        if compress != 1.0 {
            // SAFETY: builder is the submit's display list. Translation is in
            // user space so the anchor stays fixed while glyphs squeeze.
            unsafe {
                impeller::ImpellerDisplayListBuilderTranslate(builder, text.anchor_x, 0.0);
                impeller::ImpellerDisplayListBuilderScale(builder, compress, 1.0);
                impeller::ImpellerDisplayListBuilderTranslate(builder, -text.anchor_x, 0.0);
            }
        }
        let drawn = self.draw_text_run(builder, paint, text);
        // SAFETY: balances the Save above, including when the text run fails.
        unsafe { impeller::ImpellerDisplayListBuilderRestore(builder) };
        drawn
    }

    fn draw_text_run(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        text: &CanvasTextOp,
    ) -> bool {
        let style = single_line_style(&text.style);
        match prepare_brush(&text.paint.brush, text.paint.alpha, text_brush_matrix(text)) {
            PreparedBrush::Skip => true,
            PreparedBrush::Solid(color) => self.draw_text(
                builder,
                paint,
                text.x,
                text.y,
                TEXT_WIDTH,
                text.font_size,
                protocol::ParagraphId::NONE,
                color,
                text.utf8.as_ref(),
                &style,
                &[],
            ),
            PreparedBrush::Gradient(source) => {
                self.draw_gradient_text(builder, text, &style, source)
            }
        }
    }

    fn draw_gradient_text(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        text: &CanvasTextOp,
        style: &TextStyle,
        source: impeller::ImpellerColorSource,
    ) -> bool {
        let Some(text_paint) = alloc_paint(
            [255, 255, 255, 255],
            impeller::ImpellerBlendMode::kImpellerBlendModeSourceOver,
        ) else {
            // SAFETY: the gradient source is a New handle and is not stored.
            unsafe { impeller::ImpellerColorSourceRelease(source) };
            return false;
        };
        // SAFETY: the paint retains the color source. The paragraph copies that
        // paint during shape. Both handles are released before this returns;
        // the paragraph retain is the one `shape` returned.
        let paragraph = unsafe {
            impeller::ImpellerPaintSetColorSource(text_paint, source);
            impeller::ImpellerColorSourceRelease(source);
            let paragraph = self.typography.shape(
                Some(text_paint),
                text.utf8.as_ref(),
                text.font_size,
                TEXT_WIDTH,
                [255, 255, 255, 255],
                style,
                &[],
            );
            impeller::ImpellerPaintRelease(text_paint);
            paragraph
        };
        let Some(paragraph) = paragraph else {
            return false;
        };
        let point = impeller::ImpellerPoint {
            x: text.x,
            y: text.y,
        };
        // SAFETY: `paragraph` is the New handle from shape. It is stored in
        // `pending` and released with the display list. Slot `u32::MAX` is not
        // a paint-cache index; release_draw ignores it.
        unsafe {
            impeller::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point);
        }
        self.pending.push(HeldParagraph {
            paragraph,
            slot: u32::MAX,
        });
        true
    }

    fn draw_image_op<'pixels>(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        width: f32,
        height: f32,
        image: &CanvasImageOp,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        if !ctm_finite(image.ctm) {
            return true;
        }
        let (texture, tex_w, tex_h) = match self.load_texture(image.source.as_ref(), resolve) {
            TextureLoad::Ready(texture, tex_w, tex_h) => (texture, tex_w, tex_h),
            TextureLoad::Missing => {
                self.note_waiting(&image.source);
                return true;
            }
            TextureLoad::Empty => return true,
            TextureLoad::Failed => return false,
        };
        let Some(sample) =
            image_sample(image.src, image.x, image.y, image.w, image.h, tex_w, tex_h)
        else {
            return true;
        };
        let Ok(layer) = open_composite(builder, image.composite, width, height) else {
            return false;
        };
        let drawn = self.blit_canvas_image(builder, texture, image, &sample);
        close_composite(builder, layer);
        drawn
    }

    fn blit_canvas_image(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
        texture: impeller::ImpellerTexture,
        image: &CanvasImageOp,
        sample: &SampledRect,
    ) -> bool {
        let sampling = if image.smoothing {
            impeller::ImpellerTextureSampling::kImpellerTextureSamplingLinear
        } else {
            impeller::ImpellerTextureSampling::kImpellerTextureSamplingNearestNeighbor
        };
        let Some(paint) = alloc_paint(
            [255, 255, 255, image.alpha],
            impeller::ImpellerBlendMode::kImpellerBlendModeSourceOver,
        ) else {
            return false;
        };
        let src = rect(sample.src[0], sample.src[1], sample.src[2], sample.src[3]);
        let dst = rect(sample.dst[0], sample.dst[1], sample.dst[2], sample.dst[3]);
        // SAFETY: `texture` stays in the texture cache for this submit. The save
        // is restored before return. The paint is released after the draw.
        unsafe {
            impeller::ImpellerDisplayListBuilderSave(builder);
            apply_ctm(builder, image.ctm);
            impeller::ImpellerDisplayListBuilderDrawTextureRect(
                builder, texture, &src, &dst, sampling, paint,
            );
            impeller::ImpellerDisplayListBuilderRestore(builder);
            impeller::ImpellerPaintRelease(paint);
        }
        true
    }

    fn note_waiting(&mut self, source: &Arc<str>) {
        if self
            .waiting
            .iter()
            .any(|item| item.as_ref() == source.as_ref())
        {
            return;
        }
        self.waiting.push(Arc::clone(source));
    }
}

enum CanvasLayer {
    Shared,
    Isolated(impeller::ImpellerPaint),
}

fn open_canvas(
    builder: impeller::ImpellerDisplayListBuilder,
    isolation: CanvasIsolation,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Option<CanvasLayer> {
    let local = rect(0.0, 0.0, width, height);
    match isolation {
        CanvasIsolation::Shared => {
            // SAFETY: RestoreToCount pops this save. The clip is the content box
            // after the origin translation, in scene points.
            unsafe {
                impeller::ImpellerDisplayListBuilderSave(builder);
                impeller::ImpellerDisplayListBuilderTranslate(builder, x, y);
                impeller::ImpellerDisplayListBuilderClipRect(
                    builder,
                    &local,
                    impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            }
            Some(CanvasLayer::Shared)
        }
        CanvasIsolation::Isolated => {
            // SAFETY: a null paint is an allocation failure and returns before
            // any save. RestoreToCount drops the layer; the caller then releases
            // this New handle.
            let layer_paint = unsafe { impeller::ImpellerPaintNew() };
            if layer_paint.is_null() {
                return None;
            }
            paint_color(layer_paint, [255, 255, 255, 255]);
            let bounds = rect(x, y, width, height);
            // SAFETY: `layer_paint` stays alive until RestoreToCount. Source-over
            // composites the offscreen layer back. clearRect and destination
            // blends inside it do not erase the parent picture. The clip keeps
            // those ops inside the content box.
            unsafe {
                impeller::ImpellerPaintSetBlendMode(
                    layer_paint,
                    impeller::ImpellerBlendMode::kImpellerBlendModeSourceOver,
                );
                impeller::ImpellerDisplayListBuilderSaveLayer(
                    builder,
                    &bounds,
                    layer_paint,
                    ptr::null_mut(),
                );
                impeller::ImpellerDisplayListBuilderTranslate(builder, x, y);
                impeller::ImpellerDisplayListBuilderClipRect(
                    builder,
                    &local,
                    impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            }
            Some(CanvasLayer::Isolated(layer_paint))
        }
    }
}

enum CompositeGuard {
    Direct,
    Layer(impeller::ImpellerPaint),
}

fn open_composite(
    builder: impeller::ImpellerDisplayListBuilder,
    composite: CanvasComposite,
    width: f32,
    height: f32,
) -> Result<CompositeGuard, ()> {
    if composite == CanvasComposite::SourceOver {
        return Ok(CompositeGuard::Direct);
    }
    let Some(layer_paint) = alloc_paint([255, 255, 255, 255], composite_blend(composite)) else {
        return Err(());
    };
    let bounds = rect(0.0, 0.0, width, height);
    // SAFETY: close_composite restores this layer. Text has no draw paint, so
    // the blend is the layer composite. The bounds are the canvas content box.
    unsafe {
        impeller::ImpellerDisplayListBuilderSaveLayer(
            builder,
            &bounds,
            layer_paint,
            ptr::null_mut(),
        );
    }
    Ok(CompositeGuard::Layer(layer_paint))
}

fn close_composite(builder: impeller::ImpellerDisplayListBuilder, guard: CompositeGuard) {
    let CompositeGuard::Layer(layer_paint) = guard else {
        return;
    };
    // SAFETY: this is the save opened by `open_composite`. The paint is released
    // after Restore so the layer can read it while compositing.
    unsafe {
        impeller::ImpellerDisplayListBuilderRestore(builder);
        impeller::ImpellerPaintRelease(layer_paint);
    }
}

fn clip_path(builder: impeller::ImpellerDisplayListBuilder, path: &CanvasPath) -> bool {
    if path.verbs.is_empty() {
        let empty = rect(0.0, 0.0, 0.0, 0.0);
        // SAFETY: an empty clip intersects the current clip with a zero rect.
        // Later draws in this save are rejected until restore. `empty` is borrowed
        // for the call.
        unsafe {
            impeller::ImpellerDisplayListBuilderClipRect(
                builder,
                &empty,
                impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
            );
        }
        return true;
    }
    let Some(path) = take_path(path) else {
        return false;
    };
    // SAFETY: ClipPath retains the path for the display list. This drops the
    // caller's New reference. The clip is intersect and is not stroked.
    unsafe {
        impeller::ImpellerDisplayListBuilderClipPath(
            builder,
            path,
            impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
        );
        impeller::ImpellerPathRelease(path);
    }
    true
}

fn fill_path(
    builder: impeller::ImpellerDisplayListBuilder,
    path: &CanvasPath,
    paint: &CanvasPaint,
) -> bool {
    let Some(path) = take_path(path) else {
        return false;
    };
    let drawn = match prepare_brush(&paint.brush, paint.alpha, None) {
        PreparedBrush::Skip => true,
        PreparedBrush::Solid(color) => {
            draw_filled(builder, path, color, composite_blend(paint.composite))
        }
        PreparedBrush::Gradient(source) => {
            draw_filled_source(builder, path, source, composite_blend(paint.composite))
        }
    };
    // SAFETY: DrawPath retained the path. This releases the New reference.
    unsafe { impeller::ImpellerPathRelease(path) };
    drawn
}

fn draw_filled(
    builder: impeller::ImpellerDisplayListBuilder,
    path: impeller::ImpellerPath,
    color: [u8; 4],
    blend: impeller::ImpellerBlendMode::Type,
) -> bool {
    let Some(paint) = alloc_paint(color, blend) else {
        return false;
    };
    // SAFETY: the paint is released after the draw. The path outlives this call.
    unsafe {
        impeller::ImpellerDisplayListBuilderDrawPath(builder, path, paint);
        impeller::ImpellerPaintRelease(paint);
    }
    true
}

fn draw_filled_source(
    builder: impeller::ImpellerDisplayListBuilder,
    path: impeller::ImpellerPath,
    source: impeller::ImpellerColorSource,
    blend: impeller::ImpellerBlendMode::Type,
) -> bool {
    let Some(paint) = alloc_paint([255, 255, 255, 255], blend) else {
        // SAFETY: the gradient is a New handle. The paint allocation failed,
        // so nothing else retains the source.
        unsafe { impeller::ImpellerColorSourceRelease(source) };
        return false;
    };
    // SAFETY: the paint retains the color source. Both are released after the draw.
    unsafe {
        impeller::ImpellerPaintSetColorSource(paint, source);
        impeller::ImpellerColorSourceRelease(source);
        impeller::ImpellerDisplayListBuilderDrawPath(builder, path, paint);
        impeller::ImpellerPaintRelease(paint);
    }
    true
}

fn clear_rect(
    builder: impeller::ImpellerDisplayListBuilder,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    ctm: [f32; 6],
) -> bool {
    if !ctm_finite(ctm) {
        return true;
    }
    let (x, w) = normalize_axis(x, w);
    let (y, h) = normalize_axis(y, h);
    if !(w > 0.0 && h > 0.0) {
        return true;
    }
    let Some(paint) = alloc_paint(
        [0, 0, 0, 255],
        impeller::ImpellerBlendMode::kImpellerBlendModeClear,
    ) else {
        return false;
    };
    if ctm_identity(ctm) {
        let area = rect(x, y, w, h);
        // SAFETY: the paint is released after the draw. Identity needs no path.
        unsafe {
            impeller::ImpellerDisplayListBuilderDrawRect(builder, &area, paint);
            impeller::ImpellerPaintRelease(paint);
        }
        return true;
    }
    let corners = [
        map_point(ctm, x, y),
        map_point(ctm, x + w, y),
        map_point(ctm, x + w, y + h),
        map_point(ctm, x, y + h),
    ];
    let Some(path) = corner_path(corners) else {
        // SAFETY: the clear paint is a New handle and the path was not drawn.
        unsafe { impeller::ImpellerPaintRelease(paint) };
        return false;
    };
    // SAFETY: DrawPath retains the path. Both New handles are released once.
    unsafe {
        impeller::ImpellerDisplayListBuilderDrawPath(builder, path, paint);
        impeller::ImpellerPathRelease(path);
        impeller::ImpellerPaintRelease(paint);
    }
    true
}

fn take_path(path: &CanvasPath) -> Option<impeller::ImpellerPath> {
    // SAFETY: the builder is released on every path. TakePathNew returns the
    // caller's one reference; a null builder or path is a failed allocation.
    unsafe {
        let builder = impeller::ImpellerPathBuilderNew();
        if builder.is_null() {
            None
        } else {
            for verb in path.verbs.iter() {
                match *verb {
                    PathVerb::Move(x, y) => {
                        let point = impeller::ImpellerPoint { x, y };
                        impeller::ImpellerPathBuilderMoveTo(builder, &point);
                    }
                    PathVerb::Line(x, y) => {
                        let point = impeller::ImpellerPoint { x, y };
                        impeller::ImpellerPathBuilderLineTo(builder, &point);
                    }
                    PathVerb::Quad(cx, cy, x, y) => {
                        let control = impeller::ImpellerPoint { x: cx, y: cy };
                        let end = impeller::ImpellerPoint { x, y };
                        impeller::ImpellerPathBuilderQuadraticCurveTo(builder, &control, &end);
                    }
                    PathVerb::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                        let c1 = impeller::ImpellerPoint { x: c1x, y: c1y };
                        let c2 = impeller::ImpellerPoint { x: c2x, y: c2y };
                        let end = impeller::ImpellerPoint { x, y };
                        impeller::ImpellerPathBuilderCubicCurveTo(builder, &c1, &c2, &end);
                    }
                    PathVerb::Close => impeller::ImpellerPathBuilderClose(builder),
                }
            }
            let filled = impeller::ImpellerPathBuilderTakePathNew(builder, fill_type(path.fill));
            impeller::ImpellerPathBuilderRelease(builder);
            if filled.is_null() { None } else { Some(filled) }
        }
    }
}

fn corner_path(corners: [(f32, f32); 4]) -> Option<impeller::ImpellerPath> {
    let verbs: Arc<[PathVerb]> = Arc::from([
        PathVerb::Move(corners[0].0, corners[0].1),
        PathVerb::Line(corners[1].0, corners[1].1),
        PathVerb::Line(corners[2].0, corners[2].1),
        PathVerb::Line(corners[3].0, corners[3].1),
        PathVerb::Close,
    ]);
    take_path(&CanvasPath {
        verbs,
        fill: FillRule::NonZero,
    })
}

fn fill_type(fill: FillRule) -> impeller::ImpellerFillType::Type {
    match fill {
        FillRule::NonZero => impeller::ImpellerFillType::kImpellerFillTypeNonZero,
        FillRule::EvenOdd => impeller::ImpellerFillType::kImpellerFillTypeOdd,
    }
}

enum PreparedBrush {
    Solid([u8; 4]),
    Gradient(impeller::ImpellerColorSource),
    Skip,
}

fn prepare_brush(brush: &CanvasBrush, coverage: u8, matrix: Option<[f32; 16]>) -> PreparedBrush {
    match brush {
        CanvasBrush::Color(color) => {
            PreparedBrush::Solid([color[0], color[1], color[2], scale_u8(color[3], coverage)])
        }
        CanvasBrush::Linear { start, end, stops } => {
            let Some(source) = linear_source(*start, *end, stops, coverage, matrix) else {
                return PreparedBrush::Skip;
            };
            PreparedBrush::Gradient(source)
        }
        CanvasBrush::Radial {
            start,
            start_radius,
            end,
            end_radius,
            stops,
        } => {
            let Some(source) = conical_source(
                *start,
                *start_radius,
                *end,
                *end_radius,
                stops,
                coverage,
                matrix,
            ) else {
                return PreparedBrush::Skip;
            };
            PreparedBrush::Gradient(source)
        }
        CanvasBrush::Conic {
            center,
            angle,
            stops,
        } => {
            let Some(source) = sweep_source(*center, *angle, stops, coverage, matrix) else {
                return PreparedBrush::Skip;
            };
            PreparedBrush::Gradient(source)
        }
    }
}

fn linear_source(
    start: [f32; 2],
    end: [f32; 2],
    stops: &[ColorStop],
    coverage: u8,
    matrix: Option<[f32; 16]>,
) -> Option<impeller::ImpellerColorSource> {
    if !start
        .iter()
        .chain(end.iter())
        .all(|value| value.is_finite())
    {
        return None;
    }
    let (colors, offsets) = gradient_stops(stops, coverage)?;
    let from = impeller::ImpellerPoint {
        x: start[0],
        y: start[1],
    };
    let to = impeller::ImpellerPoint {
        x: end[0],
        y: end[1],
    };
    let count = u32::try_from(colors.len()).unwrap_or(0);
    let source = with_gradient_matrix(matrix, |transform| {
        // SAFETY: color and offset slices live until this call returns. A null
        // source means Impeller rejected the gradient; the caller skips the draw.
        unsafe {
            impeller::ImpellerColorSourceCreateLinearGradientNew(
                &from,
                &to,
                count,
                colors.as_ptr(),
                offsets.as_ptr(),
                impeller::ImpellerTileMode::kImpellerTileModeClamp,
                transform,
            )
        }
    });
    if source.is_null() { None } else { Some(source) }
}

fn conical_source(
    start: [f32; 2],
    start_radius: f32,
    end: [f32; 2],
    end_radius: f32,
    stops: &[ColorStop],
    coverage: u8,
    matrix: Option<[f32; 16]>,
) -> Option<impeller::ImpellerColorSource> {
    if !start_radius.is_finite()
        || !end_radius.is_finite()
        || !start
            .iter()
            .chain(end.iter())
            .all(|value| value.is_finite())
    {
        return None;
    }
    let (colors, offsets) = gradient_stops(stops, coverage)?;
    let from = impeller::ImpellerPoint {
        x: start[0],
        y: start[1],
    };
    let to = impeller::ImpellerPoint {
        x: end[0],
        y: end[1],
    };
    let count = u32::try_from(colors.len()).unwrap_or(0);
    let source = with_gradient_matrix(matrix, |transform| {
        // SAFETY: slices live for the call. The two-circle radial is a conical
        // gradient. The caller releases a non-null source once.
        unsafe {
            impeller::ImpellerColorSourceCreateConicalGradientNew(
                &from,
                start_radius,
                &to,
                end_radius,
                count,
                colors.as_ptr(),
                offsets.as_ptr(),
                impeller::ImpellerTileMode::kImpellerTileModeClamp,
                transform,
            )
        }
    });
    if source.is_null() { None } else { Some(source) }
}

fn sweep_source(
    center: [f32; 2],
    angle: f32,
    stops: &[ColorStop],
    coverage: u8,
    matrix: Option<[f32; 16]>,
) -> Option<impeller::ImpellerColorSource> {
    if !angle.is_finite() || !center[0].is_finite() || !center[1].is_finite() {
        return None;
    }
    let (colors, offsets) = gradient_stops(stops, coverage)?;
    let center = impeller::ImpellerPoint {
        x: center[0],
        y: center[1],
    };
    let start = angle * (180.0 / std::f32::consts::PI);
    let end = start + 360.0;
    let count = u32::try_from(colors.len()).unwrap_or(0);
    let source = with_gradient_matrix(matrix, |transform| {
        // SAFETY: slices live for the call. Sweep angles are degrees. The caller
        // releases a non-null source once.
        unsafe {
            impeller::ImpellerColorSourceCreateSweepGradientNew(
                &center,
                start,
                end,
                count,
                colors.as_ptr(),
                offsets.as_ptr(),
                impeller::ImpellerTileMode::kImpellerTileModeClamp,
                transform,
            )
        }
    });
    if source.is_null() { None } else { Some(source) }
}

fn with_gradient_matrix<T>(
    matrix: Option<[f32; 16]>,
    build: impl FnOnce(*const impeller::ImpellerMatrix) -> T,
) -> T {
    let stored = matrix.map(|value| impeller::ImpellerMatrix { m: value });
    let transform = match &stored {
        Some(value) => ptr::from_ref(value),
        None => ptr::null(),
    };
    build(transform)
}

fn gradient_stops(
    stops: &[ColorStop],
    coverage: u8,
) -> Option<(Vec<impeller::ImpellerColor>, Vec<f32>)> {
    if stops.len() < 2 {
        return None;
    }
    if stops.iter().any(|stop| !stop.offset.is_finite()) {
        return None;
    }
    let colors = stops
        .iter()
        .map(|stop| {
            impeller_color([
                stop.color[0],
                stop.color[1],
                stop.color[2],
                scale_u8(stop.color[3], coverage),
            ])
        })
        .collect();
    let offsets = stops.iter().map(|stop| stop.offset).collect();
    Some((colors, offsets))
}

fn alloc_paint(
    color: [u8; 4],
    blend: impeller::ImpellerBlendMode::Type,
) -> Option<impeller::ImpellerPaint> {
    // SAFETY: a null paint is an allocation failure. The caller releases it.
    let paint = unsafe { impeller::ImpellerPaintNew() };
    if paint.is_null() {
        return None;
    }
    paint_color(paint, color);
    // SAFETY: the paint was just created and is non-null. Blend and fill are
    // stored on it; the caller releases the paint.
    unsafe {
        impeller::ImpellerPaintSetBlendMode(paint, blend);
        impeller::ImpellerPaintSetDrawStyle(
            paint,
            impeller::ImpellerDrawStyle::kImpellerDrawStyleFill,
        );
    }
    Some(paint)
}

fn composite_blend(composite: CanvasComposite) -> impeller::ImpellerBlendMode::Type {
    match composite {
        CanvasComposite::SourceOver => impeller::ImpellerBlendMode::kImpellerBlendModeSourceOver,
        CanvasComposite::DestinationOver => {
            impeller::ImpellerBlendMode::kImpellerBlendModeDestinationOver
        }
        CanvasComposite::DestinationOut => {
            impeller::ImpellerBlendMode::kImpellerBlendModeDestinationOut
        }
        CanvasComposite::Multiply => impeller::ImpellerBlendMode::kImpellerBlendModeMultiply,
        CanvasComposite::Screen => impeller::ImpellerBlendMode::kImpellerBlendModeScreen,
        CanvasComposite::Lighter => impeller::ImpellerBlendMode::kImpellerBlendModePlus,
        CanvasComposite::Xor => impeller::ImpellerBlendMode::kImpellerBlendModeXor,
    }
}

fn apply_ctm(builder: impeller::ImpellerDisplayListBuilder, ctm: [f32; 6]) {
    if ctm_identity(ctm) {
        return;
    }
    let matrix = impeller::ImpellerMatrix { m: ctm_matrix(ctm) };
    // SAFETY: the matrix is borrowed for the call. Impeller appends it, so the
    // user matrix applies before the canvas origin and `point_scale`.
    unsafe { impeller::ImpellerDisplayListBuilderTransform(builder, &matrix) };
}

fn ctm_matrix(ctm: [f32; 6]) -> [f32; 16] {
    [
        ctm[0], ctm[1], 0.0, 0.0, ctm[2], ctm[3], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, ctm[4], ctm[5],
        0.0, 1.0,
    ]
}

fn text_compress(text: &CanvasTextOp) -> f32 {
    if text.compress.is_finite() && text.anchor_x.is_finite() {
        text.compress
    } else {
        1.0
    }
}

/// User space into the canvas content box: compress about `anchor_x`, then `ctm`.
fn text_user_matrix(compress: f32, anchor_x: f32, ctm: [f32; 6]) -> [f32; 6] {
    let shift = anchor_x * (1.0 - compress);
    let [a, b, c, d, e, f] = ctm;
    [
        a * compress,
        b * compress,
        c,
        d,
        a * shift + e,
        b * shift + f,
    ]
}

fn text_brush_matrix(text: &CanvasTextOp) -> Option<[f32; 16]> {
    let compress = text_compress(text);
    if compress == 1.0 && ctm_identity(text.ctm) {
        None
    } else {
        Some(ctm_matrix(text_user_matrix(
            compress,
            text.anchor_x,
            text.ctm,
        )))
    }
}

fn single_line_style(style: &TextStyle) -> TextStyle {
    let mut style = style.clone();
    style.soft_wrap = false;
    style.ellipsis = None;
    style.overflow = protocol::TextOverflow::Visible;
    style.max_lines = None;
    style.align = protocol::TextAlign::Left;
    style
}

fn positive_box(x: f32, y: f32, width: f32, height: f32) -> bool {
    x.is_finite()
        && y.is_finite()
        && width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0
}

fn ctm_finite(ctm: [f32; 6]) -> bool {
    ctm.iter().all(|value| value.is_finite())
}

fn ctm_identity(ctm: [f32; 6]) -> bool {
    ctm[0] == 1.0
        && ctm[1] == 0.0
        && ctm[2] == 0.0
        && ctm[3] == 1.0
        && ctm[4] == 0.0
        && ctm[5] == 0.0
}

fn map_point(ctm: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (
        ctm[0] * x + ctm[2] * y + ctm[4],
        ctm[1] * x + ctm[3] * y + ctm[5],
    )
}

fn normalize_axis(origin: f32, length: f32) -> (f32, f32) {
    if length < 0.0 {
        (origin + length, -length)
    } else {
        (origin, length)
    }
}

fn scale_u8(channel: u8, coverage: u8) -> u8 {
    let product = u16::from(channel) * u16::from(coverage) + 127;
    u8::try_from(product / 255).unwrap_or(u8::MAX)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> impeller::ImpellerRect {
    impeller::ImpellerRect {
        x,
        y,
        width,
        height,
    }
}

struct SampledRect {
    src: [f32; 4],
    dst: [f32; 4],
}

fn image_sample(
    src: Option<[f32; 4]>,
    dest_x: f32,
    dest_y: f32,
    dest_w: Option<f32>,
    dest_h: Option<f32>,
    tex_w: f32,
    tex_h: f32,
) -> Option<SampledRect> {
    if !(tex_w > 0.0 && tex_h > 0.0) {
        return None;
    }
    let (sx, sy, sw, sh) = match src {
        Some([x, y, w, h]) => (x, y, w, h),
        None => (0.0, 0.0, tex_w, tex_h),
    };
    if !(sw.is_finite() && sh.is_finite() && sw > 0.0 && sh > 0.0) {
        return None;
    }
    let ix0 = sx.max(0.0);
    let iy0 = sy.max(0.0);
    let ix1 = (sx + sw).min(tex_w);
    let iy1 = (sy + sh).min(tex_h);
    if !(ix1 > ix0 && iy1 > iy0) {
        return None;
    }
    let dw = dest_w.unwrap_or(sw);
    let dh = dest_h.unwrap_or(sh);
    if !(dw.is_finite() && dh.is_finite() && dw > 0.0 && dh > 0.0) {
        return None;
    }
    let left = (ix0 - sx) / sw;
    let top = (iy0 - sy) / sh;
    let right = (ix1 - sx) / sw;
    let bottom = (iy1 - sy) / sh;
    Some(SampledRect {
        src: [ix0, iy0, ix1 - ix0, iy1 - iy0],
        dst: [
            dest_x + left * dw,
            dest_y + top * dh,
            (right - left) * dw,
            (bottom - top) * dh,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::{ctm_identity, ctm_matrix, image_sample, map_point, scale_u8, text_user_matrix};

    #[test]
    fn coverage_scales_the_brush_alpha() {
        assert_eq!(scale_u8(255, 255), 255);
        assert_eq!(scale_u8(255, 0), 0);
        assert_eq!(scale_u8(255, 128), 128);
        assert_eq!(scale_u8(10, 255), 10);
    }

    #[test]
    fn clear_matrix_matches_the_user_formula() {
        let ctm = [0.0, 1.0, -1.0, 0.0, 10.0, 20.0];
        assert!(!ctm_identity(ctm));
        assert_eq!(map_point(ctm, 3.0, 4.0), (6.0, 23.0));
        let matrix = ctm_matrix([1.0, 0.0, 0.0, 1.0, 5.0, 6.0]);
        assert_eq!(matrix[0], 1.0);
        assert_eq!(matrix[5], 1.0);
        assert_eq!(matrix[12], 5.0);
        assert_eq!(matrix[13], 6.0);
        assert_eq!(matrix[15], 1.0);
    }

    #[test]
    fn a_source_rect_past_the_image_shrinks_the_dest() {
        let Some(sampled) = image_sample(
            Some([-10.0, 0.0, 20.0, 10.0]),
            0.0,
            0.0,
            Some(40.0),
            Some(10.0),
            10.0,
            10.0,
        ) else {
            panic!("intersection");
        };
        assert_eq!(sampled.src, [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(sampled.dst, [20.0, 0.0, 20.0, 10.0]);
        let Some(natural) = image_sample(None, 5.0, 6.0, None, None, 8.0, 4.0) else {
            panic!("whole image");
        };
        assert_eq!(natural.src, [0.0, 0.0, 8.0, 4.0]);
        assert_eq!(natural.dst, [5.0, 6.0, 8.0, 4.0]);
    }

    #[test]
    fn text_compress_happens_before_the_user_matrix() {
        let mapped = text_user_matrix(0.5, 4.0, [1.0, 0.0, 0.0, 1.0, 1.0, 2.0]);
        assert_eq!(map_point(mapped, 4.0, 0.0), (5.0, 2.0));
        assert_eq!(map_point(mapped, 0.0, 0.0), (3.0, 2.0));
    }
}
