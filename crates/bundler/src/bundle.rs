use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use oxc::{
    allocator::Allocator,
    ast::ast::{ImportDeclarationSpecifier, Statement},
    ast_visit::{Visit, walk},
    parser::Parser,
    span::SourceType,
};
use rolldown::{
    Bundler, BundlerOptions, ExperimentalOptions, InputItem, ModuleType, OutputFormat, Platform,
    RawMinifyOptions, SourceMapType,
    plugin::{
        HookLoadArgs, HookLoadOutput, HookLoadReturn, HookResolveIdArgs, HookResolveIdOutput,
        HookResolveIdReturn, HookTransformArgs, HookTransformOutput, HookTransformReturn,
        HookUsage, Plugin, PluginContext, Pluginable, SharedLoadPluginContext,
        SharedTransformPluginContext,
    },
};
use rolldown_common::{CodeSplittingMode, Output, ScanMode, TsConfig};
use serde::Deserialize;

/// Linker input for an application and the native modules supplied by its host.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BundleOptions {
    /// Application directory used for import resolution and map paths.
    pub root: PathBuf,
    /// Entry module to load.
    pub entry: PathBuf,
    /// Framework directory containing the source packages.
    pub framework: PathBuf,
    /// Enable release definitions and minification.
    pub release: bool,
    /// Emit reloadable module factories for a debug watch session.
    pub hot: bool,
    /// Installed or prepared plugin contract. Omit for framework fixtures.
    pub plugins: Option<PluginManifest>,
    /// Extra host modules explicitly supplied by the caller.
    pub externals: BTreeSet<String>,
    /// Virtual modules compiled as JavaScript provided by the build caller.
    pub virtual_modules: BTreeMap<String, String>,
    /// Additional compile-time JavaScript expressions by identifier.
    pub defines: BTreeMap<String, String>,
}

/// The JavaScript-facing portion of a platform's native plugin manifest.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    schema_version: u32,
    platform: String,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    modules: Vec<NativeModule>,
}

impl PluginManifest {
    /// Construct the linker contract from a platform's resolved plugin metadata.
    pub fn new(
        platform: String,
        entries: BTreeMap<String, PathBuf>,
        disabled_entries: BTreeSet<String>,
        modules: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        Self {
            schema_version: 1,
            platform,
            entries,
            disabled_entries,
            modules: modules
                .into_iter()
                .map(|(specifier, exports)| NativeModule { specifier, exports })
                .collect(),
        }
    }

    /// Exports the bytecode compiler must provide to QuickJS when linking native modules.
    pub fn exports(&self) -> impl Iterator<Item = (&str, Vec<&str>)> {
        self.modules.iter().map(|module| {
            (
                module.specifier.as_str(),
                module.exports.iter().map(String::as_str).collect(),
            )
        })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct NativeModule {
    specifier: String,
    exports: Vec<String>,
}

/// Linked ES module, its source map, and its dependencies on the native host.
#[derive(Debug)]
pub struct Bundle {
    /// JavaScript ready for QuickJS compilation.
    pub code: String,
    /// Framework warnings collected across all linked modules.
    pub warnings: Vec<String>,
    /// Source map with no trailing comment in the bundle.
    pub source_map: Option<String>,
    /// Surviving plugin imports. An omitted symbol set denotes namespace use.
    pub native_use: BTreeMap<String, Option<BTreeSet<String>>>,
}

/// Build a single ES module entirely in memory.
///
/// # Errors
///
/// Reports resolution failures, compiler diagnostics, or incompatible native imports.
pub fn bundle(options: BundleOptions) -> Result<Bundle> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(async { Linker::new(&options, false)?.link(None).await })
}

/// Incremental graph for a single debug watch session.
#[derive(Default)]
pub struct WatchLinker {
    current: Option<(BundleOptions, Linker)>,
    changes: Option<Vec<PathBuf>>,
}

impl WatchLinker {
    /// Set the edited paths for the next link. Pass no paths to rebuild the graph.
    pub fn set_changes(&mut self, changes: Option<Vec<PathBuf>>) {
        self.changes = changes;
    }

    /// Link the current sources, reusing the graph for edits to existing modules.
    /// # Errors
    /// Reports the same input and compilation errors as [bundle].
    pub fn bundle(&mut self, options: BundleOptions) -> Result<Bundle> {
        let changed = self.changes.take();
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        if let Some((previous, mut linker)) = self.current.take()
            && previous == options
            && let Some(changed) = changed
            && !changed.is_empty()
            && changed.iter().all(|path| linker.modules.contains(path))
            && let Ok(bundle) = runtime.block_on(linker.link(Some(changed)))
        {
            self.current = Some((options, linker));
            return Ok(bundle);
        }
        let mut linker = runtime.block_on(async { Linker::new(&options, true) })?;
        let bundle = runtime.block_on(linker.link(None))?;
        self.current = Some((options, linker));
        Ok(bundle)
    }
}

type NativeExports = BTreeMap<String, BTreeSet<String>>;
type HotMetadata = BTreeMap<String, crate::hot::ComponentModule>;

struct Linker {
    graph: Bundler,
    hot: bool,
    incremental: bool,
    native_exports: NativeExports,
    warnings: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
    hot_metadata: Arc<Mutex<HotMetadata>>,
    hot_externals: Arc<Mutex<BTreeSet<String>>>,
    modules: BTreeSet<PathBuf>,
}

impl Linker {
    fn new(options: &BundleOptions, incremental: bool) -> Result<Self> {
        let entry = options
            .entry
            .canonicalize()
            .with_context(|| format!("entry: {}", options.entry.display()))?;
        let root = options.root.canonicalize()?;
        let packages = options.framework.join("packages").canonicalize()?;
        if let Some(manifest) = &options.plugins
            && manifest.schema_version != 1
        {
            bail!(
                "Unsupported plugins manifest schema {}",
                manifest.schema_version
            );
        }
        let tsconfig = root.join("tsconfig.json");
        let assets = root.join("assets");
        let mut config = BundlerOptions {
            input: Some(vec![InputItem {
                import: entry.to_string_lossy().into_owned(),
                ..Default::default()
            }]),
            tsconfig: Some(if tsconfig.is_file() {
                TsConfig::Manual(tsconfig)
            } else {
                TsConfig::Auto(false)
            }),
            cwd: Some(root),
            platform: Some(Platform::Browser),
            format: Some(if options.hot {
                OutputFormat::Cjs
            } else {
                OutputFormat::Esm
            }),
            minify: Some(RawMinifyOptions::Bool(options.release)),
            sourcemap: (!options.hot).then_some(SourceMapType::Hidden),
            code_splitting: (!options.hot).then_some(CodeSplittingMode::Bool(false)),
            preserve_modules: options.hot.then_some(true),
            experimental: incremental.then_some(ExperimentalOptions {
                incremental_build: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let defines = config.define.get_or_insert_with(Default::default);
        defines.extend(options.defines.clone());
        defines
            .entry("process.env.NODE_ENV".to_owned())
            .or_insert_with(|| {
                if options.release {
                    "\"production\""
                } else {
                    "\"development\""
                }
                .to_owned()
            });
        defines
            .entry("__STOCK_REPLAY_FREEZE__".to_owned())
            .or_insert_with(|| "\"\"".to_owned());
        let native_exports: BTreeMap<_, _> = options
            .plugins
            .iter()
            .flat_map(|manifest| &manifest.modules)
            .map(|module| {
                (
                    module.specifier.clone(),
                    module.exports.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect();
        let warnings = Arc::new(Mutex::new(BTreeMap::new()));
        let hot_metadata = Arc::new(Mutex::new(BTreeMap::new()));
        let hot_externals = Arc::new(Mutex::new(BTreeSet::new()));
        let graph = Bundler::with_plugins(
            config,
            vec![Pluginable::new_shared(ImpastoPlugin {
                packages,
                assets,
                manifest: options.plugins.clone(),
                externals: options.externals.clone(),
                virtual_modules: options.virtual_modules.clone(),
                warnings: Arc::clone(&warnings),
                hot: options.hot,
                hot_metadata: Arc::clone(&hot_metadata),
                hot_externals: Arc::clone(&hot_externals),
            })],
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(Self {
            graph,
            hot: options.hot,
            incremental,
            native_exports,
            warnings,
            hot_metadata,
            hot_externals,
            modules: BTreeSet::new(),
        })
    }

    async fn link(&mut self, changed: Option<Vec<PathBuf>>) -> Result<Bundle> {
        let output = if self.incremental {
            let mode = changed.map_or(ScanMode::Full, |paths| {
                ScanMode::Partial(
                    paths
                        .iter()
                        .map(|path| path.to_string_lossy().as_ref().into())
                        .collect(),
                )
            });
            self.graph.incremental_generate(mode).await
        } else {
            self.graph.generate().await
        }
        .map_err(|errors| {
            let formatted = errors
                .into_vec()
                .into_iter()
                .map(|error| error.to_diagnostic().convert_to_string(false))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::anyhow!("{formatted}")
        })?;
        self.modules = output
            .assets
            .iter()
            .filter_map(|asset| match asset {
                Output::Chunk(chunk) => Some(&chunk.module_ids),
                Output::Asset(_) => None,
            })
            .flatten()
            .map(|id| PathBuf::from(id.as_str()))
            .collect();
        let warnings = self
            .warnings
            .lock()
            .map_err(|_| anyhow::anyhow!("compiler diagnostics unavailable"))?
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if self.hot {
            let metadata = self
                .hot_metadata
                .lock()
                .map_err(|_| anyhow::anyhow!("component metadata unavailable"))?;
            let externals = self
                .hot_externals
                .lock()
                .map_err(|_| anyhow::anyhow!("external modules unavailable"))?;
            return Ok(Bundle {
                code: crate::hot::script(&output.assets, &metadata, &externals)?,
                warnings,
                source_map: None,
                native_use: externals
                    .iter()
                    .filter(|name| self.native_exports.contains_key(*name))
                    .map(|name| (name.clone(), None))
                    .collect(),
            });
        }
        let chunk = output
            .assets
            .into_iter()
            .find_map(|asset| match asset {
                Output::Chunk(chunk) if chunk.is_entry => Some(chunk),
                _ => None,
            })
            .context("application did not produce an entry module")?;
        let chunk = Arc::unwrap_or_clone(chunk);
        let native_use = native_uses(&chunk.code, &self.native_exports)?;
        Ok(Bundle {
            code: chunk.code,
            warnings,
            source_map: chunk.map.map(|map| map.to_json_string()),
            native_use,
        })
    }
}

#[derive(Debug)]
struct ImpastoPlugin {
    packages: PathBuf,
    assets: PathBuf,
    manifest: Option<PluginManifest>,
    externals: BTreeSet<String>,
    virtual_modules: BTreeMap<String, String>,
    warnings: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
    hot: bool,
    hot_metadata: Arc<Mutex<HotMetadata>>,
    hot_externals: Arc<Mutex<BTreeSet<String>>>,
}

impl ImpastoPlugin {
    fn external(&self, specifier: &str) -> HookResolveIdReturn {
        if self.hot {
            self.hot_externals
                .lock()
                .map_err(|_| anyhow::anyhow!("external modules unavailable"))?
                .insert(specifier.to_owned());
        }
        Ok(Some(external(specifier)))
    }
}

impl Plugin for ImpastoPlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        "impasto".into()
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::ResolveId | HookUsage::Transform | HookUsage::Load
    }

    async fn resolve_id(
        &self,
        _: &PluginContext,
        args: &HookResolveIdArgs<'_>,
    ) -> HookResolveIdReturn {
        let specifier = args.specifier;
        if self.virtual_modules.contains_key(specifier) {
            return Ok(Some(HookResolveIdOutput::from_id(format!("\0{specifier}"))));
        }
        if matches!(specifier, "scene" | "reactive") || self.externals.contains(specifier) {
            return self.external(specifier);
        }
        if let Some(manifest) = &self.manifest {
            if let Some(entry) = manifest.entries.get(specifier) {
                return Ok(Some(HookResolveIdOutput::from_id(
                    entry.to_string_lossy().into_owned(),
                )));
            }
            if manifest
                .modules
                .iter()
                .any(|module| module.specifier == specifier)
            {
                return self.external(specifier);
            }
            if manifest
                .disabled_entries
                .iter()
                .any(|name| package_match(specifier, name))
            {
                bail!(
                    "{}Plugin \"{specifier}\" is unavailable on {}",
                    location(args),
                    manifest.platform
                );
            }
        }
        if specifier == "solid-js" {
            return Ok(Some(HookResolveIdOutput::from_id(
                self.packages
                    .join("reactive/src/index.ts")
                    .to_string_lossy()
                    .into_owned(),
            )));
        }
        if is_node_builtin(specifier)
            || specifier.starts_with("solid-js/")
            || specifier.starts_with("@solidjs/")
        {
            bail!(
                "{}Module \"{specifier}\" is unavailable in Impasto's QuickJS runtime",
                location(args)
            );
        }
        let Some(path) = specifier.strip_prefix("@impasto/") else {
            return Ok(None);
        };
        let (name, subpath) = path.split_once('/').unwrap_or((path, ""));
        if name.is_empty() || name.starts_with('.') || name.contains('\\') {
            return Ok(None);
        }
        let package = self.packages.join(name);
        let package_json = package.join("package.json");
        if !package_json.is_file() {
            return Ok(None);
        }
        if package.join("plugin.toml").is_file() {
            if let Some(manifest) = &self.manifest {
                bail!(
                    "{}Plugin \"@impasto/{name}\" is not enabled for {}",
                    location(args),
                    manifest.platform
                );
            }
            if subpath == "native" {
                return self.external(specifier);
            }
        }
        let exports: serde_json::Value = serde_json::from_slice(&fs::read(&package_json)?)?;
        let key = if subpath.is_empty() {
            ".".to_owned()
        } else {
            format!("./{subpath}")
        };
        let entry = exports["exports"][key]
            .as_str()
            .with_context(|| format!("{}Unknown package export \"{specifier}\"", location(args)))?;
        Ok(Some(HookResolveIdOutput::from_id(
            package.join(entry).to_string_lossy().into_owned(),
        )))
    }

    async fn load(&self, _: SharedLoadPluginContext, args: &HookLoadArgs<'_>) -> HookLoadReturn {
        if let Some(code) = crate::assets::module(&self.assets, Path::new(args.id))? {
            return Ok(Some(HookLoadOutput {
                code: code.into(),
                module_type: Some(ModuleType::Js),
                ..Default::default()
            }));
        }
        #[cfg(feature = "wgsl")]
        if Path::new(args.id)
            .extension()
            .is_some_and(|extension| extension == "wgsl")
        {
            let source = fs::read_to_string(args.id)?;
            return Ok(Some(HookLoadOutput {
                code: crate::wgsl::module(&source)
                    .with_context(|| args.id.to_owned())?
                    .into(),
                module_type: Some(ModuleType::Js),
                ..Default::default()
            }));
        }
        Ok(args
            .id
            .strip_prefix('\0')
            .and_then(|id| self.virtual_modules.get(id))
            .map(|source| HookLoadOutput {
                code: source.clone().into(),
                module_type: Some(ModuleType::Js),
                ..Default::default()
            }))
    }

    async fn transform(
        &self,
        _: SharedTransformPluginContext,
        args: &HookTransformArgs<'_>,
    ) -> HookTransformReturn {
        let source_type = match args.module_type {
            ModuleType::Tsx => SourceType::tsx(),
            ModuleType::Jsx => SourceType::jsx(),
            ModuleType::Ts if args.code.contains("createShaderProgram") => SourceType::ts(),
            _ => return Ok(None),
        };
        let (mut output, metadata) =
            crate::compile_inner(Path::new(args.id), args.code, source_type, self.hot)
                .map_err(anyhow::Error::msg)?;
        if self.hot {
            metadata.instrument(args.id, &mut output.code);
            self.hot_metadata
                .lock()
                .map_err(|_| anyhow::anyhow!("component metadata unavailable"))?
                .insert(args.id.to_owned(), metadata);
        }
        self.warnings
            .lock()
            .map_err(|_| anyhow::anyhow!("compiler diagnostics unavailable"))?
            .insert(args.id.to_owned(), output.warnings);
        Ok(Some(HookTransformOutput {
            code: Some(output.code),
            map: if self.hot {
                Default::default()
            } else {
                output.map.into()
            },
            module_type: Some(ModuleType::Js),
            ..Default::default()
        }))
    }
}

fn package_match(specifier: &str, package: &str) -> bool {
    specifier == package
        || specifier
            .strip_prefix(package)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn external(specifier: &str) -> HookResolveIdOutput {
    HookResolveIdOutput {
        external: Some(true.into()),
        ..HookResolveIdOutput::from_id(specifier)
    }
}

fn location(args: &HookResolveIdArgs<'_>) -> String {
    let Some(importer) = args.importer else {
        return String::new();
    };
    let Some(source) = fs::read_to_string(importer).ok() else {
        return format!("{importer}: ");
    };
    for (index, line) in source.lines().enumerate() {
        if let Some(column) = line
            .find(&format!("\"{}\"", args.specifier))
            .or_else(|| line.find(&format!("'{}'", args.specifier)))
        {
            return format!(
                "{importer}:{}:{}: ",
                index + 1,
                line[..column].chars().count() + 1
            );
        }
    }
    format!("{importer}: ")
}

fn is_node_builtin(specifier: &str) -> bool {
    specifier.starts_with("node:")
        || matches!(
            specifier.split('/').next().unwrap_or_default(),
            "assert"
                | "async_hooks"
                | "buffer"
                | "child_process"
                | "cluster"
                | "console"
                | "constants"
                | "crypto"
                | "dgram"
                | "diagnostics_channel"
                | "dns"
                | "domain"
                | "events"
                | "fs"
                | "http"
                | "http2"
                | "https"
                | "module"
                | "net"
                | "os"
                | "path"
                | "perf_hooks"
                | "process"
                | "punycode"
                | "querystring"
                | "readline"
                | "repl"
                | "sqlite"
                | "stream"
                | "string_decoder"
                | "sys"
                | "test"
                | "timers"
                | "tls"
                | "trace_events"
                | "tty"
                | "url"
                | "util"
                | "v8"
                | "vm"
                | "wasi"
                | "worker_threads"
                | "zlib"
        )
}

type NativeUse = BTreeMap<String, Option<BTreeSet<String>>>;

fn native_uses(code: &str, available: &BTreeMap<String, BTreeSet<String>>) -> Result<NativeUse> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
    if let Some(error) = parsed.diagnostics.into_iter().next() {
        bail!("invalid linked module: {error}");
    }
    let mut found = BTreeMap::new();
    for statement in &parsed.program.body {
        let (module, specifiers) = match statement {
            Statement::ImportDeclaration(import) => (
                import.source.value.as_str(),
                import
                    .specifiers
                    .iter()
                    .flatten()
                    .map(|specifier| match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(import) => {
                            Some(import.imported.name())
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                            Some("default".into())
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Statement::ExportFromDeclaration(export) => (
                export.source.value.as_str(),
                export
                    .specifiers
                    .iter()
                    .map(|specifier| Some(specifier.local.name()))
                    .collect(),
            ),
            Statement::ExportAllDeclaration(export) => (export.source.value.as_str(), vec![None]),
            _ => continue,
        };
        let Some(exports) = available.get(module) else {
            continue;
        };
        let used = found
            .entry(module.to_owned())
            .or_insert_with(|| Some(BTreeSet::new()));
        for imported in specifiers {
            let Some(imported) = imported else {
                *used = None;
                continue;
            };
            if !exports.contains(imported.as_str()) {
                bail!(r#"Native module "{module}" does not export "{imported}""#);
            }
            if let Some(names) = used {
                names.insert(imported.to_string());
            }
        }
    }
    struct DynamicImports<'a> {
        uses: &'a mut NativeUse,
        available: &'a BTreeMap<String, BTreeSet<String>>,
    }
    impl<'a> Visit<'a> for DynamicImports<'_> {
        fn visit_import_expression(&mut self, import: &oxc::ast::ast::ImportExpression<'a>) {
            match import.source.without_parentheses() {
                oxc::ast::ast::Expression::StringLiteral(source) => {
                    if self.available.contains_key(source.value.as_str()) {
                        self.uses.insert(source.value.to_string(), None);
                    }
                }
                _ => self
                    .uses
                    .extend(self.available.keys().cloned().map(|name| (name, None))),
            }
            walk::walk_import_expression(self, import);
        }
    }
    DynamicImports {
        uses: &mut found,
        available,
    }
    .visit_program(&parsed.program);
    Ok(found)
}
