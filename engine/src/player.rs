//! Paints a draw list. Text uses the font size and paragraph width stored on each command.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::marker::PhantomData;
use std::ptr;

use protocol::{Command, DrawList, TextAlign, TextStyle};

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
            on_release: None,
        };
        // SAFETY: `mapping.data` points at `owned`, which this function keeps on success.
        // The family pointer stays in `self.family`. RegisterFont returns false on failure
        // and does not take ownership when `on_release` is null.
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
        // The previous display list is released before any cached object it might reference.
        self.release_display();
        self.release_retired();
        if self.paragraphs_stale {
            self.release_paragraphs();
            self.paragraphs_stale = false;
        }
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
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.paint_color(paint, *color);
                    self.draw_rect(builder, paint, *x, *y, *w, *h, *corner_radius);
                    true
                }
                Command::Text {
                    x,
                    y,
                    max_width,
                    font_size,
                    color,
                    utf8,
                    style,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_text(
                        builder, paint, *x, *y, *max_width, *font_size, *color, utf8, style,
                    )
                }
                Command::Image { x, y, w, h, source } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.draw_image(builder, *x, *y, *w, *h, source, &mut resolve)
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
                    scale,
                } => {
                    scaled = self.scale_once(builder, point_scale, scaled);
                    self.push_transform(builder, *translate_x, *translate_y, *rotate_rad, *scale);
                    true
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
            self.evict_over_cap();
            return Err(Status::BadScene);
        }
        self.list = display;
        self.evict_unused();
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
        if utf8.is_empty() {
            return Err(Status::BadScene);
        }
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, style) else {
            return Err(Status::BadScene);
        };
        if !self.paragraphs_stale {
            if let Some(found) = self.paragraphs.get_mut(&key) {
                found.used = self.list_generation;
                return Ok((found.width, found.height));
            }
        }
        let mut shaped = self.build_paragraph(utf8, font_size, max_width, color, style)?;
        shaped.used = self.list_generation;
        let size = (shaped.width, shaped.height);
        self.store_paragraph(key, shaped);
        Ok(size)
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
            // `rect` and `radii` are borrowed for the call.
            unsafe {
                ffi::ImpellerDisplayListBuilderDrawRoundedRect(builder, &rect, &radii, paint);
            }
        } else {
            // SAFETY: same builder and paint as the rounded path.
            unsafe { ffi::ImpellerDisplayListBuilderDrawRect(builder, &rect, paint) };
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
        scale: f32,
    ) {
        let degrees = rotate_rad * (180.0 / std::f32::consts::PI);
        // SAFETY: `builder` lives until this submit releases it. Scale already ran, so the
        // saved transform includes `point_scale`. Translate, then clockwise rotation, then
        // uniform scale, matches the command order.
        unsafe {
            ffi::ImpellerDisplayListBuilderSave(builder);
            ffi::ImpellerDisplayListBuilderTranslate(builder, translate_x, translate_y);
            ffi::ImpellerDisplayListBuilderRotate(builder, degrees);
            ffi::ImpellerDisplayListBuilderScale(builder, scale, scale);
        }
        self.saves.push(Save::Transform);
    }

    fn pop_save(&mut self, builder: ffi::ImpellerDisplayListBuilder) -> bool {
        if self.saves.is_empty() {
            return false;
        }
        // SAFETY: `saves` only grows on Save or SaveLayer for this builder, so a pop has a
        // frame to restore. A layer paint is released after Restore, which is when it is read.
        unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
        if let Some(Save::Layer(layer_paint)) = self.saves.pop() {
            unsafe { ffi::ImpellerPaintRelease(layer_paint) };
        }
        true
    }

    fn drain_saves(&mut self, builder: ffi::ImpellerDisplayListBuilder) {
        while let Some(save) = self.saves.pop() {
            // SAFETY: each entry was pushed with Save or SaveLayer on this builder, and the
            // builder has not been released. Restore reads a layer paint before we release it.
            unsafe { ffi::ImpellerDisplayListBuilderRestore(builder) };
            if let Save::Layer(layer_paint) = save {
                unsafe { ffi::ImpellerPaintRelease(layer_paint) };
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
    ) -> bool {
        if utf8.is_empty() {
            return false;
        }
        let Some(family) = self.resolved_family(text_style.family.as_deref()) else {
            return false;
        };
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, text_style) else {
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
                self.build_paragraph(utf8, font_size, max_width, color, text_style)
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
    ) -> Result<Shaped, Status> {
        // SAFETY: the paint is released before this returns. A null paint cannot shape.
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return Err(Status::BadScene);
        }
        self.paint_color(paint, color);
        let paragraph = self.shape(Some(paint), utf8, font_size, max_width, text_style);
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

    fn shape(
        &self,
        paint: Option<ffi::ImpellerPaint>,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        text_style: &TextStyle,
    ) -> Option<ffi::ImpellerParagraph> {
        let family = self.resolved_face(text_style.family.as_deref())?;
        let family = family.as_ptr();
        let ellipsis = match &text_style.ellipsis {
            Some(text) => Some(CString::new(text.as_str()).ok()?),
            None => None,
        };
        let len = u32::try_from(utf8.len()).ok()?;
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
            if let Some(max_lines) = text_style.max_lines.filter(|lines| *lines > 0) {
                ffi::ImpellerParagraphStyleSetMaxLines(style, max_lines);
            }
            if text_style.height.is_finite() && text_style.height > 0.0 {
                ffi::ImpellerParagraphStyleSetHeight(style, text_style.height);
            }
            if let Some(ellipsis) = &ellipsis {
                ffi::ImpellerParagraphStyleSetEllipsis(style, ellipsis.as_ptr());
            }
            ffi::ImpellerParagraphBuilderPushStyle(paragraph_builder, style);
            ffi::ImpellerParagraphBuilderAddText(paragraph_builder, utf8.as_ptr(), len);
            ffi::ImpellerParagraphBuilderPopStyle(paragraph_builder);
            let paragraph =
                ffi::ImpellerParagraphBuilderBuildParagraphNew(paragraph_builder, max_width);
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
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> bool {
        if let Some(cached) = self.textures.get_mut(source) {
            cached.used = self.list_generation;
            let texture = cached.texture;
            let width = cached.width;
            let height = cached.height;
            self.blit_texture(builder, texture, x, y, w, h, width, height);
            return true;
        }
        let Some(image) = resolve(source) else {
            return false;
        };
        if image.width == 0 || image.height == 0 {
            return false;
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
        self.blit_texture(builder, texture, x, y, w, h, width, height);
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
    ) {
        let src = ffi::ImpellerRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        let dst = ffi::ImpellerRect {
            x,
            y,
            width: w,
            height: h,
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
            if let Save::Layer(paint) = save {
                // SAFETY: `submit` restores layer paints before releasing the builder.
                // Paints still in `saves` here belong to a submit that did not finish.
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

fn para_key(
    utf8: &str,
    font_size: f32,
    max_width: f32,
    color: [u8; 4],
    family: &str,
    style: &TextStyle,
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
    })
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
        let list = DrawList {
            commands: vec![protocol::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                color: [255, 255, 255, 255],
                utf8: "Hi".to_owned(),
                style: protocol::TextStyle::default(),
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
                utf8: "Hi".to_owned(),
                style: style.clone(),
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
                source: "asset:logo".to_owned(),
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
                    scale: 1.5,
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
}
