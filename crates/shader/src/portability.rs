use naga::{
    AddressSpace, Module, TypeInner,
    back::{glsl, msl, spv},
    valid,
};

/// Preflights Naga lowering for Metal, Vulkan and OpenGL ES using the effect's real entry points.
///
/// # Errors
///
/// Reports invalid WGSL or a translation failure with the failing backend.
pub fn validate_portability(source: &str, byte_budget: usize) -> Result<(), String> {
    super::validate_effect(source, byte_budget)?;
    let wgsl = super::wrap(source);
    let module =
        naga::front::wgsl::parse_str(&wgsl).map_err(|error| error.emit_to_string(&wgsl))?;
    let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
        .validate(&module)
        .map_err(|error| error.emit_to_string(&wgsl))?;
    let (module, info) = naga::back::pipeline_constants::process_overrides(
        &module,
        &info,
        None,
        &Default::default(),
    )
    .map_err(|error| format!("renderer pipeline constants: {error}"))?;
    metal(&module, &info)?;
    spv::write_vec(&module, &info, &spv::Options::default(), None)
        .map_err(|error| format!("Vulkan SPIR-V: {error}"))?;
    for entry in &module.entry_points {
        let options = glsl::Options {
            version: glsl::Version::new_gles(300),
            ..Default::default()
        };
        let pipeline = glsl::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
            multiview: None,
        };
        glsl::Writer::new(
            String::new(),
            &module,
            &info,
            &options,
            &pipeline,
            Default::default(),
        )
        .and_then(|mut writer| writer.write())
        .map_err(|error| format!("OpenGL ES: {}: {error}", entry.name))?;
    }
    Ok(())
}

fn metal(module: &Module, info: &valid::ModuleInfo) -> Result<(), String> {
    let mut slots = msl::BindingMap::default();
    for (index, (_, variable)) in module.global_variables.iter().enumerate() {
        let Some(binding) = variable.binding else {
            continue;
        };
        let index = u8::try_from(index).map_err(|_| "Metal resource slot limit exceeded")?;
        let target = match module.types[variable.ty].inner {
            TypeInner::Sampler { .. } => msl::BindTarget {
                sampler: Some(msl::BindSamplerTarget::Resource(index)),
                ..Default::default()
            },
            TypeInner::Image { .. } => msl::BindTarget {
                texture: Some(index),
                ..Default::default()
            },
            _ if variable.space == AddressSpace::Uniform => msl::BindTarget {
                buffer: Some(index),
                ..Default::default()
            },
            _ => return Err("Metal: unexpected effect binding".to_owned()),
        };
        slots.insert(binding, target);
    }
    let options = msl::Options {
        lang_version: (2, 0),
        fake_missing_bindings: false,
        per_entry_point_map: module
            .entry_points
            .iter()
            .map(|entry| {
                (
                    entry.name.clone(),
                    msl::EntryPointResources {
                        resources: slots.clone(),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let pipeline = msl::PipelineOptions {
        vertex_pulling_transform: false,
        ..Default::default()
    };
    let (_, result) = msl::write_string(module, info, &options, &pipeline)
        .map_err(|error| format!("Metal MSL: {error}"))?;
    for name in result.entry_point_names {
        name.map_err(|error| format!("Metal MSL: {error:?}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn effect_uses_all_renderer_backends() {
    validate_portability(
        r#"
        fn effect_main(position: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
            return effect_sample(0u, position / size) * effect_uniform(2u);
        }
    "#,
        4096,
    )
    .unwrap();
}
