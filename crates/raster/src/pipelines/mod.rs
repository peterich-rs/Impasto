//! Pipeline cache, bind group layouts, and the instance record layout.
//!
//! Pipelines are keyed by target format and sample count. Each pipeline
//! draws quads from instance vertex buffers and shares the frame uniform slabs.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

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
    RectSolid,
    RectShadow,
    RectCircularShadow,
    ImageArray,
    SmallImages,
    ExternalImage,
    EarlyZ,
    EarlyZEdges,
    Mesh,
    MeshStencil,
    StencilNonZero,
    StencilEvenOdd,
    StencilCover,
    StencilWrite,
    Text,
    TextUnclipped,
    TextMask,
    TextLcd,
    Image,
    BlurDown,
    BlurUp,
    Gaussian,
    Copy,
    CopySrgb,
    P3Linear,
    P3Encoded,
    Dither,
    DitherSrgb,
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
    pub image_array: Option<wgpu::BindGroupLayout>,
    pub small_images: wgpu::BindGroupLayout,
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
pub const CANVAS_STENCIL: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
pub const EARLY_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
pub const EXTRA_BYTES: usize = 128 * size_of::<crate::batch::PaintExtra>();

include!(concat!(env!("OUT_DIR"), "/pipeline_sources.rs"));

pub(crate) const INLINE_IMAGE: &str = INLINE_IMAGE_SOURCES[0];

type PipelineKey = (Target, PipelineKind, BlendKey);

pub struct PipelineCache {
    targets: HashMap<PipelineKey, wgpu::RenderPipeline>,
    prewarmed: HashSet<wgpu::TextureFormat>,
    warmers: Vec<Receiver<(PipelineKey, wgpu::RenderPipeline)>>,
    shaders: Shaders,
    pub layouts: Layouts,
    pub resident_pictures: bool,
}

#[derive(Clone)]
struct Shaders {
    ui: wgpu::ShaderModule,
    text: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    image_array: Option<wgpu::ShaderModule>,
    small_images: wgpu::ShaderModule,
    external_image: Option<wgpu::ShaderModule>,
    blur: wgpu::ShaderModule,
}

impl PipelineCache {
    pub fn new(gpu: &Gpu) -> Self {
        let _trace = tracing::debug_span!(target: "impasto::startup", "shader_modules").entered();
        let started = Instant::now();
        let f16 = usize::from(gpu.device.features().contains(wgpu::Features::SHADER_F16));
        let storage = usize::from(gpu.caps.paint_storage) << 1;
        let lcd = usize::from(gpu.caps.lcd) << 2;
        let immediates = usize::from(gpu.effect_immediates()) << 1;
        let module = |label, source: &'static str| {
            gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
        };
        let view = U(size_of::<ViewUniform>() as u64);
        let resident = gpu.caps.paint_storage
            && std::env::var("IMPASTO_RESIDENT_PICTURES").as_deref() != Ok("0");
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
                &[view, U((MAX_STOPS * 32) as u64), Slot::Paint, Slot::Array],
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
            image_array: gpu
                .image_array()
                .then(|| layout(gpu, "impasto image array", &[Slot::TextureBindingArray, S])),
            small_images: layout(gpu, "impasto small images", &[Slot::Array, S]),
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
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(EFFECT_VALUE_BYTES),
                        },
                        count: None,
                    }],
                }),
        };
        let shaders = Shaders {
            ui: module("impasto ui", UI_SOURCES[f16 | storage]),
            text: module("impasto text", TEXT_SOURCES[f16 | storage | lcd]),
            image: module("impasto image", IMAGE_SOURCES[f16]),
            image_array: gpu
                .image_array()
                .then(|| module("impasto bindless images", IMAGE_ARRAY_SOURCES[f16])),
            small_images: module("impasto small images", SMALL_IMAGES_SOURCES[f16]),
            blur: module("impasto blur", BLUR_SOURCES[f16 | immediates]),
            external_image: nv12.then(|| module("impasto NV12 image", EXTERNAL_IMAGE_SOURCES[f16])),
        };
        tracing::debug!(target: "impasto::startup", shader_us = started.elapsed().as_secs_f64() * 1_000_000.0, "raster shader modules ready");
        Self {
            targets: HashMap::new(),
            prewarmed: HashSet::new(),
            warmers: Vec::new(),
            shaders,
            layouts,
            resident_pictures: resident,
        }
    }

    /// Compile only the pipeline requested by this draw.
    pub fn get(
        &mut self,
        gpu: &Gpu,
        target: Target,
        kind: PipelineKind,
        blend: BlendKey,
    ) -> Result<wgpu::RenderPipeline, Status> {
        let key = (target, kind, blend);
        if let Some(pipeline) = self.targets.get(&key) {
            return Ok(pipeline.clone());
        }
        self.install_warmed();
        if let Some(pipeline) = self.targets.get(&key) {
            return Ok(pipeline.clone());
        }
        let pipeline = self.shaders.compile(gpu, &self.layouts, key)?;
        self.targets.insert(key, pipeline.clone());
        Ok(pipeline)
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
            keys.push((
                target,
                if self.shaders.image_array.is_some() {
                    K::ImageArray
                } else {
                    K::SmallImages
                },
                BlendKey::SrcOver,
            ));
            if gpu.caps.lcd && format.is_srgb() {
                keys.push((target, K::TextLcd, BlendKey::SrcOver));
            }
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
            [PipelineKind::RectSolid, PipelineKind::TextMask]
        } else {
            [PipelineKind::Rect, PipelineKind::TextUnclipped]
        };
        for kind in [rect, text, PipelineKind::Image] {
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
            PipelineKind::ImageArray => self.image_array.as_ref().ok_or(Status::NotReady)?,
            PipelineKind::SmallImages => &self.small_images,
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
    TextureBindingArray,
    Array,
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
                Slot::Texture | Slot::TextureBindingArray | Slot::Array | Slot::GlyphArray => (
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: if matches!(slot, Slot::Array | Slot::GlyphArray) {
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
                count: match slot {
                    Slot::TextureBindingArray => {
                        std::num::NonZeroU32::new(crate::gpu::IMAGE_ARRAY_LEN)
                    }
                    _ => None,
                },
                ty,
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

fn build(
    gpu: &Gpu,
    layouts: &Layouts,
    shaders: [&wgpu::ShaderModule; 4],
    target: Target,
    kind: PipelineKind,
    blend: BlendKey,
) -> wgpu::RenderPipeline {
    use crate::batch::{GlyphVertex, ImageVertex, RectVertex};
    use PipelineKind as K;

    let [ui, text, image, blur] = shaders;
    let rect_attrs = wgpu::vertex_attr_array![1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
        4 => Float32, 5 => Uint32, 6 => Float32, 7 => Float32];
    let text_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Uint16x4, 2 => Float16x4,
        3 => Uint32, 4 => Uint32];
    let image_attrs =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
    let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3];
    let instance = |array_stride, attributes| {
        Some(wgpu::VertexBufferLayout {
            array_stride,
            attributes,
            step_mode: wgpu::VertexStepMode::Instance,
        })
    };
    let rect = instance(size_of::<RectVertex>() as u64, &rect_attrs);
    let glyph = instance(size_of::<GlyphVertex>() as u64, &text_attrs);
    let quad = instance(size_of::<ImageVertex>() as u64, &image_attrs);
    let geometry = Some(wgpu::VertexBufferLayout {
        array_stride: size_of::<[f32; 3]>() as u64,
        attributes: &mesh_attrs,
        step_mode: wgpu::VertexStepMode::Vertex,
    });
    let view = Some(&layouts.view);
    let view_only = [view];
    let paint = [Some(&layouts.paint), Some(&layouts.material)];
    let solid = [view, Some(&layouts.material)];
    let text_mask = [view, Some(&layouts.atlas), Some(&layouts.material)];
    let text_groups = [
        Some(&layouts.paint),
        Some(&layouts.atlas),
        Some(&layouts.material),
    ];
    let image_groups = [
        view,
        match kind {
            K::ImageArray => layouts.image_array.as_ref(),
            K::SmallImages => Some(&layouts.small_images),
            K::ExternalImage => layouts.external_image.as_ref(),
            _ => Some(&layouts.material),
        },
        Some(&layouts.material),
    ];
    let effect_globals = if gpu.effect_immediates() {
        &layouts.view
    } else {
        &layouts.effects
    };
    let effects = [Some(effect_globals), Some(&layouts.material)];
    let composite = [
        Some(effect_globals),
        Some(&layouts.material),
        Some(&layouts.composite),
    ];
    let color_filter = [
        Some(effect_globals),
        Some(&layouts.material),
        Some(&layouts.effect_values),
    ];
    let backdrop_material = [
        composite[0],
        composite[1],
        composite[2],
        Some(&layouts.effect_values),
    ];
    let (module, groups, vertex, fragment) = match kind {
        K::Rect => (ui, paint.as_slice(), "vs_main", "fs_main"),
        K::RectSolid => (ui, solid.as_slice(), "vs_solid", "fs_solid"),
        K::RectShadow | K::RectCircularShadow => (ui, paint.as_slice(), "vs_shadow", "fs_shadow"),
        K::EarlyZ => (ui, view_only.as_slice(), "vs_depth_rect", "fs_depth_rect"),
        K::EarlyZEdges => (ui, solid.as_slice(), "vs_depth_solid", "fs_solid"),
        K::StencilWrite => (ui, view_only.as_slice(), "vs_stencil", "fs_stencil"),
        K::StencilNonZero | K::StencilEvenOdd => (ui, paint.as_slice(), "vs_stencil", "fs_stencil"),
        K::Mesh | K::MeshStencil | K::StencilCover => (ui, paint.as_slice(), "vs_mesh", "fs_mesh"),
        K::TextLcd => (text, text_mask.as_slice(), "vs_mask", "fs_lcd"),
        K::TextMask => (text, text_mask.as_slice(), "vs_mask", "fs_mask"),
        K::Text | K::TextUnclipped => (text, text_groups.as_slice(), "vs_main", "fs_main"),
        K::Image | K::ImageArray | K::SmallImages | K::ExternalImage => {
            (image, image_groups.as_slice(), "vs_main", "fs_main")
        }
        K::BlurDown => (blur, effects.as_slice(), "vs_full", "fs_blur_down"),
        K::BlurUp => (blur, effects.as_slice(), "vs_full", "fs_blur_up"),
        K::Gaussian => (blur, effects.as_slice(), "vs_full", "fs_gaussian"),
        K::Copy | K::MaskIntersect => (blur, effects.as_slice(), "vs_full", "fs_copy"),
        K::CopySrgb => (blur, effects.as_slice(), "vs_full", "fs_copy_srgb"),
        K::P3Linear => (blur, effects.as_slice(), "vs_full", "fs_p3_linear"),
        K::P3Encoded => (blur, effects.as_slice(), "vs_full", "fs_p3_encoded"),
        K::Dither => (blur, effects.as_slice(), "vs_full", "fs_dither"),
        K::DitherSrgb => (blur, effects.as_slice(), "vs_full", "fs_dither_srgb"),
        K::Over => (blur, composite.as_slice(), "vs_full", "fs_over"),
        K::Multiply => (blur, composite.as_slice(), "vs_full", "fs_composite"),
        K::Backdrop => (blur, composite.as_slice(), "vs_full", "fs_backdrop"),
        K::BackdropMaterial => (
            blur,
            backdrop_material.as_slice(),
            "vs_full",
            "fs_backdrop_material",
        ),
        K::ColorFilter => (blur, color_filter.as_slice(), "vs_full", "fs_color_filter"),
    };
    let rect_buffers = [rect.clone()];
    let stencil_buffers = [geometry.clone()];
    let mesh_buffers = [geometry, rect];
    let glyph_buffers = [glyph];
    let image_buffers = [quad];
    let buffers = match kind {
        K::Rect
        | K::RectSolid
        | K::RectShadow
        | K::RectCircularShadow
        | K::EarlyZ
        | K::EarlyZEdges => rect_buffers.as_slice(),
        K::Mesh | K::MeshStencil | K::StencilCover => mesh_buffers.as_slice(),
        K::StencilWrite | K::StencilNonZero | K::StencilEvenOdd => stencil_buffers.as_slice(),
        K::Text | K::TextUnclipped | K::TextMask | K::TextLcd => glyph_buffers.as_slice(),
        K::Image | K::ImageArray | K::SmallImages | K::ExternalImage => image_buffers.as_slice(),
        _ => &[],
    };
    let blend = match kind {
        K::TextLcd => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: BlendKey::SrcOver.state().alpha,
        },
        K::Rect
        | K::RectSolid
        | K::EarlyZEdges
        | K::RectShadow
        | K::RectCircularShadow
        | K::Mesh
        | K::MeshStencil
        | K::StencilCover
        | K::Text
        | K::TextUnclipped
        | K::TextMask
        | K::Image
        | K::ImageArray
        | K::SmallImages
        | K::ExternalImage
        | K::ColorFilter
        | K::Over => blend.state(),
        K::MaskIntersect => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        },
        _ => wgpu::BlendState::REPLACE,
    };
    let mut constants = Vec::new();
    if gpu.shadow_2d && matches!(kind, K::RectShadow | K::RectCircularShadow) {
        constants.push(("two_dimensional_shadow", 1.0));
    }
    if kind == K::RectCircularShadow {
        constants.push(("circular_shadow", 1.0));
    }
    if gpu.text_gamma && matches!(kind, K::Text | K::TextUnclipped | K::TextMask | K::TextLcd) {
        constants.push(("text_gamma", 1.0));
    }
    if gpu.dither_gradients && kind == K::Rect {
        constants.push(("output_dither", 1.0));
    }
    if gpu.device.features().contains(wgpu::Features::SHADER_F16)
        && matches!(
            target.format,
            wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
        )
        && matches!(
            kind,
            K::Rect
                | K::Mesh
                | K::MeshStencil
                | K::StencilCover
                | K::Text
                | K::TextUnclipped
                | K::BlurDown
                | K::BlurUp
        )
    {
        constants.push(("reduced_color_precision", 1.0));
    }
    if gpu.gradient_lut
        && matches!(
            kind,
            K::Rect | K::Mesh | K::MeshStencil | K::StencilCover | K::Text | K::TextUnclipped
        )
    {
        constants.push(("use_gradient_ramps", 1.0));
    }
    if kind == K::TextUnclipped {
        constants.push(("unclipped", 1.0));
    }
    let label = format!("impasto {kind:?} {blend:?}");
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&label),
            bind_group_layouts: groups,
            immediate_size: if module == blur && gpu.effect_immediates() {
                ParamsUniform::BYTE_LEN
            } else {
                0
            },
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vertex),
                buffers,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fragment),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.format,
                    blend: Some(blend),
                    write_mask: if matches!(
                        kind,
                        K::StencilWrite | K::StencilNonZero | K::StencilEvenOdd
                    ) {
                        wgpu::ColorWrites::empty()
                    } else {
                        wgpu::ColorWrites::ALL
                    },
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth_stencil(kind),
            multisample: wgpu::MultisampleState {
                count: target.samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        })
}

fn depth_stencil(kind: PipelineKind) -> Option<wgpu::DepthStencilState> {
    use wgpu::{CompareFunction, StencilOperation};
    match kind {
        PipelineKind::EarlyZ | PipelineKind::EarlyZEdges => Some(wgpu::DepthStencilState {
            format: EARLY_DEPTH,
            depth_write_enabled: Some(kind == PipelineKind::EarlyZ),
            depth_compare: Some(CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        PipelineKind::StencilNonZero
        | PipelineKind::StencilEvenOdd
        | PipelineKind::StencilCover => {
            let front = match kind {
                PipelineKind::StencilNonZero => StencilOperation::IncrementWrap,
                PipelineKind::StencilEvenOdd => StencilOperation::Invert,
                _ => StencilOperation::Zero,
            };
            let back = if kind == PipelineKind::StencilNonZero {
                StencilOperation::DecrementWrap
            } else {
                front
            };
            let compare = if kind == PipelineKind::StencilCover {
                CompareFunction::NotEqual
            } else {
                CompareFunction::Always
            };
            let face = |pass_op| wgpu::StencilFaceState {
                compare,
                pass_op,
                ..Default::default()
            };
            Some(wgpu::DepthStencilState {
                format: CANVAS_STENCIL,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::Always),
                stencil: wgpu::StencilState {
                    front: face(front),
                    back: face(back),
                    read_mask: 255,
                    write_mask: 255,
                },
                bias: Default::default(),
            })
        }
        PipelineKind::StencilWrite | PipelineKind::MeshStencil => {
            let face = wgpu::StencilFaceState {
                compare: CompareFunction::Equal,
                pass_op: if kind == PipelineKind::StencilWrite {
                    StencilOperation::IncrementClamp
                } else {
                    StencilOperation::Keep
                },
                ..Default::default()
            };
            Some(wgpu::DepthStencilState {
                format: CANVAS_STENCIL,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::Always),
                stencil: wgpu::StencilState {
                    front: face,
                    back: face,
                    read_mask: 255,
                    write_mask: if kind == PipelineKind::StencilWrite {
                        255
                    } else {
                        0
                    },
                },
                bias: Default::default(),
            })
        }
        _ => None,
    }
}

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
