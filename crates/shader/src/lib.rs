//! WGSL contract shared by the application compiler and the raster backend.

use naga::{Block, Handle, Statement, TypeInner};

#[cfg(feature = "portability")]
mod portability;
#[cfg(feature = "portability")]
pub use portability::validate_portability;

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

/// Validate the application portion of a runtime effect against its drawing contract.
///
/// # Errors
///
/// Reports WGSL diagnostics and violations of the runtime effect resource budgets.
pub fn validate_effect(source: &str, byte_budget: usize) -> Result<(), String> {
    validate(source, byte_budget).map(|_| ())
}

/// One reflected field in a shader's packed f32 uniform region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformField {
    pub name: String,
    /// First index accepted by `effect_uniform`.
    pub index: u32,
    /// Number of consecutive f32 components.
    pub components: u32,
}

/// Reflects an optional `EffectUniforms` structure from a runtime effect.
///
/// # Errors
///
/// Reports invalid shaders, unsupported member types, or a layout larger than
/// the renderer's packed uniform region.
pub fn uniform_fields(
    source: &str,
    source_limit: usize,
    uniform_limit: usize,
) -> Result<Vec<UniformField>, String> {
    let module = validate(source, source_limit)?;
    let Some((_, ty)) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("EffectUniforms"))
    else {
        return Ok(Vec::new());
    };
    let TypeInner::Struct { members, span } = &ty.inner else {
        return Err("EffectUniforms must be a WGSL struct".into());
    };
    if *span as usize > uniform_limit {
        return Err(format!(
            "EffectUniforms uses {span} bytes, exceeding the {uniform_limit}-byte limit"
        ));
    }
    members
        .iter()
        .map(|member| {
            let name = member
                .name
                .clone()
                .ok_or("expected a named EffectUniforms field")?;
            let components = match module.types[member.ty].inner {
                TypeInner::Scalar(naga::Scalar::F32) => 1,
                TypeInner::Vector {
                    size,
                    scalar: naga::Scalar::F32,
                } => u32::from(size as u8),
                _ => {
                    return Err(format!(
                        "EffectUniforms.{name} must be f32 or a vector of f32"
                    ));
                }
            };
            Ok(UniformField {
                name,
                index: member.offset / 4,
                components,
            })
        })
        .collect()
}

/// Validates the application code and supplies the renderer's bindings and entry points.
///
/// # Errors
///
/// Reports invalid effect code or a collision with symbols from the renderer.
pub fn wrapped_effect(source: &str, byte_budget: usize) -> Result<String, String> {
    validate_effect(source, byte_budget)?;
    let wgsl = wrap(source);
    naga::front::wgsl::parse_str(&wgsl).map_err(|error| error.emit_to_string(&wgsl))?;
    Ok(wgsl)
}

fn wrap(source: &str) -> String {
    let view = include_str!("../../raster/src/pipelines/view.wgsl");
    let clip = include_str!("../../raster/src/pipelines/clip.wgsl");
    let color = include_str!("../../raster/src/pipelines/color_precision.wgsl");
    let wrapper = include_str!("../../raster/src/pipelines/runtime_effect.wgsl");
    format!(
        "{source}\nalias ReducedColor = vec4<f32>;\nalias ReducedScalar = f32;\n{color}\n{view}\n{clip}\n{wrapper}"
    )
}

fn validate(source: &str, byte_budget: usize) -> Result<naga::Module, String> {
    if source.is_empty() || source.len() > byte_budget {
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
    Ok(module)
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
