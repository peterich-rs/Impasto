//! Impeller player. It paints a [`protocol::DrawList`] and does not read scene bytes.

mod script;
mod status;

pub use script::split_scripts;

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

/// A caret in paragraph space. `unit` is a UTF-16 index. `x` and `y` are
/// relative to the paragraph origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretHit {
    pub unit: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The caret sits on the previous line of a soft break.
    pub upstream: bool,
}

/// One paragraph line in the same space as [`CaretHit`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetric {
    pub start: u32,
    pub end: u32,
    pub end_with_break: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub hard_break: bool,
}

/// RGBA pixels borrowed until [`Engine::submit`] returns.
pub struct ImagePixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}
