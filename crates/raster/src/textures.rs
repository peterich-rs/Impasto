//! Image textures keyed by source, and the `io`-thread upload factory.
//!
//! `GpuImage` is created on any thread from a clone of the device and
//! queue. The raster adopts it into its cache; a result from a previous
//! device generation is rejected.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::gpu::Gpu;

static NEXT_UPLOAD: AtomicU64 = AtomicU64::new(1);

/// An uploaded texture. `Send + Sync` so the `io` thread hands it over.
#[derive(Clone)]
pub struct GpuImage {
    view: Arc<wgpu::TextureView>,
    pub(crate) material: Arc<crate::pipelines::MaterialCache>,
    pub width: u32,
    pub height: u32,
    /// Device generation the texture belongs to.
    pub device: u64,
    /// Bumped when the producer publishes a new frame for the same source.
    pub revision: u64,
    /// Producer pixels are premultiplied in sRGB before upload.
    pub(crate) premultiplied_srgb: bool,
}

impl std::fmt::Debug for GpuImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GpuImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl GpuImage {
    /// Copies straight-alpha RGBA8 sRGB pixels into a new texture.
    pub(crate) fn upload(
        gpu: &Gpu,
        device: u64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<Self> {
        if gpu.is_lost() || width == 0 || height == 0 || width.max(height) > gpu.max_texture_size()
        {
            return None;
        }
        let bytes = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if rgba.len() < bytes {
            return None;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels = crate::color::premultiply_rgba(&rgba[..bytes]);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        Some(Self {
            view: Arc::new(view),
            material: Arc::default(),
            width,
            height,
            device,
            revision: NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed),
            premultiplied_srgb: false,
        })
    }

    #[cfg(target_os = "ios")]
    pub(crate) fn from_metal(
        texture: wgpu::Texture,
        pixels: crate::PixelSize,
        device: u64,
    ) -> Self {
        let view = texture.create_view(&Default::default());
        Self {
            view: Arc::new(view),
            material: Arc::default(),
            width: pixels.width,
            height: pixels.height,
            device,
            revision: NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed),
            premultiplied_srgb: true,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn width(&self) -> f32 {
        self.width as f32
    }

    pub fn height(&self) -> f32 {
        self.height as f32
    }
}

/// Device access for background uploads. `Send + Sync`.
#[derive(Clone)]
pub struct TextureFactory {
    gpu: Gpu,
    device_generation: u64,
}

impl TextureFactory {
    pub(crate) fn new(gpu: &Gpu, device_generation: u64) -> Self {
        Self {
            gpu: gpu.clone(),
            device_generation,
        }
    }

    /// Copies straight-alpha RGBA8 sRGB pixels into a new texture.
    pub fn upload_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> Option<GpuImage> {
        GpuImage::upload(&self.gpu, self.device_generation, width, height, rgba)
    }

    pub fn device_generation(&self) -> u64 {
        self.device_generation
    }
}

/// Result of a cache lookup for the compiler.
pub enum Load {
    Ready { width: f32, height: f32 },
    Missing,
    Empty,
    Failed,
}

#[derive(Clone)]
struct Cached {
    image: GpuImage,
    used: u64,
}

/// Image textures the raster adopted this device generation.
#[derive(Clone, Default)]
pub struct TextureStore {
    map: HashMap<String, Cached>,
    generation: u64,
    device: u64,
}

impl TextureStore {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn same_geometry(&self, source: &str, previous: &Self) -> bool {
        let size = |store: &Self| store.get(source).map(|image| (image.width, image.height));
        size(self) == size(previous)
    }

    pub fn touch(&mut self, source: &str) {
        if let Some(cached) = self.map.get_mut(source) {
            cached.used = self.generation;
        }
    }

    /// Looks up `source`, uploading pixels on a miss. `resolve` borrows
    /// the decoded pixels for the duration of the call.
    pub fn load<'a>(
        &mut self,
        gpu: &Gpu,
        source: &str,
        resolve: &mut dyn FnMut(&str) -> Option<crate::ImagePixels<'a>>,
    ) -> Load {
        if let Some(cached) = self.map.get_mut(source) {
            cached.used = self.generation;
            return Load::Ready {
                width: cached.image.width(),
                height: cached.image.height(),
            };
        }
        let Some(image) = resolve(source) else {
            return if source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX) {
                Load::Empty
            } else {
                Load::Missing
            };
        };
        if image.width == 0 || image.height == 0 {
            return Load::Empty;
        }
        match GpuImage::upload(gpu, self.device, image.width, image.height, image.rgba) {
            Some(uploaded) => self.insert(source, uploaded),
            None => Load::Failed,
        }
    }

    fn insert(&mut self, source: &str, image: GpuImage) -> Load {
        let load = Load::Ready {
            width: image.width(),
            height: image.height(),
        };
        self.map.insert(
            source.to_owned(),
            Cached {
                image,
                used: self.generation,
            },
        );
        load
    }

    /// Adopts a texture uploaded on another thread. Stale generations and
    /// older revisions are rejected.
    pub fn install(&mut self, source: &str, image: GpuImage) -> bool {
        if image.device != self.device {
            return false;
        }
        if let Some(cached) = self.map.get(source)
            && cached.image.revision >= image.revision
        {
            return false;
        }
        self.insert(source, image);
        true
    }

    pub fn get(&self, source: &str) -> Option<&GpuImage> {
        self.map.get(source).map(|cached| &cached.image)
    }

    pub fn forget(&mut self, source: &str) {
        self.map.remove(source);
    }

    pub fn begin_frame(&mut self, device: u64) {
        self.generation = self.generation.wrapping_add(1);
        self.device = device;
    }

    pub fn evict_unused(&mut self) {
        let generation = self.generation;
        self.map.retain(|_, cached| cached.used == generation);
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}
