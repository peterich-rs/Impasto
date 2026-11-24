//! Validates application WGSL and builds device-scoped fragment pipelines.

use std::collections::HashMap;
use std::sync::Arc;

use engine::scene::shader::MAX_SHADER_SOURCE_BYTES;
use engine::scene::{LayerId, ShaderProgram, ShaderProgramId};
use wgpu::naga::{self, Block, Handle, Statement, TypeInner};

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
const MAX_FUNCTIONS: usize = 32;
const MAX_OPERATIONS: usize = 1024;
const MAX_LOCAL_BYTES: u32 = 4096;
const MAX_TEXTURE_SAMPLES: usize = 16;
const MAX_BLOCK_DEPTH: usize = 32;
// Matching signatures let validation resolve public helpers while keeping
// framework globals and entry points invisible to the application.
const HELPERS: &str = "
fn effect_uniform(index: u32) -> f32 { return 0.0; }
fn effect_time() -> f32 { return 0.0; }
fn effect_sample(index: u32, uv: vec2<f32>) -> vec4<f32> { return vec4(0.0); }
";

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
    if source.is_empty() || source.len() > MAX_SHADER_SOURCE_BYTES {
        return Err("runtime effect source exceeds its byte budget".into());
    }
    let sandbox = format!("{source}\n{HELPERS}");
    let module =
        naga::front::wgsl::parse_str(&sandbox).map_err(|error| error.emit_to_string(&sandbox))?;
    if !module.global_variables.is_empty()
        || !module.entry_points.is_empty()
        || !module.overrides.is_empty()
    {
        return Err(
            "runtime effects cannot declare global variables, entry points, or overrides".into(),
        );
    }
    if module.functions.len() > MAX_FUNCTIONS {
        return Err("runtime effect exceeds its function budget".into());
    }
    let (_, entry) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("effect_main"))
        .ok_or_else(|| "missing effect_main".to_owned())?;
    let vec_type = |ty: Handle<naga::Type>, expected| {
        matches!(
            module.types[ty].inner,
            TypeInner::Vector { size, scalar: naga::Scalar::F32 } if size == expected
        )
    };
    if entry.arguments.len() != 2
        || !entry
            .arguments
            .iter()
            .all(|arg| vec_type(arg.ty, naga::VectorSize::Bi))
        || !entry
            .result
            .as_ref()
            .is_some_and(|result| vec_type(result.ty, naga::VectorSize::Quad))
    {
        return Err("expected fn effect_main(vec2<f32>, vec2<f32>) -> vec4<f32>".into());
    }
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|error| error.to_string())?;
    if module
        .types
        .iter()
        .any(|(id, _)| layouter[id].size > MAX_LOCAL_BYTES)
    {
        return Err("runtime effect exceeds its type size budget".into());
    }
    // Naga orders WGSL declarations by dependency and rejects cycles before
    // lowering. Its function handles therefore index already accumulated costs.
    let mut costs = Vec::with_capacity(module.functions.len());
    for (_, function) in module.functions.iter() {
        let bytes = function
            .local_variables
            .iter()
            .fold(0_u32, |bytes, (_, variable)| {
                bytes.saturating_add(layouter[variable.ty].size)
            });
        if bytes > MAX_LOCAL_BYTES {
            return Err("runtime effect exceeds its local memory budget".into());
        }
        let mut cost = Cost {
            operations: function.expressions.len(),
            texture_samples: usize::from(function.name.as_deref() == Some("effect_sample")),
        };
        visit_block(&function.body, &costs, &mut cost, 0)?;
        costs.push(cost);
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|error| error.emit_to_string(&sandbox))?;
    let wgsl = wrap_source(source);
    // This also checks collisions with every wrapper symbol, including
    // aliases that disappear during lowering.
    naga::front::wgsl::parse_str(&wgsl).map_err(|error| error.emit_to_string(&wgsl))?;
    Ok(wgsl)
}

#[derive(Clone, Copy)]
struct Cost {
    operations: usize,
    texture_samples: usize,
}

fn visit_block(block: &Block, costs: &[Cost], cost: &mut Cost, depth: usize) -> Result<(), String> {
    if depth > MAX_BLOCK_DEPTH {
        return Err("runtime effect exceeds its block nesting budget".into());
    }
    for statement in block {
        cost.operations = cost.operations.saturating_add(1);
        match statement {
            Statement::Loop { .. } | Statement::Kill => {
                return Err("runtime effects cannot loop or discard fragments".into());
            }
            Statement::Block(block) => visit_block(block, costs, cost, depth + 1)?,
            Statement::If { accept, reject, .. } => {
                visit_block(accept, costs, cost, depth + 1)?;
                visit_block(reject, costs, cost, depth + 1)?;
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    visit_block(&case.body, costs, cost, depth + 1)?;
                }
            }
            Statement::Call { function, .. } => {
                let target = costs
                    .get(function.index())
                    .ok_or("runtime effect has an unresolved function dependency")?;
                cost.operations = cost.operations.saturating_add(target.operations);
                cost.texture_samples = cost.texture_samples.saturating_add(target.texture_samples);
            }
            _ => {}
        }
        if cost.operations > MAX_OPERATIONS {
            return Err("runtime effect exceeds its expanded instruction budget".into());
        }
        if cost.texture_samples > MAX_TEXTURE_SAMPLES {
            return Err("runtime effect exceeds its texture sample budget".into());
        }
    }
    Ok(())
}

fn wrap_source(source: &str) -> String {
    let view = include_str!("pipelines/view.wgsl");
    let clip = include_str!("pipelines/clip.wgsl");
    let color = include_str!("pipelines/color_precision.wgsl");
    let wrapper = include_str!("pipelines/runtime_effect.wgsl");
    format!(
        "{source}\nalias ReducedColor = vec4<f32>;\nalias ReducedScalar = f32;\n{color}\n{view}\n{clip}\n{wrapper}"
    )
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
