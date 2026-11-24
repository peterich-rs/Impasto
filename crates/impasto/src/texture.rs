use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Straight-alpha, tightly packed sRGB RGBA8 pixels.
pub struct ExternalFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[doc(hidden)]
pub enum TextureFrame {
    Rgba(ExternalFrame),
    #[cfg(target_os = "ios")]
    Metal {
        texture: MetalTexture,
        fallback: ExternalFrame,
    },
}

#[cfg(target_os = "ios")]
#[derive(Clone)]
pub struct MetalTexture(Arc<MetalOwner>);

#[cfg(target_os = "ios")]
struct MetalOwner {
    address: usize,
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

#[cfg(target_os = "ios")]
impl Drop for MetalOwner {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

#[cfg(target_os = "ios")]
impl MetalTexture {
    /// Wraps a ready BGRA8 Metal texture published by a platform producer.
    ///
    /// # Safety
    /// The address must name a live BGRA8 Metal texture. The producer must
    /// finish all writes before publication and keep the texture and its
    /// backing storage immutable until the owned release callback runs.
    #[expect(
        unsafe_code,
        reason = "the platform producer establishes the Metal lifetime and synchronization contract"
    )]
    pub unsafe fn from_raw(
        texture: std::ptr::NonNull<std::ffi::c_void>,
        release: impl FnOnce() + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(MetalOwner {
            address: texture.as_ptr().expose_provenance(),
            release: Some(Box::new(release)),
        }))
    }

    #[doc(hidden)]
    pub fn as_ptr(&self) -> *const std::ffi::c_void {
        std::ptr::with_exposed_provenance(self.0.address)
    }
}

type Post = Arc<dyn Fn(&str, Option<TextureFrame>) -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExternalTextureRegistry {
    post: Option<Post>,
    closed: Arc<AtomicBool>,
}

impl ExternalTextureRegistry {
    #[doc(hidden)]
    pub fn new(post: impl Fn(&str, Option<TextureFrame>) -> bool + Send + Sync + 'static) -> Self {
        Self {
            post: Some(Arc::new(post)),
            closed: Arc::default(),
        }
    }

    pub fn allocate(&self) -> Result<ExternalTexture, TextureError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if self.post.is_none() || self.closed.load(Ordering::Acquire) {
            return Err(TextureError::Unavailable);
        }
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| TextureError::Unavailable)?;
        Ok(ExternalTexture {
            name: format!("native_{id:x}"),
            registry: self.clone(),
        })
    }

    #[doc(hidden)]
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

pub struct ExternalTexture {
    name: String,
    registry: ExternalTextureRegistry,
}

impl ExternalTexture {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn publish_rgba(&self, frame: ExternalFrame) -> Result<(), TextureError> {
        Self::validate(&frame)?;
        self.publish(TextureFrame::Rgba(frame))
    }

    #[cfg(target_os = "ios")]
    pub fn publish_metal(
        &self,
        texture: MetalTexture,
        fallback: ExternalFrame,
    ) -> Result<(), TextureError> {
        Self::validate(&fallback)?;
        self.publish(TextureFrame::Metal { texture, fallback })
    }

    fn validate(frame: &ExternalFrame) -> Result<(), TextureError> {
        let size = u64::from(frame.width) * u64::from(frame.height);
        if size == 0 || size > 2_073_600 || u64::try_from(frame.pixels.len()) != Ok(size * 4) {
            Err(TextureError::InvalidFrame)
        } else {
            Ok(())
        }
    }

    fn publish(&self, frame: TextureFrame) -> Result<(), TextureError> {
        if let Some(post) = &self.registry.post
            && !self.registry.closed.load(Ordering::Acquire)
            && post(&self.name, Some(frame))
        {
            Ok(())
        } else {
            Err(TextureError::Unavailable)
        }
    }

    pub fn clear(&self) {
        if let Some(post) = &self.registry.post
            && !self.registry.closed.load(Ordering::Acquire)
        {
            post(&self.name, None);
        }
    }
}

impl Drop for ExternalTexture {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug)]
pub enum TextureError {
    Unavailable,
    InvalidFrame,
}

impl std::fmt::Display for TextureError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => out.write_str("the texture host is unavailable"),
            Self::InvalidFrame => out.write_str("invalid RGBA frame geometry"),
        }
    }
}

impl std::error::Error for TextureError {}
