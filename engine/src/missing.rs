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
        _utf8: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
    ) -> Result<(f32, f32), Status> {
        Err(Status::NotLinked)
    }
}

#[cfg(target_os = "android")]
pub fn note_swapchain_extent(_width: u32, _height: u32) {}
