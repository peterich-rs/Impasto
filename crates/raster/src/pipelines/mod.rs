//! Pipeline cache, bind group layouts, and the instance record layout.
//!
//! Pipelines are keyed by target format and sample count. Each pipeline
//! draws quads from instance vertex buffers and shares the frame uniform slabs.

mod descriptor;

use descriptor::build;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use indexmap::IndexMap;

use crate::gpu::Gpu;
use crate::status::Status;

/// One pipeline variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub format: wgpu::TextureFormat,
    pub samples: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendKey {
    #[default]
    SrcOver,
    DstOver,
    DstOut,
    Lighter,
    Xor,
    Screen,
}

impl BlendKey {
    pub fn from_mode(mode: u32) -> Option<Self> {
        match mode {
            0 => Some(Self::SrcOver),
            2 => Some(Self::Screen),
            3 => Some(Self::DstOver),
            4 => Some(Self::DstOut),
            5 => Some(Self::Lighter),
            6 => Some(Self::Xor),
            _ => None,
        }
    }

    fn state(self) -> wgpu::BlendState {
        use wgpu::BlendFactor::*;
        let (src_factor, dst_factor) = match self {
            Self::SrcOver => (One, OneMinusSrcAlpha),
            Self::DstOver => (OneMinusDstAlpha, One),
            Self::DstOut => (Zero, OneMinusSrcAlpha),
            Self::Lighter => (One, One),
            Self::Xor => (OneMinusDstAlpha, OneMinusSrcAlpha),
            Self::Screen => (One, OneMinusSrc),
        };
        let color = wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        let alpha = if self == Self::Screen {
            wgpu::BlendComponent {
                dst_factor: OneMinusSrcAlpha,
                ..color
            }
        } else {
            color
        };
        wgpu::BlendState { color, alpha }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PipelineKind {
    Rect,
    RectUnclipped,
    RectDither,
    RectDitherUnclipped,
    RectSolid,
    RectSolidUnclipped,
    RectShadow,
    RectCircularShadow,
    ExternalImage,
    Mesh,
    Text,
    TextUnclipped,
    TextMask,
    Image,
    ImageUnclipped,
    BlurDown,
    BlurUp,
    Gaussian,
    Copy,
    CopySrgb,
    P3Linear,
    P3Encoded,
    Over,
    Multiply,
    Backdrop,
    BackdropMaterial,
    ColorFilter,
    MaskIntersect,
}

/// Bind group layouts shared by every target.
#[derive(Clone)]
pub struct Layouts {
    pub view: wgpu::BindGroupLayout,
    pub paint: wgpu::BindGroupLayout,
    pub effects: wgpu::BindGroupLayout,
    pub runtime_effect: wgpu::BindGroupLayout,
    pub runtime_textures: wgpu::BindGroupLayout,
    pub material: wgpu::BindGroupLayout,
    pub external_image: Option<wgpu::BindGroupLayout>,
    pub atlas: wgpu::BindGroupLayout,
    pub composite: wgpu::BindGroupLayout,
    pub effect_values: wgpu::BindGroupLayout,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewUniform {
    pub transform: [[f32; 4]; 4],
    pub meta: [f32; 4],
    pub target_info: [f32; 4],
    pub mask_rect: [f32; 4],
    pub clip_boxes: [[f32; 4]; 4],
    pub clip_radii: [[f32; 4]; 4],
    pub clip_curves: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParamsUniform {
    pub values: [f32; 4],
    pub uv_bounds: [f32; 4],
    pub backdrop_origin: [f32; 4],
    pub backdrop_bounds: [f32; 4],
}

pub const EFFECT_VALUE_BYTES: u64 = size_of::<[[f32; 4]; 5]>() as u64;

impl ParamsUniform {
    pub const BYTE_LEN: u32 = 64;
}
const _: () = assert!(size_of::<ParamsUniform>() == ParamsUniform::BYTE_LEN as usize);

/// Stop pairs: each stop is a premultiplied color then `(offset, 0, 0, 0)`.
pub const MAX_STOPS: usize = 256;
pub const EXTRA_BYTES: usize = 128 * size_of::<crate::batch::PaintExtra>();

include!(concat!(env!("OUT_DIR"), "/pipeline_sources.rs"));

pub(crate) const INLINE_IMAGE: &str = INLINE_IMAGE_SOURCES[0];

type PipelineKey = (Target, PipelineKind, BlendKey);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineId(usize);

pub struct PipelineCache {
    targets: IndexMap<PipelineKey, wgpu::RenderPipeline>,
    prewarmed: HashSet<wgpu::TextureFormat>,
    warmers: Vec<Receiver<(PipelineKey, wgpu::RenderPipeline)>>,
    shaders: Shaders,
    pub layouts: Layouts,
    frame_compiles: u32,
}

#[derive(Clone)]
struct Shaders {
    ui: wgpu::ShaderModule,
    text: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    external_image: Option<wgpu::ShaderModule>,
    blur: wgpu::ShaderModule,
}

impl PipelineCache {
    pub fn new(gpu: &Gpu) -> Self {
        let _trace = tracing::debug_span!(target: "impasto::startup", "shader_modules").entered();
        let started = Instant::now();
        let f16 = usize::from(gpu.device.features().contains(wgpu::Features::SHADER_F16));
        let storage = usize::from(gpu.caps.paint_storage) << 1;
        let immediates = usize::from(gpu.effect_immediates()) << 1;
        let module = |label, source: &'static str| {
            gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
        };
        let view = U(size_of::<ViewUniform>() as u64);
        let nv12 = gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_NV12);
        let layouts = Layouts {
            external_image: nv12.then(|| {
                layout(
                    gpu,
                    "impasto NV12 image",
                    &[T, S, T, Slot::FixedUniform(80)],
                )
            }),
            view: layout(gpu, "impasto view", &[view]),
            paint: layout(
                gpu,
                "impasto paint",
                &[view, U((MAX_STOPS * 32) as u64), Slot::Paint],
            ),
            effects: layout(
                gpu,
                "impasto effects",
                &[view, U(size_of::<ParamsUniform>() as u64)],
            ),
            runtime_effect: layout(
                gpu,
                "impasto runtime effect",
                &[
                    view,
                    U(size_of::<crate::runtime_effect::EffectParams>() as u64),
                ],
            ),
            runtime_textures: layout(gpu, "impasto effect inputs", &[T, T, T, T, S]),
            material: layout(gpu, "impasto material", &[T, S]),
            atlas: layout(
                gpu,
                "impasto atlas",
                &[Slot::GlyphArray, Slot::GlyphArray, S],
            ),
            composite: layout(gpu, "impasto composite", &[T, S, T]),
            effect_values: gpu
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("impasto effect values"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(EFFECT_VALUE_BYTES),
                        },
                        count: None,
                    }],
                }),
        };
        let shaders = Shaders {
            ui: module("impasto ui", UI_SOURCES[f16 | storage]),
            text: module("impasto text", TEXT_SOURCES[f16 | storage]),
            image: module("impasto image", IMAGE_SOURCES[f16]),
            blur: module("impasto blur", BLUR_SOURCES[f16 | immediates]),
            external_image: nv12.then(|| module("impasto NV12 image", EXTERNAL_IMAGE_SOURCES[f16])),
        };
        tracing::debug!(target: "impasto::startup", shader_us = started.elapsed().as_secs_f64() * 1_000_000.0, "raster shader modules ready");
        Self {
            targets: IndexMap::new(),
            prewarmed: HashSet::new(),
            warmers: Vec::new(),
            shaders,
            layouts,
            frame_compiles: 0,
        }
    }

    /// Compile only the pipeline requested by this draw.
    pub fn get(
        &mut self,
        gpu: &Gpu,
        target: Target,
        kind: PipelineKind,
        blend: BlendKey,
    ) -> Result<PipelineId, Status> {
        let key = (target, kind, blend);
        if let Some(index) = self.targets.get_index_of(&key) {
            return Ok(PipelineId(index));
        }
        self.install_warmed();
        if let Some(index) = self.targets.get_index_of(&key) {
            return Ok(PipelineId(index));
        }
        self.frame_compiles += 1;
        let pipeline = self.shaders.compile(gpu, &self.layouts, key)?;
        Ok(PipelineId(self.targets.insert_full(key, pipeline).0))
    }

    pub fn pipeline(&self, id: PipelineId) -> &wgpu::RenderPipeline {
        &self.targets[id.0]
    }

    pub fn reset_frame_compiles(&mut self) {
        self.frame_compiles = 0;
    }

    pub fn frame_compiles(&self) -> u32 {
        self.frame_compiles
    }

    pub fn install_warmed(&mut self) {
        self.warmers.retain(|receiver| {
            loop {
                match receiver.try_recv() {
                    Ok((key, pipeline)) => {
                        self.targets.entry(key).or_insert(pipeline);
                    }
                    Err(mpsc::TryRecvError::Empty) => return true,
                    Err(mpsc::TryRecvError::Disconnected) => return false,
                }
            }
        });
    }

    pub fn prewarm_effects(&mut self, gpu: &Gpu, formats: &[wgpu::TextureFormat]) {
        use PipelineKind as K;
        let formats: Vec<_> = formats
            .iter()
            .copied()
            .filter(|format| self.prewarmed.insert(*format))
            .collect();
        if formats.is_empty() {
            return;
        }
        let mut keys = Vec::new();
        for &format in &formats {
            let target = Target { format, samples: 1 };
            let variants = [
                K::RectSolid,
                K::Rect,
                K::RectUnclipped,
                K::RectDither,
                K::RectDitherUnclipped,
                K::TextMask,
                K::Image,
                K::Copy,
                K::Text,
                K::Over,
                K::Backdrop,
                K::BackdropMaterial,
                K::ColorFilter,
                K::Multiply,
                K::Gaussian,
                K::BlurDown,
                K::BlurUp,
                K::RectShadow,
                K::RectCircularShadow,
            ];
            keys.extend(
                variants
                    .into_iter()
                    .map(|kind| (target, kind, BlendKey::SrcOver)),
            );
            if format == wgpu::TextureFormat::Rgba8UnormSrgb {
                let samples = if gpu.caps.msaa4_srgb { 4 } else { 1 };
                keys.push((Target { format, samples }, K::Mesh, BlendKey::SrcOver));
            }
        }
        keys.retain(|key| !self.targets.contains_key(key));
        let (send, receive) = mpsc::channel();
        let gpu = gpu.clone();
        let shaders = self.shaders.clone();
        let layouts = self.layouts.clone();
        match std::thread::Builder::new()
            .name("impasto-pipelines".into())
            .spawn(move || {
                for key in keys {
                    if gpu.is_lost() {
                        break;
                    }
                    if let Ok(pipeline) = shaders.compile(&gpu, &layouts, key)
                        && send.send((key, pipeline)).is_err()
                    {
                        break;
                    }
                }
            }) {
            Ok(_) => self.warmers.push(receive),
            Err(error) => {
                for format in formats {
                    self.prewarmed.remove(&format);
                }
                tracing::debug!(target: "impasto::startup", %error, "effect prewarm unavailable");
            }
        }
    }

    pub fn prewarm(
        &mut self,
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        specialize: bool,
        surface: bool,
    ) -> Result<(), Status> {
        let _trace = tracing::debug_span!(target: "impasto::startup", "pipeline_prewarm").entered();
        let started = Instant::now();
        let target = Target { format, samples: 1 };
        let count = self.targets.len();
        let [rect, text] = if specialize {
            [PipelineKind::RectSolidUnclipped, PipelineKind::TextMask]
        } else {
            [PipelineKind::RectUnclipped, PipelineKind::TextUnclipped]
        };
        for kind in [rect, text, PipelineKind::ImageUnclipped] {
            self.get(gpu, target, kind, BlendKey::SrcOver)?;
        }
        if surface {
            for kind in [PipelineKind::Rect, PipelineKind::Copy] {
                self.get(gpu, target, kind, BlendKey::SrcOver)?;
            }
        }
        tracing::debug!(target: "impasto::startup", ?format, variants = self.targets.len() - count,
            prewarm_us = started.elapsed().as_secs_f64() * 1_000_000.0, "pipeline prewarm complete");
        Ok(())
    }
}

impl Shaders {
    fn compile(
        &self,
        gpu: &Gpu,
        layouts: &Layouts,
        (target, kind, blend): PipelineKey,
    ) -> Result<wgpu::RenderPipeline, Status> {
        let _trace = tracing::debug_span!(target: "impasto::startup", "pipeline_create").entered();
        let started = Instant::now();
        let image = match kind {
            PipelineKind::ExternalImage => self.external_image.as_ref().ok_or(Status::NotReady)?,
            _ => &self.image,
        };
        let pipeline = gpu.validation(|| {
            Ok(build(
                gpu,
                layouts,
                [&self.ui, &self.text, image, &self.blur],
                target,
                kind,
                blend,
            ))
        })?;
        tracing::debug!(target: "impasto::startup", ?target, ?kind, ?blend,
            pipeline_us = started.elapsed().as_secs_f64() * 1_000_000.0, "raster pipeline ready");
        gpu.pipelines_updated();
        Ok(pipeline)
    }
}

/// Binding kinds in declaration order, matching each WGSL `@group(0)`.
#[derive(Clone, Copy)]
enum Slot {
    Uniform(u64),
    FixedUniform(u64),
    Paint,
    Texture,
    GlyphArray,
    Sampler,
}

fn layout(gpu: &Gpu, label: &str, slots: &[Slot]) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = slots
        .iter()
        .zip(0u32..)
        .map(|(slot, binding)| {
            let (ty, visibility) = match slot {
                Slot::Uniform(size) | Slot::FixedUniform(size) => (
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: !matches!(slot, Slot::FixedUniform(_)),
                        min_binding_size: wgpu::BufferSize::new(*size),
                    },
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                ),
                Slot::Paint => (
                    wgpu::BindingType::Buffer {
                        ty: if gpu.caps.paint_storage {
                            wgpu::BufferBindingType::Storage { read_only: true }
                        } else {
                            wgpu::BufferBindingType::Uniform
                        },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(if gpu.caps.paint_storage {
                            size_of::<crate::batch::PaintExtra>() as u64
                        } else {
                            EXTRA_BYTES as u64
                        }),
                    },
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                ),
                Slot::Texture | Slot::GlyphArray => (
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: if matches!(slot, Slot::GlyphArray) {
                            wgpu::TextureViewDimension::D2Array
                        } else {
                            wgpu::TextureViewDimension::D2
                        },
                        multisampled: false,
                    },
                    if matches!(slot, Slot::GlyphArray) {
                        wgpu::ShaderStages::VERTEX_FRAGMENT
                    } else {
                        wgpu::ShaderStages::FRAGMENT
                    },
                ),
                Slot::Sampler => (
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    wgpu::ShaderStages::FRAGMENT,
                ),
            };
            wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty,
                count: None,
            }
        })
        .collect();
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        })
}

use Slot::{Sampler as S, Texture as T, Uniform as U};

pub fn linear_sampler(gpu: &Gpu) -> wgpu::Sampler {
    gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("impasto linear"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    })
}

pub fn nearest_sampler(gpu: &Gpu) -> wgpu::Sampler {
    gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("impasto nearest"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    })
}

/// A buffer initialized with bytes for a single encoding.
pub fn buffer_from(
    gpu: &Gpu,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> Result<wgpu::Buffer, Status> {
    let size = (contents.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: size != 0,
    });
    if size != 0 {
        let mut mapped = buffer.get_mapped_range_mut(..).map_err(|error| {
            tracing::warn!(target: "impasto::gpu", %label, %error, "draw buffer mapping failed");
            Status::PresentFailed
        })?;
        mapped.slice(..contents.len()).copy_from_slice(contents);
        drop(mapped);
        buffer.unmap();
    }
    Ok(buffer)
}

pub fn bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    resources: &[wgpu::BindingResource<'_>],
) -> wgpu::BindGroup {
    let entries: Vec<_> = (0..)
        .zip(resources.iter().cloned())
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
        .collect();
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("impasto material"),
        layout,
        entries: &entries,
    })
}

#[derive(Default)]
pub struct MaterialCache {
    linear: std::sync::OnceLock<wgpu::BindGroup>,
    nearest: std::sync::OnceLock<wgpu::BindGroup>,
}

impl MaterialCache {
    pub fn nv12(
        &self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        luma: &wgpu::TextureView,
        planes: &crate::textures::Nv12Planes,
        sampler: &wgpu::Sampler,
        nearest: bool,
    ) -> (wgpu::BindGroup, bool) {
        self.get_or_insert(nearest, || {
            bind_group(
                gpu,
                layout,
                &[
                    wgpu::BindingResource::TextureView(luma),
                    wgpu::BindingResource::Sampler(sampler),
                    wgpu::BindingResource::TextureView(&planes.chroma),
                    planes.conversion.as_entire_binding(),
                ],
            )
        })
    }

    pub fn get(
        &self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        nearest: bool,
    ) -> (wgpu::BindGroup, bool) {
        self.get_or_insert(nearest, || {
            bind_group(
                gpu,
                layout,
                &[
                    wgpu::BindingResource::TextureView(view),
                    wgpu::BindingResource::Sampler(sampler),
                ],
            )
        })
    }

    fn get_or_insert(
        &self,
        nearest: bool,
        create: impl FnOnce() -> wgpu::BindGroup,
    ) -> (wgpu::BindGroup, bool) {
        let slot = if nearest { &self.nearest } else { &self.linear };
        let mut created = false;
        let group = slot.get_or_init(|| {
            created = true;
            create()
        });
        (group.clone(), created)
    }
}
