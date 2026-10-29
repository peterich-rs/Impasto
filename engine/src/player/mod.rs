//! Paints a layer tree. Text uses the font size and paragraph width stored on each command.

use std::ffi::{CString, c_void};
use std::marker::PhantomData;
use std::ptr;

use protocol::{
    ColorStop, ContentMode, Gradient, LayerId, LayerKind, LayerTree, PictureId, TextAlign,
};

use crate::{ImagePixels, Status};

pub(crate) struct RetainedPicture {
    pub(crate) generation: u64,
    pub(crate) list: ffi::ImpellerDisplayList,
    /// Paragraphs this display list draws. Released only with the display list,
    /// after any root list that flattened it.
    pub(crate) held: Vec<ffi::ImpellerParagraph>,
}

pub(crate) enum Save {
    Clip,
    Transform,
    /// Paint read by `Restore`. Released after that restore.
    Layer(ffi::ImpellerPaint),
}

pub struct Engine {
    context: ffi::ImpellerContext,
    list: ffi::ImpellerDisplayList,
    /// Temporaries the installed root display list still references.
    held: Vec<ffi::ImpellerParagraph>,
    pictures: crate::pictures::PictureCache,
    textures: crate::textures::TextureStore,
    typography: crate::typography::TypographyHandle,
    last_submit: crate::SubmitStats,
    /// `dlopen` handle for `libvulkan.so`. Null until [`Engine::create_vulkan`].
    #[cfg(target_os = "android")]
    vulkan_lib: *mut c_void,
    /// Impeller objects are created and used on the render thread.
    render_thread: PhantomData<*mut ()>,
}

#[cfg(target_os = "android")]
#[path = "../vulkan.rs"]
mod vulkan;

#[cfg(target_os = "android")]
pub use vulkan::VulkanSwapchain;
#[cfg(target_os = "android")]
pub use vulkan::note_swapchain_extent;

pub(crate) mod ffi;
mod images;
mod ops;

impl Engine {
    #[cfg(not(target_os = "android"))]
    pub fn create_metal() -> Result<Self, Status> {
        // SAFETY: ImpellerContextCreateMetalNew and ImpellerTypographyContextNew are the
        // documented constructors. A null handle means this SDK cannot create Metal.
        // Release is a no-op on null, per impeller.h.
        let (context, typography) = unsafe {
            let context = ffi::ImpellerContextCreateMetalNew(ffi::IMPELLER_VERSION);
            let typography = ffi::ImpellerTypographyContextNew();
            if context.is_null() || typography.is_null() {
                ffi::ImpellerTypographyContextRelease(typography);
                ffi::ImpellerContextRelease(context);
                return Err(Status::NotLinked);
            }
            (context, typography)
        };
        Ok(Self::blank(context, typography))
    }

    pub(crate) fn blank(
        context: ffi::ImpellerContext,
        typography: ffi::ImpellerTypographyContext,
    ) -> Self {
        Self {
            context,
            list: ptr::null_mut(),
            held: Vec::new(),
            pictures: crate::pictures::PictureCache::new(),
            textures: crate::textures::TextureStore::new(),
            typography: std::mem::ManuallyDrop::new(crate::typography::Typography::new(typography)),
            last_submit: crate::SubmitStats::default(),
            #[cfg(target_os = "android")]
            vulkan_lib: ptr::null_mut(),
            render_thread: PhantomData,
        }
    }

    /// Shared with the shaper. The engine keeps one clone so the context outlives display lists.
    pub fn typography(&self) -> std::rc::Rc<std::cell::RefCell<crate::typography::Typography>> {
        std::rc::Rc::clone(&self.typography)
    }

    pub fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        let installed = !self.list.is_null();
        let mut typography = self.typography.borrow_mut();
        typography.register_face(data, family)?;
        if installed {
            typography.mark_stale();
        } else {
            typography.discard_measure_cache();
        }
        Ok(())
    }

    /// Composite `tree`. Pictures are cached by [`PictureId`] and generation.
    ///
    /// `point_scale` is applied once on the root builder. A cache hit still scans
    /// commands to mark textures used. Child display lists stay until the picture
    /// leaves the tree, and only after the root list that flattened them is released.
    pub fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        if point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        let typography = std::rc::Rc::clone(&self.typography);
        let mut typography = typography.borrow_mut();
        typography.begin_submit();
        self.last_submit = crate::SubmitStats::default();
        let mut swap = crate::pictures::PictureSwap::default();
        let mut pending = Vec::new();
        if let Err(status) = self.pictures.prepare(
            tree,
            point_scale,
            &mut resolve,
            &mut swap,
            &mut typography,
            &mut self.textures,
            &mut pending,
            &mut self.last_submit,
            self.context,
        ) {
            self.last_submit = crate::SubmitStats::default();
            self.pictures.rollback(swap);
            crate::painter::release_pending(&mut pending);
            return Err(status);
        }
        let builder = unsafe { ffi::ImpellerDisplayListBuilderNew(ptr::null()) };
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if builder.is_null() || paint.is_null() {
            unsafe {
                ffi::ImpellerDisplayListBuilderRelease(builder);
                ffi::ImpellerPaintRelease(paint);
            }
            self.pictures.rollback(swap);
            crate::painter::release_pending(&mut pending);
            return Err(Status::BadScene);
        }
        let display = {
            let mut painter = crate::painter::Painter {
                typography: &mut typography,
                textures: &mut self.textures,
                picture_cache: Some(&self.pictures),
                pending: &mut pending,
                saves: Vec::new(),
                impeller: self.context,
            };
            if !tree.layers.is_empty() {
                painter.scale_once(builder, point_scale, false);
            }
            let drawn =
                tree.layers.is_empty() || painter.draw_layers(tree, builder, Some(tree.root));
            let failed = !drawn || !painter.saves.is_empty();
            let display = if failed {
                ptr::null_mut()
            } else {
                unsafe { ffi::ImpellerDisplayListBuilderCreateDisplayListNew(builder) }
            };
            painter.drain_saves(builder);
            display
        };
        unsafe {
            ffi::ImpellerDisplayListBuilderRelease(builder);
            ffi::ImpellerPaintRelease(paint);
        }
        if display.is_null() {
            self.last_submit = crate::SubmitStats::default();
            crate::painter::release_pending(&mut pending);
            self.pictures.rollback(swap);
            return Err(Status::BadScene);
        }
        self.release_display();
        typography.set_list_installed(false);
        for (_, picture) in swap.replaced {
            release_retained(picture);
        }
        self.pictures.evict_except(tree);
        typography.reap_retired();
        self.release_held();
        self.held = std::mem::take(&mut pending);
        typography.clear_stale();
        typography.set_list_installed(true);
        self.list = display;
        typography.evict();
        typography.evict_over_cap();
        let generation = typography.generation();
        self.textures.evict(generation);
        self.textures.evict_over_cap(generation);
        self.last_submit.layer_count = u32::try_from(tree.layers.len()).unwrap_or(u32::MAX);
        typography.note_paragraph_limit();
        self.textures.note_limit();
        self.pictures.note_limit();
        Ok(())
    }

    /// Pictures built since the engine was created. The cache key is picture id and generation.
    pub fn picture_builds(&self) -> u64 {
        self.pictures.builds()
    }

    /// Paragraph and texture entries retained after the last submit.
    pub fn cache_counts(&self) -> (usize, usize) {
        (self.typography.borrow().measured_len(), self.textures.len())
    }

    /// Counts from the last successful [`Engine::submit_layers`].
    pub fn submit_stats(&self) -> crate::SubmitStats {
        self.last_submit
    }

    #[cfg(not(target_os = "android"))]
    pub fn draw_metal(&mut self, metal_drawable: *mut c_void) -> Result<(), Status> {
        if self.context.is_null() || self.list.is_null() {
            return Err(Status::NotReady);
        }
        if metal_drawable.is_null() {
            return Err(Status::PresentFailed);
        }
        // SAFETY: `metal_drawable` is a CAMetalDrawable from the system default device,
        // kept alive by the caller for this call. The context uses that same device.
        let surface = unsafe {
            ffi::ImpellerSurfaceCreateWrappedMetalDrawableNew(self.context, metal_drawable)
        };
        if surface.is_null() {
            return Err(Status::PresentFailed);
        }
        let presented = unsafe {
            let drew = ffi::ImpellerSurfaceDrawDisplayList(surface, self.list);
            let presented = drew && ffi::ImpellerSurfacePresent(surface);
            ffi::ImpellerSurfaceRelease(surface);
            presented
        };
        if presented {
            Ok(())
        } else {
            Err(Status::PresentFailed)
        }
    }

    pub(crate) fn release_display(&mut self) {
        // SAFETY: `list` is null or the display list from the previous submit.
        unsafe {
            ffi::ImpellerDisplayListRelease(self.list);
            self.list = ptr::null_mut();
        }
    }

    pub(crate) fn release_held(&mut self) {
        for paragraph in self.held.drain(..) {
            // SAFETY: the display list that referenced these temporaries was just released.
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        }
    }

    pub(crate) fn release_frame(&mut self) {
        self.release_display();
        self.pictures.release_all();
        self.release_held();
        self.textures.release_all();
        let typography = std::rc::Rc::clone(&self.typography);
        let mut typography = typography.borrow_mut();
        typography.set_list_installed(false);
        typography.release_all_paragraphs();
    }

    /// Test helper. Each call uses a new picture generation so the commands are rebuilt.
    #[cfg(test)]
    pub(crate) fn paint<'pixels>(
        &mut self,
        list: &protocol::DrawList,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = PictureId(1);
        let picture = std::sync::Arc::new(protocol::Picture {
            id,
            generation,
            origin: (0.0, 0.0),
            commands: std::sync::Arc::from(list.commands.as_slice()),
        });
        let tree = LayerTree {
            layers: vec![protocol::Layer {
                kind: LayerKind::Picture { picture: id },
                parent: None,
                first_child: None,
                next: None,
            }],
            pictures: vec![picture],
            root: LayerId(0),
            epoch: generation,
        };
        self.submit_layers(&tree, point_scale, resolve)
    }

    #[cfg(test)]
    pub(crate) fn cached_textures(&self) -> usize {
        self.textures.len()
    }

    #[cfg(test)]
    pub(crate) fn cached_paragraphs(&self) -> usize {
        self.typography.borrow().measured_len()
    }

    #[cfg(test)]
    pub(crate) fn retained_pictures(&self) -> usize {
        self.pictures.len()
    }
}

pub(crate) fn release_retained(picture: RetainedPicture) {
    unsafe { ffi::ImpellerDisplayListRelease(picture.list) };
    for paragraph in picture.held {
        unsafe { ffi::ImpellerParagraphRelease(paragraph) };
    }
}

pub(crate) fn single_picture_child(tree: &LayerTree, id: LayerId) -> Option<PictureId> {
    let layer = tree.layers.get(id.0 as usize)?;
    let child = tree.layers.get(layer.first_child?.0 as usize)?;
    if child.next.is_some() {
        return None;
    }
    match child.kind {
        LayerKind::Picture { picture } => Some(picture),
        _ => None,
    }
}

impl crate::Surface for Engine {
    fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        Engine::register_font(self, data, family)
    }

    fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        Engine::submit_layers(self, tree, point_scale, resolve)
    }

    #[cfg(not(target_os = "android"))]
    fn draw_metal(&mut self, metal_drawable: *mut c_void) -> Result<(), Status> {
        Engine::draw_metal(self, metal_drawable)
    }

    fn picture_builds(&self) -> u64 {
        Engine::picture_builds(self)
    }

    fn cache_counts(&self) -> (usize, usize) {
        Engine::cache_counts(self)
    }

    fn submit_stats(&self) -> crate::SubmitStats {
        Engine::submit_stats(self)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.release_frame();
        // Drop the typography context (and face bytes) before the Impeller context when this
        // engine holds the last `Rc`. A shaper clone keeps the context until it drops.
        // SAFETY: `context` came from the context constructor. Release is a no-op on null.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.typography);
            ffi::ImpellerContextRelease(self.context);
        }
        #[cfg(target_os = "android")]
        if !self.vulkan_lib.is_null() {
            vulkan::close_library(self.vulkan_lib);
        }
    }
}

pub(crate) fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// SAFETY: `paragraph` is a live Impeller paragraph.
/// `ImpellerParagraphGetLineMetrics` returns the paragraph's cached metrics
/// without a new reference (`GetC`, not `Leak`). The paragraph releases that
/// object. Releasing it here frees the cache, and the next query frees the
/// same allocation.
pub(crate) unsafe fn read_lines(
    paragraph: ffi::ImpellerParagraph,
) -> Result<Vec<crate::LineMetric>, Status> {
    let metrics = unsafe { ffi::ImpellerParagraphGetLineMetrics(paragraph) };
    if metrics.is_null() {
        return Ok(Vec::new());
    }
    let count = unsafe { ffi::ImpellerParagraphGetLineCount(paragraph) } as usize;
    let mut lines = Vec::with_capacity(count);
    for line in 0..count {
        let baseline = unsafe { ffi::ImpellerLineMetricsGetBaseline(metrics, line) };
        let ascent = unsafe { ffi::ImpellerLineMetricsGetAscent(metrics, line) };
        let height = unsafe { ffi::ImpellerLineMetricsGetHeight(metrics, line) };
        let y = (baseline - ascent) as f32;
        lines.push(crate::LineMetric {
            start: unsafe { ffi::ImpellerLineMetricsGetCodeUnitStartIndex(metrics, line) } as u32,
            end: unsafe { ffi::ImpellerLineMetricsGetCodeUnitEndIndex(metrics, line) } as u32,
            end_with_break: unsafe {
                ffi::ImpellerLineMetricsGetCodeUnitEndIndexIncludingNewline(metrics, line)
            } as u32,
            x: unsafe { ffi::ImpellerLineMetricsGetLeft(metrics, line) } as f32,
            y,
            w: unsafe { ffi::ImpellerLineMetricsGetWidth(metrics, line) } as f32,
            h: height as f32,
            baseline: baseline as f32,
            hard_break: unsafe { ffi::ImpellerLineMetricsIsHardbreak(metrics, line) },
        });
    }
    Ok(lines)
}

pub(crate) fn apply_decoration(
    style: ffi::ImpellerParagraphStyle,
    color: [u8; 4],
    underline: bool,
    line_through: bool,
) {
    let mut types = 0i32;
    if underline {
        types |= ffi::ImpellerTextDecorationType::kImpellerTextDecorationTypeUnderline as i32;
    }
    if line_through {
        types |= ffi::ImpellerTextDecorationType::kImpellerTextDecorationTypeLineThrough as i32;
    }
    if types == 0 {
        return;
    }
    #[cfg(test)]
    paint_trace::decoration(types);
    let decoration = ffi::ImpellerTextDecoration {
        types,
        color: impeller_color(color),
        style: ffi::ImpellerTextDecorationStyle::kImpellerTextDecorationStyleSolid,
        thickness_multiplier: 1.0,
    };
    unsafe { ffi::ImpellerParagraphStyleSetTextDecoration(style, &decoration) };
}

pub(crate) unsafe fn push_runs(
    style: ffi::ImpellerParagraphStyle,
    paragraph_builder: ffi::ImpellerParagraphBuilder,
    family: *const std::ffi::c_char,
    cjk: Option<&CString>,
    utf8: &str,
) {
    let runs = if cjk.is_some() {
        crate::split_scripts(utf8)
    } else {
        vec![(utf8, false)]
    };
    for (slice, use_cjk) in runs {
        if slice.is_empty() {
            continue;
        }
        let run_family = if use_cjk {
            cjk.map(|name| name.as_ptr()).unwrap_or(family)
        } else {
            family
        };
        unsafe {
            ffi::ImpellerParagraphStyleSetFontFamily(style, run_family);
            let slice_len = u32::try_from(slice.len()).unwrap_or(0);
            ffi::ImpellerParagraphBuilderPushStyle(paragraph_builder, style);
            ffi::ImpellerParagraphBuilderAddText(paragraph_builder, slice.as_ptr(), slice_len);
            ffi::ImpellerParagraphBuilderPopStyle(paragraph_builder);
        }
    }
}

pub(crate) fn paint_color(paint: ffi::ImpellerPaint, color: [u8; 4]) {
    let color = impeller_color(color);
    unsafe { ffi::ImpellerPaintSetColor(paint, &color) };
}

pub(crate) fn impeller_color(color: [u8; 4]) -> ffi::ImpellerColor {
    ffi::ImpellerColor {
        red: color[0] as f32 / 255.0,
        green: color[1] as f32 / 255.0,
        blue: color[2] as f32 / 255.0,
        alpha: color[3] as f32 / 255.0,
        color_space: ffi::ImpellerColorSpace::kImpellerColorSpaceSRGB,
    }
}

pub(crate) fn content_rects(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tex_w: f32,
    tex_h: f32,
    mode: ContentMode,
) -> Option<(ffi::ImpellerRect, ffi::ImpellerRect)> {
    match mode {
        ContentMode::Fill => Some((
            ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: tex_w,
                height: tex_h,
            },
            ffi::ImpellerRect {
                x,
                y,
                width: w,
                height: h,
            },
        )),
        ContentMode::Fit => {
            if tex_w <= 0.0 || tex_h <= 0.0 {
                return None;
            }
            let scale = (w / tex_w).min(h / tex_h);
            let dw = tex_w * scale;
            let dh = tex_h * scale;
            Some((
                ffi::ImpellerRect {
                    x: 0.0,
                    y: 0.0,
                    width: tex_w,
                    height: tex_h,
                },
                ffi::ImpellerRect {
                    x: x + (w - dw) * 0.5,
                    y: y + (h - dh) * 0.5,
                    width: dw,
                    height: dh,
                },
            ))
        }
        ContentMode::Crop => {
            if tex_w <= 0.0 || tex_h <= 0.0 || w <= 0.0 || h <= 0.0 {
                return None;
            }
            let scale = (w / tex_w).max(h / tex_h);
            let src_w = w / scale;
            let src_h = h / scale;
            Some((
                ffi::ImpellerRect {
                    x: (tex_w - src_w) * 0.5,
                    y: (tex_h - src_h) * 0.5,
                    width: src_w,
                    height: src_h,
                },
                ffi::ImpellerRect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            ))
        }
        ContentMode::NineSlice { .. } => None,
    }
}

pub(crate) fn slice_rects(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tex_w: f32,
    tex_h: f32,
    mode: ContentMode,
) -> Result<Vec<(ffi::ImpellerRect, ffi::ImpellerRect)>, ()> {
    let ContentMode::NineSlice {
        left,
        top,
        right,
        bottom,
    } = mode
    else {
        return content_rects(x, y, w, h, tex_w, tex_h, mode)
            .map(|pair| vec![pair])
            .ok_or(());
    };
    if ![left, top, right, bottom, w, h, tex_w, tex_h]
        .iter()
        .all(|value| value.is_finite())
        || left < 0.0
        || top < 0.0
        || right < 0.0
        || bottom < 0.0
    {
        return Err(());
    }
    if left + right >= w || top + bottom >= h {
        return content_rects(x, y, w, h, tex_w, tex_h, ContentMode::Fill)
            .map(|pair| vec![pair])
            .ok_or(());
    }
    if left + right > tex_w || top + bottom > tex_h {
        return Err(());
    }
    let src_cols = [left, tex_w - left - right, right];
    let src_rows = [top, tex_h - top - bottom, bottom];
    let dst_cols = [left, w - left - right, right];
    let dst_rows = [top, h - top - bottom, bottom];
    let mut pairs = Vec::new();
    let mut src_y = 0.0;
    let mut dst_y = y;
    for row in 0..3 {
        let mut src_x = 0.0;
        let mut dst_x = x;
        for col in 0..3 {
            let sw = src_cols[col];
            let sh = src_rows[row];
            let dw = dst_cols[col];
            let dh = dst_rows[row];
            if sw > 0.0 && sh > 0.0 && dw > 0.0 && dh > 0.0 {
                if src_x < 0.0
                    || src_y < 0.0
                    || src_x + sw > tex_w + 0.01
                    || src_y + sh > tex_h + 0.01
                {
                    return Err(());
                }
                pairs.push((
                    ffi::ImpellerRect {
                        x: src_x,
                        y: src_y,
                        width: sw,
                        height: sh,
                    },
                    ffi::ImpellerRect {
                        x: dst_x,
                        y: dst_y,
                        width: dw,
                        height: dh,
                    },
                ));
            }
            src_x += sw;
            dst_x += dw;
        }
        src_y += src_rows[row];
        dst_y += dst_rows[row];
    }
    Ok(pairs)
}

pub(crate) fn gradient_solid(gradient: Option<&Gradient>) -> Option<[u8; 4]> {
    let Some(gradient) = gradient else {
        return None;
    };
    match prepared_gradient(gradient) {
        PreparedGradient::Solid(color) => Some(color),
        PreparedGradient::Paint { .. } => None,
        PreparedGradient::Invalid => None,
    }
}

pub(crate) enum PreparedGradient {
    Invalid,
    Solid([u8; 4]),
    Paint {
        kind: GradientKind,
        stops: Vec<ColorStop>,
    },
}

pub(crate) enum GradientKind {
    Linear {
        start: [f32; 2],
        end: [f32; 2],
    },
    Radial {
        center: [f32; 2],
        radius: f32,
    },
    Sweep {
        center: [f32; 2],
        start_angle_rad: f32,
        end_angle_rad: f32,
    },
}

pub(crate) fn prepared_gradient(gradient: &Gradient) -> PreparedGradient {
    match gradient {
        Gradient::Linear { start, end, stops } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if start[0] == end[0] && start[1] == end[1] {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Linear {
                    start: *start,
                    end: *end,
                },
                stops,
            }
        }
        Gradient::Radial {
            center,
            radius,
            stops,
        } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if !radius.is_finite() || *radius <= 0.0 {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Radial {
                    center: *center,
                    radius: *radius,
                },
                stops,
            }
        }
        Gradient::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
            stops,
        } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if !start_angle_rad.is_finite() || !end_angle_rad.is_finite() {
                return PreparedGradient::Invalid;
            }
            if start_angle_rad == end_angle_rad {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Sweep {
                    center: *center,
                    start_angle_rad: *start_angle_rad,
                    end_angle_rad: *end_angle_rad,
                },
                stops,
            }
        }
    }
}

pub(crate) fn normalize_stops(stops: &[ColorStop]) -> Option<Vec<ColorStop>> {
    if stops.len() < 2 {
        return None;
    }
    if stops
        .iter()
        .any(|stop| !stop.offset.is_finite() || stop.offset < 0.0 || stop.offset > 1.0)
    {
        return None;
    }
    let mut indexed: Vec<(usize, ColorStop)> = stops.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.offset
            .partial_cmp(&b.1.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    Some(indexed.into_iter().map(|(_, stop)| stop).collect())
}

pub(crate) fn color_source(
    gradient: Option<&Gradient>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Option<ffi::ImpellerColorSource> {
    let PreparedGradient::Paint { kind, stops } = prepared_gradient(gradient?) else {
        return None;
    };
    let colors: Vec<ffi::ImpellerColor> = stops
        .iter()
        .map(|stop| impeller_color(stop.color))
        .collect();
    let offsets: Vec<f32> = stops.iter().map(|stop| stop.offset).collect();
    let count = u32::try_from(stops.len()).unwrap_or(0);
    match kind {
        GradientKind::Linear { start, end } => {
            let start = ffi::ImpellerPoint {
                x: x + start[0] * w,
                y: y + start[1] * h,
            };
            let end = ffi::ImpellerPoint {
                x: x + end[0] * w,
                y: y + end[1] * h,
            };
            let source = unsafe {
                ffi::ImpellerColorSourceCreateLinearGradientNew(
                    &start,
                    &end,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
        GradientKind::Radial { center, radius } => {
            let center = ffi::ImpellerPoint {
                x: x + center[0] * w,
                y: y + center[1] * h,
            };
            let scene_radius = radius * w.min(h);
            let source = unsafe {
                ffi::ImpellerColorSourceCreateRadialGradientNew(
                    &center,
                    scene_radius,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
        GradientKind::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
        } => {
            let center = ffi::ImpellerPoint {
                x: x + center[0] * w,
                y: y + center[1] * h,
            };
            let start = start_angle_rad * (180.0 / std::f32::consts::PI);
            let end = end_angle_rad * (180.0 / std::f32::consts::PI);
            let source = unsafe {
                ffi::ImpellerColorSourceCreateSweepGradientNew(
                    &center,
                    start,
                    end,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
    }
}

pub(crate) fn rect_path(x: f32, y: f32, w: f32, h: f32, corner_radius: f32) -> ffi::ImpellerPath {
    let builder = unsafe { ffi::ImpellerPathBuilderNew() };
    if builder.is_null() {
        return ptr::null_mut();
    }
    let rect = ffi::ImpellerRect {
        x,
        y,
        width: w,
        height: h,
    };
    let radius = clamped_radius(corner_radius, w, h);
    unsafe {
        if radius > 0.0 {
            let radii = uniform_radii(radius);
            ffi::ImpellerPathBuilderAddRoundedRect(builder, &rect, &radii);
        } else {
            ffi::ImpellerPathBuilderAddRect(builder, &rect);
        }
        let path = ffi::ImpellerPathBuilderTakePathNew(
            builder,
            ffi::ImpellerFillType::kImpellerFillTypeNonZero,
        );
        ffi::ImpellerPathBuilderRelease(builder);
        path
    }
}

pub(crate) fn impeller_weight(weight: u16) -> ffi::ImpellerFontWeight::Type {
    match weight {
        100 => ffi::ImpellerFontWeight::kImpellerFontWeight100,
        200 => ffi::ImpellerFontWeight::kImpellerFontWeight200,
        300 => ffi::ImpellerFontWeight::kImpellerFontWeight300,
        500 => ffi::ImpellerFontWeight::kImpellerFontWeight500,
        600 => ffi::ImpellerFontWeight::kImpellerFontWeight600,
        700 => ffi::ImpellerFontWeight::kImpellerFontWeight700,
        800 => ffi::ImpellerFontWeight::kImpellerFontWeight800,
        900 => ffi::ImpellerFontWeight::kImpellerFontWeight900,
        _ => ffi::ImpellerFontWeight::kImpellerFontWeight400,
    }
}

pub(crate) fn impeller_align(align: TextAlign) -> ffi::ImpellerTextAlignment::Type {
    match align {
        TextAlign::Left => ffi::ImpellerTextAlignment::kImpellerTextAlignmentLeft,
        TextAlign::Center => ffi::ImpellerTextAlignment::kImpellerTextAlignmentCenter,
        TextAlign::Right => ffi::ImpellerTextAlignment::kImpellerTextAlignmentRight,
    }
}

pub(crate) fn clamped_radius(radius: f32, width: f32, height: f32) -> f32 {
    if !radius.is_finite() || radius <= 0.0 || width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    radius.min(width.min(height) * 0.5)
}

/// Impeller calls this when it drops a font mapping. The bytes stay in `Engine::faces`.
pub(crate) unsafe extern "C" fn keep_font_bytes(_user_data: *mut c_void) {}

/// Arguments passed into Impeller for units that must stay in scene points.
#[cfg(test)]
mod paint_trace {
    use std::cell::RefCell;

    #[derive(Clone, Debug, Default)]
    pub struct Trace {
        pub stroke_widths: Vec<f32>,
        pub blur_sigmas: Vec<(f32, f32)>,
        pub shadows: Vec<(f32, f32)>,
        pub decorations: Vec<i32>,
    }

    thread_local! {
        static TRACE: RefCell<Trace> = RefCell::new(Trace::default());
    }

    pub fn clear() {
        TRACE.with(|trace| *trace.borrow_mut() = Trace::default());
    }

    pub fn take() -> Trace {
        TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
    }

    pub fn stroke(width: f32) {
        TRACE.with(|trace| trace.borrow_mut().stroke_widths.push(width));
    }

    pub fn blur(sigma_x: f32, sigma_y: f32) {
        TRACE.with(|trace| trace.borrow_mut().blur_sigmas.push((sigma_x, sigma_y)));
    }

    pub fn shadow(elevation: f32, device_pixel_ratio: f32) {
        TRACE.with(|trace| {
            trace
                .borrow_mut()
                .shadows
                .push((elevation, device_pixel_ratio));
        });
    }

    pub fn decoration(types: i32) {
        TRACE.with(|trace| trace.borrow_mut().decorations.push(types));
    }
}

pub(crate) fn uniform_radii(radius: f32) -> ffi::ImpellerRoundingRadii {
    let corner = ffi::ImpellerPoint {
        x: radius,
        y: radius,
    };
    ffi::ImpellerRoundingRadii {
        top_left: corner,
        bottom_left: corner,
        top_right: corner,
        bottom_right: corner,
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::Engine;
    use crate::{ImagePixels, Status};
    use protocol::DrawList;

    #[test]
    #[ignore]
    fn submit_bench() {
        let mut engine = match Engine::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        const WARMUP: u64 = 10;
        const SAMPLES: u64 = 40;
        let commands = scrolled_commands();
        let mut samples = Vec::with_capacity(SAMPLES as usize);
        for index in 0..WARMUP + SAMPLES {
            let tree = picture_tree(0.0, index + 1, std::sync::Arc::clone(&commands));
            let started = std::time::Instant::now();
            engine.submit_layers(&tree, 2.0, |_| None).expect("submit");
            if index >= WARMUP {
                samples.push(started.elapsed());
            }
        }
        samples.sort();
        let p95 = samples[37];
        println!("submit_p95={:.3}ms", p95.as_secs_f64() * 1000.0);
    }

    #[test]
    fn rejected_font_bytes_return_bad_scene() {
        let mut engine = match Engine::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let secret = b"SECRET-FONT-BYTES-DO-NOT-LOG";
        assert_eq!(
            engine.register_font(secret, "Custom"),
            Err(Status::BadScene)
        );
    }

    #[test]
    fn non_positive_scale_is_rejected() {
        let mut engine = match Engine::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let list = DrawList {
            commands: Vec::new(),
        };
        assert_eq!(engine.paint(&list, 0.0, |_| None), Err(Status::BadScene));
        assert_eq!(
            engine.submit_layers(&protocol::LayerTree::empty(), 0.0, |_| None),
            Err(Status::BadScene)
        );
    }

    #[test]
    fn measured_paragraph_matches_a_second_query_and_submits() {
        let mut engine = match Engine::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let font = match std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") {
            Ok(font) => font,
            Err(_) => return,
        };
        engine
            .register_font(&font, "UIFWSans")
            .expect("font registers");
        let first = engine
            .typography()
            .borrow_mut()
            .measure_paragraph(
                "Hi",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &protocol::TextStyle::default(),
            )
            .expect("measure");
        let second = engine
            .typography()
            .borrow_mut()
            .measure_paragraph(
                "Hi",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &protocol::TextStyle::default(),
            )
            .expect("cached measure");
        assert_eq!(first, second);
        assert!(first.0 > 0.0 && first.1 > 0.0);
        let style = protocol::TextStyle::default();
        let color = [255, 255, 255, 255];
        let lines = engine
            .typography()
            .borrow_mut()
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("lines");
        let again = engine
            .typography()
            .borrow_mut()
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("cached lines");
        assert_eq!(lines.len(), again.len());
        assert!(!lines.is_empty());
        let caret = engine
            .typography()
            .borrow_mut()
            .caret_for("Hi", 28.0, 280.0, color, &style, 2, false)
            .expect("caret at end");
        assert!(caret.x > 0.0);
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: protocol::ParagraphId::NONE,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(protocol::TextStyle::default()),
                spans: std::sync::Arc::from([]),
                links: std::sync::Arc::from([]),
            }],
        };
        engine
            .paint(&list, 1.0, |_| None)
            .expect("measured text submits");
    }

    #[test]
    fn second_submit_reuses_the_texture() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = logo_list();
        let rgba = [0u8; 4];
        let mut calls = 0;
        let submit = |engine: &mut Engine, calls: &mut i32| {
            engine.paint(&list, 1.0, |_| {
                *calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
        };
        submit(&mut engine, &mut calls).expect("first image");
        submit(&mut engine, &mut calls).expect("second image");
        assert_eq!(calls, 1);
        assert_eq!(engine.cached_textures(), 1);
        let empty = DrawList {
            commands: Vec::new(),
        };
        engine.paint(&empty, 1.0, |_| None).expect("empty");
        assert_eq!(engine.cached_textures(), 0);
    }

    #[test]
    fn failed_submit_keeps_the_texture() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = logo_list();
        let rgba = [0u8; 4];
        let mut calls = 0;
        engine
            .paint(&list, 1.0, |_| {
                calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
            .expect("image");
        let broken = DrawList {
            commands: vec![protocol::Command::Pop],
        };
        assert_eq!(engine.paint(&broken, 1.0, |_| None), Err(Status::BadScene));
        assert_eq!(engine.cached_textures(), 1);
        engine
            .paint(&list, 1.0, |_| {
                calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
            .expect("reused image");
        assert_eq!(calls, 1);
    }

    #[test]
    fn measured_paragraph_survives_submit() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = protocol::TextStyle::default();
        let first = engine
            .typography()
            .borrow_mut()
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("measure");
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: protocol::ParagraphId::NONE,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(style.clone()),
                spans: std::sync::Arc::from([]),
                links: std::sync::Arc::from([]),
            }],
        };
        engine.paint(&list, 1.0, |_| None).expect("submit");
        let second = engine
            .typography()
            .borrow_mut()
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("cached");
        assert_eq!(first, second);
        assert_eq!(engine.cached_paragraphs(), 1);
    }

    #[test]
    fn released_paragraph_id_does_not_paint_the_new_color() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = protocol::TextStyle::default();
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let old = engine
            .typography()
            .borrow_mut()
            .retain_paragraph("Hi", 28.0, 280.0, red, &style, &[])
            .expect("retain red");
        engine.typography().borrow_mut().release_paragraph(old);
        assert!(!engine.typography().borrow().paragraph_live(old));
        let next = engine
            .typography()
            .borrow_mut()
            .retain_paragraph("Hi", 28.0, 280.0, green, &style, &[])
            .expect("retain green");
        assert_eq!(old.index, next.index);
        assert_ne!(old.generation, next.generation);
        assert!(engine.typography().borrow().paragraph_live(next));
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: old,
                color: red,
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(style),
                spans: std::sync::Arc::from([]),
                links: std::sync::Arc::from([]),
            }],
        };
        engine
            .paint(&list, 1.0, |_| None)
            .expect("stale id rebuilds");
        assert_eq!(engine.typography().borrow().text_paint_probe(), (true, red));
        assert!(engine.typography().borrow().paragraph_live(next));
    }

    #[test]
    fn unused_paragraph_is_evicted() {
        let Some(mut engine) = fonted() else {
            return;
        };
        engine
            .typography()
            .borrow_mut()
            .measure_paragraph(
                "A",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &protocol::TextStyle::default(),
            )
            .expect("measure");
        let list = DrawList {
            commands: Vec::new(),
        };
        engine.paint(&list, 1.0, |_| None).expect("empty");
        assert_eq!(engine.cached_paragraphs(), 0);
    }

    fn fonted() -> Option<Engine> {
        let mut engine = opened()?;
        let font = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").ok()?;
        engine
            .register_font(&font, "UIFWSans")
            .expect("font registers");
        Some(engine)
    }

    fn logo_list() -> DrawList {
        DrawList {
            commands: vec![protocol::Command::Image {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                source: std::sync::Arc::from("asset:logo"),
                mode: protocol::ContentMode::Fill,
                required: true,
            }],
        }
    }

    fn opened() -> Option<Engine> {
        match Engine::create_metal() {
            Ok(engine) => Some(engine),
            Err(Status::NotLinked) => None,
            Err(status) => panic!("{status}"),
        }
    }

    fn solid_rect(corner_radius: f32) -> protocol::Command {
        protocol::Command::Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 8.0,
            corner_radius,
            color: [9, 8, 7, 255],
            stroke_width: 0.0,
            stroke_color: [0, 0, 0, 0],
            gradient: None,
            shadow: None,
            box_shadows: std::sync::Arc::from([]),
        }
    }

    #[test]
    fn clip_and_opacity_groups_submit() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                protocol::Command::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 8.0,
                    corner_radius: 2.0,
                },
                solid_rect(4.0),
                protocol::Command::Pop,
                protocol::Command::PushOpacity {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 8.0,
                    alpha: 128,
                },
                solid_rect(0.0),
                protocol::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("clip and opacity submit");
    }

    #[test]
    fn nested_clip_and_transform_submit() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                protocol::Command::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    corner_radius: 0.0,
                },
                protocol::Command::PushTransform {
                    translate_x: 4.0,
                    translate_y: 2.0,
                    rotate_rad: 0.0,
                    scale_x: 1.5,
                    scale_y: 1.5,
                },
                solid_rect(0.0),
                protocol::Command::Pop,
                protocol::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("transform submit");
    }

    #[test]
    fn extra_pop_is_rejected() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![protocol::Command::Pop],
        };
        assert_eq!(engine.paint(&list, 1.0, |_| None), Err(Status::BadScene));
    }

    #[test]
    fn unclosed_clip_is_rejected() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![protocol::Command::PushClip {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
                corner_radius: 0.0,
            }],
        };
        assert_eq!(engine.paint(&list, 1.0, |_| None), Err(Status::BadScene));
    }

    #[test]
    fn fit_letterboxes_a_square_into_a_wide_box() {
        let (_src, dst) = super::content_rects(
            0.0,
            0.0,
            200.0,
            100.0,
            100.0,
            100.0,
            protocol::ContentMode::Fit,
        )
        .expect("fit");
        assert_eq!(
            (dst.x, dst.y, dst.width, dst.height),
            (50.0, 0.0, 100.0, 100.0)
        );
    }

    #[test]
    fn optional_missing_image_submits_and_required_does_not() {
        let Some(mut engine) = opened() else {
            return;
        };
        engine
            .paint(&image_command(false), 1.0, |_| None)
            .expect("optional image");
        assert_eq!(
            engine.paint(&image_command(true), 1.0, |_| None),
            Err(Status::BadScene)
        );
    }

    fn image_command(required: bool) -> DrawList {
        DrawList {
            commands: vec![protocol::Command::Image {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                source: std::sync::Arc::from("asset:missing"),
                mode: protocol::ContentMode::Fill,
                required,
            }],
        }
    }

    #[test]
    fn paint_units_stay_in_scene_points_at_scale_two() {
        let Some(mut engine) = opened() else {
            return;
        };
        super::paint_trace::clear();
        let list = DrawList {
            commands: vec![
                protocol::Command::PushBlur {
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    sigma: 4.0,
                },
                protocol::Command::Rect {
                    x: 1.0,
                    y: 2.0,
                    w: 10.0,
                    h: 8.0,
                    corner_radius: 0.0,
                    color: [1, 2, 3, 255],
                    stroke_width: 3.0,
                    stroke_color: [4, 5, 6, 255],
                    gradient: None,
                    shadow: Some(protocol::Shadow {
                        color: [0, 0, 0, 128],
                        elevation: 5.0,
                    }),
                    box_shadows: std::sync::Arc::from([]),
                },
                protocol::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("scene-point paint");
        let trace = super::paint_trace::take();
        assert_eq!(trace.stroke_widths, vec![3.0]);
        assert_eq!(trace.blur_sigmas, vec![(4.0, 4.0)]);
        assert_eq!(trace.shadows, vec![(5.0, 2.0)]);
    }

    #[test]
    fn span_family_weight_and_line_through_split_the_paragraph_key() {
        let Some(mut engine) = fonted() else {
            return;
        };
        super::paint_trace::clear();
        let color = [0, 0, 0, 255];
        let base = protocol::TextStyle::default();
        let spans = |family: &str, weight: u16| {
            vec![
                protocol::TextSpan {
                    utf8: "ab".to_owned(),
                    color,
                    weight: 400,
                    underline: false,
                    line_through: false,
                    family: Some("UIFWSans".to_owned()),
                },
                protocol::TextSpan {
                    utf8: "cd".to_owned(),
                    color,
                    weight,
                    underline: false,
                    line_through: false,
                    family: Some(family.to_owned()),
                },
            ]
        };
        engine
            .typography()
            .borrow_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 700))
            .expect("first span key");
        engine
            .typography()
            .borrow_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 700))
            .expect("same span key");
        assert_eq!(engine.cached_paragraphs(), 1);
        engine
            .typography()
            .borrow_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("UIFWSans", 700))
            .expect("family changes the key");
        assert_eq!(engine.cached_paragraphs(), 2);
        engine
            .typography()
            .borrow_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 400))
            .expect("weight changes the key");
        assert_eq!(engine.cached_paragraphs(), 3);
        let mut struck = base.clone();
        struck.line_through = true;
        engine
            .typography()
            .borrow_mut()
            .measure_paragraph("Hi", 16.0, 200.0, color, &struck)
            .expect("line through");
        let line_through =
            super::ffi::ImpellerTextDecorationType::kImpellerTextDecorationTypeLineThrough as i32;
        let trace = super::paint_trace::take();
        assert!(
            trace
                .decorations
                .iter()
                .any(|types| types & line_through != 0),
            "{:?}",
            trace.decorations
        );
    }

    #[test]
    fn scroll_offset_keeps_the_picture_display_list() {
        let Some(mut engine) = opened() else {
            return;
        };
        let mut commands = Vec::with_capacity(5000);
        for row in 0..5000 {
            let mut command = solid_rect(0.0);
            if let protocol::Command::Rect { y, .. } = &mut command {
                *y = row as f32 * 8.0;
            }
            commands.push(command);
        }
        let commands = std::sync::Arc::<[protocol::Command]>::from(commands);
        let first = picture_tree(0.0, 1, std::sync::Arc::clone(&commands));
        engine
            .submit_layers(&first, 1.0, |_| None)
            .expect("first layer tree");
        assert_eq!(engine.picture_builds(), 1);
        let scrolled = picture_tree(40.0, 1, commands);
        engine
            .submit_layers(&scrolled, 2.0, |_| None)
            .expect("scrolled layer tree");
        assert_eq!(engine.picture_builds(), 1);
        assert_eq!(engine.retained_pictures(), 1);
        let rewritten = picture_tree(40.0, 2, scrolled_commands());
        engine
            .submit_layers(&rewritten, 2.0, |_| None)
            .expect("rewritten picture");
        assert_eq!(engine.picture_builds(), 2);
    }

    #[test]
    fn register_font_rebuilds_picture_display_lists() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = std::sync::Arc::new(protocol::TextStyle::default());
        let command = protocol::Command::Text {
            x: 0.0,
            y: 0.0,
            max_width: 280.0,
            font_size: 28.0,
            paragraph: protocol::ParagraphId::NONE,
            color: [255, 255, 255, 255],
            utf8: std::sync::Arc::from("Hi"),
            style,
            spans: std::sync::Arc::from([]),
            links: std::sync::Arc::from([]),
        };
        let tree = picture_tree(0.0, 1, std::sync::Arc::from([command]));
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("text picture");
        assert_eq!(engine.picture_builds(), 1);
        let font =
            std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("font file");
        engine
            .register_font(&font, "UIFWSans")
            .expect("font registers again");
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("picture after font");
        assert_eq!(engine.picture_builds(), 2);
    }

    #[test]
    fn layer_image_stays_cached_when_the_picture_is_reused() {
        let Some(mut engine) = opened() else {
            return;
        };
        let command = protocol::Command::Image {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            source: std::sync::Arc::from("asset:logo"),
            mode: protocol::ContentMode::Fill,
            required: true,
        };
        let tree = picture_tree(0.0, 1, std::sync::Arc::from([command]));
        let rgba = [0u8; 4];
        let mut calls = 0;
        let submit = |engine: &mut Engine, tree: &protocol::LayerTree, calls: &mut i32| {
            engine.submit_layers(tree, 1.0, |_| {
                *calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
        };
        submit(&mut engine, &tree, &mut calls).expect("first picture");
        submit(&mut engine, &tree, &mut calls).expect("cached picture");
        assert_eq!(calls, 1);
        assert_eq!(engine.cached_textures(), 1);
        let empty = protocol::LayerTree::empty();
        engine
            .submit_layers(&empty, 1.0, |_| None)
            .expect("empty tree");
        assert_eq!(engine.retained_pictures(), 0);
        assert_eq!(engine.cached_textures(), 0);
    }

    fn scrolled_commands() -> std::sync::Arc<[protocol::Command]> {
        std::sync::Arc::from([solid_rect(0.0)])
    }

    fn picture_tree(
        offset_y: f32,
        generation: u64,
        commands: std::sync::Arc<[protocol::Command]>,
    ) -> protocol::LayerTree {
        use protocol::{Layer, LayerId, LayerKind, Picture, PictureId};
        let id = PictureId(7);
        let picture = std::sync::Arc::new(Picture {
            id,
            generation,
            origin: (3.0, 4.0),
            commands,
        });
        protocol::LayerTree {
            layers: vec![
                Layer {
                    kind: LayerKind::Offset {
                        x: 0.0,
                        y: offset_y,
                    },
                    parent: None,
                    first_child: Some(LayerId(1)),
                    next: None,
                },
                Layer {
                    kind: LayerKind::Picture { picture: id },
                    parent: Some(LayerId(0)),
                    first_child: None,
                    next: None,
                },
            ],
            pictures: vec![picture],
            root: LayerId(0),
            epoch: generation,
        }
    }

    #[test]
    fn picture_reuse_across_a_mixed_edit_sequence() {
        let Some(mut engine) = opened() else {
            return;
        };
        let commands = scrolled_commands();
        let tree = many_pictures(5000, 1, 0.0, commands.clone());
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("first submit");
        let built = engine.picture_builds();
        let moved = many_pictures(5000, 1, 4.0, commands.clone());
        engine.submit_layers(&moved, 1.0, |_| None).expect("offset");
        assert_eq!(engine.picture_builds(), built);
        engine.submit_layers(&moved, 2.0, |_| None).expect("scale");
        assert_eq!(engine.picture_builds(), built);
        let mut bumped = many_pictures(5000, 1, 4.0, commands);
        bumped.pictures[0] = std::sync::Arc::new(protocol::Picture {
            id: bumped.pictures[0].id,
            generation: 2,
            origin: bumped.pictures[0].origin,
            commands: std::sync::Arc::clone(&bumped.pictures[0].commands),
        });
        engine
            .submit_layers(&bumped, 2.0, |_| None)
            .expect("generation");
        assert_eq!(engine.picture_builds(), built + 1);
    }

    fn many_pictures(
        count: u32,
        generation: u64,
        offset_y: f32,
        commands: std::sync::Arc<[protocol::Command]>,
    ) -> protocol::LayerTree {
        use protocol::{Layer, LayerId, LayerKind, Picture, PictureId};
        let mut layers = Vec::with_capacity(count as usize + 1);
        let mut pictures = Vec::with_capacity(count as usize);
        layers.push(Layer {
            kind: LayerKind::Offset {
                x: 0.0,
                y: offset_y,
            },
            parent: None,
            first_child: (count > 0).then_some(LayerId(1)),
            next: None,
        });
        for index in 0..count {
            let id = PictureId(u64::from(index) + 1);
            pictures.push(std::sync::Arc::new(Picture {
                id,
                generation,
                origin: (0.0, 0.0),
                commands: std::sync::Arc::clone(&commands),
            }));
            let layer_index = index + 1;
            layers.push(Layer {
                kind: LayerKind::Picture { picture: id },
                parent: Some(LayerId(0)),
                first_child: None,
                next: (layer_index < count).then_some(LayerId(layer_index + 1)),
            });
        }
        protocol::LayerTree {
            layers,
            pictures,
            root: LayerId(0),
            epoch: generation,
        }
    }
}
