//! Resolves plugin inputs and generates target-specific Cargo projects and declarations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{app::App, toolchain};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub path: PathBuf,
    pub targets: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plugin {
    plugin: Package,
    js: Option<Js>,
    #[serde(default)]
    capabilities: Capabilities,
    #[serde(default)]
    ios: Platform,
    #[serde(default)]
    android: Platform,
    #[serde(default)]
    macos: Platform,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    targets: Vec<String>,
    min_impasto_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Js {
    specifier: String,
    entry: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Capabilities {
    requires: Vec<String>,
    optional: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Platform {
    pub sources: Vec<PathBuf>,
    pub frameworks: BTreeSet<String>,
    pub weak_frameworks: BTreeSet<String>,
    pub usage_description_keys: BTreeSet<String>,
    pub required_entitlements: BTreeSet<String>,
    pub min_version: Option<String>,
    pub permissions: BTreeSet<String>,
    pub gradle_dependencies: BTreeSet<String>,
    pub uses_features: Vec<UsesFeature>,
    pub min_sdk: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsesFeature {
    pub name: String,
    pub required: bool,
}

#[derive(Deserialize, Serialize)]
pub struct Descriptor {
    pub name: String,
    pub specifier: String,
    pub typescript: String,
    pub exports: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    platform: String,
    modules: Vec<Descriptor>,
    types: Vec<PathBuf>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform_inputs: Platform,
    #[serde(default)]
    watch_roots: Vec<PathBuf>,
}

pub struct NativeBuild {
    pub workspace: PathBuf,
    pub manifest: PathBuf,
    pub package: String,
    data: Manifest,
    _lock: fs::File,
}

impl NativeBuild {
    pub fn links_native(&self) -> bool {
        !self.package.is_empty()
    }

    pub fn descriptors(&self) -> &[Descriptor] {
        &self.data.modules
    }

    pub fn inputs(&self) -> &Platform {
        &self.data.platform_inputs
    }
}

pub fn configured(app: &App) -> bool {
    app.native.is_some() || !app.plugins.is_empty()
}

pub fn standalone<'a>(app: &'a App, target: Option<&'a str>) -> Result<Option<&'a str>> {
    if let Some(target) = target {
        app.require(target)?;
        return Ok(Some(target));
    }
    match app.targets.as_slice() {
        _ if !configured(app) => Ok(None),
        [target] => Ok(Some(target)),
        _ => bail!("应用包含 Rust 插件，需要 --platform macos、ios 或 android"),
    }
}

fn under(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.is_absolute() {
        bail!("插件资源必须相对插件目录: {}", relative.display());
    }
    let path = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("插件资源不存在: {}", root.join(relative).display()))?;
    if !path.starts_with(root) {
        bail!("插件资源超出插件目录: {}", relative.display());
    }
    Ok(path)
}

fn cargo_package(root: &Path) -> Result<(PathBuf, String)> {
    let manifest = root.join("Cargo.toml");
    let cfg: toml::Value = toml::from_str(
        &fs::read_to_string(&manifest).with_context(|| format!("读取 {}", manifest.display()))?,
    )?;
    let name = cfg
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} 需要一个 Cargo 包", manifest.display()))?;
    if !name
        .bytes()
        .all(|item| item.is_ascii_alphanumeric() || matches!(item, b'_' | b'-'))
    {
        bail!("Cargo 包名无效: {name}");
    }
    Ok((root.to_owned(), name.to_owned()))
}

fn resolve(app: &App, target: &str) -> Result<Resolved> {
    let mut result = Resolved::default();
    let allowed = app.modules(target);
    let reserved = embedder::first_party_modules();
    if let Some(native) = &app.native {
        result
            .crates
            .push(cargo_package(&app.root.join(&native.path).canonicalize()?)?);
    }
    for (alias, dependency) in &app.plugins {
        let root = app
            .root
            .join(&dependency.path)
            .canonicalize()
            .with_context(|| format!("插件 {alias} 路径不存在"))?;
        let plugin: Plugin = toml::from_str(&fs::read_to_string(root.join("plugin.toml"))?)
            .with_context(|| format!("解析插件 {alias} 的 plugin.toml"))?;
        if plugin.plugin.targets.is_empty()
            || plugin
                .plugin
                .targets
                .iter()
                .any(|target| !matches!(target.as_str(), "macos" | "ios" | "android"))
        {
            bail!("插件 {alias} 的 targets 无效");
        }
        if dependency.targets.as_ref().is_some_and(|targets| {
            targets.is_empty()
                || targets.iter().any(|target| {
                    !app.targets.contains(target) || !plugin.plugin.targets.contains(target)
                })
        }) {
            bail!("插件 {alias} 的 targets 必须同时属于应用和插件");
        }
        let enabled = dependency
            .targets
            .as_ref()
            .unwrap_or(&plugin.plugin.targets)
            .iter()
            .any(|item| item == target);
        if let Some(js) = plugin.js {
            validate_specifier(&js.specifier, &reserved)?;
            if enabled {
                let entry = under(&root, &js.entry)?;
                if result.entries.insert(js.specifier.clone(), entry).is_some() {
                    bail!("重复的插件入口 {}", js.specifier);
                }
            } else {
                result.disabled_entries.insert(js.specifier);
            }
        }
        if !enabled {
            continue;
        }
        if let Some(minimum) = &plugin.plugin.min_impasto_version
            && version(minimum)? > version(impasto::VERSION)?
        {
            bail!("插件 {alias} 需要 Impasto {minimum} 或更新版本");
        }
        for cap in plugin
            .capabilities
            .requires
            .iter()
            .chain(&plugin.capabilities.optional)
        {
            if !reserved
                .iter()
                .any(|module| embedder::package_folder(module.specifier) == Some(cap))
            {
                bail!("插件 {alias} 声明未知能力 {cap}");
            }
        }
        for cap in &plugin.capabilities.requires {
            if !allowed.split(',').any(|name| name == cap) {
                bail!("插件 {alias} 在 {target} 上需要 [permissions].{cap}");
            }
        }
        let platform = match target {
            "macos" => plugin.macos,
            "ios" => plugin.ios,
            "android" => plugin.android,
            _ => bail!("未知插件目标 {target}"),
        };
        result
            .platform
            .merge(platform, &root)
            .with_context(|| format!("合并插件 {alias} 的 {target} 设置"))?;
        result.crates.push(cargo_package(&root)?);
    }
    let usage = match target {
        "ios" => Some(&app.ios.usage_descriptions),
        "macos" => Some(&app.macos.usage_descriptions),
        _ => None,
    };
    if let Some(usage) = usage {
        for key in &result.platform.usage_description_keys {
            if !usage.contains_key(key) {
                bail!("缺少 [{target}.usage_descriptions].{key}");
            }
        }
    }
    if target == "macos" {
        for key in &result.platform.required_entitlements {
            if app.macos.entitlements.get(key) != Some(&true) {
                bail!("插件需要 [macos.entitlements] 中启用 {key}");
            }
        }
    }
    Ok(result)
}

fn version(version: &str) -> Result<Vec<u32>> {
    let mut parts = version
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !(2..=3).contains(&parts.len()) {
        bail!("无效的版本 {version}");
    }
    parts.resize(3, 0);
    Ok(parts)
}

fn validate_specifier(specifier: &str, reserved: &[engine::session::Module]) -> Result<()> {
    if !impasto::abi::valid_specifier(specifier)
        || matches!(
            specifier,
            "scene" | "reactive" | "#reactive" | "settings" | "solid-js" | "app.js"
        )
        || reserved.iter().any(|module| module.specifier == specifier)
    {
        bail!("插件模块使用了保留或无效的 specifier {specifier:?}");
    }
    if let Some(path) = specifier.strip_prefix("@impasto/") {
        let (package, subpath) = path.split_once('/').unwrap_or((path, ""));
        let key = if subpath.is_empty() {
            ".".into()
        } else {
            format!("./{subpath}")
        };
        let manifest = toolchain::framework()?
            .join("packages")
            .join(package)
            .join("package.json");
        if manifest.is_file() {
            let package: serde_json::Value = serde_json::from_slice(&fs::read(manifest)?)?;
            if package
                .get("exports")
                .and_then(|exports| exports.get(key))
                .is_some()
            {
                bail!("插件入口 {specifier:?} 与框架包冲突");
            }
        }
    }
    Ok(())
}

impl Platform {
    fn merge(&mut self, other: Self, root: &Path) -> Result<()> {
        for source in &other.sources {
            let source = under(root, source)?;
            if !self.sources.contains(&source) {
                self.sources.push(source);
            }
        }
        self.frameworks.extend(other.frameworks);
        self.weak_frameworks.extend(other.weak_frameworks);
        if let Some(framework) = self.frameworks.intersection(&self.weak_frameworks).next() {
            bail!("Framework {framework} 的链接方式冲突");
        }
        for framework in self.frameworks.iter().chain(&self.weak_frameworks) {
            if !framework
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
                || framework.is_empty()
            {
                bail!("无效的 Framework 名称 {framework:?}");
            }
        }
        self.usage_description_keys
            .extend(other.usage_description_keys);
        self.required_entitlements
            .extend(other.required_entitlements);
        self.permissions.extend(other.permissions);
        self.min_sdk = self.min_sdk.max(other.min_sdk);
        if let Some(minimum) = other.min_version {
            let requested = version(&minimum)?;
            let current = self
                .min_version
                .as_deref()
                .map(version)
                .transpose()?
                .unwrap_or_default();
            if requested > current {
                self.min_version = Some(minimum);
            }
        }
        for feature in other.uses_features {
            if let Some(existing) = self
                .uses_features
                .iter_mut()
                .find(|current| current.name == feature.name)
            {
                existing.required |= feature.required;
            } else {
                self.uses_features.push(feature);
            }
        }
        for dependency in other.gradle_dependencies {
            let coords = dependency.split(':').collect::<Vec<_>>();
            let [group, artifact, version] = coords.as_slice() else {
                bail!("Gradle 依赖需要固定 group:artifact:version: {dependency}");
            };
            if coords.iter().any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-' | b'_'))
            }) {
                bail!("Gradle 依赖坐标无效: {dependency}");
            }
            let prefix = format!("{group}:{artifact}:");
            if self
                .gradle_dependencies
                .iter()
                .any(|entry| entry.starts_with(&prefix) && !entry.ends_with(&format!(":{version}")))
            {
                bail!("Gradle 依赖版本冲突: {dependency}");
            }
            self.gradle_dependencies.insert(dependency);
        }
        Ok(())
    }

    pub fn ios_version(&self) -> Result<&str> {
        if let Some(value) = &self.min_version
            && version(value)? > [15, 0, 0].to_vec()
        {
            return Ok(value);
        }
        Ok("15.0")
    }
}

#[derive(Default)]
struct Resolved {
    crates: Vec<(PathBuf, String)>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform: Platform,
}

fn path_literal(path: &Path) -> Result<String> {
    let path = path.to_str().context("Cargo 路径需要 UTF-8")?;
    Ok(toml::Value::String(path.to_owned()).to_string())
}

fn write(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path)
        .as_deref()
        .is_ok_and(|old| old == content)
    {
        return Ok(());
    }
    let temp = path.with_extension("impasto-tmp");
    fs::write(&temp, content)?;
    fs::rename(temp, path)?;
    Ok(())
}

pub fn prepare(app: &App, target: &str, verbose: bool) -> Result<Option<NativeBuild>> {
    if !configured(app) {
        return Ok(None);
    }
    let resolved = resolve(app, target)?;
    if resolved.crates.is_empty() && resolved.disabled_entries.is_empty() {
        return Ok(None);
    }
    let framework = toolchain::framework()?;
    let workspace = app.root.join("build");
    let registry = workspace.join("plugins").join(target);
    let entry = workspace.join(target).join("native");
    fs::create_dir_all(&registry)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(workspace.join("plugins.lock"))?;
    lock.lock()?;
    if resolved.crates.is_empty() {
        let manifest = registry.join("manifest.json");
        let data = Manifest {
            schema_version: 1,
            platform: target.into(),
            modules: Vec::new(),
            types: Vec::new(),
            entries: BTreeMap::new(),
            disabled_entries: resolved.disabled_entries,
            platform_inputs: Platform::default(),
            watch_roots: Vec::new(),
        };
        let declarations = registry.join("types");
        if declarations.is_dir() {
            fs::remove_dir_all(declarations)?;
        }
        write(&manifest, &serde_json::to_string(&data)?)?;
        return Ok(Some(NativeBuild {
            workspace,
            manifest,
            package: String::new(),
            data,
            _lock: lock,
        }));
    }
    fs::create_dir_all(registry.join("src/bin"))?;
    fs::create_dir_all(entry.join("src"))?;
    if !workspace.join("Cargo.lock").is_file() {
        fs::copy(framework.join("Cargo.lock"), workspace.join("Cargo.lock"))?;
    }
    let registry_name = format!("impasto-registry-{target}");
    use std::hash::{Hash as _, Hasher as _};
    let mut hash = std::hash::DefaultHasher::new();
    (app.root.as_path(), target).hash(&mut hash);
    let descriptor_bin = format!("impasto-descriptors-{:016x}", hash.finish());
    let package = format!("impasto-app-{target}");
    let framework_manifest: toml::Value =
        toml::from_str(&fs::read_to_string(framework.join("Cargo.toml"))?)?;
    let profiles = framework_manifest
        .get("profile")
        .map(|profile| toml::to_string(&BTreeMap::from([("profile", profile)])))
        .transpose()?
        .unwrap_or_default();
    write(
        &workspace.join("Cargo.toml"),
        &format!(
            "[workspace]\nresolver = \"3\"\nmembers = [\"plugins/{target}\", \"{target}/native\"]\n{profiles}"
        ),
    )?;
    let sdk = path_literal(&framework.join("crates/impasto"))?;
    let mut package_toml = format!(
        "[package]\nname = \"{registry_name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[[bin]]\nname = \"{descriptor_bin}\"\npath = \"src/bin/descriptors.rs\"\n\n[dependencies]\nimpasto = {{ path = {sdk} }}\nserde_json = \"1\"\n"
    );
    let mut calls = String::new();
    for (index, (path, name)) in resolved.crates.iter().enumerate() {
        use std::fmt::Write as _;
        let alias = format!("plugin_{index}");
        writeln!(
            package_toml,
            "{alias} = {{ package = {name:?}, path = {} }}",
            path_literal(path)?
        )?;
        writeln!(calls, "    {alias}::register(registrar);")?;
    }
    write(&registry.join("Cargo.toml"), &package_toml)?;
    write(
        &registry.join("src/lib.rs"),
        &format!("pub fn register(registrar: &mut impasto::PluginRegistrar) {{\n{calls}}}\n"),
    )?;
    write(
        &registry.join("src/bin/descriptors.rs"),
        &format!(
            r#"fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let mut registry = impasto::PluginRegistrar::default();
    {}::register(&mut registry);
    let items = registry.describe()?.into_iter().map(|module| serde_json::json!({{
        "name": module.name, "specifier": module.specifier, "exports": module.exports, "typescript": module.typescript
    }})).collect::<Vec<_>>();
    serde_json::to_writer(std::io::stdout().lock(), &items)?;
    Ok(())
}}
"#,
            registry_name.replace('-', "_")
        ),
    )?;
    write_entry(
        &entry,
        &registry,
        &framework,
        target,
        &registry_name,
        &package,
    )?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&workspace)
        .args([
            "run",
            "--quiet",
            "-p",
            &registry_name,
            "--bin",
            &descriptor_bin,
        ])
        .arg("--target-dir")
        .arg(framework.join("target"));
    if target == "macos" && !resolved.platform.sources.is_empty() {
        // The host exporter can type-check the same macOS declarations without
        // linking the application's Swift or Objective-C objects.
        let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_else(|_| {
            std::env::var("RUSTFLAGS")
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("\x1f")
        });
        let flags = [
            flags.as_str(),
            "--cfg",
            "impasto_metadata",
            "--check-cfg",
            "cfg(impasto_metadata)",
        ]
        .into_iter()
        .filter(|flag| !flag.is_empty())
        .collect::<Vec<_>>()
        .join("\x1f");
        command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    if verbose {
        eprintln!("导出 {target} 插件声明: {command:?}");
    }
    let output = command.output().context("启动 Rust 插件描述导出器")?;
    if !output.status.success() {
        bail!(
            "导出 {target} 插件声明失败\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let modules: Vec<Descriptor> =
        serde_json::from_slice(&output.stdout).context("解析原生模块描述")?;
    let watch_roots = local_dependencies(&workspace, &framework)?;
    let reserved = embedder::first_party_modules();
    let reserved_names = reserved
        .iter()
        .map(|module| module.name)
        .collect::<BTreeSet<_>>();
    let mut used = resolved
        .entries
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for module in &modules {
        validate_specifier(&module.specifier, &reserved)?;
        if reserved_names.contains(module.name.as_str()) || !used.insert(&module.specifier) {
            bail!("插件声明与其他模块冲突: {}", module.specifier);
        }
    }
    let dir = registry.join("types");
    fs::create_dir_all(&dir)?;
    let mut types = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        let path = dir.join(format!("module-{index}.d.ts"));
        let text = format!(
            "// Generated by Impasto.\ndeclare module {:?} {{\n{}\n}}\n",
            module.specifier,
            module.typescript.replace("export declare ", "export ")
        );
        write(&path, &text)?;
        types.push(path);
    }
    for path in fs::read_dir(&dir)? {
        let path = path?.path();
        if path.is_file() && !types.contains(&path) {
            fs::remove_file(path)?;
        }
    }
    let manifest = registry.join("manifest.json");
    let data = Manifest {
        schema_version: 1,
        platform: target.into(),
        modules,
        types,
        entries: resolved.entries,
        disabled_entries: resolved.disabled_entries,
        platform_inputs: resolved.platform,
        watch_roots,
    };
    write(
        &registry.join("exports.json"),
        &serde_json::to_string(&data.modules)?,
    )?;
    write(&manifest, &serde_json::to_string(&data)?)?;
    Ok(Some(NativeBuild {
        workspace,
        manifest,
        package,
        data,
        _lock: lock,
    }))
}

#[derive(Deserialize)]
struct CargoGraph {
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    manifest_path: PathBuf,
    source: Option<String>,
}

fn local_dependencies(workspace: &Path, framework: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("cargo")
        .current_dir(workspace)
        .args(["metadata", "--locked", "--format-version", "1"])
        .output()
        .context("解析插件 Cargo 依赖图")?;
    if !output.status.success() {
        bail!(
            "读取插件依赖失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let graph: CargoGraph = serde_json::from_slice(&output.stdout)?;
    for (name, folder) in [
        ("impasto", "crates/impasto"),
        ("javascript", "crates/javascript"),
    ] {
        let expected = framework.join(folder).join("Cargo.toml").canonicalize()?;
        for package in graph.packages.iter().filter(|package| package.name == name) {
            if package.source.is_some() || package.manifest_path.canonicalize()? != expected {
                bail!(
                    "插件必须链接工具链提供的 {name}: {}",
                    package.manifest_path.display()
                );
            }
        }
    }
    Ok(graph
        .packages
        .into_iter()
        .filter(|package| package.source.is_none())
        .filter_map(|package| package.manifest_path.parent().map(Path::to_owned))
        .filter(|path| !path.starts_with(workspace))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn write_entry(
    entry: &Path,
    registry: &Path,
    framework: &Path,
    target: &str,
    registry_name: &str,
    package: &str,
) -> Result<()> {
    let host = path_literal(&framework.join("crates/embedder").join(target))?;
    let registry = path_literal(registry)?;
    let extra = if target == "android" {
        ", features = [\"app-entry\"]"
    } else {
        ""
    };
    let product = match target {
        "macos" => "[[bin]]\nname = \"impasto-app\"\npath = \"src/main.rs\"",
        "ios" => "[lib]\nname = \"embedder_ios\"\ncrate-type = [\"staticlib\"]",
        "android" => "[lib]\nname = \"impasto\"\ncrate-type = [\"cdylib\"]",
        _ => bail!("未知原生入口 {target}"),
    };
    let config = format!(
        "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n{product}\n\n[dependencies]\nhost = {{ package = \"embedder-{target}\", path = {host}{extra} }}\nplugins = {{ package = {registry_name:?}, path = {registry} }}\n"
    );
    write(&entry.join("Cargo.toml"), &config)?;
    let install = format!(
        "host::install_plugins(plugins::register, include_str!(\"../../../plugins/{target}/exports.json\"))"
    );
    let (file, source) = match target {
        "macos" => (
            "main.rs",
            format!(
                "fn main() -> std::process::ExitCode {{\n    if let Err(error) = {install} {{\n        eprintln!(\"{{error}}\");\n        return std::process::ExitCode::FAILURE;\n    }}\n    host::main()\n}}\n"
            ),
        ),
        "ios" => (
            "lib.rs",
            format!(
                "#[unsafe(no_mangle)]\npub extern \"C\" fn impasto_install_app_plugins() -> bool {{\n    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();\n    *INSTALLED.get_or_init(|| {install}.is_ok())\n}}\n"
            ),
        ),
        "android" => ("lib.rs", android_entry(&install)?),
        _ => bail!("未知原生入口 {target}"),
    };
    write(&entry.join("src").join(file), &source)
}

fn android_entry(install: &str) -> Result<String> {
    use quote::{ToTokens, quote};
    use std::fmt::Write as _;
    let entry = syn::parse_file(include_str!(
        "../../embedder/android/src/app/entrypoints.rs"
    ))?;
    let mut source = format!(
        r#"use host::app_jni::*;

#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> jint {{
    if {install}.is_ok() {{ 0x0001_0006 }} else {{ -1 }}
}}
"#
    );
    for item in entry.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        let mut signature = function.sig;
        let name = &signature.ident;
        if !name.to_string().starts_with("Java_") {
            continue;
        }
        let name = name.clone();
        let mut params = Vec::new();
        for param in &mut signature.inputs {
            let syn::FnArg::Typed(param) = param else {
                bail!("JNI receiver is invalid")
            };
            let syn::Pat::Ident(pat) = &mut *param.pat else {
                bail!("JNI parameter is invalid")
            };
            pat.mutability = None;
            params.push(pat.ident.clone());
        }
        writeln!(
            source,
            "{}",
            quote! {
                #[unsafe(no_mangle)]
                pub #signature { host::app_jni::#name(#(#params),*) }
            }
            .to_token_stream()
        )?;
    }
    Ok(source)
}

pub fn watched_paths(app: &App) -> BTreeSet<PathBuf> {
    let mut roots = app
        .native
        .iter()
        .map(|entry| app.root.join(&entry.path))
        .chain(app.plugins.values().map(|entry| app.root.join(&entry.path)))
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect::<BTreeSet<_>>();
    if configured(app) {
        for target in &app.targets {
            if let Some(path) = cached_manifest(app, target)
                && let Ok(data) = fs::read(path)
                && let Ok(manifest) = serde_json::from_slice::<Manifest>(&data)
            {
                roots.extend(manifest.watch_roots);
            }
        }
    }
    roots
}

pub fn is_wrapper_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts" | "tsx" | "js" | "jsx")
    )
}

pub fn cached_manifest(app: &App, target: &str) -> Option<PathBuf> {
    let path = app
        .root
        .join("build/plugins")
        .join(target)
        .join("manifest.json");
    path.is_file().then_some(path)
}
