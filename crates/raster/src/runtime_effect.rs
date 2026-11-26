//! Validates application WGSL and builds device-scoped fragment pipelines.

use std::collections::HashMap;
use std::sync::Arc;

use engine::scene::shader::MAX_SHADER_SOURCE_BYTES;
use engine::scene::{LayerId, ShaderProgram, ShaderProgramId};

use crate::gpu::Gpu;
use crate::pipelines::Layouts;

/// A shader rejected before it can affect the enclosing frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectFailure {
    pub shader: ShaderProgramId,
    pub reason: String,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct EffectParams {
    pub uniforms: [[f32; 4]; 8],
    pub mappings: [[f32; 4]; 4],
    pub sample_modes: [u32; 4],
}

type PipelineKey = (Arc<str>, wgpu::TextureFormat);
pub(crate) type ShaderUniformOverrides = HashMap<LayerId, [Option<f32>; 32]>;

#[derive(Default)]
pub(crate) struct RuntimeEffects {
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    rejected: HashMap<Arc<str>, String>,
    reported: HashMap<ShaderProgramId, Arc<str>>,
    failures: Vec<EffectFailure>,
}

const MAX_PIPELINES: usize = 128;
const MAX_FAILURES: usize = 64;
impl RuntimeEffects {
    pub fn pipeline(
        &mut self,
        gpu: &Gpu,
        layouts: &Layouts,
        program: &ShaderProgram,
        format: wgpu::TextureFormat,
    ) -> Option<wgpu::RenderPipeline> {
        let key = (Arc::clone(&program.source), format);
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Some(pipeline.clone());
        }
        if let Some(reason) = self.rejected.get(&program.source) {
            let reason = reason.clone();
            self.report(program, reason);
            return None;
        }
        let _span = tracing::debug_span!(target: "impasto::effect", "shader_compile",
            shader_id = program.id.0)
        .entered();
        let pipeline = validate_source(&program.source).and_then(|wgsl| {
            gpu.validation(|| Ok(create_pipeline(gpu, layouts, wgsl, format)))
                .map_err(|error| error.to_string())
        });
        match pipeline {
            Ok(pipeline) => {
                if self.pipelines.len() >= MAX_PIPELINES {
                    self.pipelines.clear();
                }
                self.pipelines.insert(key, pipeline.clone());
                Some(pipeline)
            }
            Err(reason) => {
                if self.rejected.len() >= MAX_FAILURES {
                    self.rejected.clear();
                }
                self.rejected
                    .insert(Arc::clone(&program.source), reason.clone());
                self.report(program, reason);
                None
            }
        }
    }

    fn report(&mut self, program: &ShaderProgram, reason: String) {
        if self.reported.get(&program.id) == Some(&program.source) {
            return;
        }
        if self.reported.len() >= MAX_FAILURES {
            self.reported.clear();
        }
        self.reported
            .insert(program.id, Arc::clone(&program.source));
        tracing::warn!(target: "impasto::effect", shader_id = program.id.0, %reason,
            "runtime effect rejected");
        if self.failures.len() < MAX_FAILURES {
            self.failures.push(EffectFailure {
                shader: program.id,
                reason,
            });
        }
    }

    pub fn take_failures(&mut self) -> Vec<EffectFailure> {
        std::mem::take(&mut self.failures)
    }
}

fn validate_source(source: &str) -> Result<String, String> {
    shader::wrapped_effect(source, MAX_SHADER_SOURCE_BYTES)
}

fn create_pipeline(
    gpu: &Gpu,
    layouts: &Layouts,
    wgsl: String,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let device = &gpu.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("impasto runtime effect"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("impasto runtime effect"),
        bind_group_layouts: &[
            Some(&layouts.runtime_effect),
            Some(&layouts.runtime_textures),
            Some(&layouts.material),
        ],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("impasto runtime effect"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("effect_vertex"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<[f32; 4]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x4],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("effect_fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: gpu.pipeline_cache(),
    })
}

#[cfg(test)]
mod tests {
    use super::validate_source;

    #[test]
    fn validates_the_fragment_contract() {
        validate_source("fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> { return effect_sample(0, p / size) * effect_uniform(3) + vec4(effect_time()); }").unwrap();
        for body in [
            "fn effect_main(p: vec2<f32>) -> vec4<f32> { return vec4(1.0); }",
            "@group(2) @binding(0) var<storage, read_write> out: array<u32>;",
            "fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> { loop {} }",
            "fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> { return vec4(view.info.x); }",
        ] {
            assert!(validate_source(body).is_err(), "{body}");
        }
    }

    #[test]
    fn supports_forward_calls_and_enforces_expanded_budgets() {
        let source = "
            fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
                return twice(p / size);
            }
            fn twice(uv: vec2<f32>) -> vec4<f32> { return sample(uv) + sample(uv); }
            fn sample(uv: vec2<f32>) -> vec4<f32> { return effect_sample(0, uv); }
        ";
        validate_source(source).unwrap();
        let recursive = source.replace("effect_sample(0, uv)", "twice(uv)");
        assert!(validate_source(&recursive).unwrap_err().contains("cyclic"));
        let over_budget = source.replace("twice(p / size)", &["twice(p / size)"; 9].join(" + "));
        assert!(
            validate_source(&over_budget)
                .unwrap_err()
                .contains("texture sample budget")
        );
    }

    #[test]
    fn wrapper_declarations_are_reserved() {
        for name in [
            "View",
            "EffectParams",
            "ReducedColor",
            "clip_coverage",
            "effect_vertex",
            "effect_fragment",
        ] {
            let source = format!(
                "alias {name} = f32;
                 fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {{ return vec4(0.0); }}"
            );
            let error = validate_source(&source).unwrap_err();
            assert!(
                error.contains("redefinition") && error.contains(name),
                "{error}"
            );
        }
    }
}
