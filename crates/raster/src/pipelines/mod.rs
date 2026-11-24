//! Pipeline cache, bind group layouts, and the instance record layout.
//!
//! Pipelines are keyed by target format and sample count. Each pipeline
//! draws quads from instance vertex buffers and shares the frame uniform slabs.

use std::collections::HashMap;
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
    ExternalImage,
    EarlyZ,
    Mesh,
    MeshStencil,
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
    MaskIntersect,
}

/// Bind group layouts shared by every target.
pub struct Layouts {
    pub view: wgpu::BindGroupLayout,
    pub paint: wgpu::BindGroupLayout,
    pub effects: wgpu::BindGroupLayout,
    pub material: wgpu::BindGroupLayout,
    pub image_array: Option<wgpu::BindGroupLayout>,
    pub external_image: Option<wgpu::BindGroupLayout>,
    pub atlas: wgpu::BindGroupLayout,
    pub composite: wgpu::BindGroupLayout,
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

impl ParamsUniform {
    pub const BYTE_LEN: u32 = 64;
}
const _: () = assert!(size_of::<ParamsUniform>() == ParamsUniform::BYTE_LEN as usize);

/// Stop pairs: each stop is a premultiplied color then `(offset, 0, 0, 0)`.
pub const MAX_STOPS: usize = 256;
pub const CANVAS_STENCIL: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
pub const EARLY_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
pub const EXTRA_BYTES: usize = 128 * size_of::<crate::batch::PaintExtra>();

const UI_WGSL: &str = include_str!("ui.wgsl");
const TEXT_WGSL: &str = include_str!("text.wgsl");
const IMAGE_WGSL: &str = include_str!("image.wgsl");
const BLUR_WGSL: &str = include_str!("blur.wgsl");

pub struct PipelineCache {
    targets: HashMap<(Target, PipelineKind, BlendKey), wgpu::RenderPipeline>,
    ui: wgpu::ShaderModule,
    text: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    image_array: Option<wgpu::ShaderModule>,
    external_image: Option<wgpu::ShaderModule>,
    blur: wgpu::ShaderModule,
    pub layouts: Layouts,
    pub resident_pictures: bool,
}

impl PipelineCache {
    pub fn new(gpu: &Gpu) -> Self {
        let _trace = tracing::debug_span!(target: "impasto::startup", "shader_modules").entered();
        let started = Instant::now();
        let reduced_color = if gpu.device.features().contains(wgpu::Features::SHADER_F16) {
            "enable f16;\nalias ReducedColor = vec4<f16>;\nalias ReducedScalar = f16;"
        } else {
            "alias ReducedColor = vec4<f32>;\nalias ReducedScalar = f32;"
        };
        let color_precision = include_str!("color_precision.wgsl");
        let module = |label, source: &str, clipped: bool, gradients: bool| {
            let view = include_str!("view.wgsl");
            let clip = if clipped {
                include_str!("clip.wgsl")
            } else {
                ""
            };
            let gradient = if gradients {
                include_str!("gradient.wgsl")
            } else {
                ""
            };
            let source = if label == "impasto blur" && gpu.effect_immediates() {
                std::borrow::Cow::Owned(
                    source.replace("@group(0) @binding(1) var<uniform>", "var<immediate>"),
                )
            } else {
                std::borrow::Cow::Borrowed(source)
            };
            let paint = if gradients {
                if gpu.caps.paint_storage {
                    "@group(0) @binding(2) var<storage, read> paint: array<PaintExtra>;"
                } else {
                    "@group(0) @binding(2) var<uniform> paint: array<PaintExtra, 128>;"
                }
            } else {
                ""
            };
            let (enable, lcd) = if gpu.caps.lcd && label == "impasto text" {
                (
                    "enable dual_source_blending;",
                    include_str!("text_lcd.wgsl"),
                )
            } else {
                (
                    if label == "impasto bindless images" {
                        "enable wgpu_binding_array;"
                    } else {
                        ""
                    },
                    "",
                )
            };
            gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(
                        format!("{enable}\n{reduced_color}\n{color_precision}\n{view}\n{source}\n{clip}\n{gradient}\n{paint}\n{lcd}")
                            .into(),
                    ),
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
            material: layout(gpu, "impasto material", &[T, S]),
            image_array: gpu
                .image_array()
                .then(|| layout(gpu, "impasto image array", &[Slot::TextureBindingArray, S])),
            atlas: layout(
                gpu,
                "impasto atlas",
                &[Slot::GlyphArray, Slot::GlyphArray, S],
            ),
            composite: layout(gpu, "impasto composite", &[T, S, T]),
        };
        let cache = Self {
            targets: HashMap::new(),
            ui: module("impasto ui", UI_WGSL, true, true),
            text: module("impasto text", TEXT_WGSL, true, true),
            image: module("impasto image", IMAGE_WGSL, true, false),
            image_array: gpu.image_array().then(|| {
                let array = IMAGE_WGSL
                    .replace(
                        "var image_tex: texture_2d<f32>;",
                        &format!(
                            "var image_textures: binding_array<texture_2d<f32>, {}>;",
                            crate::gpu::IMAGE_ARRAY_LEN
                        ),
                    )
                    .replace(
                        "textureSampleLevel(image_tex,",
                        "textureSampleLevel(image_textures[u32(data.info.y)],",
                    )
                    .replace(
                        "textureSampleGrad(image_tex,",
                        "textureSampleGrad(image_textures[u32(data.info.y)],",
                    );
                module("impasto bindless images", &array, true, false)
            }),
            blur: module("impasto blur", BLUR_WGSL, false, false),
            external_image: nv12.then(|| {
                let vertex = IMAGE_WGSL
                    .split_once("@fragment")
                    .map_or(IMAGE_WGSL, |(vertex, _)| vertex);
                module(
                    "impasto NV12 image",
                    &format!("{vertex}{}", include_str!("external_image.wgsl")),
                    true,
                    false,
                )
            }),
            layouts,
            resident_pictures: resident,
        };
        tracing::debug!(target: "impasto::startup", shader_us = started.elapsed().as_secs_f64() * 1_000_000.0, "raster shader modules ready");
        cache
    }

    /// Compile only the pipeline requested by this draw.
    pub fn get(
        &mut self,
        gpu: &Gpu,
        target: Target,
        kind: PipelineKind,
        blend: BlendKey,
    ) -> Result<wgpu::RenderPipeline, Status> {
        use std::collections::hash_map::Entry;
        let key = (target, kind, blend);
        match self.targets.entry(key) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let _trace =
                    tracing::debug_span!(target: "impasto::startup", "pipeline_create").entered();
                let started = Instant::now();
                let pipeline = gpu.validation(|| {
                    let (ui, image, text) = match kind {
                        PipelineKind::ExternalImage => (
                            &self.ui,
                            self.external_image.as_ref().ok_or(Status::NotReady)?,
                            &self.text,
                        ),
                        PipelineKind::ImageArray => (
                            &self.ui,
                            self.image_array.as_ref().ok_or(Status::NotReady)?,
                            &self.text,
                        ),
                        _ => (&self.ui, &self.image, &self.text),
                    };
                    Ok(build(
                        gpu,
                        &self.layouts,
                        [ui, text, image, &self.blur],
                        target,
                        kind,
                        blend,
                    ))
                })?;
                tracing::debug!(target: "impasto::startup", ?target, ?kind, ?blend,
                    pipeline_us = started.elapsed().as_secs_f64() * 1_000_000.0, "raster pipeline ready");
                gpu.pipelines_updated();
                Ok(entry.insert(pipeline).clone())
            }
        }
    }

    pub fn prewarm(
        &mut self,
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        specialize: bool,
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
        tracing::debug!(target: "impasto::startup", ?format, variants = self.targets.len() - count,
            prewarm_us = started.elapsed().as_secs_f64() * 1_000_000.0, "pipeline prewarm complete");
        Ok(())
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
    let (module, groups, vertex, fragment) = match kind {
        K::Rect => (ui, paint.as_slice(), "vs_main", "fs_main"),
        K::RectSolid => (ui, solid.as_slice(), "vs_solid", "fs_solid"),
        K::RectShadow | K::RectCircularShadow => (ui, paint.as_slice(), "vs_shadow", "fs_shadow"),
        K::EarlyZ => (ui, view_only.as_slice(), "vs_depth_rect", "fs_depth_rect"),
        K::StencilWrite => (ui, view_only.as_slice(), "vs_stencil", "fs_stencil"),
        K::Mesh | K::MeshStencil => (ui, paint.as_slice(), "vs_mesh", "fs_mesh"),
        K::TextLcd => (text, text_mask.as_slice(), "vs_mask", "fs_lcd"),
        K::TextMask => (text, text_mask.as_slice(), "vs_mask", "fs_mask"),
        K::Text | K::TextUnclipped => (text, text_groups.as_slice(), "vs_main", "fs_main"),
        K::Image | K::ImageArray | K::ExternalImage => {
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
    };
    let rect_buffers = [rect.clone()];
    let stencil_buffers = [geometry.clone()];
    let mesh_buffers = [geometry, rect];
    let glyph_buffers = [glyph];
    let image_buffers = [quad];
    let buffers = match kind {
        K::Rect | K::RectSolid | K::RectShadow | K::RectCircularShadow | K::EarlyZ => {
            rect_buffers.as_slice()
        }
        K::Mesh | K::MeshStencil => mesh_buffers.as_slice(),
        K::StencilWrite => stencil_buffers.as_slice(),
        K::Text | K::TextUnclipped | K::TextMask | K::TextLcd => glyph_buffers.as_slice(),
        K::Image | K::ImageArray | K::ExternalImage => image_buffers.as_slice(),
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
        | K::RectShadow
        | K::RectCircularShadow
        | K::Mesh
        | K::MeshStencil
        | K::Text
        | K::TextUnclipped
        | K::TextMask
        | K::Image
        | K::ImageArray
        | K::ExternalImage
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
    if gpu.dither_output && kind == K::Rect {
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
            K::Rect | K::Mesh | K::MeshStencil | K::Text | K::TextUnclipped
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
                    write_mask: if kind == K::StencilWrite {
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
        PipelineKind::EarlyZ => Some(wgpu::DepthStencilState {
            format: EARLY_DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
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
