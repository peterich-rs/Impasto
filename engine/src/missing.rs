use crate::{ImagePixels, Status};
use protocol::LayerTree;

pub struct Typography;

pub struct Engine {
    typography: std::rc::Rc<std::cell::RefCell<Typography>>,
}

#[cfg(target_os = "android")]
pub struct VulkanSwapchain;

impl Engine {
    #[cfg(not(target_os = "android"))]
    pub fn create_metal() -> Result<Self, Status> {
        Err(Status::NotLinked)
    }

    pub fn register_font(&mut self, _data: &[u8], _family: &str) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

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
    ) -> Result<VulkanSwapchain, Status> {
        Err(Status::NotLinked)
    }

    #[cfg(target_os = "android")]
    pub fn draw_vulkan(&mut self, _swapchain: &VulkanSwapchain) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    pub fn typography(&self) -> std::rc::Rc<std::cell::RefCell<Typography>> {
        std::rc::Rc::clone(&self.typography)
    }
}

impl Typography {
    pub fn measure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> Result<(f32, f32), Status> {
        self.measure_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    pub fn measure_with_spans(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _spans: &[protocol::TextSpan],
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }

    pub fn caret_at_point(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _x: f32,
        _y: f32,
    ) -> Result<crate::CaretHit, Status> {
        Err(Status::NotLinked)
    }

    pub fn caret_at_unit(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _unit: u32,
    ) -> Result<crate::CaretHit, Status> {
        Err(Status::NotLinked)
    }

    pub fn word_range(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
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
        style: &protocol::TextStyle,
    ) -> Result<Vec<crate::LineMetric>, Status> {
        self.lines_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    pub fn lines_with_spans(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _spans: &[protocol::TextSpan],
    ) -> Result<Vec<crate::LineMetric>, Status> {
        Err(Status::NotLinked)
    }

    pub fn measure_paint(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _spans: &[protocol::TextSpan],
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }

    pub fn retain_measured(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _spans: &[protocol::TextSpan],
    ) -> Result<(protocol::ParagraphId, f32, f32, Vec<crate::LineMetric>, f32), Status> {
        Err(Status::NotLinked)
    }

    pub fn take_shape_stats(&mut self) -> crate::ShapeStats {
        crate::ShapeStats::default()
    }

    pub fn caret_for(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _unit: u32,
        _upstream: bool,
    ) -> Result<crate::CaretHit, Status> {
        Err(Status::NotLinked)
    }

    pub fn retain_paragraph(
        &mut self,
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        _spans: &[protocol::TextSpan],
    ) -> Result<protocol::ParagraphId, Status> {
        Err(Status::NotLinked)
    }

    pub fn release_paragraph(&mut self, _id: protocol::ParagraphId) {}

    pub fn paragraph_live(&self, _id: protocol::ParagraphId) -> bool {
        false
    }

    pub fn text_paint_probe(&self) -> (bool, [u8; 4]) {
        (false, [0, 0, 0, 0])
    }

    pub fn picture_builds(&self) -> u64 {
        0
    }

    pub fn cache_counts(&self) -> (usize, usize) {
        (0, 0)
    }

    pub fn submit_stats(&self) -> crate::SubmitStats {
        crate::SubmitStats::default()
    }
}

#[cfg(target_os = "android")]
pub fn note_swapchain_extent(_width: u32, _height: u32) {}

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
    fn draw_metal(&mut self, metal_drawable: *mut std::ffi::c_void) -> Result<(), Status> {
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
