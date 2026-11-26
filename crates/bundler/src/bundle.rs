use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail};
use oxc::{
    allocator::Allocator,
    ast::{
        AstKind,
        ast::{Expression, ImportDeclarationSpecifier, Statement, UnaryOperator},
    },
    ast_visit::{Visit, walk},
    parser::Parser,
    semantic::SemanticBuilder,
    span::{GetSpan, SourceType, Span},
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
    /// Split dynamic imports into loadable ES modules.
    pub split: bool,
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
    /// Files in the linked module graph.
    pub inputs: BTreeSet<PathBuf>,
    /// Extra modules referenced by the entry or another chunk.
    pub chunks: BTreeMap<String, ChunkOutput>,
    /// Native uses originating in the entry chunk.
    pub entry_native_use: NativeUse,
    /// Static dependencies loaded with the entry.
    pub entry_imports: Vec<String>,
    /// Rendered source bytes by input module in the entry.
    pub rendered_modules: BTreeMap<String, usize>,
    /// Source modules that originally imported each compiled pool entry.
    pub pool_consumers: BTreeMap<String, BTreeSet<String>>,
}

/// One secondary output of a split build.
#[derive(Debug)]
pub struct ChunkOutput {
    pub code: String,
    pub source_map: Option<String>,
    pub native_use: NativeUse,
    pub rendered_modules: BTreeMap<String, usize>,
    pub imports: Vec<String>,
}

/// Build a single ES module entirely in memory.
///
/// # Errors
///
/// Reports resolution failures, compiler diagnostics, or incompatible native imports.
pub fn bundle(options: BundleOptions) -> Result<Bundle> {
    bundle_with_cache(options, None)
}

/// Compile multiple entries with a shared cache of framework source transforms.
///
/// # Errors
/// Reports the same resolution and compilation errors as [bundle].
pub fn bundle_with_cache(options: BundleOptions, cache: Option<&ModuleCache>) -> Result<Bundle> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(async {
            Linker::new(&options, false, cache.cloned())?
                .link(None)
                .await
        })
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
        self.bundle_with_cache(options, None)
    }

    /// Reuse the link graph and share framework transforms across entry points.
    /// # Errors
    /// Reports the same input and compilation errors as [bundle].
    pub fn bundle_with_cache(
        &mut self,
        options: BundleOptions,
        cache: Option<&ModuleCache>,
    ) -> Result<Bundle> {
        let changed = self.changes.take();
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        if let Some((previous, mut linker)) = self.current.take()
            && previous == options
            && let Some(changed) = changed
            && !changed.is_empty()
            && changed
                .iter()
                .all(|path| linker.modules.contains(path) && !linker.config_inputs.contains(path))
            && let Ok(changed) = linker.with_folded_dependents(changed)
            && let Ok(bundle) = runtime.block_on(linker.link(Some(changed)))
        {
            self.current = Some((options, linker));
            return Ok(bundle);
        }
        let mut linker = runtime.block_on(async { Linker::new(&options, true, cache.cloned()) })?;
        let bundle = runtime.block_on(linker.link(None))?;
        self.current = Some((options, linker));
        Ok(bundle)
    }
}

type NativeExports = BTreeMap<String, BTreeSet<String>>;
type HotMetadata = BTreeMap<String, crate::hot::ComponentModule>;
type FoldedInputs = BTreeMap<PathBuf, BTreeSet<PathBuf>>;
type ModulePools = BTreeMap<String, BTreeSet<String>>;

struct Linker {
    graph: Bundler,
    hot: bool,
    incremental: bool,
    native_exports: NativeExports,
    warnings: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
    hot_metadata: Arc<Mutex<HotMetadata>>,
    hot_externals: Arc<Mutex<BTreeSet<String>>>,
    modules: BTreeSet<PathBuf>,
    scanned: Arc<Mutex<BTreeSet<PathBuf>>>,
    config_inputs: BTreeSet<PathBuf>,
    folded: Arc<Mutex<FoldedInputs>>,
    module_pools: Arc<Mutex<ModulePools>>,
}

impl Linker {
    fn new(options: &BundleOptions, incremental: bool, cache: Option<ModuleCache>) -> Result<Self> {
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
        let locales = Arc::new(crate::i18n::Locales::load(&root)?);
        let scanned = Arc::new(Mutex::new(BTreeSet::new()));
        let theme = crate::ThemeTokens::load(&root)?;
        let config_inputs = locales
            .inputs
            .iter()
            .cloned()
            .chain([root.join("app.toml"), root.join("src/theme.ts")])
            .filter(|path| path.is_file())
            .collect();
        let mut virtual_modules = options.virtual_modules.clone();
        virtual_modules.insert(crate::i18n::MODULE.to_owned(), locales.source()?);
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
            cwd: Some(root.clone()),
            platform: Some(Platform::Browser),
            format: Some(if options.hot {
                OutputFormat::Cjs
            } else {
                OutputFormat::Esm
            }),
            minify: Some(RawMinifyOptions::Bool(options.release)),
            sourcemap: Some(SourceMapType::Hidden),
            code_splitting: (!options.hot).then_some(CodeSplittingMode::Bool(options.split)),
            entry_filenames: (!options.hot && options.split).then(|| "app.js".to_owned().into()),
            chunk_filenames: (!options.hot && options.split)
                .then(|| "chunks/[name]-[hash].js".to_owned().into()),
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
        let folded = Arc::new(Mutex::new(FoldedInputs::new()));
        let module_pools = Arc::new(Mutex::new(ModulePools::new()));
        let graph = Bundler::with_plugins(
            config,
            vec![Pluginable::new_shared(ImpastoPlugin {
                packages,
                assets,
                root,
                framework: options.framework.canonicalize()?,
                manifest: options.plugins.clone(),
                externals: options.externals.clone(),
                virtual_modules,
                scanned: Arc::clone(&scanned),
                locales: Arc::clone(&locales),
                theme,
                release: options.release,
                folded: Arc::clone(&folded),
                module_pools: Arc::clone(&module_pools),
                cache: if options.hot { None } else { cache },
                pools: Mutex::default(),
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
            scanned,
            config_inputs,
            folded,
            module_pools,
        })
    }

    fn with_folded_dependents(&self, changed: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        let folded = self
            .folded
            .lock()
            .map_err(|_| anyhow::anyhow!("folded inputs unavailable"))?;
        let changed = changed.into_iter().collect::<BTreeSet<_>>();
        let mut all = changed.clone();
        for (importer, sources) in folded.iter() {
            if !sources.is_disjoint(&changed) {
                all.insert(importer.clone());
            }
        }
        Ok(all.into_iter().collect())
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
        for asset in &output.assets {
            if let Output::Chunk(chunk) = asset {
                check_browser_globals(&chunk.code, &chunk.filename, chunk.map.as_ref())?;
            }
        }
        self.modules = output
            .assets
            .iter()
            .filter_map(|asset| match asset {
                Output::Chunk(chunk) => Some(&chunk.module_ids),
                Output::Asset(_) => None,
            })
            .flatten()
            .map(|id| PathBuf::from(id.as_str()))
            .chain(self.config_inputs.iter().cloned())
            .collect();
        self.modules.extend(
            self.scanned
                .lock()
                .map_err(|_| anyhow::anyhow!("scanned inputs unavailable"))?
                .iter()
                .cloned(),
        );
        {
            let folded = self
                .folded
                .lock()
                .map_err(|_| anyhow::anyhow!("folded inputs unavailable"))?;
            self.modules.extend(folded.values().flatten().cloned());
        }
        let warnings = self
            .warnings
            .lock()
            .map_err(|_| anyhow::anyhow!("compiler diagnostics unavailable"))?
            .values()
            .flatten()
            .cloned()
            .chain(
                output
                    .warnings
                    .iter()
                    .map(|warning| warning.to_diagnostic().convert_to_string(false)),
            )
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
                inputs: self.modules.clone(),
                chunks: BTreeMap::new(),
                entry_native_use: BTreeMap::new(),
                entry_imports: Vec::new(),
                rendered_modules: BTreeMap::new(),
                pool_consumers: BTreeMap::new(),
                native_use: externals
                    .iter()
                    .filter(|name| self.native_exports.contains_key(*name))
                    .map(|name| (name.clone(), None))
                    .collect(),
            });
        }
        let mut entry = None;
        let mut native_use = NativeUse::new();
        let mut chunks = BTreeMap::new();
        for asset in output.assets {
            let Output::Chunk(chunk) = asset else {
                continue;
            };
            let chunk = Arc::unwrap_or_clone(chunk);
            let uses = native_uses(&chunk.code, &self.native_exports)?;
            for (name, names) in &uses {
                let aggregate = native_use
                    .entry(name.clone())
                    .or_insert_with(|| Some(BTreeSet::new()));
                match (aggregate.as_mut(), names) {
                    (Some(all), Some(names)) => all.extend(names.iter().cloned()),
                    (_, None) => *aggregate = None,
                    (None, Some(_)) => {}
                }
            }
            let rendered_modules = chunk
                .modules
                .keys
                .iter()
                .zip(&chunk.modules.values)
                .map(|(id, module)| {
                    (
                        id.as_str().trim_start_matches('\0').to_owned(),
                        module.rendered_length(),
                    )
                })
                .collect();
            let item = ChunkOutput {
                code: chunk.code,
                source_map: chunk.map.map(|map| map.to_json_string()),
                native_use: uses,
                rendered_modules,
                imports: chunk.imports.iter().map(ToString::to_string).collect(),
            };
            if chunk.is_entry {
                entry = Some(item);
            } else {
                chunks.insert(chunk.filename.to_string(), item);
            }
        }
        let entry = entry.context("application did not produce an entry module")?;
        let mut pool_consumers = ModulePools::new();
        for (source, pools) in self
            .module_pools
            .lock()
            .map_err(|_| anyhow::anyhow!("pool consumers unavailable"))?
            .iter()
        {
            if self.modules.contains(Path::new(source)) {
                for pool in pools {
                    pool_consumers
                        .entry(pool.clone())
                        .or_default()
                        .insert(source.clone());
                }
            }
        }
        Ok(Bundle {
            code: entry.code,
            warnings,
            source_map: entry.source_map,
            native_use,
            inputs: self.modules.clone(),
            chunks,
            entry_native_use: entry.native_use,
            entry_imports: entry.imports,
            rendered_modules: entry.rendered_modules,
            pool_consumers,
        })
    }
}

type CachedTransform = Arc<OnceLock<Result<Arc<LinkedTransform>, String>>>;
type TransformKey = (String, SourceType, String);

#[derive(Debug)]
struct LinkedTransform {
    output: crate::CompiledModule,
    pools: crate::pools::PoolModules,
}

/// Framework transforms shared by the independent entries in one test run.
#[derive(Clone, Debug, Default)]
pub struct ModuleCache {
    entries: Arc<Mutex<HashMap<TransformKey, CachedTransform>>>,
}

impl ModuleCache {
    fn transform(
        &self,
        id: &str,
        source: &str,
        source_type: SourceType,
    ) -> Result<Arc<LinkedTransform>> {
        const MAX_ENTRIES: usize = 512;
        let slot = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| anyhow::anyhow!("transform cache unavailable"))?;
            let key = (id.to_owned(), source_type, source.to_owned());
            if let Some(slot) = entries.get(&key) {
                Some(Arc::clone(slot))
            } else if entries.len() < MAX_ENTRIES {
                Some(Arc::clone(entries.entry(key).or_default()))
            } else {
                None
            }
        };
        if let Some(slot) = slot {
            return slot
                .get_or_init(|| linked_transform(id, source, source_type, None).map(Arc::new))
                .clone()
                .map_err(anyhow::Error::msg);
        }
        linked_transform(id, source, source_type, None)
            .map(Arc::new)
            .map_err(anyhow::Error::msg)
    }
}

fn linked_transform(
    id: &str,
    source: &str,
    source_type: SourceType,
    link: Option<crate::LinkContext<'_>>,
) -> Result<LinkedTransform, String> {
    let mut pools = crate::pools::PoolModules::default();
    let (output, _) = crate::compile_inner(
        Path::new(id),
        source,
        source_type,
        false,
        Some(&mut pools),
        link,
    )?;
    Ok(LinkedTransform { output, pools })
}

#[derive(Debug)]
struct ImpastoPlugin {
    packages: PathBuf,
    assets: PathBuf,
    root: PathBuf,
    framework: PathBuf,
    manifest: Option<PluginManifest>,
    externals: BTreeSet<String>,
    virtual_modules: BTreeMap<String, String>,
    scanned: Arc<Mutex<BTreeSet<PathBuf>>>,
    locales: Arc<crate::i18n::Locales>,
    theme: crate::ThemeTokens,
    release: bool,
    folded: Arc<Mutex<FoldedInputs>>,
    module_pools: Arc<Mutex<ModulePools>>,
    cache: Option<ModuleCache>,
    pools: Mutex<crate::pools::PoolModules>,
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
        if specifier.starts_with(crate::pools::PREFIX)
            || self.virtual_modules.contains_key(specifier)
        {
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
        let path = Path::new(args.id);
        if path.is_absolute() && path.is_file() {
            self.scanned
                .lock()
                .map_err(|_| anyhow::anyhow!("scanned inputs unavailable"))?
                .insert(path.to_owned());
        }
        if let Some(id) = args.id.strip_prefix('\0')
            && id.starts_with(crate::pools::PREFIX)
        {
            let pools = self
                .pools
                .lock()
                .map_err(|_| anyhow::anyhow!("scene pool unavailable"))?;
            let source = pools.get(id).context("unknown scene pool module")?;
            return Ok(Some(HookLoadOutput {
                code: source.to_owned().into(),
                module_type: Some(ModuleType::Js),
                ..Default::default()
            }));
        }
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
            ModuleType::Ts
                if Path::new(args.id).starts_with(&self.root)
                    && !Path::new(args.id).starts_with(&self.packages)
                    && !Path::new(args.id).starts_with(self.root.join("node_modules"))
                    || args.code.contains("@impasto/i18n")
                    || args.code.contains("createShaderProgram")
                    || (args.code.contains("await")
                        && (args.code.contains("onCleanup")
                            || args.code.contains("createRenderEffect"))) =>
            {
                SourceType::ts()
            }
            ModuleType::Js if args.code.contains("@impasto/i18n") => SourceType::mjs(),
            _ => return Ok(None),
        };
        let mut dependencies = BTreeSet::new();
        let package_entry = |name: &str| {
            if let Some(manifest) = &self.manifest {
                if let Some(entry) = manifest.entries.get(name) {
                    return entry.canonicalize().ok();
                }
                if manifest
                    .disabled_entries
                    .iter()
                    .any(|package| package_match(name, package))
                {
                    return None;
                }
            }
            crate::styles::builtin_export(name, &self.packages)
        };
        let link = crate::LinkContext {
            assets: &self.assets,
            root: &self.root,
            framework: &self.framework,
            locales: &self.locales,
            theme: &self.theme,
            package_entry: &package_entry,
            release: self.release,
            dependencies: &mut dependencies,
        };
        let (code, warnings, map) = if self.hot {
            let (mut output, metadata) = crate::compile_inner(
                Path::new(args.id),
                args.code,
                source_type,
                true,
                None,
                Some(link),
            )
            .map_err(anyhow::Error::msg)?;
            metadata.instrument(args.id, &mut output.code, &mut output.map);
            self.hot_metadata
                .lock()
                .map_err(|_| anyhow::anyhow!("component metadata unavailable"))?
                .insert(args.id.to_owned(), metadata);
            (output.code, output.warnings, output.map.into())
        } else {
            let compiled = if let Some(cache) = &self.cache
                && Path::new(args.id).starts_with(&self.packages)
                && !args.code.contains("@impasto/i18n")
            {
                cache.transform(args.id, args.code, source_type)?
            } else {
                Arc::new(
                    linked_transform(args.id, args.code, source_type, Some(link))
                        .map_err(anyhow::Error::msg)?,
                )
            };
            self.pools
                .lock()
                .map_err(|_| anyhow::anyhow!("scene pool unavailable"))?
                .extend(&compiled.pools)
                .map_err(anyhow::Error::msg)?;
            self.module_pools
                .lock()
                .map_err(|_| anyhow::anyhow!("pool consumers unavailable"))?
                .insert(
                    args.id.to_owned(),
                    compiled.pools.ids().map(str::to_owned).collect(),
                );
            (
                compiled.output.code.clone(),
                compiled.output.warnings.clone(),
                compiled.output.map.clone().into(),
            )
        };
        self.warnings
            .lock()
            .map_err(|_| anyhow::anyhow!("compiler diagnostics unavailable"))?
            .insert(args.id.to_owned(), warnings);
        self.folded
            .lock()
            .map_err(|_| anyhow::anyhow!("folded inputs unavailable"))?
            .insert(PathBuf::from(args.id), dependencies);
        Ok(Some(HookTransformOutput {
            code: Some(code),
            map,
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

fn check_browser_globals(
    code: &str,
    filename: &str,
    map: Option<&oxc_sourcemap::SourceMap<'_>>,
) -> Result<()> {
    const UNAVAILABLE: &[&str] = &[
        "document",
        "window",
        "XMLHttpRequest",
        "localStorage",
        "sessionStorage",
        "navigator",
    ];
    if !UNAVAILABLE.iter().any(|name| code.contains(name)) {
        return Ok(());
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
    if let Some(error) = parsed.diagnostics.into_iter().next() {
        bail!("invalid linked module: {error}");
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program)
        .semantic;
    let scoping = semantic.scoping();
    let nodes = semantic.nodes();
    let mut unsupported = Vec::new();
    for (name, references) in scoping.root_unresolved_references() {
        if name.as_str() != "globalThis" && !UNAVAILABLE.contains(&name.as_str()) {
            continue;
        }
        for reference in references {
            let mut node = nodes.get_node(scoping.get_reference(*reference).node_id());
            let mut name = name.as_str();
            if name == "globalThis" {
                node = nodes.parent_node(node.id());
                let property = match node.kind() {
                    AstKind::StaticMemberExpression(member) => member.property.name.as_str(),
                    AstKind::ComputedMemberExpression(member) => {
                        let Expression::StringLiteral(property) =
                            member.expression.get_inner_expression()
                        else {
                            continue;
                        };
                        property.value.as_str()
                    }
                    _ => continue,
                };
                if !UNAVAILABLE.contains(&property) {
                    continue;
                }
                name = property;
            }
            if matches!(nodes.parent_kind(node.id()), AstKind::UnaryExpression(unary)
                if unary.operator == UnaryOperator::Typeof)
            {
                continue;
            }
            unsupported.push((node.span(), name));
        }
    }
    if let Some((span, name)) = unsupported.into_iter().min_by_key(|(span, _)| span.start) {
        let location = linked_location(code, span, filename, map);
        bail!("{location}: browser global \"{name}\" is unavailable in Impasto's QuickJS runtime");
    }
    Ok(())
}

fn linked_location(
    code: &str,
    span: Span,
    filename: &str,
    map: Option<&oxc_sourcemap::SourceMap<'_>>,
) -> String {
    let preceding = &code[..span.start as usize];
    let line =
        u32::try_from(preceding.bytes().filter(|&byte| byte == b'\n').count()).unwrap_or(u32::MAX);
    let column = preceding.rsplit('\n').next().map_or(0, |line| {
        u32::try_from(line.encode_utf16().count()).unwrap_or(u32::MAX)
    });
    if let Some(map) = map
        && let Some(token) =
            map.lookup_source_view_token(&map.generate_lookup_table(), line, column)
        && let Some(source) = token.get_source()
    {
        return format!(
            "{source}:{}:{}",
            token.get_src_line() + 1,
            token.get_src_col() + 1
        );
    }
    format!("{filename}:{}:{}", line + 1, column + 1)
}

pub type NativeUse = BTreeMap<String, Option<BTreeSet<String>>>;

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
                Expression::StringLiteral(source) => {
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
