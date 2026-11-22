//! Presentation for one Android native window.

#[cfg(feature = "impeller")]
use embedder::SurfaceTarget;
use embedder::{NativePtr, OpenSurface, Presented, Surface, SurfaceId};
use raster::{
    AcquireFollowup, PixelSize, Rasterizer, VulkanPhase, VulkanSwapchain, acquire_followup,
};

#[derive(Clone, Copy)]
pub(crate) enum Renderer {
    Auto,
    Vulkan,
    Gles,
}

#[cfg(feature = "impeller")]
#[derive(Default)]
struct VulkanProbe {
    target: Option<SurfaceId>,
    attempts: u8,
    opened: bool,
}

impl Renderer {
    pub fn create_raster(self) -> Result<Rasterizer, raster::Status> {
        match self {
            #[cfg(feature = "impeller")]
            Self::Auto if !Rasterizer::vulkan_has_safe_shutdown() => {
                tracing::warn!(target: "impasto::render", "Impeller SDK lacks standalone Vulkan shutdown, using GLES");
                create_gles()
            }
            Self::Auto => Rasterizer::create_vulkan().or_else(|error| {
                tracing::warn!(target: "impasto::render", %error, "Vulkan unavailable, trying GLES");
                create_gles()
            }),
            Self::Vulkan => Rasterizer::create_vulkan(),
            Self::Gles => create_gles(),
        }
    }

    pub fn surfaces(self, first_frame: impl Fn(SurfaceId) + Send + Sync + 'static) -> OpenSurface {
        let first_frame = std::sync::Arc::new(first_frame);
        #[cfg(feature = "impeller")]
        let mut probe = VulkanProbe::default();
        Box::new(move |engine, target| {
            let notify = std::sync::Arc::clone(&first_frame);
            let id = target.id;
            let notify = move || notify(id);
            #[cfg(feature = "impeller")]
            if engine.is_gles() {
                return open_gles(engine, target, notify);
            }
            match engine.create_swapchain(target.layer.0, target.pixels) {
                Ok(swapchain) => {
                    #[cfg(feature = "impeller")]
                    {
                        probe.opened = true;
                    }
                    tracing::info!(target: "impasto::render::surface", surface = id.0, backend = "vulkan", "Android surface opened");
                    Ok(Box::new(VulkanSurface::new(
                        target.layer,
                        swapchain,
                        notify,
                    )))
                }
                Err(error) => {
                    #[cfg(feature = "impeller")]
                    if matches!(self, Self::Auto) && !probe.opened && target.pixels.is_usable() {
                        if probe.target != Some(id) {
                            probe.target = Some(id);
                            probe.attempts = 0;
                        }
                        probe.attempts = probe.attempts.saturating_add(1);
                        if probe.attempts >= 3 || error == raster::Status::DeviceLost {
                            tracing::warn!(target: "impasto::render::surface", surface = id.0, %error, "Vulkan window could not open, trying GLES");
                            engine.use_gles()?;
                            return open_gles(engine, target, notify);
                        }
                    }
                    Err(error)
                }
            }
        })
    }
}

fn create_gles() -> Result<Rasterizer, raster::Status> {
    #[cfg(feature = "impeller")]
    {
        Rasterizer::create_gles()
    }
    #[cfg(not(feature = "impeller"))]
    {
        Err(raster::Status::NotLinked)
    }
}

#[cfg(feature = "impeller")]
fn open_gles(
    engine: &Rasterizer,
    target: SurfaceTarget,
    first_frame: impl FnOnce() + 'static,
) -> Result<Box<dyn Surface>, raster::Status> {
    let id = target.id;
    let surface = GlesSurface::open(engine, target, first_frame)?;
    tracing::info!(target: "impasto::render::surface", surface = id.0, backend = "gles", "Android surface opened");
    Ok(Box::new(surface))
}

#[cfg(feature = "impeller")]
struct GlesSurface {
    native: NativePtr,
    window: Option<raster::GlesWindow>,
    pixels: PixelSize,
    first_frame: Option<Box<dyn FnOnce()>>,
}

#[cfg(feature = "impeller")]
impl GlesSurface {
    fn open(
        engine: &Rasterizer,
        target: SurfaceTarget,
        first_frame: impl FnOnce() + 'static,
    ) -> Result<Self, raster::Status> {
        // SAFETY: the Android host owns this window until surface teardown is acknowledged.
        let window = unsafe { engine.create_gles_window(target.layer.0)? };
        Ok(Self {
            native: target.layer,
            window: Some(window),
            pixels: target.pixels,
            first_frame: Some(Box::new(first_frame)),
        })
    }
}

#[cfg(feature = "impeller")]
impl Surface for GlesSurface {
    fn recover_device(&mut self, engine: &mut Rasterizer) -> Result<(), raster::Status> {
        self.window = None;
        engine.recreate_graphics(Rasterizer::create_gles)
    }

    fn present(
        &mut self,
        engine: &mut Rasterizer,
        pixels: PixelSize,
    ) -> Result<Presented, raster::Status> {
        let _trace = engine::session::trace::Section::begin(c"Impasto GLES draw and swap");
        if pixels.is_usable() && pixels != self.pixels {
            self.window = None;
            self.pixels = pixels;
        }
        let window = match &mut self.window {
            Some(window) => window,
            // SAFETY: the host retains the same native window until this surface is destroyed.
            slot @ None => slot.insert(unsafe { engine.create_gles_window(self.native.0)? }),
        };
        let result = engine.draw_gles(window, self.pixels);
        if result == Err(raster::Status::SurfaceLost) {
            self.window = None;
        }
        result?;
        if let Some(notify) = self.first_frame.take() {
            notify();
        }
        Ok(Presented::Continue)
    }
}

struct VulkanSurface {
    window: NativePtr,
    swapchain: Option<VulkanSwapchain>,
    misses: u32,
    first_frame: Option<Box<dyn FnOnce()>>,
}

impl VulkanSurface {
    fn new(
        window: NativePtr,
        swapchain: VulkanSwapchain,
        first_frame: impl FnOnce() + 'static,
    ) -> Self {
        Self {
            window,
            swapchain: Some(swapchain),
            misses: 0,
            first_frame: Some(Box::new(first_frame)),
        }
    }
}

impl Surface for VulkanSurface {
    #[cfg(feature = "impeller")]
    fn recover_device(&mut self, engine: &mut Rasterizer) -> Result<(), raster::Status> {
        self.swapchain = None;
        self.misses = 0;
        engine.recreate_graphics(Rasterizer::create_vulkan)
    }

    fn present(
        &mut self,
        engine: &mut Rasterizer,
        pixels: PixelSize,
    ) -> Result<Presented, raster::Status> {
        // The standalone Impeller API has no equivalent of UpdateSurfaceSize.
        // Some drivers keep acquiring old-sized buffers after a window resize.
        if pixels.is_usable()
            && self
                .swapchain
                .as_ref()
                .is_some_and(|chain| chain.extent() != pixels)
        {
            self.swapchain = None;
            self.misses = 0;
        }
        // A transient rebuild failure leaves the slot vacant so the next present can retry.
        let swapchain = match &mut self.swapchain {
            Some(swapchain) => swapchain,
            slot @ None => slot.insert(engine.create_swapchain(self.window.0, pixels)?),
        };
        let drawn = engine.draw_vulkan_with_trace(swapchain, |phase| {
            engine::session::trace::Section::begin(match phase {
                VulkanPhase::Acquire => c"Impasto Vulkan acquire",
                VulkanPhase::Draw => c"Impasto Vulkan draw",
                VulkanPhase::Present => c"Impasto Vulkan present",
            })
        });
        match drawn {
            Ok(()) => {
                self.misses = 0;
                if let Some(notify) = self.first_frame.take() {
                    notify();
                }
                Ok(Presented::Continue)
            }
            Err(raster::Status::AcquireMiss) => {
                self.misses = self.misses.saturating_add(1);
                if acquire_followup(self.misses) == AcquireFollowup::Rebuild {
                    self.swapchain = None;
                    self.misses = 0;
                }
                Err(raster::Status::AcquireMiss)
            }
            Err(raster::Status::SurfaceLost) => {
                self.swapchain = None;
                self.misses = 0;
                Err(raster::Status::SurfaceLost)
            }
            Err(status) => {
                self.misses = 0;
                Err(status)
            }
        }
    }
}
