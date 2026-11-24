//! Image textures keyed by source, and the `io`-thread upload factory.
//!
//! `GpuImage` is created on any thread from a clone of the device and
//! queue. The raster adopts it into its cache; a result from a previous
//! device generation is rejected.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{ImageUploadStats, gpu::Gpu};

static NEXT_UPLOAD: AtomicU64 = AtomicU64::new(1);

/// An uploaded texture. `Send + Sync` so the `io` thread hands it over.
#[derive(Clone)]
pub struct GpuImage {
    view: Arc<wgpu::TextureView>,
    #[cfg(target_os = "android")]
    nv12: Option<Nv12Planes>,
    pub(crate) material: Arc<crate::pipelines::MaterialCache>,
    pub width: u32,
    pub height: u32,
    /// Device generation the texture belongs to.
    pub device: u64,
    /// Bumped when the producer publishes a new frame for the same source.
    pub revision: u64,
    /// Producer pixels are premultiplied in sRGB before upload.
    pub(crate) premultiplied_srgb: bool,
    pub upload: ImageUploadStats,
}

#[derive(Clone)]
pub(crate) struct Nv12Planes {
    pub chroma: wgpu::TextureView,
    pub conversion: wgpu::Buffer,
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
    pub(crate) fn from_rendered(texture: wgpu::Texture, device: u64) -> Self {
        Self {
            view: Arc::new(texture.create_view(&Default::default())),
            #[cfg(target_os = "android")]
            nv12: None,
            material: Arc::default(),
            width: texture.width(),
            height: texture.height(),
            device,
            revision: NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed),
            premultiplied_srgb: false,
            upload: ImageUploadStats::default(),
        }
    }

    /// Copies straight-alpha RGBA8 sRGB pixels into a new texture.
    pub(crate) fn upload(
        gpu: &Gpu,
        device: u64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<Self> {
        let (image, command) = Self::prepare(gpu, device, width, height, rgba)?;
        if let Some(command) = command {
            gpu.validation(|| {
                gpu.submit([command]);
                Ok(())
            })
            .ok()?;
        }
        Some(image)
    }

    fn prepare(
        gpu: &Gpu,
        device: u64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<(Self, Option<wgpu::CommandBuffer>)> {
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
        let size = [width, height];
        let rgba = &rgba[..bytes];
        let converted = (width.max(height) >= 128)
            .then(|| gpu.image_upload())
            .flatten()
            .and_then(|upload| gpu.validation(|| Ok(upload.prepare(gpu, size, rgba))).ok());
        let (texture, upload, conversion) = converted
            .map(|(image, stats, command)| (image, stats, Some(command)))
            .unwrap_or_else(|| {
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("impasto image"),
                    size: crate::compositor::extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let pixels = crate::color::premultiply_rgba(rgba);
                crate::image_upload::upload_rgba(gpu, &texture, size, &pixels);
                (
                    texture,
                    ImageUploadStats {
                        source_bytes: bytes as u64,
                        resident_bytes: bytes as u64,
                        ..Default::default()
                    },
                    None,
                )
            });
        if gpu.is_lost() {
            return None;
        }
        let view = texture.create_view(&Default::default());
        Some((
            Self {
                view: Arc::new(view),
                #[cfg(target_os = "android")]
                nv12: None,
                material: Arc::default(),
                width,
                height,
                device,
                revision: NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed),
                premultiplied_srgb: false,
                upload,
            },
            conversion,
        ))
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
            upload: ImageUploadStats::default(),
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn from_android(
        gpu: &Gpu,
        texture: wgpu::Texture,
        size: crate::PixelSize,
        device: u64,
        luma_weights: (f32, f32),
        narrow: bool,
        chroma_offset: [f32; 2],
    ) -> Result<Self, crate::Status> {
        let (view, nv12) = if texture.format() == wgpu::TextureFormat::NV12 {
            let plane = |aspect| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    aspect,
                    ..Default::default()
                })
            };
            let (kr, kb) = luma_weights;
            let kg = 1.0 - kr - kb;
            let (luma, black, chroma, neutral) = if narrow {
                (255.0 / 219.0, -16.0 / 219.0, 255.0 / 224.0, -128.0 / 224.0)
            } else {
                (1.0, 0.0, 1.0, -128.0 / 255.0)
            };
            let u = [0.0, -2.0 * kb * (1.0 - kb) / kg, 2.0 * (1.0 - kb)];
            let v = [2.0 * (1.0 - kr), -2.0 * kr * (1.0 - kr) / kg, 0.0];
            let offset: [f32; 3] = std::array::from_fn(|i| black + neutral * (u[i] + v[i]));
            let constants = [
                [luma, luma, luma, 0.0],
                [u[0] * chroma, u[1] * chroma, u[2] * chroma, 0.0],
                [v[0] * chroma, v[1] * chroma, v[2] * chroma, 0.0],
                [offset[0], offset[1], offset[2], 0.0],
                [chroma_offset[0], chroma_offset[1], 0.0, 0.0],
            ];
            let conversion = crate::pipelines::buffer_from(
                gpu,
                "impasto NV12 conversion",
                bytemuck::bytes_of(&constants),
                wgpu::BufferUsages::UNIFORM,
            )?;
            (
                plane(wgpu::TextureAspect::Plane0),
                Some(Nv12Planes {
                    chroma: plane(wgpu::TextureAspect::Plane1),
                    conversion,
                }),
            )
        } else {
            (texture.create_view(&Default::default()), None)
        };
        Ok(Self {
            view: Arc::new(view),
            premultiplied_srgb: nv12.is_none() && !texture.format().is_srgb(),
            nv12,
            material: Arc::default(),
            width: size.width,
            height: size.height,
            device,
            revision: NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed),
            upload: ImageUploadStats::default(),
        })
    }

    pub(crate) fn nv12(&self) -> Option<&Nv12Planes> {
        #[cfg(target_os = "android")]
        return self.nv12.as_ref();
        #[cfg(not(target_os = "android"))]
        None
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

struct Cached {
    image: GpuImage,
    used: u64,
}

/// Keeps the presented textures alive while replacements await a valid scene.
pub struct TextureStore {
    active: HashMap<Arc<str>, Cached>,
    gpu_views: HashMap<Arc<str>, GpuImage>,
    staged: HashMap<Arc<str>, Option<Cached>>,
    generation: u64,
    device: u64,
    invalidated: bool,
    uploads: ImageUploadStats,
}

impl TextureStore {
    pub fn new(device: u64) -> Self {
        Self {
            active: HashMap::new(),
            gpu_views: HashMap::new(),
            staged: HashMap::new(),
            generation: 0,
            device,
            invalidated: false,
            uploads: ImageUploadStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn get(&self, source: &str) -> Option<&GpuImage> {
        if source.starts_with(engine::scene::image::GPU_VIEW_TEXTURE_PREFIX) {
            self.gpu_views.get(source)
        } else {
            self.active.get(source).map(|cached| &cached.image)
        }
    }

    pub(crate) fn set_gpu_views(&mut self, views: HashMap<Arc<str>, GpuImage>) {
        self.gpu_views = views;
    }

    pub(crate) fn has_gpu_views(&self) -> bool {
        !self.gpu_views.is_empty()
    }

    fn candidate(&self, source: &str) -> Option<&GpuImage> {
        match self.staged.get(source) {
            Some(entry) => entry.as_ref().map(|cached| &cached.image),
            None if !self.invalidated => self.get(source),
            None => None,
        }
    }

    /// Adopts a texture uploaded on another thread. Stale generations and
    /// older revisions are rejected.
    pub fn install(&mut self, source: &str, image: GpuImage) {
        if image.device != self.device
            || self
                .candidate(source)
                .is_some_and(|old| old.revision >= image.revision)
        {
            return;
        }
        self.uploads.add(image.upload);
        let entry = Some(Cached { image, used: 0 });
        if let Some(slot) = self.staged.get_mut(source) {
            *slot = entry;
        } else {
            self.staged.insert(Arc::from(source), entry);
        }
    }

    pub fn forget(&mut self, source: &str) {
        if let Some(slot) = self.staged.get_mut(source) {
            *slot = None;
        } else if let Some((key, _)) = self.active.get_key_value(source) {
            self.staged.insert(Arc::clone(key), None);
        }
    }

    pub fn clear(&mut self) {
        self.staged.clear();
        self.invalidated = true;
    }

    pub fn begin_frame(&mut self) -> TextureTransaction<'_> {
        self.generation = self.generation.wrapping_add(1);
        TextureTransaction {
            store: self,
            added: HashMap::new(),
            conversions: Vec::new(),
        }
    }
}

pub struct TextureTransaction<'a> {
    store: &'a mut TextureStore,
    added: HashMap<Arc<str>, Cached>,
    conversions: Vec<wgpu::CommandBuffer>,
}

impl TextureTransaction<'_> {
    pub fn get(&self, source: &str) -> Option<&GpuImage> {
        self.added
            .get(source)
            .map(|cached| &cached.image)
            .or_else(|| self.store.candidate(source))
    }

    pub fn same_geometry(&self, source: &str) -> bool {
        let size = |image: &GpuImage| (image.width, image.height);
        self.get(source).map(size) == self.store.get(source).map(size)
    }

    pub fn same_revision(&self, source: &str) -> bool {
        let revision = |image: &GpuImage| image.revision;
        self.get(source).map(revision) == self.store.get(source).map(revision)
    }

    pub fn touch(&mut self, source: &str) {
        let cached = if let Some(entry) = self.store.staged.get_mut(source) {
            entry.as_mut()
        } else if !self.store.invalidated {
            self.store.active.get_mut(source)
        } else {
            None
        };
        if let Some(cached) = cached {
            cached.used = self.store.generation;
        }
    }

    /// Resolves a source during compilation. Only uploads made by a
    /// successful transaction become visible to the next frame.
    pub fn load<'pixels>(
        &mut self,
        gpu: &Gpu,
        source: &Arc<str>,
        resolve: &mut dyn FnMut(&str) -> Option<crate::ImagePixels<'pixels>>,
    ) -> Load {
        let ready = |image: &GpuImage| Load::Ready {
            width: image.width(),
            height: image.height(),
        };
        if let Some(image) = self.get(source) {
            let result = ready(image);
            self.touch(source);
            return result;
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
        let Some((image, conversion)) = GpuImage::prepare(
            gpu,
            self.store.device,
            image.width,
            image.height,
            image.rgba,
        ) else {
            return Load::Failed;
        };
        self.conversions.extend(conversion);
        let result = ready(&image);
        self.added.insert(
            Arc::clone(source),
            Cached {
                image,
                used: self.store.generation,
            },
        );
        result
    }

    pub fn commit(self, gpu: &Gpu) -> Result<ImageUploadStats, crate::Status> {
        if !self.conversions.is_empty() {
            gpu.validation(|| {
                gpu.submit(self.conversions);
                Ok(())
            })?;
        }
        let store = self.store;
        let generation = store.generation;
        if store.invalidated {
            store.active.clear();
        } else {
            store.active.retain(|_, cached| cached.used == generation);
        }
        for (source, entry) in store.staged.drain() {
            if let Some(cached) = entry
                && cached.used == generation
            {
                store.active.insert(source, cached);
            }
        }
        for (source, cached) in self.added {
            store.uploads.add(cached.image.upload);
            store.active.insert(source, cached);
        }
        store.invalidated = false;
        Ok(std::mem::take(&mut store.uploads))
    }
}
