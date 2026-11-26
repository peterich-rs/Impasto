//! Impasto TSX compilation and application linking.

use std::path::Path;

use oxc::{
    allocator::Allocator,
    codegen::{Codegen, CodegenOptions},
    diagnostics::OxcDiagnostic,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{JsxOptions, TransformOptions, Transformer, TypeScriptOptions},
};

#[cfg(feature = "bundle")]
mod assets;
#[cfg(feature = "bundle")]
mod bundle;
#[cfg(feature = "bundle")]
mod function_symbols;
#[cfg(feature = "bundle")]
mod hot;
#[cfg(feature = "bundle")]
pub use function_symbols::function_symbols;
#[cfg(feature = "bundle")]
mod svg;
#[cfg(feature = "bundle")]
pub use bundle::{Bundle, BundleOptions, PluginManifest, WatchLinker, bundle};
#[cfg(feature = "bundle")]
pub use hot::delta;

#[cfg(feature = "wgsl")]
mod diagnostics;
mod jsx;
mod props;
mod styles;
#[cfg(all(feature = "wgsl", feature = "bundle"))]
mod wgsl;
#[cfg(all(feature = "wgsl", feature = "bundle"))]
pub use wgsl::shader_declarations;

/// JavaScript and its original source positions.
#[derive(Debug)]
pub struct CompiledModule {
    /// Transformed ES module text.
    pub code: String,
    /// Framework diagnostics that allow code generation to continue.
    pub warnings: Vec<String>,
    /// Source map from the transformed module to its input.
    pub map: oxc_sourcemap::SourceMap<'static>,
}

/// Compile a JavaScript or TypeScript module using Impasto's JSX semantics.
///
/// # Errors
///
/// Returns a source diagnostic when parsing or lowering the module fails.
pub fn compile_module(path: &Path, source: &str) -> Result<CompiledModule, String> {
    let source_type = SourceType::from_path(path).map_err(|error| error.to_string())?;
    compile(path, source, source_type)
}

fn compile(path: &Path, source: &str, source_type: SourceType) -> Result<CompiledModule, String> {
    compile_inner(path, source, source_type, false).map(|(compiled, _)| compiled)
}

#[cfg(feature = "bundle")]
type HotMetadata = hot::ComponentModule;
#[cfg(not(feature = "bundle"))]
type HotMetadata = ();

fn compile_inner(
    path: &Path,
    source: &str,
    source_type: SourceType,
    hot: bool,
) -> Result<(CompiledModule, HotMetadata), String> {
    let diagnostic = |error: OxcDiagnostic| {
        format!(
            "{}\n{}",
            path.display(),
            error.render_with_source_code(source.to_owned())
        )
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(error) = parsed.diagnostics.into_iter().next() {
        return Err(diagnostic(error));
    }
    let mut program = parsed.program;
    let warnings = props::check(&program).into_iter().map(diagnostic).collect();
    let semantic = SemanticBuilder::new().with_enum_eval(true).build(&program);
    if let Some(error) = semantic.diagnostics.into_iter().next() {
        return Err(diagnostic(error));
    }
    #[cfg(feature = "bundle")]
    let metadata = if hot {
        hot::components(&program, source)
    } else {
        HotMetadata::default()
    };
    #[cfg(not(feature = "bundle"))]
    let metadata = ();
    let options = TransformOptions {
        jsx: JsxOptions::disable(),
        typescript: TypeScriptOptions {
            only_remove_type_imports: true,
            ..TypeScriptOptions::default()
        },
        ..TransformOptions::default()
    };
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if let Some(error) = transformed.diagnostics.into_iter().next() {
        return Err(diagnostic(error));
    }
    #[cfg(feature = "wgsl")]
    diagnostics::check_shaders(&program, &transformed.scoping).map_err(diagnostic)?;
    jsx::lower(&allocator, &mut program, &transformed.scoping, source, hot).map_err(diagnostic)?;
    let output = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: (!hot).then(|| path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let map = if hot {
        oxc_sourcemap::SourceMap::default()
    } else {
        output
            .map
            .ok_or_else(|| format!("{}: missing source map", path.display()))?
    };
    Ok((
        CompiledModule {
            code: output.code,
            warnings,
            map: map.into_owned(),
        },
        metadata,
    ))
}

/// Compile browser-provided TSX to JavaScript and its source map as JSON.
///
/// # Errors
///
/// Reports any parsing or JSX lowering diagnostic returned by the compiler.
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn compile_tsx(filename: &str, source: &str) -> Result<String, String> {
    let compiled = compile_module(Path::new(filename), source)?;
    Ok(serde_json::json!({
        "code": compiled.code,
        "warnings": compiled.warnings,
        "map": compiled.map.to_json_string(),
    })
    .to_string())
}
