use crate::{ImagePixels, Status};
use engine::scene::LayerTree;

pub struct Typography;

/// No GPU texture. Upload is unavailable when Impeller is not linked.
#[derive(Debug)]
pub struct GpuImage {
    pub width: u32,
    pub height: u32,
}

/// Placeholder. [`Rasterizer::texture_factory`] is always `None`.
pub struct TextureFactory;

impl TextureFactory {
    pub fn upload_rgba(&self, _width: u32, _height: u32, _rgba: &[u8]) -> Option<GpuImage> {
        None
    }
}

pub struct Rasterizer;

#[cfg(target_os = "android")]
pub struct VulkanSwapchain;

#[cfg(target_os = "android")]
impl VulkanSwapchain {
    pub fn extent(&self) -> crate::PixelSize {
        crate::PixelSize::default()
    }

    pub fn set_extent(&mut self, _size: crate::PixelSize) {}
}

impl Rasterizer {
    pub fn invalidate_pictures(&mut self) {}

    pub fn forget_texture(&mut self, _source: &str) {}

    #[cfg(not(target_os = "android"))]
    pub fn create_metal() -> Result<Self, Status> {
        Err(Status::NotLinked)
    }

    /// Rasterizer that cannot submit or present. Hostshell tests use this when they do not draw.
    pub fn inert() -> Self {
        Self
    }

    pub fn register_shared_font(
        &mut self,
        _data: std::sync::Arc<[u8]>,
        _family: &str,
    ) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    pub fn register_font(&mut self, _data: &[u8], _family: &str) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    /// No GPU objects exist on the stub. Drawing stays [`Status::NotLinked`].
    pub fn trim(&mut self) {}

    pub fn texture_factory(&self) -> Option<TextureFactory> {
        None
    }

    pub fn typography(&self) -> std::rc::Rc<std::cell::RefCell<Typography>> {
        std::rc::Rc::new(std::cell::RefCell::new(Typography))
    }

    pub fn install_uploaded(&mut self, _source: &str, _image: &GpuImage) {}

    pub fn submit_layers<'pixels>(
        &mut self,
        _tree: &LayerTree,
        _point_scale: f32,
        _resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    #[cfg(not(target_os = "android"))]
    pub fn draw_metal(&mut self, _metal_drawable: *mut std::ffi::c_void) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    #[cfg(target_os = "android")]
    pub fn create_vulkan() -> Result<Self, Status> {
        Err(Status::NotLinked)
    }

    #[cfg(target_os = "android")]
    pub fn create_swapchain(
        &self,
        _native_window: *mut std::ffi::c_void,
        _pixels: crate::PixelSize,
    ) -> Result<VulkanSwapchain, Status> {
        Err(Status::NotLinked)
    }

    #[cfg(target_os = "android")]
    pub fn draw_vulkan(&mut self, swapchain: &VulkanSwapchain) -> Result<(), Status> {
        self.draw_vulkan_with_trace(swapchain, |_| ())
    }

    #[cfg(target_os = "android")]
    pub fn draw_vulkan_with_trace<G>(
        &mut self,
        _swapchain: &VulkanSwapchain,
        _section: impl FnMut(crate::VulkanPhase) -> G,
    ) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    pub fn take_shape_stats(&mut self) -> crate::ShapeStats {
        crate::ShapeStats::default()
    }

    pub fn picture_builds(&self) -> u64 {
        0
    }

    pub fn submit_stats(&self) -> crate::SubmitStats {
        crate::SubmitStats::default()
    }
}

impl Typography {
    pub(crate) fn discard_measure_cache(&mut self) {}

    pub fn standalone() -> Result<Self, Status> {
        Err(Status::NotLinked)
    }

    pub fn register_shared_face(
        &mut self,
        _data: std::sync::Arc<[u8]>,
        _family: &str,
    ) -> Result<bool, Status> {
        Err(Status::NotLinked)
    }

    pub fn begin_measure(&mut self) -> u64 {
        0
    }

    pub fn end_measure(&mut self) {}

    pub fn register_face(&mut self, _data: &[u8], _family: &str) -> Result<bool, Status> {
        Err(Status::NotLinked)
    }

    pub(crate) fn measure_key(
        &mut self,
        _key: &engine::text::MetricsKey,
        _color: [u8; 4],
        _paints: Option<&[engine::scene::SpanPaint]>,
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }

    pub(crate) fn lines_key(
        &mut self,
        _key: &engine::text::MetricsKey,
        _color: [u8; 4],
        _paints: Option<&[engine::scene::SpanPaint]>,
    ) -> Result<Vec<engine::text::TextLine>, Status> {
        Err(Status::NotLinked)
    }

    pub(crate) fn touch_key(&mut self, _key: &engine::text::PaintKey) -> bool {
        false
    }

    pub(crate) fn min_intrinsic_key(&self, _key: &engine::text::MetricsKey) -> Result<f32, Status> {
        Err(Status::NotLinked)
    }

    pub(crate) fn measured_min_intrinsic_key(
        &mut self,
        _key: &engine::text::PaintKey,
    ) -> Result<f32, Status> {
        Err(Status::NotLinked)
    }

    pub fn touch_measure(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> bool {
        false
    }

    pub fn measure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &engine::scene::TextStyle,
    ) -> Result<(f32, f32), Status> {
        self.measure_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    pub fn measure_with_spans(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }

    pub fn min_intrinsic_width(
        &self,
        _utf8: &str,
        _font_size: f32,
        _text_style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<f32, Status> {
        Err(Status::NotLinked)
    }

    pub fn measured_min_intrinsic_width(
        &mut self,
        _query: engine::text::ParagraphQuery<'_>,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<f32, Status> {
        Err(Status::NotLinked)
    }

    pub fn caret_at_point(
        &mut self,
        _query: engine::text::ParagraphQuery<'_>,
        _x: f32,
        _y: f32,
    ) -> Result<engine::text::Caret, Status> {
        Err(Status::NotLinked)
    }

    pub fn caret_at_unit(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _unit: u32,
    ) -> Result<engine::text::Caret, Status> {
        Err(Status::NotLinked)
    }

    pub fn word_range(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _unit: u32,
    ) -> Result<(u32, u32), Status> {
        Err(Status::NotLinked)
    }

    pub fn lines(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &engine::scene::TextStyle,
    ) -> Result<Vec<engine::text::TextLine>, Status> {
        self.lines_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    pub fn lines_with_spans(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<Vec<engine::text::TextLine>, Status> {
        Err(Status::NotLinked)
    }

    pub fn measure_paint(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }

    pub fn retain_measured(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<
        (
            engine::scene::ParagraphId,
            f32,
            f32,
            Vec<engine::text::TextLine>,
            f32,
        ),
        Status,
    > {
        Err(Status::NotLinked)
    }

    pub fn take_shape_stats(&mut self) -> crate::ShapeStats {
        crate::ShapeStats::default()
    }

    pub fn shape_stats(&self) -> crate::ShapeStats {
        crate::ShapeStats::default()
    }

    pub fn caret_for(
        &mut self,
        _query: engine::text::ParagraphQuery<'_>,
        _unit: u32,
        _upstream: bool,
    ) -> Result<engine::text::Caret, Status> {
        Err(Status::NotLinked)
    }

    pub fn retain_paragraph(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &engine::scene::TextStyle,
        _spans: &[engine::scene::TextSpan],
    ) -> Result<engine::scene::ParagraphId, Status> {
        Err(Status::NotLinked)
    }

    pub fn release_paragraph(&mut self, _id: engine::scene::ParagraphId) {}

    pub fn paragraph_live(&self, _id: engine::scene::ParagraphId) -> bool {
        false
    }

    pub fn text_paint_probe(&self) -> (bool, [u8; 4]) {
        (false, [0, 0, 0, 0])
    }
}

impl crate::Surface for Rasterizer {
    fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        Rasterizer::register_font(self, data, family)
    }

    fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        Rasterizer::submit_layers(self, tree, point_scale, resolve)
    }

    #[cfg(not(target_os = "android"))]
    fn draw_metal(&mut self, metal_drawable: *mut std::ffi::c_void) -> Result<(), Status> {
        Rasterizer::draw_metal(self, metal_drawable)
    }

    fn picture_builds(&self) -> u64 {
        Rasterizer::picture_builds(self)
    }

    fn submit_stats(&self) -> crate::SubmitStats {
        Rasterizer::submit_stats(self)
    }
}
