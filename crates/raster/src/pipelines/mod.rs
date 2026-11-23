//! Pipeline cache, bind group layouts, and the instance record layout.
//!
//! Pipelines are keyed by target format and sample count. Each pipeline
//! draws instanced quads from one uniform buffer; the fragment shader reads
//! its per-instance record. Gradient stops live in one uniform buffer that
//! every UI and text pipeline binds.
//!
//! Instance records are raw `f32` words. The Rust structs below must match
//! the WGSL structs field for field, with every `vec4` 16-byte aligned.

use std::collections::HashMap;

use crate::gpu::Gpu;
use crate::status::Status;

/// One pipeline variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub format: wgpu::TextureFormat,
    pub samples: u32,
}

/// Pipelines for one target.
pub struct Pipelines {
    pub rect: wgpu::RenderPipeline,
    pub mesh: wgpu::RenderPipeline,
    pub text: wgpu::RenderPipeline,
    pub image: wgpu::RenderPipeline,
    pub blur: wgpu::RenderPipeline,
    pub composite: wgpu::RenderPipeline,
    pub backdrop: wgpu::RenderPipeline,
    pub copy: wgpu::RenderPipeline,
    pub copy_srgb: wgpu::RenderPipeline,
    pub over: wgpu::RenderPipeline,
}

/// Bind group layouts shared by every target.
pub struct Layouts {
    pub ui: wgpu::BindGroupLayout,
    pub text: wgpu::BindGroupLayout,
    pub image: wgpu::BindGroupLayout,
    pub blur: wgpu::BindGroupLayout,
    pub composite: wgpu::BindGroupLayout,
}

/// View uniform: transform (16 f32) + scale + padding. Matches `View` in WGSL.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewUniform {
    pub transform: [[f32; 4]; 4],
    pub scale: f32,
    pub pad: [f32; 3],
}

/// Effect parameters: texel size xy, down/up flag, opacity or blend mode.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParamsUniform {
    pub values: [f32; 4],
}

/// Stop pairs: each stop is a premultiplied color then `(offset, 0, 0, 0)`.
pub const MAX_STOPS: usize = 256;

const UI_WGSL: &str = include_str!("ui.wgsl");
const TEXT_WGSL: &str = include_str!("text.wgsl");
const IMAGE_WGSL: &str = include_str!("image.wgsl");
const BLUR_WGSL: &str = include_str!("blur.wgsl");

pub struct PipelineCache {
    targets: HashMap<Target, Pipelines>,
    ui: wgpu::ShaderModule,
    text: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    blur: wgpu::ShaderModule,
    pub layouts: Layouts,
}

impl PipelineCache {
    pub fn new(gpu: &Gpu) -> Self {
        let module = |label, source: &str| {
            gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
        };
        let layouts = Layouts {
            // ui.wgsl: view, stops, instances, clip mask, clip sampler, mask atlas, mask sampler
            ui: layout(gpu, "impasto ui", &[U, U, U, T, S]),
            // text.wgsl: view, stops, instances, mask, color, atlas sampler, clip, clip sampler
            text: layout(gpu, "impasto text", &[U, U, U, T, T, S, T, S]),
            // image.wgsl: view, instances, image, linear, nearest, clip, clip sampler
            image: layout(gpu, "impasto image", &[U, U, T, S, T, S]),
            // blur.wgsl: view, params, source, linear
            blur: layout(gpu, "impasto blur", &[U, U, T, S]),
            // blur.wgsl group 1: backdrop, sampler
            composite: layout(gpu, "impasto composite", &[T, S, T]),
        };
        Self {
            targets: HashMap::new(),
            ui: module("impasto ui", UI_WGSL),
            text: module("impasto text", TEXT_WGSL),
            image: module("impasto image", IMAGE_WGSL),
            blur: module("impasto blur", BLUR_WGSL),
            layouts,
        }
    }

    /// Pipelines for a target, built on first use.
    pub fn get(&mut self, gpu: &Gpu, target: Target) -> Result<&Pipelines, Status> {
        if !self.targets.contains_key(&target) {
            let built = gpu.validation(|| {
                build(
                    gpu,
                    &self.layouts,
                    [&self.ui, &self.text, &self.image, &self.blur],
                    target,
                )
            })?;
            self.targets.insert(target, built);
        }
        Ok(self
            .targets
            .get(&target)
            .unwrap_or_else(|| unreachable!("inserted above")))
    }
}

/// Binding kinds in declaration order, matching each WGSL `@group(0)`.
enum Slot {
    Uniform,
    Texture,
    Sampler,
}

fn layout(gpu: &Gpu, label: &str, slots: &[Slot]) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = slots
        .iter()
        .zip(0u32..)
        .map(|(slot, binding)| {
            let (ty, visibility) = match slot {
                Slot::Uniform => (
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                ),
                Slot::Texture => (
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
) -> Result<Pipelines, Status> {
    let [ui, text, image, blur] = shaders;
    let device = &gpu.device;
    let blend = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
    let targets = |blend: wgpu::BlendState| {
        vec![Some(wgpu::ColorTargetState {
            format: target.format,
            blend: Some(blend),
            write_mask: wgpu::ColorWrites::ALL,
        })]
    };
    let multisample = wgpu::MultisampleState {
        count: target.samples,
        ..Default::default()
    };
    let single = wgpu::MultisampleState::default();
    let mesh = wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    };
    let mesh_layouts = [Some(mesh)];
    let instanced = |label: &str,
                     module: &wgpu::ShaderModule,
                     layout: &wgpu::BindGroupLayout,
                     is_mesh: bool| {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(if is_mesh { "vs_mesh" } else { "vs_main" }),
                compilation_options: Default::default(),
                buffers: if is_mesh { &mesh_layouts } else { &[] },
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(if is_mesh { "fs_mesh" } else { "fs_main" }),
                compilation_options: Default::default(),
                targets: &targets(blend),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample,
            multiview_mask: None,
            cache: None,
        })
    };
    let full = |label: &str, entry: &str, groups: &[&wgpu::BindGroupLayout], blend| {
        let group_refs: Vec<Option<&wgpu::BindGroupLayout>> =
            groups.iter().map(|group| Some(*group)).collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &group_refs,
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: blur,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: blur,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &targets(blend),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: single,
            multiview_mask: None,
            cache: None,
        })
    };
    let replace = wgpu::BlendState::REPLACE;
    Ok(Pipelines {
        rect: instanced("impasto rect", ui, &layouts.ui, false),
        mesh: instanced("impasto mesh", ui, &layouts.ui, true),
        text: instanced("impasto text", text, &layouts.text, false),
        image: instanced("impasto image", image, &layouts.image, false),
        blur: full("impasto blur", "fs_blur", &[&layouts.blur], replace),
        composite: full(
            "impasto composite",
            "fs_composite",
            &[&layouts.blur, &layouts.composite],
            replace,
        ),
        backdrop: full(
            "impasto backdrop",
            "fs_backdrop",
            &[&layouts.blur, &layouts.composite],
            replace,
        ),
        copy: full("impasto copy", "fs_copy", &[&layouts.blur], replace),
        copy_srgb: full(
            "impasto copy sRGB",
            "fs_copy_srgb",
            &[&layouts.blur],
            replace,
        ),
        over: full(
            "impasto opacity",
            "fs_over",
            &[&layouts.blur, &layouts.composite],
            blend,
        ),
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
