//! Paints a draw list. Text uses the font size and paragraph width stored on each command.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::marker::PhantomData;
use std::ptr;

use protocol::{
    BlendMode, ColorStop, Command, ContentMode, DrawList, Gradient, Shadow, TextAlign, TextSpan,
    TextStyle,
};

use crate::{ImagePixels, Status};

mod ffi {
    #![allow(
        non_camel_case_types,
        non_snake_case,
        non_upper_case_globals,
        dead_code,
        clippy::all
    )]
    include!(concat!(env!("OUT_DIR"), "/impeller.rs"));
    include!(concat!(env!("OUT_DIR"), "/impeller_version.rs"));
}

const PARAGRAPH_LIMIT: usize = 256;
const TEXTURE_LIMIT: usize = 64;

#[derive(Clone, Hash, PartialEq, Eq)]
struct ParaKey {
    utf8: String,
    font_size: u32,
    max_width: u32,
    color: [u8; 4],
    family: String,
    max_lines: u32,
    ellipsis: String,
    align: TextAlign,
    weight: u16,
    direction_rtl: bool,
    height_bits: u32,
    underline: bool,
    line_through: bool,
    soft_wrap: bool,
    overflow: protocol::TextOverflow,
    spans: Vec<SpanKey>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SpanKey {
    utf8: String,
    family: String,
    weight: u16,
    color: [u8; 4],
    underline: bool,
    line_through: bool,
}

struct Shaped {
    width: f32,
    height: f32,
    paragraph: ffi::ImpellerParagraph,
    used: u64,
}

struct CachedTexture {
    texture: ffi::ImpellerTexture,
    width: f32,
    height: f32,
    used: u64,
}

struct Face {
    bytes: Vec<u8>,
    family: CString,
}

enum Save {
    Clip,
    Transform,
    /// Paint read by `Restore`. Released after that restore.
    Layer(ffi::ImpellerPaint),
    /// Invalid blur or SrcOver blend: no Impeller save, matching pop is skipped.
    Skipped,
}

pub struct Engine {
    context: ffi::ImpellerContext,
    typography: ffi::ImpellerTypographyContext,
    list: ffi::ImpellerDisplayList,
    textures: HashMap<String, CachedTexture>,
    paragraphs: HashMap<ParaKey, Shaped>,
    /// Paragraphs replaced while a display list might still reference them.
    retired: Vec<ffi::ImpellerParagraph>,
    list_generation: u64,
    paragraphs_stale: bool,
    faces: Vec<Face>,
    default_family: Option<CString>,
    /// `dlopen` handle for `libvulkan.so`. Null until [`Engine::create_vulkan`].
    #[cfg(target_os = "android")]
    vulkan_lib: *mut c_void,
    /// Save/SaveLayer pairs opened by the submit in progress.
    saves: Vec<Save>,
    /// Impeller objects are created and used on the render thread.
    render_thread: PhantomData<*mut ()>,
}

#[cfg(target_os = "android")]
#[path = "vulkan.rs"]
mod vulkan;

#[cfg(target_os = "android")]
pub use vulkan::VulkanSwapchain;
#[cfg(target_os = "android")]
pub use vulkan::note_swapchain_extent;

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

    pub(super) fn blank(
        context: ffi::ImpellerContext,
        typography: ffi::ImpellerTypographyContext,
    ) -> Self {
        Self {
            context,
            typography,
            list: ptr::null_mut(),
            textures: HashMap::new(),
            paragraphs: HashMap::new(),
            retired: Vec::new(),
            list_generation: 0,
            paragraphs_stale: false,
            faces: Vec::new(),
            default_family: None,
            #[cfg(target_os = "android")]
            vulkan_lib: ptr::null_mut(),
            saves: Vec::new(),
            render_thread: PhantomData,
        }
    }

    pub fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        if data.is_empty() {
            return Err(Status::BadScene);
        }
        let family = CString::new(family).map_err(|_| Status::BadScene)?;
        if family.as_bytes().is_empty() {
            return Err(Status::BadScene);
        }
        let owned = data.to_vec();
        let mapping = ffi::ImpellerMapping {
            data: owned.as_ptr(),
            length: owned.len() as u64,
            // Impeller invokes this even when registration fails. A null pointer
            // crashes. The callback does not free `owned`.
            on_release: Some(keep_font_bytes),
        };
        // SAFETY: `mapping.data` points at `owned` for this call. Failure drops the
        // mapping inside Impeller and returns false before `owned` is stored. Success
        // moves `owned` into `faces`, which outlives the typeface that borrowed it.
        let registered = unsafe {
            ffi::ImpellerTypographyContextRegisterFont(
                self.typography,
                &mapping,
                ptr::null_mut(),
                family.as_ptr(),
            )
        };
        if !registered {
            return Err(Status::BadScene);
        }
        if self.default_family.is_none() {
            self.default_family = Some(family.clone());
        }
        if let Some(face) = self
            .faces
            .iter_mut()
            .find(|face| face.family.as_bytes() == family.as_bytes())
        {
            face.bytes = owned;
            face.family = family;
        } else {
            self.faces.push(Face {
                bytes: owned,
                family,
            });
        }
        if self.list.is_null() {
            self.release_paragraphs();
            self.paragraphs_stale = false;
        } else {
            self.paragraphs_stale = true;
        }
        Ok(())
    }

    pub fn submit<'pixels>(
        &mut self,
        list: &DrawList,
        point_scale: f32,
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        if point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        self.list_generation = self.list_generation.wrapping_add(1);
        // The installed display list stays until this build succeeds. Releasing it
        // first left the player empty when the new list failed.
        // SAFETY: the context was created in `create_metal` and is released in Drop.
        let builder = unsafe { ffi::ImpellerDisplayListBuilderNew(ptr::null()) };
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if builder.is_null() || paint.is_null() {
            unsafe {
                ffi::ImpellerDisplayListBuilderRelease(builder);
                ffi::ImpellerPaintRelease(paint);
            }
            return Err(Status::BadScene);
        }
        let mut scaled = false;
        let mut failed = false;
        for command in &list.commands {
            let drawn = match command {
                Command::Fill { color } => {
                    self.paint_color(paint, *color);
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
                        point_scale,
                    )
                }
                Command::Text {
                    x,
                    y,
                    max_width,
                    font_size,
                    color,
                    utf8,
                    style,
                    spans,
                    ..
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_text(
                        builder, paint, *x, *y, *max_width, *font_size, *color, utf8, style, spans,
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
                    self.draw_image(
                        builder,
                        *x,
                        *y,
                        *w,
                        *h,
                        source,
                        *mode,
                        *required,
                        &mut resolve,
                    )
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
                failed = true;
                break;
            }
        }
        if !self.saves.is_empty() {
            failed = true;
        }
        let display = if failed {
            ptr::null_mut()
        } else {
            // SAFETY: `builder` is the one created above. The save stack matches `self.saves`,
            // which is empty on this path, so the recorded clips and layers are balanced.
            unsafe { ffi::ImpellerDisplayListBuilderCreateDisplayListNew(builder) }
        };
        self.drain_saves(builder);
        unsafe {
            ffi::ImpellerDisplayListBuilderRelease(builder);
            ffi::ImpellerPaintRelease(paint);
        }
        if display.is_null() {
            return Err(Status::BadScene);
        }
        // Drop the installed list before retiring paragraphs it still references.
        self.release_display();
        self.release_retired();
        self.paragraphs_stale = false;
        self.list = display;
        self.evict_unused();
        self.evict_over_cap();
        Ok(())
    }

    /// Shapes `utf8` with `color` on this engine's typography context.
    ///
    /// The paragraph is kept and drawn by the next [`Engine::submit`] that
    /// carries the same text, size, width, and color.
    pub fn measure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<(f32, f32), Status> {
        self.measure_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    /// Same paragraph as [`Engine::measure_paragraph`], including each span in the cache key.
    pub fn measure_with_spans(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<(f32, f32), Status> {
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
            return Err(Status::BadScene);
        }
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, style, spans) else {
            return Err(Status::BadScene);
        };
        if !self.paragraphs_stale {
            if let Some(found) = self.paragraphs.get_mut(&key) {
                found.used = self.list_generation;
                return Ok((found.width, found.height));
            }
        }
        let mut shaped = self.build_paragraph(utf8, font_size, max_width, color, style, spans)?;
        shaped.used = self.list_generation;
        if style.overflow != protocol::TextOverflow::Visible {
            shaped.width = shaped.width.min(max_width);
        }
        let size = (shaped.width, shaped.height);
        self.store_paragraph(key, shaped);
        Ok(size)
    }

    /// Caret nearest a point in paragraph space. The paragraph is the one
    /// [`Engine::measure_paragraph`] caches.
    pub fn caret_at_point(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Result<crate::CaretHit, Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        // SAFETY: `paragraph` is a live cached paragraph owned by this engine.
        // The glyph info is released before return. Bounds are written by Impeller.
        unsafe {
            let info = ffi::ImpellerParagraphCreateGlyphInfoAtParagraphCoordinatesNew(
                paragraph,
                f64::from(x),
                f64::from(y),
            );
            if info.is_null() {
                return Ok(crate::CaretHit {
                    unit: utf16_len(utf8),
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: font_size.max(0.0),
                    upstream: false,
                });
            }
            let begin = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeBegin(info);
            let end = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeEnd(info);
            let mut bounds = ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            };
            ffi::ImpellerGlyphInfoGetGraphemeClusterBounds(info, &mut bounds);
            ffi::ImpellerGlyphInfoRelease(info);
            let mid = bounds.x + bounds.width * 0.5;
            let at_end = x >= mid;
            Ok(crate::CaretHit {
                unit: u32::try_from(if at_end { end } else { begin }).unwrap_or(0),
                x: if at_end {
                    bounds.x + bounds.width
                } else {
                    bounds.x
                },
                y: bounds.y,
                w: 1.0,
                h: bounds.height,
                upstream: false,
            })
        }
    }

    /// Caret rectangle for a UTF-16 index in the cached paragraph.
    pub fn caret_at_unit(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> Result<crate::CaretHit, Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        let end_unit = utf16_len(utf8);
        let unit = unit.min(end_unit);
        if unit == end_unit {
            let lines = self.lines(utf8, font_size, max_width, color, style)?;
            if let Some(last) = lines.last() {
                return Ok(crate::CaretHit {
                    unit,
                    x: last.x + last.w,
                    y: last.y,
                    w: 1.0,
                    h: last.h,
                    upstream: false,
                });
            }
            return Ok(crate::CaretHit {
                unit,
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: font_size.max(0.0),
                upstream: false,
            });
        }
        // SAFETY: `paragraph` is a live cached paragraph. Glyph info is released here.
        unsafe {
            let info =
                ffi::ImpellerParagraphCreateGlyphInfoAtCodeUnitIndexNew(paragraph, unit as usize);
            if info.is_null() {
                return Ok(crate::CaretHit {
                    unit,
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: font_size.max(0.0),
                    upstream: false,
                });
            }
            let begin = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeBegin(info);
            let mut bounds = ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            };
            ffi::ImpellerGlyphInfoGetGraphemeClusterBounds(info, &mut bounds);
            ffi::ImpellerGlyphInfoRelease(info);
            let x = if unit as usize > begin {
                bounds.x + bounds.width
            } else {
                bounds.x
            };
            Ok(crate::CaretHit {
                unit,
                x,
                y: bounds.y,
                w: 1.0,
                h: bounds.height,
                upstream: false,
            })
        }
    }

    /// UTF-16 word bounds around `unit`, from the cached paragraph.
    pub fn word_range(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> Result<(u32, u32), Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        let mut range = ffi::ImpellerRange { start: 0, end: 0 };
        // SAFETY: `paragraph` is a live cached paragraph. `range` is a local out-param.
        unsafe {
            ffi::ImpellerParagraphGetWordBoundary(paragraph, unit as usize, &mut range);
        }
        Ok((
            u32::try_from(range.start).unwrap_or(0),
            u32::try_from(range.end).unwrap_or(0),
        ))
    }

    /// Laid-out lines of the cached paragraph.
    pub fn lines(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<Vec<crate::LineMetric>, Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        // SAFETY: `paragraph` is cached and alive. Line metrics stay with it.
        unsafe { read_lines(paragraph) }
    }

    /// Caret at `unit`. `upstream` selects the previous line at a soft break.
    pub fn caret_for(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
        upstream: bool,
    ) -> Result<crate::CaretHit, Status> {
        let lines = self.lines(utf8, font_size, max_width, color, style)?;
        if upstream
            && let Some((index, _)) = lines
                .iter()
                .enumerate()
                .find(|(_, line)| line.start == unit)
            && index > 0
            && !lines[index - 1].hard_break
        {
            let prev = &lines[index - 1];
            return Ok(crate::CaretHit {
                unit,
                x: prev.x + prev.w,
                y: prev.y,
                w: 1.0,
                h: prev.h,
                upstream: true,
            });
        }
        self.caret_at_unit(utf8, font_size, max_width, color, style, unit)
    }

    fn ensure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<ffi::ImpellerParagraph, Status> {
        self.measure_paragraph(utf8, font_size, max_width, color, style)?;
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, style, &[]) else {
            return Err(Status::BadScene);
        };
        self.paragraphs
            .get(&key)
            .map(|shaped| shaped.paragraph)
            .ok_or(Status::BadScene)
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

    fn draw_rect(
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
        point_scale: f32,
    ) -> bool {
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return false;
        }
        let stroke = stroke_width.is_finite() && stroke_width > 0.0;
        let fill = color[3] > 0 || gradient.is_some();
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
                self.paint_color(paint, fill_color);
                self.set_stroke(paint, stroke_width);
                unsafe {
                    ffi::ImpellerPaintSetDrawStyle(
                        paint,
                        ffi::ImpellerDrawStyle::kImpellerDrawStyleStrokeAndFill,
                    );
                }
                self.fill_rect(builder, paint, x, y, w, h, corner_radius);
            } else {
                self.paint_color(paint, fill_color);
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
                self.paint_color(stroke_paint, stroke_color);
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
            self.paint_color(paint, stroke_color);
            self.set_stroke(paint, stroke_width);
            unsafe {
                ffi::ImpellerPaintSetDrawStyle(
                    paint,
                    ffi::ImpellerDrawStyle::kImpellerDrawStyleStroke,
                );
            }
            self.fill_rect(builder, paint, x, y, w, h, corner_radius);
        } else if fill {
            self.paint_color(paint, fill_color);
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

    fn fill_rect(
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

    fn set_stroke(&self, paint: ffi::ImpellerPaint, width: f32) {
        #[cfg(test)]
        paint_trace::stroke(width);
        unsafe { ffi::ImpellerPaintSetStrokeWidth(paint, width) };
    }

    fn paint_gradient(
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

    fn draw_shadow(
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

    fn push_clip(
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

    fn push_opacity(
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
        self.paint_color(layer_paint, [255, 255, 255, alpha]);
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

    fn push_transform(
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

    fn push_blur(
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
            self.saves.push(Save::Skipped);
            return true;
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

    fn push_blend(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    ) -> bool {
        let blend = match mode {
            BlendMode::SrcOver => {
                self.saves.push(Save::Skipped);
                return true;
            }
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

    fn pop_save(&mut self, builder: ffi::ImpellerDisplayListBuilder) -> bool {
        if self.saves.is_empty() {
            return false;
        }
        match self.saves.pop() {
            Some(Save::Skipped) => true,
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

    fn drain_saves(&mut self, builder: ffi::ImpellerDisplayListBuilder) {
        while let Some(save) = self.saves.pop() {
            match save {
                Save::Skipped => {}
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

    fn scale_once(
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

    fn paint_color(&self, paint: ffi::ImpellerPaint, color: [u8; 4]) {
        let color = ffi::ImpellerColor {
            red: color[0] as f32 / 255.0,
            green: color[1] as f32 / 255.0,
            blue: color[2] as f32 / 255.0,
            alpha: color[3] as f32 / 255.0,
            color_space: ffi::ImpellerColorSpace::kImpellerColorSpaceSRGB,
        };
        unsafe { ffi::ImpellerPaintSetColor(paint, &color) };
    }

    fn draw_text(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        _paint: ffi::ImpellerPaint,
        x: f32,
        y: f32,
        max_width: f32,
        font_size: f32,
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
        let Some(family) = self.resolved_family(text_style.family.as_deref()) else {
            return false;
        };
        let Some(key) = para_key(
            utf8, font_size, max_width, color, &family, text_style, spans,
        ) else {
            return false;
        };
        let cached = if self.paragraphs_stale {
            None
        } else {
            self.paragraphs.get_mut(&key).map(|shaped| {
                shaped.used = self.list_generation;
                shaped.paragraph
            })
        };
        let paragraph = if let Some(paragraph) = cached {
            paragraph
        } else {
            let Ok(mut shaped) =
                self.build_paragraph(utf8, font_size, max_width, color, text_style, spans)
            else {
                return false;
            };
            shaped.used = self.list_generation;
            let paragraph = shaped.paragraph;
            self.store_paragraph(key, shaped);
            paragraph
        };
        let point = ffi::ImpellerPoint { x, y };
        // SAFETY: `paragraph` stays in `self.paragraphs` until a later submit drops it,
        // and only after the display list that referenced it has been released.
        unsafe { ffi::ImpellerDisplayListBuilderDrawParagraph(builder, paragraph, &point) };
        true
    }

    fn build_paragraph(
        &self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        text_style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<Shaped, Status> {
        // SAFETY: the paint is released before this returns. A null paint cannot shape.
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return Err(Status::BadScene);
        }
        self.paint_color(paint, color);
        let paragraph = self.shape(
            Some(paint),
            utf8,
            font_size,
            max_width,
            color,
            text_style,
            spans,
        );
        unsafe { ffi::ImpellerPaintRelease(paint) };
        let Some(paragraph) = paragraph else {
            return Err(Status::BadScene);
        };
        // SAFETY: `paragraph` is a live object from `shape` and is stored by the caller.
        let (width, height) = unsafe {
            (
                ffi::ImpellerParagraphGetLongestLineWidth(paragraph),
                ffi::ImpellerParagraphGetHeight(paragraph),
            )
        };
        if !width.is_finite() || !height.is_finite() {
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
            return Err(Status::BadScene);
        }
        Ok(Shaped {
            width,
            height,
            paragraph,
            used: self.list_generation,
        })
    }

    /// Family actually passed to Impeller. A missing name falls back to the first
    /// registered face so a typo does not fail the whole frame.
    fn resolved_family(&self, requested: Option<&str>) -> Option<String> {
        self.resolved_face(requested)
            .and_then(|family| family.to_str().ok().map(str::to_owned))
    }

    fn resolved_face(&self, requested: Option<&str>) -> Option<&CString> {
        if let Some(name) = requested.filter(|name| !name.is_empty())
            && let Some(face) = self
                .faces
                .iter()
                .find(|face| face.family.as_bytes() == name.as_bytes())
        {
            return Some(&face.family);
        }
        let default = self.default_family.as_ref()?;
        self.faces
            .iter()
            .find(|face| face.family.as_c_str() == default.as_c_str())
            .map(|face| &face.family)
    }

    fn has_family(&self, name: &CString) -> bool {
        self.faces
            .iter()
            .any(|face| face.family.as_c_str() == name.as_c_str())
    }

    fn shape(
        &self,
        paint: Option<ffi::ImpellerPaint>,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        text_style: &TextStyle,
        spans: &[TextSpan],
    ) -> Option<ffi::ImpellerParagraph> {
        let family = self.resolved_face(text_style.family.as_deref())?;
        let family = family.as_ptr();
        let ellipsis = text_style
            .ellipsis_str()
            .and_then(|text| CString::new(text).ok());
        let layout_width = if text_style.soft_wrap || ellipsis.is_some() {
            max_width
        } else {
            max_width.max(1_000_000.0)
        };
        // SAFETY: style and the builder are released on every path. The returned
        // paragraph is owned by the caller. Font size stays in scene points; the
        // display-list scale applied before the first non-fill command scales it.
        // `family` and `ellipsis` outlive this block.
        unsafe {
            let style = ffi::ImpellerParagraphStyleNew();
            let paragraph_builder = ffi::ImpellerParagraphBuilderNew(self.typography);
            if style.is_null() || paragraph_builder.is_null() {
                ffi::ImpellerParagraphStyleRelease(style);
                ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
                return None;
            }
            if let Some(paint) = paint {
                ffi::ImpellerParagraphStyleSetForeground(style, paint);
            }
            ffi::ImpellerParagraphStyleSetFontFamily(style, family);
            ffi::ImpellerParagraphStyleSetFontSize(style, font_size);
            ffi::ImpellerParagraphStyleSetFontWeight(style, impeller_weight(text_style.weight));
            ffi::ImpellerParagraphStyleSetFontStyle(
                style,
                ffi::ImpellerFontStyle::kImpellerFontStyleNormal,
            );
            ffi::ImpellerParagraphStyleSetTextAlignment(style, impeller_align(text_style.align));
            ffi::ImpellerParagraphStyleSetTextDirection(
                style,
                if text_style.direction_rtl {
                    ffi::ImpellerTextDirection::kImpellerTextDirectionRTL
                } else {
                    ffi::ImpellerTextDirection::kImpellerTextDirectionLTR
                },
            );
            if text_style.overflow != protocol::TextOverflow::Visible
                && let Some(max_lines) = text_style.max_lines.filter(|lines| *lines > 0)
            {
                ffi::ImpellerParagraphStyleSetMaxLines(style, max_lines);
            }
            if text_style.height.is_finite() && text_style.height > 0.0 {
                ffi::ImpellerParagraphStyleSetHeight(style, text_style.height);
            }
            if let Some(ellipsis) = &ellipsis {
                ffi::ImpellerParagraphStyleSetEllipsis(style, ellipsis.as_ptr());
            }
            apply_decoration(style, color, text_style.underline, text_style.line_through);
            let fallback = CString::new("UIFWSansCJK").ok();
            let cjk = fallback.as_ref().filter(|name| self.has_family(name));
            if spans.is_empty() {
                push_runs(self, style, paragraph_builder, family, cjk, utf8);
            } else {
                for span in spans {
                    if span.utf8.is_empty() {
                        continue;
                    }
                    let span_paint = ffi::ImpellerPaintNew();
                    if span_paint.is_null() {
                        ffi::ImpellerParagraphStyleRelease(style);
                        ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
                        return None;
                    }
                    self.paint_color(span_paint, span.color);
                    ffi::ImpellerParagraphStyleSetForeground(style, span_paint);
                    ffi::ImpellerParagraphStyleSetFontWeight(style, impeller_weight(span.weight));
                    apply_decoration(style, span.color, span.underline, span.line_through);
                    let span_family = self
                        .resolved_face(span.family.as_deref())
                        .map(|name| name.as_ptr())
                        .unwrap_or(family);
                    push_runs(self, style, paragraph_builder, span_family, cjk, &span.utf8);
                    ffi::ImpellerPaintRelease(span_paint);
                }
            }
            let paragraph =
                ffi::ImpellerParagraphBuilderBuildParagraphNew(paragraph_builder, layout_width);
            ffi::ImpellerParagraphStyleRelease(style);
            ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
            if paragraph.is_null() {
                None
            } else {
                Some(paragraph)
            }
        }
    }

    fn draw_image<'pixels>(
        &mut self,
        builder: ffi::ImpellerDisplayListBuilder,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: &str,
        mode: ContentMode,
        required: bool,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        if let Some(cached) = self.textures.get_mut(source) {
            cached.used = self.list_generation;
            let texture = cached.texture;
            let width = cached.width;
            let height = cached.height;
            self.blit_texture(builder, texture, x, y, w, h, width, height, mode);
            return true;
        }
        let Some(image) = resolve(source) else {
            return !required;
        };
        if image.width == 0 || image.height == 0 {
            return true;
        }
        let Some(bytes) = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return false;
        };
        if image.rgba.len() < bytes {
            return false;
        }
        let descriptor = ffi::ImpellerTextureDescriptor {
            pixel_format: ffi::ImpellerPixelFormat::kImpellerPixelFormatRGBA8888,
            size: ffi::ImpellerISize {
                width: image.width as i64,
                height: image.height as i64,
            },
            mip_count: 1,
        };
        let mapping = ffi::ImpellerMapping {
            data: image.rgba.as_ptr(),
            length: bytes as u64,
            on_release: None,
        };
        // SAFETY: `mapping` borrows `image.rgba` until TextureCreate returns. Impeller copies
        // the pixels. The texture handle stays in `textures` until it is evicted or dropped.
        let texture = unsafe {
            ffi::ImpellerTextureCreateWithContentsNew(
                self.context,
                &descriptor,
                &mapping,
                ptr::null_mut(),
            )
        };
        if texture.is_null() {
            return false;
        }
        let width = image.width as f32;
        let height = image.height as f32;
        self.textures.insert(
            source.to_owned(),
            CachedTexture {
                texture,
                width,
                height,
                used: self.list_generation,
            },
        );
        self.blit_texture(builder, texture, x, y, w, h, width, height, mode);
        true
    }

    fn blit_texture(
        &self,
        builder: ffi::ImpellerDisplayListBuilder,
        texture: ffi::ImpellerTexture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        width: f32,
        height: f32,
        mode: ContentMode,
    ) {
        let Some((src, dst)) = content_rects(x, y, w, h, width, height, mode) else {
            return;
        };
        // SAFETY: `texture` is owned by `self.textures` for at least this submit.
        unsafe {
            ffi::ImpellerDisplayListBuilderDrawTextureRect(
                builder,
                texture,
                &src,
                &dst,
                ffi::ImpellerTextureSampling::kImpellerTextureSamplingLinear,
                ptr::null_mut(),
            );
        }
    }

    fn store_paragraph(&mut self, key: ParaKey, shaped: Shaped) {
        if let Some(old) = self.paragraphs.insert(key, shaped) {
            self.retire_paragraph(old.paragraph);
        }
    }

    fn retire_paragraph(&mut self, paragraph: ffi::ImpellerParagraph) {
        if self.list.is_null() {
            // SAFETY: no display list is installed, so this is the last retain.
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        } else {
            self.retired.push(paragraph);
        }
    }

    fn release_display(&mut self) {
        self.release_saved_paints();
        // SAFETY: `list` is null or the display list from the previous submit.
        unsafe {
            ffi::ImpellerDisplayListRelease(self.list);
            self.list = ptr::null_mut();
        }
    }

    fn release_saved_paints(&mut self) {
        for save in self.saves.drain(..) {
            if let Save::Layer(paint) = save
                && !paint.is_null()
            {
                unsafe { ffi::ImpellerPaintRelease(paint) };
            }
        }
    }

    fn release_retired(&mut self) {
        for paragraph in self.retired.drain(..) {
            // SAFETY: these were detached after a newer paragraph replaced them, and the
            // display list that could reference them has already been released.
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        }
    }

    fn release_paragraphs(&mut self) {
        for (_, shaped) in self.paragraphs.drain() {
            // SAFETY: callers release the display list first, or no list is installed.
            unsafe { ffi::ImpellerParagraphRelease(shaped.paragraph) };
        }
    }

    fn release_textures(&mut self) {
        for (_, cached) in self.textures.drain() {
            // SAFETY: callers release the display list first, or no list is installed.
            unsafe { ffi::ImpellerTextureRelease(cached.texture) };
        }
    }

    fn evict_unused(&mut self) {
        let generation = self.list_generation;
        self.evict_paragraphs(|used| used != generation);
        self.evict_textures(|used| used != generation);
    }

    fn evict_over_cap(&mut self) {
        let generation = self.list_generation;
        if self.paragraphs.len() > PARAGRAPH_LIMIT * 4 {
            self.evict_paragraphs(|used| used != generation);
        }
        if self.textures.len() > TEXTURE_LIMIT * 4 {
            self.evict_textures(|used| used != generation);
        }
    }

    fn evict_paragraphs(&mut self, drop_used: impl Fn(u64) -> bool) {
        let stale: Vec<ParaKey> = self
            .paragraphs
            .iter()
            .filter(|(_, shaped)| drop_used(shaped.used))
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(shaped) = self.paragraphs.remove(&key) {
                // SAFETY: a paragraph is evicted only after the display list that drew it
                // was released, or it was never installed.
                unsafe { ffi::ImpellerParagraphRelease(shaped.paragraph) };
            }
        }
    }

    fn evict_textures(&mut self, drop_used: impl Fn(u64) -> bool) {
        let stale: Vec<String> = self
            .textures
            .iter()
            .filter(|(_, cached)| drop_used(cached.used))
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(cached) = self.textures.remove(&key) {
                // SAFETY: a texture is evicted only after the display list that drew it
                // was released, or it was never installed.
                unsafe { ffi::ImpellerTextureRelease(cached.texture) };
            }
        }
    }

    fn release_frame(&mut self) {
        self.release_display();
        self.release_retired();
        self.release_paragraphs();
        self.release_textures();
    }

    #[cfg(test)]
    fn cached_textures(&self) -> usize {
        self.textures.len()
    }

    #[cfg(test)]
    fn cached_paragraphs(&self) -> usize {
        self.paragraphs.len()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.release_frame();
        // SAFETY: both handles came from the context constructor. Release is a no-op on null.
        // Font bytes are dropped after the typography context, matching the previous player.
        unsafe {
            ffi::ImpellerTypographyContextRelease(self.typography);
            ffi::ImpellerContextRelease(self.context);
        }
        #[cfg(target_os = "android")]
        if !self.vulkan_lib.is_null() {
            vulkan::close_library(self.vulkan_lib);
        }
    }
}

fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// SAFETY: `paragraph` is a live Impeller paragraph.
/// `ImpellerParagraphGetLineMetrics` returns the paragraph's cached metrics
/// without a new reference (`GetC`, not `Leak`). The paragraph releases that
/// object. Releasing it here frees the cache, and the next query frees the
/// same allocation.
unsafe fn read_lines(paragraph: ffi::ImpellerParagraph) -> Result<Vec<crate::LineMetric>, Status> {
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
            hard_break: unsafe { ffi::ImpellerLineMetricsIsHardbreak(metrics, line) },
        });
    }
    Ok(lines)
}

fn para_key(
    utf8: &str,
    font_size: f32,
    max_width: f32,
    color: [u8; 4],
    family: &str,
    style: &TextStyle,
    spans: &[TextSpan],
) -> Option<ParaKey> {
    if !font_size.is_finite() || font_size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
        return None;
    }
    Some(ParaKey {
        utf8: utf8.to_owned(),
        font_size: font_size.to_bits(),
        max_width: max_width.to_bits(),
        color,
        family: family.to_owned(),
        max_lines: style.max_lines.unwrap_or(0),
        ellipsis: style.ellipsis.clone().unwrap_or_default(),
        align: style.align,
        weight: style.weight,
        direction_rtl: style.direction_rtl,
        height_bits: style.height.to_bits(),
        underline: style.underline,
        line_through: style.line_through,
        soft_wrap: style.soft_wrap,
        overflow: style.overflow,
        spans: spans
            .iter()
            .map(|span| SpanKey {
                utf8: span.utf8.clone(),
                family: span.family.clone().unwrap_or_default(),
                weight: span.weight,
                color: span.color,
                underline: span.underline,
                line_through: span.line_through,
            })
            .collect(),
    })
}

fn apply_decoration(
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

unsafe fn push_runs(
    _engine: &Engine,
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

fn impeller_color(color: [u8; 4]) -> ffi::ImpellerColor {
    ffi::ImpellerColor {
        red: color[0] as f32 / 255.0,
        green: color[1] as f32 / 255.0,
        blue: color[2] as f32 / 255.0,
        alpha: color[3] as f32 / 255.0,
        color_space: ffi::ImpellerColorSpace::kImpellerColorSpaceSRGB,
    }
}

fn content_rects(
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
    }
}

fn gradient_solid(gradient: Option<&Gradient>) -> Option<[u8; 4]> {
    let Some(gradient) = gradient else {
        return None;
    };
    match prepared_gradient(gradient) {
        PreparedGradient::Solid(color) => Some(color),
        PreparedGradient::Paint { .. } => None,
        PreparedGradient::Invalid => None,
    }
}

enum PreparedGradient {
    Invalid,
    Solid([u8; 4]),
    Paint {
        kind: GradientKind,
        stops: Vec<ColorStop>,
    },
}

enum GradientKind {
    Linear { start: [f32; 2], end: [f32; 2] },
    Radial { center: [f32; 2], radius: f32 },
}

fn prepared_gradient(gradient: &Gradient) -> PreparedGradient {
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
    }
}

fn normalize_stops(stops: &[ColorStop]) -> Option<Vec<ColorStop>> {
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

fn color_source(
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
    }
}

fn rect_path(x: f32, y: f32, w: f32, h: f32, corner_radius: f32) -> ffi::ImpellerPath {
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

fn impeller_weight(weight: u16) -> ffi::ImpellerFontWeight::Type {
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

fn impeller_align(align: TextAlign) -> ffi::ImpellerTextAlignment::Type {
    match align {
        TextAlign::Left => ffi::ImpellerTextAlignment::kImpellerTextAlignmentLeft,
        TextAlign::Center => ffi::ImpellerTextAlignment::kImpellerTextAlignmentCenter,
        TextAlign::Right => ffi::ImpellerTextAlignment::kImpellerTextAlignmentRight,
    }
}

fn clamped_radius(radius: f32, width: f32, height: f32) -> f32 {
    if !radius.is_finite() || radius <= 0.0 || width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    radius.min(width.min(height) * 0.5)
}

/// Impeller calls this when it drops a font mapping. The bytes stay in `Engine::faces`.
unsafe extern "C" fn keep_font_bytes(_user_data: *mut c_void) {}

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

fn uniform_radii(radius: f32) -> ffi::ImpellerRoundingRadii {
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
    use protocol::{DrawList, scene_to_draw_list};

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
        assert_eq!(engine.submit(&list, 0.0, |_| None), Err(Status::BadScene));
    }

    #[test]
    fn counter_list_builds_a_display_list() {
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
        let list = scene_to_draw_list(&protocol::counter());
        let rgba = vec![0u8; 56 * 56 * 4];
        engine
            .submit(&list, 1.0, |source| {
                if source == "asset:logo" {
                    Some(ImagePixels {
                        width: 56,
                        height: 56,
                        rgba: &rgba,
                    })
                } else {
                    None
                }
            })
            .expect("counter draw list submits");
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
            .measure_paragraph(
                "Hi",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &protocol::TextStyle::default(),
            )
            .expect("measure");
        let second = engine
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
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("lines");
        let again = engine
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("cached lines");
        assert_eq!(lines.len(), again.len());
        assert!(!lines.is_empty());
        let caret = engine
            .caret_for("Hi", 28.0, 280.0, color, &style, 2, false)
            .expect("caret at end");
        assert!(caret.x > 0.0);
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: protocol::TextStyle::default(),
                spans: Vec::new(),
                links: Vec::new(),
            }],
        };
        engine
            .submit(&list, 1.0, |_| None)
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
            engine.submit(&list, 1.0, |_| {
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
        engine.submit(&empty, 1.0, |_| None).expect("empty");
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
            .submit(&list, 1.0, |_| {
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
        assert_eq!(engine.submit(&broken, 1.0, |_| None), Err(Status::BadScene));
        assert_eq!(engine.cached_textures(), 1);
        engine
            .submit(&list, 1.0, |_| {
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
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("measure");
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: style.clone(),
                spans: Vec::new(),
                links: Vec::new(),
            }],
        };
        engine.submit(&list, 1.0, |_| None).expect("submit");
        let second = engine
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("cached");
        assert_eq!(first, second);
        assert_eq!(engine.cached_paragraphs(), 1);
    }

    #[test]
    fn unused_paragraph_is_evicted() {
        let Some(mut engine) = fonted() else {
            return;
        };
        engine
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
        engine.submit(&list, 1.0, |_| None).expect("empty");
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
            .submit(&list, 2.0, |_| None)
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
            .submit(&list, 2.0, |_| None)
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
        assert_eq!(engine.submit(&list, 1.0, |_| None), Err(Status::BadScene));
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
        assert_eq!(engine.submit(&list, 1.0, |_| None), Err(Status::BadScene));
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
            .submit(&image_command(false), 1.0, |_| None)
            .expect("optional image");
        assert_eq!(
            engine.submit(&image_command(true), 1.0, |_| None),
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
                },
                protocol::Command::Pop,
            ],
        };
        engine
            .submit(&list, 2.0, |_| None)
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
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 700))
            .expect("first span key");
        engine
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 700))
            .expect("same span key");
        assert_eq!(engine.cached_paragraphs(), 1);
        engine
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("UIFWSans", 700))
            .expect("family changes the key");
        assert_eq!(engine.cached_paragraphs(), 2);
        engine
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 400))
            .expect("weight changes the key");
        assert_eq!(engine.cached_paragraphs(), 3);
        let mut struck = base.clone();
        struck.line_through = true;
        engine
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
}
