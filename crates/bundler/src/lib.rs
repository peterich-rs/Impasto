//! Impasto TSX compilation and application linking.

use std::path::Path;

#[cfg(feature = "bundle")]
pub mod i18n;
#[cfg(feature = "bundle")]
mod theme;
#[cfg(feature = "bundle")]
pub use theme::ThemeTokens;

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
pub use bundle::{
    Bundle, BundleOptions, ChunkOutput, ModuleCache, NativeUse, PluginManifest, WatchLinker,
    bundle, bundle_with_cache,
};
#[cfg(feature = "bundle")]
pub use hot::delta;

#[cfg(feature = "wgsl")]
mod diagnostics;
mod jsx;
mod pools;
mod props;
mod styles;
#[cfg(all(feature = "wgsl", feature = "bundle"))]
mod wgsl;
#[cfg(all(feature = "wgsl", feature = "bundle"))]
pub use wgsl::{ShaderDeclarations, shader_declarations, shader_types};

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
    compile_inner(
        path,
        source,
        source_type,
        false,
        None,
        #[cfg(feature = "bundle")]
        None,
    )
    .map(|(compiled, _)| compiled)
}

#[cfg(feature = "bundle")]
type HotMetadata = hot::ComponentModule;
#[cfg(not(feature = "bundle"))]
type HotMetadata = ();

#[cfg(feature = "bundle")]
struct LinkContext<'a> {
    assets: &'a Path,
    root: &'a Path,
    framework: &'a Path,
    locales: &'a i18n::Locales,
    theme: &'a ThemeTokens,
    package_entry: &'a dyn Fn(&str) -> Option<std::path::PathBuf>,
    release: bool,
    dependencies: &'a mut std::collections::BTreeSet<std::path::PathBuf>,
}

fn compile_inner(
    path: &Path,
    source: &str,
    source_type: SourceType,
    hot: bool,
    pools: Option<&mut pools::PoolModules>,
    #[cfg(feature = "bundle")] link: Option<LinkContext<'_>>,
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
    let semantic = SemanticBuilder::new()
        .with_enum_eval(true)
        .with_build_nodes(source_type.is_jsx() || hot || source.contains("await"))
        .build(&program);
    if let Some(error) = semantic.diagnostics.into_iter().next() {
        return Err(diagnostic(error));
    }
    let warnings = props::check(&program, &semantic.semantic)
        .into_iter()
        .map(diagnostic)
        .collect();
    #[cfg(feature = "bundle")]
    let metadata = if hot {
        hot::components(&program, source, &semantic.semantic)
    } else {
        HotMetadata::default()
    };
    #[cfg(not(feature = "bundle"))]
    let metadata = ();
    #[cfg(feature = "bundle")]
    let mut link = link;
    let source_file = if hot {
        #[cfg(feature = "bundle")]
        let file = link.as_ref().and_then(|link| {
            path.strip_prefix(link.root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .or_else(|| {
                    path.strip_prefix(link.framework)
                        .ok()
                        .map(|path| format!("@framework/{}", path.display()))
                })
        });
        #[cfg(not(feature = "bundle"))]
        let file: Option<String> = None;
        file.map(|file| file.replace('\\', "/"))
    } else {
        None
    };
    let component_names = source_file.as_ref().map_or_else(Default::default, |_| {
        jsx::component_names(&semantic.semantic)
    });
    let constants = if source_type.is_jsx() {
        let imports = Default::default();
        #[cfg(feature = "bundle")]
        let imports = if let Some(link) = &mut link {
            styles::linked_constants(
                path,
                &program,
                link.assets,
                link.package_entry,
                link.dependencies,
            )
        } else {
            imports
        };
        let constants = styles::Constants::analyze(&semantic.semantic, imports);
        #[cfg(feature = "bundle")]
        let constants = constants.with_theme_tokens(
            link.as_ref()
                .filter(|link| path.starts_with(link.root))
                .map(|link| link.theme),
        );
        constants
    } else {
        styles::Constants::default()
    };
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
    #[cfg(feature = "bundle")]
    if let Some(link) = link {
        link.locales
            .rewrite(
                &allocator,
                &mut program,
                &transformed.scoping,
                source,
                link.release,
            )
            .map_err(diagnostic)?;
    }
    jsx::lower(
        &allocator,
        &mut program,
        &transformed.scoping,
        source,
        &constants,
        pools,
        jsx::LoweringOptions {
            hot,
            debug_source: source_file.as_deref().map(|file| jsx::DebugSource {
                file,
                components: &component_names,
            }),
        },
    )
    .map_err(diagnostic)?;
    let output = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let map = output
        .map
        .ok_or_else(|| format!("{}: missing source map", path.display()))?;
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
