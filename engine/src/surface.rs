//! Compile-time contract shared by the Impeller player and the `NotLinked` stub.

use std::ffi::c_void;

use protocol::LayerTree;

use crate::{ImagePixels, Status, SubmitStats};

/// Operations the host shell calls on the concrete [`crate::Engine`].
///
/// Not used as `dyn Surface`. Host code keeps the cfg-selected `Engine`.
pub trait Surface {
    fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status>;

    fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status>;

    #[cfg(not(target_os = "android"))]
    fn draw_metal(&mut self, metal_drawable: *mut c_void) -> Result<(), Status>;

    fn picture_builds(&self) -> u64;

    fn cache_counts(&self) -> (usize, usize);

    fn submit_stats(&self) -> SubmitStats;
}
