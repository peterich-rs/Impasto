use super::ffi;
#[cfg(test)]
use super::paint_trace;
use super::*;
use protocol::{BlendMode, Command, Shadow, TextSpan, TextStyle};

impl<'a> crate::painter::Painter<'a> {
    pub(crate) fn play_commands<'pixels>(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        paint: ffi::ImpellerPaint,
        commands: &[Command],
        point_scale: f32,
        apply_scale: bool,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        let mut scaled = !apply_scale;
        for command in commands {
            let drawn = match command {
                Command::Fill { color } => {
                    paint_color(paint, *color);
                    unsafe { ffi::ImpellerDisplayListBuilderDrawPaint(builder, paint) };
                    true
                }
                Command::Rect {
                    x,
                    y,
                    w,
                    h,
                    corner_radius,
                    color,
                    stroke_width,
                    stroke_color,
                    gradient,
                    shadow,
                    box_shadows,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_rect(
                        builder,
                        paint,
                        *x,
                        *y,
                        *w,
                        *h,
                        *corner_radius,
                        *color,
                        *stroke_width,
                        *stroke_color,
                        gradient.as_ref(),
                        shadow.as_ref(),
                        box_shadows,
                        point_scale,
                    )
                }
                Command::Text {
                    x,
                    y,
                    max_width,
                    font_size,
                    paragraph,
                    color,
                    utf8,
                    style,
                    spans,
                    ..
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_text(
                        builder, paint, *x, *y, *max_width, *font_size, *paragraph, *color, utf8,
                        style, spans,
                    )
                }
                Command::Image {
                    x,
                    y,
                    w,
                    h,
                    source,
                    mode,
                    required,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_image(builder, *x, *y, *w, *h, source, *mode, *required, resolve)
                }
                Command::PushClip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_clip(builder, *x, *y, *w, *h, *corner_radius);
                    true
                }
                Command::PushOpacity { x, y, w, h, alpha } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_opacity(builder, *x, *y, *w, *h, *alpha)
                }
                Command::PushTransform {
                    translate_x,
                    translate_y,
                    rotate_rad,
                    scale_x,
                    scale_y,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_transform(
                        builder,
                        *translate_x,
                        *translate_y,
                        *rotate_rad,
                        *scale_x,
                        *scale_y,
                    );
                    true
                }
                Command::PushBlur { x, y, w, h, sigma } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_blur(builder, *x, *y, *w, *h, *sigma, false)
                }
                Command::PushBackdropBlur { x, y, w, h, sigma } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_blur(builder, *x, *y, *w, *h, *sigma, true)
                }
                Command::PushBlend { x, y, w, h, mode } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_blend(builder, *x, *y, *w, *h, *mode)
                }
                Command::Pop => self.pop_save(builder),
            };
            if !drawn {
                return false;
            }
        }
        true
    }

    pub(crate) fn draw_layers(
        &mut self,
        tree: &LayerTree,
        builder: ffi::ImpellerDisplayListBuilder,
        start: Option<LayerId>,
    ) -> bool {
        let mut current = start;
        while let Some(id) = current {
            let Some(layer) = tree.layers.get(id.0 as usize) else {
                return false;
            };
            let kind = layer.kind.clone();
            let first_child = layer.first_child;
            let next = layer.next;
            let drawn = match kind {
                LayerKind::Offset { x, y } => {
                    unsafe {
                        ffi::ImpellerDisplayListBuilderSave(builder);
                        ffi::ImpellerDisplayListBuilderTranslate(builder, x, y);
                    }
                    self.saves.push(Save::Transform);
                    let drawn = self.draw_layers(tree, builder, first_child);
                    self.pop_save(builder) && drawn
                }
                LayerKind::Transform {
                    translate_x,
                    translate_y,
                    rotate_rad,
                    scale_x,
                    scale_y,
                } => {
                    self.push_transform(
                        builder,
                        translate_x,
                        translate_y,
                        rotate_rad,
                        scale_x,
                        scale_y,
                    );
                    let drawn = self.draw_layers(tree, builder, first_child);
                    self.pop_save(builder) && drawn
                }
                LayerKind::Clip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius,
                } => {
                    self.push_clip(builder, x, y, w, h, corner_radius);
                    let drawn = self.draw_layers(tree, builder, first_child);
                    self.pop_save(builder) && drawn
                }
                LayerKind::Opacity { alpha, x, y, w, h } => {
                    if let Some(picture) = single_picture_child(tree, id) {
                        self.draw_picture(tree, builder, picture, alpha as f32 / 255.0)
                    } else if self.push_opacity(builder, x, y, w, h, alpha) {
                        let drawn = self.draw_layers(tree, builder, first_child);
                        self.pop_save(builder) && drawn
                    } else {
                        false
                    }
                }
                LayerKind::Blur { x, y, w, h, sigma } => {
                    if !self.push_blur(builder, x, y, w, h, sigma, false) {
                        false
                    } else {
                        let drawn = self.draw_layers(tree, builder, first_child);
                        self.pop_save(builder) && drawn
                    }
                }
                LayerKind::BackdropBlur { x, y, w, h, sigma } => {
                    if !self.push_blur(builder, x, y, w, h, sigma, true) {
                        false
                    } else {
                        let drawn = self.draw_layers(tree, builder, first_child);
                        self.pop_save(builder) && drawn
                    }
                }
                LayerKind::Blend { x, y, w, h, mode } => {
                    if !self.push_blend(builder, x, y, w, h, mode) {
                        false
                    } else {
                        let drawn = self.draw_layers(tree, builder, first_child);
                        self.pop_save(builder) && drawn
                    }
                }
                LayerKind::Picture { picture } => self.draw_picture(tree, builder, picture, 1.0),
            };
            if !drawn {
                return false;
            }
            current = next;
        }
        true
    }

    pub(crate) fn draw_picture(
        &mut self,
        tree: &LayerTree,
        builder: ffi::ImpellerDisplayListBuilder,
        id: PictureId,
        opacity: f32,
    ) -> bool {
        let Some(picture) = tree.picture(id) else {
            return false;
        };
        let origin = picture.origin;
        let Some(retained) = self.picture_cache.unwrap().get(&id) else {
            return false;
        };
        if retained.list.is_null()
            || !origin.0.is_finite()
            || !origin.1.is_finite()
            || !opacity.is_finite()
        {
            return false;
        }
        let list = retained.list;
        unsafe {
            ffi::ImpellerDisplayListBuilderSave(builder);
            ffi::ImpellerDisplayListBuilderTranslate(builder, origin.0, origin.1);
            ffi::ImpellerDisplayListBuilderDrawDisplayList(builder, list, opacity);
            ffi::ImpellerDisplayListBuilderRestore(builder);
        }
        true
    }

    pub(crate) fn draw_rect(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        _paint: ffi::ImpellerPaint,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
        color: [u8; 4],
        stroke_width: f32,
        stroke_color: [u8; 4],
        gradient: Option<&Gradient>,
        shadow: Option<&Shadow>,
        box_shadows: &[protocol::BoxShadow],
        point_scale: f32,
    ) -> bool {
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return false;
        }
        let stroke = stroke_width.is_finite() && stroke_width > 0.0;
        let fill = color[3] > 0 || gradient.is_some();
        for shadow in box_shadows {
            self.draw_box_shadow(builder, x, y, w, h, corner_radius, shadow);
        }
        if let Some(shadow) = shadow
            && shadow.elevation.is_finite()
            && shadow.elevation > 0.0
        {
            self.draw_shadow(
                builder,
                x,
                y,
                w,
                h,
                corner_radius,
                shadow,
                color[3] < 255,
                point_scale,
            );
        }
        let fill_color = gradient_solid(gradient).unwrap_or(color);
        if fill && stroke {
            if stroke_color == fill_color && gradient.is_none() {
                paint_color(paint, fill_color);
                self.set_stroke(paint, stroke_width);
                unsafe {
                    ffi::ImpellerPaintSetDrawStyle(
                        paint,
                        ffi::ImpellerDrawStyle::kImpellerDrawStyleStrokeAndFill,
                    );
                }
                self.fill_rect(builder, paint, x, y, w, h, corner_radius);
            } else {
                paint_color(paint, fill_color);
                unsafe {
                    ffi::ImpellerPaintSetDrawStyle(
                        paint,
                        ffi::ImpellerDrawStyle::kImpellerDrawStyleFill,
                    );
                }
                self.paint_gradient(paint, gradient, x, y, w, h);
                self.fill_rect(builder, paint, x, y, w, h, corner_radius);
                let stroke_paint = unsafe { ffi::ImpellerPaintNew() };
                if stroke_paint.is_null() {
                    unsafe { ffi::ImpellerPaintRelease(paint) };
                    return false;
                }
                paint_color(stroke_paint, stroke_color);
                self.set_stroke(stroke_paint, stroke_width);
                unsafe {
                    ffi::ImpellerPaintSetDrawStyle(
                        stroke_paint,
                        ffi::ImpellerDrawStyle::kImpellerDrawStyleStroke,
                    );
                }
                self.fill_rect(builder, stroke_paint, x, y, w, h, corner_radius);
                unsafe { ffi::ImpellerPaintRelease(stroke_paint) };
            }
        } else if stroke {
            paint_color(paint, stroke_color);
            self.set_stroke(paint, stroke_width);
            unsafe {
                ffi::ImpellerPaintSetDrawStyle(
                    paint,
                    ffi::ImpellerDrawStyle::kImpellerDrawStyleStroke,
                );
            }
            self.fill_rect(builder, paint, x, y, w, h, corner_radius);
        } else if fill {
            paint_color(paint, fill_color);
            unsafe {
                ffi::ImpellerPaintSetDrawStyle(
                    paint,
                    ffi::ImpellerDrawStyle::kImpellerDrawStyleFill,
                );
            }
            self.paint_gradient(paint, gradient, x, y, w, h);
            self.fill_rect(builder, paint, x, y, w, h, corner_radius);
        }
        unsafe { ffi::ImpellerPaintRelease(paint) };
        true
    }

    pub(crate) fn fill_rect(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        paint: ffi::ImpellerPaint,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    ) {
        let rect = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        let radius = clamped_radius(corner_radius, w, h);
        if radius > 0.0 {
            let radii = uniform_radii(radius);
            // SAFETY: `builder` and `paint` live until this submit releases them.
            unsafe {
                ffi::ImpellerDisplayListBuilderDrawRoundedRect(builder, &rect, &radii, paint);
            }
        } else {
            unsafe { ffi::ImpellerDisplayListBuilderDrawRect(builder, &rect, paint) };
        }
    }

    pub(crate) fn set_stroke(&self, paint: ffi::ImpellerPaint, width: f32) {
        #[cfg(test)]
        paint_trace::stroke(width);
        unsafe { ffi::ImpellerPaintSetStrokeWidth(paint, width) };
    }

    pub(crate) fn paint_gradient(
        &self,
        paint: ffi::ImpellerPaint,
        gradient: Option<&Gradient>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        let Some(source) = color_source(gradient, x, y, w, h) else {
            return;
        };
        unsafe {
            ffi::ImpellerPaintSetColorSource(paint, source);
            ffi::ImpellerColorSourceRelease(source);
        }
    }

    pub(crate) fn draw_box_shadow(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
        shadow: &protocol::BoxShadow,
    ) {
        if !shadow.blur_sigma.is_finite()
            || shadow.blur_sigma < 0.0
            || !shadow.spread.is_finite()
            || !shadow.offset[0].is_finite()
            || !shadow.offset[1].is_finite()
        {
            return;
        }
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return;
        }
        paint_color(paint, shadow.color);
        unsafe {
            ffi::ImpellerPaintSetDrawStyle(paint, ffi::ImpellerDrawStyle::kImpellerDrawStyleFill);
        }
        if shadow.blur_sigma > 0.0 {
            let filter = unsafe {
                ffi::ImpellerImageFilterCreateBlurNew(
                    shadow.blur_sigma,
                    shadow.blur_sigma,
                    ffi::ImpellerTileMode::kImpellerTileModeDecal,
                )
            };
            if !filter.is_null() {
                unsafe {
                    ffi::ImpellerPaintSetImageFilter(paint, filter);
                    ffi::ImpellerImageFilterRelease(filter);
                }
            }
        }
        let spread = shadow.spread;
        self.fill_rect(
            builder,
            paint,
            x + shadow.offset[0] - spread,
            y + shadow.offset[1] - spread,
            w + spread * 2.0,
            h + spread * 2.0,
            corner_radius + spread,
        );
        unsafe { ffi::ImpellerPaintRelease(paint) };
    }

    pub(crate) fn draw_shadow(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
        shadow: &Shadow,
        occluder_is_transparent: bool,
        point_scale: f32,
    ) {
        let path = rect_path(x, y, w, h, corner_radius);
        if path.is_null() {
            return;
        }
        let color = impeller_color(shadow.color);
        #[cfg(test)]
        paint_trace::shadow(shadow.elevation, point_scale);
        unsafe {
            ffi::ImpellerDisplayListBuilderDrawShadow(
                builder,
                path,
                &color,
                shadow.elevation,
                occluder_is_transparent,
                point_scale,
            );
            ffi::ImpellerPathRelease(path);
        }
    }

    pub(crate) fn push_clip(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    ) {
        let rect = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        let radius = clamped_radius(corner_radius, w, h);
        // SAFETY: `builder` lives until this submit releases it. Scale already ran, so the
        // saved transform includes `point_scale`. `rect` and `radii` are borrowed for the call.
        unsafe {
            ffi::ImpellerDisplayListBuilderSave(builder);
            if radius > 0.0 {
                let radii = uniform_radii(radius);
                ffi::ImpellerDisplayListBuilderClipRoundedRect(
                    builder,
                    &rect,
                    &radii,
                    ffi::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            } else {
                ffi::ImpellerDisplayListBuilderClipRect(
                    builder,
                    &rect,
                    ffi::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            }
        }
        self.saves.push(Save::Clip);
    }

    pub(crate) fn push_opacity(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: u8,
    ) -> bool {
        // SAFETY: a null paint means Impeller could not allocate one. The builder is unchanged.
        let layer_paint = unsafe { ffi::ImpellerPaintNew() };
        if layer_paint.is_null() {
            return false;
        }
        paint_color(layer_paint, [255, 255, 255, alpha]);
        let bounds = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        // SAFETY: `layer_paint` stays in `saves` until Restore reads it. The header applies
        // this paint when the layer is restored. `bounds` is borrowed for the call.
        unsafe {
            ffi::ImpellerDisplayListBuilderSaveLayer(
                builder,
                &bounds,
                layer_paint,
                ptr::null_mut(),
            );
        }
        self.saves.push(Save::Layer(layer_paint));
        true
    }

    pub(crate) fn push_transform(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    ) {
        let degrees = rotate_rad * (180.0 / std::f32::consts::PI);
        // SAFETY: `builder` lives until this submit releases it. Scale already ran, so the
        // saved transform includes `point_scale`. Translate, then clockwise rotation, then
        // scale, matches the command order. Do not scale first.
        unsafe {
            ffi::ImpellerDisplayListBuilderSave(builder);
            ffi::ImpellerDisplayListBuilderTranslate(builder, translate_x, translate_y);
            ffi::ImpellerDisplayListBuilderRotate(builder, degrees);
            ffi::ImpellerDisplayListBuilderScale(builder, scale_x, scale_y);
        }
        self.saves.push(Save::Transform);
    }

    pub(crate) fn push_blur(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
        backdrop: bool,
    ) -> bool {
        if !sigma.is_finite() || sigma <= 0.0 {
            return false;
        }
        #[cfg(test)]
        paint_trace::blur(sigma, sigma);
        let filter = unsafe {
            ffi::ImpellerImageFilterCreateBlurNew(
                sigma,
                sigma,
                ffi::ImpellerTileMode::kImpellerTileModeClamp,
            )
        };
        if filter.is_null() {
            return false;
        }
        let bounds = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        if backdrop {
            unsafe {
                ffi::ImpellerDisplayListBuilderSaveLayer(builder, &bounds, ptr::null_mut(), filter);
                ffi::ImpellerImageFilterRelease(filter);
            }
            self.saves.push(Save::Layer(ptr::null_mut()));
        } else {
            let layer_paint = unsafe { ffi::ImpellerPaintNew() };
            if layer_paint.is_null() {
                unsafe { ffi::ImpellerImageFilterRelease(filter) };
                return false;
            }
            unsafe {
                ffi::ImpellerPaintSetImageFilter(layer_paint, filter);
                ffi::ImpellerDisplayListBuilderSaveLayer(
                    builder,
                    &bounds,
                    layer_paint,
                    ptr::null_mut(),
                );
                ffi::ImpellerImageFilterRelease(filter);
            }
            self.saves.push(Save::Layer(layer_paint));
        }
        true
    }

    pub(crate) fn push_blend(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    ) -> bool {
        let blend = match mode {
            BlendMode::SrcOver => return false,
            BlendMode::Multiply => ffi::ImpellerBlendMode::kImpellerBlendModeMultiply,
            BlendMode::Screen => ffi::ImpellerBlendMode::kImpellerBlendModeScreen,
        };
        let layer_paint = unsafe { ffi::ImpellerPaintNew() };
        if layer_paint.is_null() {
            return false;
        }
        let bounds = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        unsafe {
            ffi::ImpellerPaintSetBlendMode(layer_paint, blend);
            ffi::ImpellerDisplayListBuilderSaveLayer(
                builder,
                &bounds,
                layer_paint,
                ptr::null_mut(),
            );
        }
        self.saves.push(Save::Layer(layer_paint));
        true
    }

    pub(crate) fn pop_save(&mut self, builder: ffi::ImpellerDisplayListBuilder) -> bool {
        if self.saves.is_empty() {
            return false;
        }
        match self.saves.pop() {
            Some(Save::Layer(layer_paint)) => {
                unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
                if !layer_paint.is_null() {
                    unsafe { ffi::ImpellerPaintRelease(layer_paint) };
                }
                true
            }
            Some(_) => {
                unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
                true
            }
            None => false,
        }
    }

    pub(crate) fn drain_saves(&mut self, builder: ffi::ImpellerDisplayListBuilder) {
        while let Some(save) = self.saves.pop() {
            match save {
                Save::Layer(layer_paint) => {
                    unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
                    if !layer_paint.is_null() {
                        unsafe { ffi::ImpellerPaintRelease(layer_paint) };
                    }
                }
                Save::Clip | Save::Transform => {
                    unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
                }
            }
        }
    }

    pub(crate) fn scale_once(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        point_scale: f32,
        scaled: bool,
    ) -> bool {
        if !scaled {
            unsafe { ffi::ImpellerDisplayListBuilderScale(builder, point_scale, point_scale) };
        }
        true
    }

    pub(crate) fn draw_text(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        _paint: ffi::ImpellerPaint,
        x: f32,
        y: f32,
        max_width: f32,
        font_size: f32,
        paragraph_id: protocol::ParagraphId,
        color: [u8; 4],
        utf8: &str,
        text_style: &TextStyle,
        spans: &[TextSpan],
    ) -> bool {
        let owned;
        let utf8 = if spans.is_empty() {
            utf8
        } else {
            owned = spans
                .iter()
                .map(|span| span.utf8.as_str())
                .collect::<String>();
            debug_assert_eq!(owned, utf8);
            &owned
        };
        if utf8.is_empty() {
            return false;
        }
        if !self.typography.stale()
            && let Some(paragraph) = self.typography.live_paint(paragraph_id)
        {
            let painted = self
                .typography
                .slot_color(paragraph_id.index)
                .unwrap_or(color);
            self.typography.store_text_probe(false, painted);
            let point = ffi::ImpellerPoint { x, y };
            // SAFETY: `paragraph` is the slot's live object. `release_paragraph` nulls
            // the slot before the pointer is freed, and this branch checks the generation.
            unsafe { ffi::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point) };
            return true;
        }
        let Ok(shaped) = self
            .typography
            .build_paragraph(utf8, font_size, max_width, color, text_style, spans)
        else {
            return false;
        };
        self.typography.store_text_probe(true, color);
        let point = ffi::ImpellerPoint { x, y };
        let paragraph = shaped.paragraph;
        // SAFETY: `paragraph` is owned by this engine. A matching stale slot takes it
        // over; otherwise it stays in `pending` until the display list that
        // references it is released.
        unsafe { ffi::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point) };
        let replaced = if self.typography.stale() {
            self.typography
                .replace_retained(paragraph_id, color, paragraph)
        } else {
            None
        };
        match replaced {
            Some(old) if !old.is_null() => self.typography.retire_paragraph(old),
            Some(_) => {}
            None => self.pending.push(paragraph),
        }
        true
    }
}
