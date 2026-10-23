//! Impeller player. It paints a [`protocol::DrawList`] and does not read scene bytes.

mod status;

#[cfg(impeller_missing)]
mod missing;
#[cfg(impeller)]
mod player;

pub use status::Status;

#[cfg(impeller_missing)]
pub use missing::Engine;
#[cfg(impeller)]
pub use player::Engine;

#[cfg(all(impeller_missing, target_os = "android"))]
pub use missing::VulkanSwapchain;
#[cfg(all(impeller_missing, target_os = "android"))]
pub use missing::note_swapchain_extent;
#[cfg(all(impeller, target_os = "android"))]
pub use player::VulkanSwapchain;
#[cfg(all(impeller, target_os = "android"))]
pub use player::note_swapchain_extent;

/// RGBA pixels borrowed until [`Engine::submit`] returns.
pub struct ImagePixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}
