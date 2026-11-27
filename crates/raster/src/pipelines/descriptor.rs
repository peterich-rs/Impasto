//! Render pipeline descriptors and device-dependent variants.

use super::{BlendKey, Layouts, ParamsUniform, PipelineKind, Target};
use crate::gpu::Gpu;

#[derive(Clone, Copy)]
enum Shader {
    Ui,
    Text,
    Image,
    Effects,
}

enum Bindings {
    Paint,
    Solid,
    TextMask,
    Text,
    Image,
    Effects,
    Composite,
    BackdropMaterial,
    ColorFilter,
}

#[derive(Clone, Copy)]
enum Vertices {
    Rect,
    Mesh,
    Glyph,
    Image,
    None,
}

fn stages(kind: PipelineKind) -> (Shader, Bindings, Vertices, [&'static str; 2]) {
    use PipelineKind as K;

    match kind {
        K::Rect | K::RectUnclipped | K::RectDither | K::RectDitherUnclipped => (
            Shader::Ui,
            Bindings::Paint,
            Vertices::Rect,
            ["vs_main", "fs_main"],
        ),
        K::RectSolid | K::RectSolidUnclipped => (
            Shader::Ui,
            Bindings::Solid,
            Vertices::Rect,
            ["vs_solid", "fs_solid"],
        ),
        K::RectShadow | K::RectCircularShadow => (
            Shader::Ui,
            Bindings::Paint,
            Vertices::Rect,
            ["vs_shadow", "fs_shadow"],
        ),
        K::Mesh => (
            Shader::Ui,
            Bindings::Paint,
            Vertices::Mesh,
            ["vs_mesh", "fs_mesh"],
        ),
        K::TextMask => (
            Shader::Text,
            Bindings::TextMask,
            Vertices::Glyph,
            ["vs_mask", "fs_mask"],
        ),
        K::Text | K::TextUnclipped => (
            Shader::Text,
            Bindings::Text,
            Vertices::Glyph,
            ["vs_main", "fs_main"],
        ),
        K::Image | K::ImageUnclipped | K::ExternalImage => (
            Shader::Image,
            Bindings::Image,
            Vertices::Image,
            ["vs_main", "fs_main"],
        ),
        K::BlurDown => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_blur_down"],
        ),
        K::BlurUp => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_blur_up"],
        ),
        K::Gaussian => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_gaussian"],
        ),
        K::Copy | K::MaskIntersect => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_copy"],
        ),
        K::CopySrgb => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_copy_srgb"],
        ),
        K::P3Linear => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_p3_linear"],
        ),
        K::P3Encoded => (
            Shader::Effects,
            Bindings::Effects,
            Vertices::None,
            ["vs_full", "fs_p3_encoded"],
        ),
        K::Over => (
            Shader::Effects,
            Bindings::Composite,
            Vertices::None,
            ["vs_full", "fs_over"],
        ),
        K::Multiply => (
            Shader::Effects,
            Bindings::Composite,
            Vertices::None,
            ["vs_full", "fs_composite"],
        ),
        K::Backdrop => (
            Shader::Effects,
            Bindings::Composite,
            Vertices::None,
            ["vs_full", "fs_backdrop"],
        ),
        K::BackdropMaterial => (
            Shader::Effects,
            Bindings::BackdropMaterial,
            Vertices::None,
            ["vs_full", "fs_backdrop_material"],
        ),
        K::ColorFilter => (
            Shader::Effects,
            Bindings::ColorFilter,
            Vertices::None,
            ["vs_full", "fs_color_filter"],
        ),
    }
}

pub(super) fn build(
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
        4 => Float32, 5 => Uint32, 6 => Uint32, 7 => Float32];
    let text_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Uint16x4, 2 => Float16x4,
        3 => Uint32, 4 => Uint32];
    let image_attrs =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x4];
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
    let paint_layout = &layouts.paint;
    let paint = [Some(paint_layout), Some(&layouts.material)];
    let solid = [view, Some(&layouts.material)];
    let text_mask = [view, Some(&layouts.atlas), Some(&layouts.material)];
    let text_groups = [
        Some(paint_layout),
        Some(&layouts.atlas),
        Some(&layouts.material),
    ];
    let image_groups = [
        view,
        match kind {
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
    let (shader, bindings, vertices, [vertex, fragment]) = stages(kind);
    let module = match shader {
        Shader::Ui => ui,
        Shader::Text => text,
        Shader::Image => image,
        Shader::Effects => blur,
    };
    let groups = match bindings {
        Bindings::Paint => paint.as_slice(),
        Bindings::Solid => solid.as_slice(),
        Bindings::TextMask => text_mask.as_slice(),
        Bindings::Text => text_groups.as_slice(),
        Bindings::Image => image_groups.as_slice(),
        Bindings::Effects => effects.as_slice(),
        Bindings::Composite => composite.as_slice(),
        Bindings::BackdropMaterial => backdrop_material.as_slice(),
        Bindings::ColorFilter => color_filter.as_slice(),
    };
    let rect_buffers = [rect.clone()];
    let mesh_buffers = [geometry, rect];
    let glyph_buffers = [glyph];
    let image_buffers = [quad];
    let buffers = match vertices {
        Vertices::Rect => rect_buffers.as_slice(),
        Vertices::Mesh => mesh_buffers.as_slice(),
        Vertices::Glyph => glyph_buffers.as_slice(),
        Vertices::Image => image_buffers.as_slice(),
        Vertices::None => &[],
    };
    let blend = color_blend(kind, blend);
    let constants = fragment_constants(gpu, target, kind);
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
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[],
                    ..Default::default()
                },
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
            cache: gpu.pipeline_cache(),
        })
}

fn color_blend(kind: PipelineKind, blend: BlendKey) -> wgpu::BlendState {
    use PipelineKind as K;
    match kind {
        K::Rect
        | K::RectUnclipped
        | K::RectDither
        | K::RectDitherUnclipped
        | K::RectSolid
        | K::RectSolidUnclipped
        | K::RectShadow
        | K::RectCircularShadow
        | K::Mesh
        | K::Text
        | K::TextUnclipped
        | K::TextMask
        | K::Image
        | K::ImageUnclipped
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
    }
}

fn fragment_constants(gpu: &Gpu, target: Target, kind: PipelineKind) -> Vec<(&'static str, f64)> {
    use PipelineKind as K;
    let mut constants = Vec::new();
    if kind == K::RectCircularShadow {
        constants.push(("circular_shadow", 1.0));
    }
    if matches!(kind, K::RectDither | K::RectDitherUnclipped) {
        constants.push(("output_dither", 1.0));
    }
    if matches!(
        kind,
        K::RectUnclipped | K::RectDitherUnclipped | K::RectSolidUnclipped | K::ImageUnclipped
    ) {
        constants.push(("unclipped", 1.0));
    }
    if gpu.device.features().contains(wgpu::Features::SHADER_F16)
        && matches!(
            target.format,
            wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
        )
        && matches!(
            kind,
            K::Rect
                | K::RectUnclipped
                | K::RectDither
                | K::RectDitherUnclipped
                | K::Mesh
                | K::Text
                | K::TextUnclipped
                | K::BlurDown
                | K::BlurUp
        )
    {
        constants.push(("reduced_color_precision", 1.0));
    }
    if kind == K::TextUnclipped {
        constants.push(("unclipped", 1.0));
    }
    constants
}
