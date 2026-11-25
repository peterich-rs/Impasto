#[cfg(not(any(target_os = "ios", target_os = "android")))]
use impasto::PluginContext;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
use crate::{CameraError, Facing};

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::Camera;

#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
pub use ios::Camera;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[derive(Clone)]
pub struct Camera;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
impl Camera {
    pub fn new(_: &PluginContext, _: Facing) -> Result<Self, CameraError> {
        Err(CameraError::unavailable())
    }

    pub async fn start(&self, _: u64, _: &str) -> Result<(), CameraError> {
        Err(CameraError::unavailable())
    }

    pub fn stop(&self, _: u64) {}

    pub fn resume(&self, _: u64, _: &str) {}
}
