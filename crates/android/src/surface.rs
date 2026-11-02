//! Vulkan present. The swapchain stays across resizes and drops with this surface.

use engine::{AcquireFollowup, Engine, PixelSize, VulkanSwapchain, acquire_followup};
use hostshell::{NativePtr, Presented, Surface};

pub struct VulkanSurface {
    window: NativePtr,
    swapchain: Option<VulkanSwapchain>,
    misses: u32,
}

impl VulkanSurface {
    pub fn open(
        engine: &mut Engine,
        window: NativePtr,
        pixels: PixelSize,
    ) -> Result<Self, engine::Status> {
        let swapchain = engine.create_swapchain(window.0, pixels)?;
        Ok(Self {
            window,
            swapchain: Some(swapchain),
            misses: 0,
        })
    }

    fn miss(
        &mut self,
        engine: &mut Engine,
        pixels: PixelSize,
    ) -> Result<Presented, engine::Status> {
        self.misses = self.misses.saturating_add(1);
        if acquire_followup(self.misses) == AcquireFollowup::Rebuild {
            self.swapchain = None;
            self.misses = 0;
            let chain = engine.create_swapchain(self.window.0, pixels)?;
            self.swapchain = Some(chain);
        }
        Err(engine::Status::AcquireMiss)
    }
}

impl Surface for VulkanSurface {
    fn present(
        &mut self,
        engine: &mut Engine,
        pixels: PixelSize,
    ) -> Result<Presented, engine::Status> {
        let drawn = {
            let Some(swapchain) = self.swapchain.as_mut() else {
                return Err(engine::Status::NotReady);
            };
            swapchain.set_extent(pixels);
            engine.draw_vulkan(swapchain)
        };
        match drawn {
            Ok(()) => {
                self.misses = 0;
                Ok(Presented::Continue)
            }
            Err(engine::Status::AcquireMiss) => self.miss(engine, pixels),
            Err(status) => Err(status),
        }
    }
}
