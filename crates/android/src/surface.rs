//! Vulkan present. The swapchain stays across resizes and drops with this surface.

use engine::{Engine, VulkanSwapchain};
use hostshell::{NativePtr, Presented, Surface};

pub struct VulkanSurface {
    swapchain: Option<VulkanSwapchain>,
}

impl VulkanSurface {
    pub fn open(engine: &mut Engine, window: NativePtr) -> Result<Self, engine::Status> {
        let swapchain = engine.create_swapchain(window.0)?;
        Ok(Self {
            swapchain: Some(swapchain),
        })
    }
}

impl Surface for VulkanSurface {
    fn present(&mut self, engine: &mut Engine) -> Result<Presented, engine::Status> {
        let Some(swapchain) = self.swapchain.as_ref() else {
            return Err(engine::Status::NotReady);
        };
        match engine.draw_vulkan(swapchain) {
            Ok(()) => Ok(Presented::Continue),
            Err(status) => Err(status),
        }
    }
}
