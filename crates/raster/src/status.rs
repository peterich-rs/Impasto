//! Failure states of the wgpu raster queue.
//!
//! Hosts map these onto their present and recovery paths. A `Status` never
//! carries a message: the raster logs context before returning one.

use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    #[error("scene is not drawable")]
    BadScene,
    #[error("frame is not ready")]
    NotReady,
    #[error("present failed")]
    PresentFailed,
    #[error("the window surface was lost")]
    SurfaceLost,
    #[error("the graphics device was lost")]
    DeviceLost,
    /// The drawable was not acquired. Skip this present. The surface stays.
    #[error("acquire missed")]
    AcquireMiss,
    #[error("graphics memory exhausted")]
    OutOfMemory,
    /// Surface is occluded.
    #[error("surface is occluded")]
    Occluded,
    /// Frame glyph resources exceeded the synchronous budget and are preparing asynchronously.
    #[error("frame resources are preparing")]
    ResourcesPending,
    /// No adapter or device could be created for the requested backend.
    #[error("no graphics device: {0:?}")]
    NoDevice(RasterBackend),
}

impl Status {
    /// Errors that keep the surface and retry on a later frame.
    pub const fn is_retry(self) -> bool {
        matches!(
            self,
            Self::PresentFailed
                | Self::SurfaceLost
                | Self::NotReady
                | Self::AcquireMiss
                | Self::OutOfMemory
        )
    }
}

/// Graphics API behind one [`crate::Rasterizer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RasterBackend {
    Metal,
    Vulkan,
    Gles,
    Dx12,
}

impl RasterBackend {
    /// Backends a platform tries when the caller did not force one, in order.
    pub fn candidates() -> &'static [Self] {
        #[cfg(target_os = "android")]
        {
            &[Self::Vulkan, Self::Gles]
        }
        #[cfg(all(target_vendor = "apple", not(target_os = "android")))]
        {
            &[Self::Metal]
        }
        #[cfg(all(target_os = "windows", not(target_vendor = "apple")))]
        {
            &[Self::Dx12, Self::Vulkan]
        }
        #[cfg(not(any(target_os = "android", target_vendor = "apple", target_os = "windows")))]
        {
            &[Self::Vulkan, Self::Gles]
        }
    }

    pub fn backends(self) -> wgpu::Backends {
        match self {
            Self::Metal => wgpu::Backends::METAL,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Gles => wgpu::Backends::GL,
            Self::Dx12 => wgpu::Backends::DX12,
        }
    }

    pub fn from_adapter(adapter: &wgpu::Adapter) -> Option<Self> {
        let backend = adapter.get_info().backend;
        Some(match backend {
            wgpu::Backend::Metal => Self::Metal,
            wgpu::Backend::Vulkan => Self::Vulkan,
            wgpu::Backend::Gl => Self::Gles,
            wgpu::Backend::Dx12 => Self::Dx12,
            _ => return None,
        })
    }
}

/// Error handling for hot frame encoding and presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValidationMode {
    #[default]
    Auto,
    Strict,
    AsyncOnly,
}

/// Options for [`crate::Rasterizer::create`] and [`crate::Rasterizer::headless`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterOptions {
    /// `None` tries the platform's default backends in order.
    pub backend: Option<RasterBackend>,
    /// Prefer a software or integrated adapter. Headless tests use this to
    /// avoid depending on a discrete GPU.
    pub force_fallback_adapter: bool,
    /// Request pass timestamp support when creating the device.
    pub enable_gpu_timing: bool,
    pub validation: ValidationMode,
}

#[cfg(test)]
mod tests {
    use super::RasterBackend;

    #[test]
    fn candidates_start_with_the_platform_default() {
        let first = RasterBackend::candidates()[0];
        if cfg!(target_vendor = "apple") && !cfg!(target_os = "android") {
            assert_eq!(first, RasterBackend::Metal);
        } else if cfg!(target_os = "android") {
            assert_eq!(first, RasterBackend::Vulkan);
        }
    }
}
