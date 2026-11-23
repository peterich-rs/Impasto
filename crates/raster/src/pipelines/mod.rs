//! Pipeline cache, bind group layouts, and the instance record layout.
//!
//! Pipelines are keyed by target format and sample count. Each pipeline
//! draws quads from instance vertex buffers and shares the frame uniform slabs.

use std::collections::HashMap;

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
    Mesh,
    Text,
    Image,
    BlurDown,
    BlurUp,
    Copy,
    CopySrgb,
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
    pub clip_radii: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParamsUniform {
    pub values: [f32; 4],
    pub uv_bounds: [f32; 4],
}

/// Stop pairs: each stop is a premultiplied color then `(offset, 0, 0, 0)`.
pub const MAX_STOPS: usize = 256;

const UI_WGSL: &str = include_str!("ui.wgsl");
const TEXT_WGSL: &str = include_str!("text.wgsl");
const IMAGE_WGSL: &str = include_str!("image.wgsl");
const BLUR_WGSL: &str = include_str!("blur.wgsl");

pub struct PipelineCache {
    targets: HashMap<(Target, PipelineKind, BlendKey), wgpu::RenderPipeline>,
    ui: wgpu::ShaderModule,
    text: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    blur: wgpu::ShaderModule,
    pub layouts: Layouts,
}

impl PipelineCache {
    pub fn new(gpu: &Gpu) -> Self {
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
            gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(
                        format!("{view}\n{source}\n{clip}\n{gradient}").into(),
                    ),
                })
        };
        let view = U(size_of::<ViewUniform>() as u64);
        let layouts = Layouts {
            view: layout(gpu, "impasto view", &[view]),
            paint: layout(gpu, "impasto paint", &[view, U((MAX_STOPS * 32) as u64)]),
            effects: layout(
                gpu,
                "impasto effects",
                &[view, U(size_of::<ParamsUniform>() as u64)],
            ),
            material: layout(gpu, "impasto material", &[T, S]),
            atlas: layout(gpu, "impasto atlas", &[Slot::Array, Slot::Array, S]),
            composite: layout(gpu, "impasto composite", &[T, S, T]),
        };
        Self {
            targets: HashMap::new(),
            ui: module("impasto ui", UI_WGSL, true, true),
            text: module("impasto text", TEXT_WGSL, true, true),
            image: module("impasto image", IMAGE_WGSL, true, false),
            blur: module("impasto blur", BLUR_WGSL, false, false),
            layouts,
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
        use std::collections::hash_map::Entry;
        let key = (target, kind, blend);
        match self.targets.entry(key) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let pipeline = gpu.validation(|| {
                    Ok(build(
                        gpu,
                        &self.layouts,
                        [&self.ui, &self.text, &self.image, &self.blur],
                        target,
                        kind,
                        blend,
                    ))
                })?;
                Ok(entry.insert(pipeline).clone())
            }
        }
    }
}

/// Binding kinds in declaration order, matching each WGSL `@group(0)`.
#[derive(Clone, Copy)]
enum Slot {
    Uniform(u64),
    Texture,
    Array,
    Sampler,
}

fn layout(gpu: &Gpu, label: &str, slots: &[Slot]) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = slots
        .iter()
        .zip(0u32..)
        .map(|(slot, binding)| {
            let (ty, visibility) = match slot {
                Slot::Uniform(size) => (
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(*size),
                    },
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                ),
                Slot::Texture | Slot::Array => (
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: if matches!(slot, Slot::Array) {
                            wgpu::TextureViewDimension::D2Array
                        } else {
                            wgpu::TextureViewDimension::D2
                        },
                        multisampled: false,
                    },
                    wgpu::ShaderStages::FRAGMENT,
                ),
                Slot::Sampler => (
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    wgpu::ShaderStages::FRAGMENT,
                ),
            };
            wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                count: None,
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
        4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
    let text_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
        3 => Float32x4, 4 => Float32x4, 5 => Float32x4];
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
    let paint = [Some(&layouts.paint), Some(&layouts.material)];
    let text_groups = [
        Some(&layouts.paint),
        Some(&layouts.atlas),
        Some(&layouts.material),
    ];
    let image_groups = [
        Some(&layouts.view),
        Some(&layouts.material),
        Some(&layouts.material),
    ];
    let effects = [Some(&layouts.effects), Some(&layouts.material)];
    let composite = [
        Some(&layouts.effects),
        Some(&layouts.material),
        Some(&layouts.composite),
    ];
    let (module, groups, vertex, fragment) = match kind {
        K::Rect => (ui, paint.as_slice(), "vs_main", "fs_main"),
        K::Mesh => (ui, paint.as_slice(), "vs_mesh", "fs_mesh"),
        K::Text => (text, text_groups.as_slice(), "vs_main", "fs_main"),
        K::Image => (image, image_groups.as_slice(), "vs_main", "fs_main"),
        K::BlurDown => (blur, effects.as_slice(), "vs_full", "fs_blur_down"),
        K::BlurUp => (blur, effects.as_slice(), "vs_full", "fs_blur_up"),
        K::Copy | K::MaskIntersect => (blur, effects.as_slice(), "vs_full", "fs_copy"),
        K::CopySrgb => (blur, effects.as_slice(), "vs_full", "fs_copy_srgb"),
        K::Over => (blur, composite.as_slice(), "vs_full", "fs_over"),
        K::Multiply => (blur, composite.as_slice(), "vs_full", "fs_composite"),
        K::Backdrop => (blur, composite.as_slice(), "vs_full", "fs_backdrop"),
    };
    let rect_buffers = [rect.clone()];
    let mesh_buffers = [geometry, rect];
    let glyph_buffers = [glyph];
    let image_buffers = [quad];
    let buffers = match kind {
        K::Rect => rect_buffers.as_slice(),
        K::Mesh => mesh_buffers.as_slice(),
        K::Text => glyph_buffers.as_slice(),
        K::Image => image_buffers.as_slice(),
        _ => &[],
    };
    let blend = match kind {
        K::Rect | K::Mesh | K::Text | K::Image | K::Over => blend.state(),
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
    let label = format!("impasto {kind:?} {blend:?}");
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&label),
            bind_group_layouts: groups,
            immediate_size: 0,
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
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: target.samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
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
    pub fn get(
        &self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        nearest: bool,
    ) -> (wgpu::BindGroup, bool) {
        let slot = if nearest { &self.nearest } else { &self.linear };
        let mut created = false;
        let group = slot.get_or_init(|| {
            created = true;
            bind_group(
                gpu,
                layout,
                &[
                    wgpu::BindingResource::TextureView(view),
                    wgpu::BindingResource::Sampler(sampler),
                ],
            )
        });
        (group.clone(), created)
    }
}
