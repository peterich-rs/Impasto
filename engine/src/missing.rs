use crate::{ImagePixels, Status};
use protocol::DrawList;

pub struct Engine;

#[cfg(target_os = "android")]
pub struct VulkanSwapchain;

impl Engine {
    pub fn create_metal() -> Result<Self, Status> {
        Err(Status::NotLinked)
    }

    pub fn register_font(&mut self, _data: &[u8], _family: &str) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

    pub fn submit<'pixels>(
        &mut self,
        _list: &DrawList,
        _point_scale: f32,
        _resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        Err(Status::NotLinked)
    }

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
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
    ) -> Result<Vec<crate::LineMetric>, Status> {
        Err(Status::NotLinked)
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
}

#[cfg(target_os = "android")]
pub fn note_swapchain_extent(_width: u32, _height: u32) {}
