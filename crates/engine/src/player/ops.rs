#[cfg(test)]
use super::paint_trace;
use super::*;
use protocol::{BlendMode, Command, Shadow, TextSpan, TextStyle};

impl<'a> crate::painter::Painter<'a> {
    pub(crate) fn play_commands<'pixels>(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
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
                    unsafe { impeller::ImpellerDisplayListBuilderDrawPaint(builder, paint) };
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
                Command::Canvas {
                    x,
                    y,
                    width,
                    height,
                    list,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.play_canvas(builder, paint, *x, *y, *width, *height, list, resolve)
                }
            };
            if !drawn {
                tracing::error!(
                    target: "impasto::render::present",
                    command = command_label(command),
                    "draw command failed"
                );
                return false;
            }
        }
        true
    }

    pub(crate) fn draw_layers(
        &mut self,
        tree: &LayerTree,
        builder: impeller::ImpellerDisplayListBuilder,
        start: Option<LayerId>,
    ) -> bool {
        self.draw_layers_filtered(tree, builder, start, None)
    }

    pub(crate) fn draw_layers_filtered(
        &mut self,
        tree: &LayerTree,
        builder: impeller::ImpellerDisplayListBuilder,
        start: Option<LayerId>,
        included: Option<&std::collections::HashSet<LayerId>>,
    ) -> bool {
        let mut current = start;
        while let Some(id) = current {
            let Some(layer) = tree.layers.get(id.0 as usize) else {
                return false;
            };
            let kind = layer.kind.clone();
            let first_child = layer.first_child;
            let next = layer.next;
            if included.is_some_and(|set| !set.contains(&id)) {
                current = next;
                continue;
            }
            let drawn = match kind {
                LayerKind::PlatformView { .. } => true,
                LayerKind::Offset { x, y } => {
                    unsafe {
                        impeller::ImpellerDisplayListBuilderSave(builder);
                        impeller::ImpellerDisplayListBuilderTranslate(builder, x, y);
                    }
                    self.saves.push(Save::Transform);
                    let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
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
                    let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
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
                    let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
                    self.pop_save(builder) && drawn
                }
                LayerKind::Opacity { alpha, x, y, w, h } => {
                    if let Some(picture) = single_picture_child(tree, id) {
                        self.draw_picture(tree, builder, picture, alpha as f32 / 255.0)
                    } else if self.push_opacity(builder, x, y, w, h, alpha) {
                        let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
                        self.pop_save(builder) && drawn
                    } else {
                        false
                    }
                }
                LayerKind::Blur { x, y, w, h, sigma } => {
                    if !self.push_blur(builder, x, y, w, h, sigma, false) {
                        false
                    } else {
                        let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
                        self.pop_save(builder) && drawn
                    }
                }
                LayerKind::BackdropBlur { x, y, w, h, sigma } => {
                    if !self.push_blur(builder, x, y, w, h, sigma, true) {
                        false
                    } else {
                        let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
                        self.pop_save(builder) && drawn
                    }
                }
                LayerKind::Blend { x, y, w, h, mode } => {
                    if !self.push_blend(builder, x, y, w, h, mode) {
                        false
                    } else {
                        let drawn = self.draw_layers_filtered(tree, builder, first_child, included);
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
        builder: impeller::ImpellerDisplayListBuilder,
        id: PictureId,
        opacity: f32,
    ) -> bool {
        let Some(picture) = tree.picture(id) else {
            tracing::error!(
                target: "impasto::render::present",
                "picture missing"
            );
            return false;
        };
        let origin = picture.origin;
        let Some(cache) = self.picture_cache else {
            tracing::error!(target: "impasto::render::present", "picture cache missing");
            return false;
        };
        let Some(retained) = cache.get(&id) else {
            tracing::error!(target: "impasto::render::present", "picture not retained");
            return false;
        };
        if retained.list.is_null()
            || !origin.0.is_finite()
            || !origin.1.is_finite()
            || !opacity.is_finite()
        {
            tracing::error!(
                target: "impasto::render::present",
                null_list = retained.list.is_null(),
                x = origin.0,
                y = origin.1,
                opacity,
                "picture not drawable"
            );
            return false;
        }
        let list = retained.list;
        unsafe {
            impeller::ImpellerDisplayListBuilderSave(builder);
            impeller::ImpellerDisplayListBuilderTranslate(builder, origin.0, origin.1);
            impeller::ImpellerDisplayListBuilderDrawDisplayList(builder, list, opacity);
            impeller::ImpellerDisplayListBuilderRestore(builder);
        }
        true
    }

    pub(crate) fn draw_rect(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
        _paint: impeller::ImpellerPaint,
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
        let paint = unsafe { impeller::ImpellerPaintNew() };
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
                    impeller::ImpellerPaintSetDrawStyle(
                        paint,
                        impeller::ImpellerDrawStyle::kImpellerDrawStyleStrokeAndFill,
                    );
                }
                self.fill_rect(builder, paint, x, y, w, h, corner_radius);
            } else {
                paint_color(paint, fill_color);
                unsafe {
                    impeller::ImpellerPaintSetDrawStyle(
                        paint,
                        impeller::ImpellerDrawStyle::kImpellerDrawStyleFill,
                    );
                }
                self.paint_gradient(paint, gradient, x, y, w, h);
                self.fill_rect(builder, paint, x, y, w, h, corner_radius);
                let stroke_paint = unsafe { impeller::ImpellerPaintNew() };
                if stroke_paint.is_null() {
                    unsafe { impeller::ImpellerPaintRelease(paint) };
                    return false;
                }
                paint_color(stroke_paint, stroke_color);
                self.set_stroke(stroke_paint, stroke_width);
                unsafe {
                    impeller::ImpellerPaintSetDrawStyle(
                        stroke_paint,
                        impeller::ImpellerDrawStyle::kImpellerDrawStyleStroke,
                    );
                }
                self.fill_rect(builder, stroke_paint, x, y, w, h, corner_radius);
                unsafe { impeller::ImpellerPaintRelease(stroke_paint) };
            }
        } else if stroke {
            paint_color(paint, stroke_color);
            self.set_stroke(paint, stroke_width);
            unsafe {
                impeller::ImpellerPaintSetDrawStyle(
                    paint,
                    impeller::ImpellerDrawStyle::kImpellerDrawStyleStroke,
                );
            }
            self.fill_rect(builder, paint, x, y, w, h, corner_radius);
        } else if fill {
            paint_color(paint, fill_color);
            unsafe {
                impeller::ImpellerPaintSetDrawStyle(
                    paint,
                    impeller::ImpellerDrawStyle::kImpellerDrawStyleFill,
                );
            }
            self.paint_gradient(paint, gradient, x, y, w, h);
            self.fill_rect(builder, paint, x, y, w, h, corner_radius);
        }
        unsafe { impeller::ImpellerPaintRelease(paint) };
        true
    }

    pub(crate) fn fill_rect(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
        paint: impeller::ImpellerPaint,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    ) {
        let rect = impeller::ImpellerRect {
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
                impeller::ImpellerDisplayListBuilderDrawRoundedRect(builder, &rect, &radii, paint);
            }
        } else {
            unsafe { impeller::ImpellerDisplayListBuilderDrawRect(builder, &rect, paint) };
        }
    }

    pub(crate) fn set_stroke(&self, paint: impeller::ImpellerPaint, width: f32) {
        #[cfg(test)]
        paint_trace::stroke(width);
        unsafe { impeller::ImpellerPaintSetStrokeWidth(paint, width) };
    }

    pub(crate) fn paint_gradient(
        &self,
        paint: impeller::ImpellerPaint,
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
            impeller::ImpellerPaintSetColorSource(paint, source);
            impeller::ImpellerColorSourceRelease(source);
        }
    }

    pub(crate) fn draw_box_shadow(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
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
        let paint = unsafe { impeller::ImpellerPaintNew() };
        if paint.is_null() {
            return;
        }
        paint_color(paint, shadow.color);
        unsafe {
            impeller::ImpellerPaintSetDrawStyle(
                paint,
                impeller::ImpellerDrawStyle::kImpellerDrawStyleFill,
            );
        }
        if shadow.blur_sigma > 0.0 {
            let filter = unsafe {
                impeller::ImpellerImageFilterCreateBlurNew(
                    shadow.blur_sigma,
                    shadow.blur_sigma,
                    impeller::ImpellerTileMode::kImpellerTileModeDecal,
                )
            };
            if !filter.is_null() {
                unsafe {
                    impeller::ImpellerPaintSetImageFilter(paint, filter);
                    impeller::ImpellerImageFilterRelease(filter);
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
        unsafe { impeller::ImpellerPaintRelease(paint) };
    }

    pub(crate) fn draw_shadow(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
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
            impeller::ImpellerDisplayListBuilderDrawShadow(
                builder,
                path,
                &color,
                shadow.elevation,
                occluder_is_transparent,
                point_scale,
            );
            impeller::ImpellerPathRelease(path);
        }
    }

    pub(crate) fn push_clip(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    ) {
        let rect = impeller::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        let radius = clamped_radius(corner_radius, w, h);
        // SAFETY: `builder` lives until this submit releases it. Scale already ran, so the
        // saved transform includes `point_scale`. `rect` and `radii` are borrowed for the call.
        unsafe {
            impeller::ImpellerDisplayListBuilderSave(builder);
            if radius > 0.0 {
                let radii = uniform_radii(radius);
                impeller::ImpellerDisplayListBuilderClipRoundedRect(
                    builder,
                    &rect,
                    &radii,
                    impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            } else {
                impeller::ImpellerDisplayListBuilderClipRect(
                    builder,
                    &rect,
                    impeller::ImpellerClipOperation::kImpellerClipOperationIntersect,
                );
            }
        }
        self.saves.push(Save::Clip);
    }

    pub(crate) fn push_opacity(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: u8,
    ) -> bool {
        // SAFETY: a null paint means Impeller could not allocate one. The builder is unchanged.
        let layer_paint = unsafe { impeller::ImpellerPaintNew() };
        if layer_paint.is_null() {
            return false;
        }
        paint_color(layer_paint, [255, 255, 255, alpha]);
        let bounds = impeller::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        // SAFETY: `layer_paint` stays in `saves` until Restore reads it. The header applies
        // this paint when the layer is restored. `bounds` is borrowed for the call.
        unsafe {
            impeller::ImpellerDisplayListBuilderSaveLayer(
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
        builder: impeller::ImpellerDisplayListBuilder,
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
            impeller::ImpellerDisplayListBuilderSave(builder);
            impeller::ImpellerDisplayListBuilderTranslate(builder, translate_x, translate_y);
            impeller::ImpellerDisplayListBuilderRotate(builder, degrees);
            impeller::ImpellerDisplayListBuilderScale(builder, scale_x, scale_y);
        }
        self.saves.push(Save::Transform);
    }

    pub(crate) fn push_blur(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
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
            impeller::ImpellerImageFilterCreateBlurNew(
                sigma,
                sigma,
                impeller::ImpellerTileMode::kImpellerTileModeClamp,
            )
        };
        if filter.is_null() {
            return false;
        }
        let bounds = impeller::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        if backdrop {
            unsafe {
                impeller::ImpellerDisplayListBuilderSaveLayer(
                    builder,
                    &bounds,
                    ptr::null_mut(),
                    filter,
                );
                impeller::ImpellerImageFilterRelease(filter);
            }
            self.saves.push(Save::Layer(ptr::null_mut()));
        } else {
            let layer_paint = unsafe { impeller::ImpellerPaintNew() };
            if layer_paint.is_null() {
                unsafe { impeller::ImpellerImageFilterRelease(filter) };
                return false;
            }
            unsafe {
                impeller::ImpellerPaintSetImageFilter(layer_paint, filter);
                impeller::ImpellerDisplayListBuilderSaveLayer(
                    builder,
                    &bounds,
                    layer_paint,
                    ptr::null_mut(),
                );
                impeller::ImpellerImageFilterRelease(filter);
            }
            self.saves.push(Save::Layer(layer_paint));
        }
        true
    }

    pub(crate) fn push_blend(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    ) -> bool {
        let blend = match mode {
            // SrcOver is the default composite. Recording omits it. If a command
            // still arrives, balance Pop without a GPU save and without failing the frame.
            BlendMode::SrcOver => {
                self.saves.push(Save::Skipped);
                return true;
            }
            BlendMode::Multiply => impeller::ImpellerBlendMode::kImpellerBlendModeMultiply,
            BlendMode::Screen => impeller::ImpellerBlendMode::kImpellerBlendModeScreen,
        };
        let layer_paint = unsafe { impeller::ImpellerPaintNew() };
        if layer_paint.is_null() {
            return false;
        }
        let bounds = impeller::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
        };
        unsafe {
            impeller::ImpellerPaintSetBlendMode(layer_paint, blend);
            impeller::ImpellerDisplayListBuilderSaveLayer(
                builder,
                &bounds,
                layer_paint,
                ptr::null_mut(),
            );
        }
        self.saves.push(Save::Layer(layer_paint));
        true
    }

    pub(crate) fn pop_save(&mut self, builder: impeller::ImpellerDisplayListBuilder) -> bool {
        if self.saves.is_empty() {
            return false;
        }
        match self.saves.pop() {
            Some(Save::Layer(layer_paint)) => {
                unsafe { impeller::ImpellerDisplayListBuilderRestore(builder) };
                if !layer_paint.is_null() {
                    unsafe { impeller::ImpellerPaintRelease(layer_paint) };
                }
                true
            }
            Some(Save::Skipped) => true,
            Some(Save::Clip | Save::Transform) => {
                unsafe { impeller::ImpellerDisplayListBuilderRestore(builder) };
                true
            }
            None => false,
        }
    }

    pub(crate) fn drain_saves(&mut self, builder: impeller::ImpellerDisplayListBuilder) {
        while let Some(save) = self.saves.pop() {
            match save {
                Save::Layer(layer_paint) => {
                    unsafe { impeller::ImpellerDisplayListBuilderRestore(builder) };
                    if !layer_paint.is_null() {
                        unsafe { impeller::ImpellerPaintRelease(layer_paint) };
                    }
                }
                Save::Clip | Save::Transform => {
                    unsafe { impeller::ImpellerDisplayListBuilderRestore(builder) };
                }
                Save::Skipped => {}
            }
        }
    }

    pub(crate) fn scale_once(
        &self,
        builder: impeller::ImpellerDisplayListBuilder,
        point_scale: f32,
        scaled: bool,
    ) -> bool {
        if !scaled {
            unsafe { impeller::ImpellerDisplayListBuilderScale(builder, point_scale, point_scale) };
        }
        true
    }

    pub(crate) fn draw_text(
        &mut self,
        builder: impeller::ImpellerDisplayListBuilder,
        _paint: impeller::ImpellerPaint,
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
        // A collapsed soft-wrapped run has no line to draw. Failing the whole
        // frame hides the page that contains it, such as a closed dialog.
        // `!(max_width > 0)` also covers NaN, which has no line either.
        let collapsed = max_width
            .partial_cmp(&0.0)
            .is_none_or(|order| order != std::cmp::Ordering::Greater);
        if text_style.soft_wrap && collapsed {
            return true;
        }
        if !self.typography.stale()
            && let Some(paragraph) = self.typography.live_paint(paragraph_id)
        {
            let painted = self
                .typography
                .slot_color(paragraph_id.index)
                .unwrap_or(color);
            self.typography.store_text_probe(false, painted);
            let point = impeller::ImpellerPoint { x, y };
            // SAFETY: `paragraph` is the slot's live object. `release_paragraph` nulls
            // the slot before the pointer is freed, and this branch checks the generation.
            unsafe {
                impeller::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point)
            };
            return true;
        }
        let Ok((paragraph, slot)) = self
            .typography
            .acquire_draw(utf8, font_size, max_width, color, text_style, spans)
        else {
            let preview: String = utf8.chars().take(32).collect();
            tracing::error!(
                target: "impasto::render::present",
                font_size,
                max_width,
                text = %preview,
                "text shape failed"
            );
            return false;
        };
        self.typography.store_text_probe(true, color);
        let point = impeller::ImpellerPoint { x, y };
        // SAFETY: `paragraph` is an extra retain of the cache slot. `release_draw`
        // drops the cache's live count when this display list is released.
        unsafe { impeller::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point) };
        self.pending.push(HeldParagraph { paragraph, slot });
        true
    }
}

fn command_label(command: &Command) -> &'static str {
    match command {
        Command::Fill { .. } => "fill",
        Command::Rect { .. } => "rect",
        Command::Text { .. } => "text",
        Command::Image { .. } => "image",
        Command::PushClip { .. } => "clip",
        Command::PushOpacity { .. } => "opacity",
        Command::PushTransform { .. } => "transform",
        Command::PushBlur { .. } => "blur",
        Command::PushBackdropBlur { .. } => "backdrop",
        Command::PushBlend { .. } => "blend",
        Command::Pop => "pop",
        Command::Canvas { .. } => "canvas",
    }
}
