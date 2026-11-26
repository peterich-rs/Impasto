//! Resolves plugin inputs and generates target-specific Cargo projects and declarations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{app::App, toolchain};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub path: PathBuf,
}

pub struct Dependency {
    pub path: PathBuf,
    dependencies: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plugin {
    plugin: Package,
    rust: Local,
    #[serde(default)]
    ios: PlatformProject,
    #[serde(default)]
    android: PlatformProject,
    #[serde(default)]
    macos: PlatformProject,
    #[serde(default, rename = "dependencies")]
    _dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    platforms: Vec<String>,
    min_impasto_version: Option<String>,
}

#[derive(Default, Deserialize)]
struct JavaScriptPackage {
    name: Option<String>,
    #[serde(default)]
    exports: BTreeMap<String, String>,
}

pub fn discover(root: &Path) -> Result<BTreeMap<String, Dependency>> {
    let framework = toolchain::framework()?;
    let root = root.canonicalize()?;
    let mut found = BTreeMap::<String, Dependency>::new();
    let mut edges = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
    let mut pending = vec![root.clone()];
    while let Some(owner) = pending.pop() {
        if edges.contains_key(&owner) {
            continue;
        }
        let file = owner.join("package.json");
        let manifest: serde_json::Value = if file.is_file() {
            serde_json::from_slice(&fs::read(&file)?)
                .with_context(|| format!("解析 {}", file.display()))?
        } else {
            serde_json::Value::Null
        };
        let explicit = crate::dependencies::declared(&owner, &framework)?;
        let mut manifest_entries = manifest
            .get("dependencies")
            .and_then(serde_json::Value::as_object)
            .map_or_else(BTreeMap::new, |entries| {
                entries
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_str().map(str::to_owned)))
                    .collect()
            });
        manifest_entries.extend(explicit.keys().map(|name| (name.clone(), None)));
        let mut children = BTreeSet::new();
        for (name, requirement) in &manifest_entries {
            if !valid_package_name(name) {
                bail!("无效的 package.json 依赖 {name:?}");
            }
            let local = requirement
                .as_deref()
                .and_then(|value| {
                    value
                        .strip_prefix("file:")
                        .or_else(|| value.strip_prefix("link:"))
                })
                .map(|path| owner.join(path));
            let installed = owner
                .ancestors()
                .map(|path| path.join("node_modules").join(name))
                .find(|path| path.join("package.json").is_file());
            let bundled = name
                .strip_prefix("@impasto/")
                .map(|folder| framework.join("packages").join(folder));
            let Some(path) = explicit
                .get(name)
                .cloned()
                .or(local)
                .or(installed)
                .or(bundled)
                .filter(|path| path.join("package.json").is_file())
            else {
                continue;
            };
            let path = path.canonicalize()?;
            children.insert(path.clone());
            if path != root && path.join("plugin.toml").is_file() {
                let package: serde_json::Value =
                    serde_json::from_slice(&fs::read(path.join("package.json"))?)?;
                if package.get("name").and_then(serde_json::Value::as_str) != Some(name) {
                    bail!(
                        "原生插件目录 {} 的 package.json 名称应为 {name}",
                        path.display()
                    );
                }
                if let Some(previous) = found.get(name) {
                    if previous.path != path {
                        bail!("依赖图包含多个 {name} 原生插件版本");
                    }
                } else {
                    found.insert(
                        name.clone(),
                        Dependency {
                            path: path.clone(),
                            dependencies: BTreeSet::new(),
                        },
                    );
                }
            }
            pending.push(path);
        }
        edges.insert(owner, children);
    }

    let names = found
        .iter()
        .map(|(name, dependency)| (dependency.path.clone(), name.clone()))
        .collect::<BTreeMap<_, _>>();
    for (name, dependency) in &mut found {
        let mut seen = BTreeSet::new();
        let mut pending = edges[&dependency.path].iter().collect::<Vec<_>>();
        while let Some(child) = pending.pop() {
            if seen.insert(child) {
                if child == &root && root.join("plugin.toml").is_file() {
                    bail!("原生插件依赖存在环路，{name} 依赖当前插件");
                }
                if let Some(name) = names.get(child) {
                    dependency.dependencies.insert(name.clone());
                }
                pending.extend(&edges[child]);
            }
        }
    }
    Ok(found)
}

fn ordered_plugins(plugins: &BTreeMap<String, Dependency>) -> Result<Vec<&str>> {
    fn visit<'a>(
        name: &'a str,
        plugins: &'a BTreeMap<String, Dependency>,
        stack: &mut Vec<&'a str>,
        complete: &mut BTreeSet<&'a str>,
        ordered: &mut Vec<&'a str>,
    ) -> Result<()> {
        if complete.contains(name) {
            return Ok(());
        }
        if let Some(start) = stack.iter().position(|entry| *entry == name) {
            let cycle = stack[start..]
                .iter()
                .copied()
                .chain([name])
                .collect::<Vec<_>>();
            bail!("原生插件依赖存在环路 {}", cycle.join(" -> "));
        }
        stack.push(name);
        for next in &plugins[name].dependencies {
            visit(next, plugins, stack, complete, ordered)?;
        }
        stack.pop();
        complete.insert(name);
        ordered.push(name);
        Ok(())
    }

    let mut stack = Vec::new();
    let mut complete = BTreeSet::new();
    let mut ordered = Vec::new();
    for name in plugins.keys() {
        visit(name, plugins, &mut stack, &mut complete, &mut ordered)?;
    }
    Ok(ordered)
}

pub(crate) fn crate_name(package: &str) -> String {
    package
        .trim_start_matches('@')
        .replace(['/', '.', '_'], "-")
}

pub(crate) fn plugin_crate(root: &Path) -> Result<(Local, Vec<String>)> {
    let file = root.join("plugin.toml");
    let plugin: Plugin = toml::from_str(&fs::read_to_string(&file)?)
        .with_context(|| format!("解析 {}", file.display()))?;
    Ok((plugin.rust, plugin.plugin.platforms))
}

pub(crate) fn valid_package_name(name: &str) -> bool {
    let valid = |part: &str| {
        !part.is_empty()
            && !matches!(part, "." | "..")
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    };
    if let Some(scoped) = name.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, package)| valid(scope) && valid(package))
    } else {
        valid(name)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PlatformProject {
    path: Option<PathBuf>,
    package: Option<String>,
    module: Option<String>,
    plugin_class: Option<String>,
    usage_description_keys: BTreeSet<String>,
    required_entitlements: BTreeSet<String>,
    permissions: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct NativeProject {
    pub path: PathBuf,
    pub package: String,
    pub gradle_module: String,
    pub module: String,
    pub plugin_class: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Platform {
    pub projects: Vec<NativeProject>,
    pub usage_description_keys: BTreeSet<String>,
    pub required_entitlements: BTreeSet<String>,
    permission_rules: BTreeMap<String, BTreeMap<String, Vec<PermissionMethod>>>,
    pub android_permissions: BTreeSet<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct PermissionMethod {
    specifier: String,
    export: String,
    #[serde(default)]
    object_export: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Descriptor {
    pub name: String,
    pub specifier: String,
    pub typescript: String,
    pub exports: Vec<String>,
    #[serde(default, rename = "crate")]
    owner: String,
    #[serde(default, rename = "exportAbi")]
    export_abi: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    export_signatures: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    selectable_exports: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    schema_version: u32,
    #[serde(default)]
    script_abi: u32,
    platform: String,
    modules: Vec<Descriptor>,
    types: Vec<PathBuf>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform_inputs: Platform,
    #[serde(default)]
    watch_roots: Vec<PathBuf>,
    #[serde(default)]
    native_dependencies: BTreeMap<String, BTreeSet<String>>,
}

impl Manifest {
    pub(crate) fn linker_manifest(&self) -> bundler::PluginManifest {
        bundler::PluginManifest::new(
            self.platform.clone(),
            self.entries.clone(),
            self.disabled_entries.clone(),
            self.modules
                .iter()
                .map(|module| (module.specifier.clone(), module.exports.clone())),
        )
    }

    pub(crate) fn validate_update(&self, installed: &[u8], linked: &bundler::Bundle) -> Result<()> {
        let installed: InstalledContract = serde_json::from_slice(installed)?;
        if installed.schema_version != self.schema_version
            || installed.platform != self.platform
            || installed.script_abi != SCRIPT_ABI
        {
            bail!("已安装壳的原生契约版本或平台不匹配，请重新构建并安装应用");
        }
        let roots = self
            .modules
            .iter()
            .filter(|module| linked.native_use.contains_key(&module.specifier))
            .map(|module| module.owner.clone())
            .collect::<BTreeSet<_>>();
        let (retained, dependencies) = self.dependency_closure(roots);
        let required_permissions =
            self.platform_inputs
                .permissions_for(linked, &dependencies, &retained);
        if let Some(permission) = required_permissions
            .difference(&installed.android_permissions)
            .next()
        {
            bail!("已安装壳缺少 Android 权限 {permission}，请重新构建并安装应用");
        }
        for (specifier, exports) in &linked.native_use {
            let current = self
                .modules
                .iter()
                .find(|item| &item.specifier == specifier);
            let prior = installed
                .modules
                .iter()
                .find(|item| &item.specifier == specifier);
            let (Some(current), Some(prior)) = (current, prior) else {
                bail!("已安装壳缺少原生模块 {specifier}，请重新构建并安装应用");
            };
            let required = exports.as_ref().map_or_else(
                || current.exports.iter().collect::<Vec<_>>(),
                |exports| exports.iter().collect(),
            );
            for name in required {
                let signature = current.export_abi.get(name);
                if signature.is_none() || signature != prior.export_abi.get(name) {
                    bail!("原生接口 {specifier}::{name} 与已安装壳不兼容，请重新构建并安装应用");
                }
            }
        }
        Ok(())
    }

    fn dependency_closure(
        &self,
        mut retained: BTreeSet<String>,
    ) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut pending = retained.iter().cloned().collect::<Vec<_>>();
        let mut dependencies = BTreeSet::new();
        while let Some(owner) = pending.pop() {
            for dependency in self.native_dependencies.get(&owner).into_iter().flatten() {
                dependencies.insert(dependency.clone());
                if retained.insert(dependency.clone()) {
                    pending.push(dependency.clone());
                }
            }
        }
        (retained, dependencies)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledContract {
    schema_version: u32,
    #[serde(default)]
    script_abi: u32,
    platform: String,
    modules: Vec<InstalledModule>,
    #[serde(default)]
    android_permissions: BTreeSet<String>,
}

// Increment when generated JavaScript requires a new scene or runtime binding.
const SCRIPT_ABI: u32 = 2;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledModule {
    specifier: String,
    #[serde(default)]
    export_abi: BTreeMap<String, String>,
}

pub struct NativeBuild {
    pub workspace: PathBuf,
    pub package: String,
    data: Manifest,
    crates: Vec<(PathBuf, String)>,
    native_exports: String,
    _lock: fs::File,
}

impl NativeBuild {
    pub fn inputs(&self) -> &Platform {
        &self.data.platform_inputs
    }

    pub(crate) fn native_exports(&self) -> &str {
        &self.native_exports
    }

    pub fn linker_manifest(&self) -> bundler::PluginManifest {
        self.data.linker_manifest()
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>> {
        let contract = InstalledContract {
            schema_version: self.data.schema_version,
            script_abi: SCRIPT_ABI,
            platform: self.data.platform.clone(),
            android_permissions: self.data.platform_inputs.android_permissions.clone(),
            modules: self
                .data
                .modules
                .iter()
                .map(|module| InstalledModule {
                    specifier: module.specifier.clone(),
                    export_abi: module.export_abi.clone(),
                })
                .collect(),
        };
        Ok(serde_json::to_vec_pretty(&contract)?)
    }

    pub(crate) fn test_runner(&self, framework: &Path, gpu: bool) -> Result<Option<PathBuf>> {
        let packages = framework.join("packages");
        if self
            .crates
            .iter()
            .all(|(path, _)| path.starts_with(&packages))
        {
            return Ok(None);
        }
        use std::hash::{Hash as _, Hasher as _};
        let mut hash = std::hash::DefaultHasher::new();
        self.workspace.hash(&mut hash);
        self.data.platform.hash(&mut hash);
        let name = format!("impasto-native-tests-{:016x}", hash.finish());
        let registry = format!("registry-{}", self.data.platform);
        let directory = self.workspace.join(&name);
        fs::create_dir_all(directory.join("src"))?;
        let features = if gpu {
            ", features = [\"headless-gpu\"]"
        } else {
            ""
        };
        let manifest = format!(
            "[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nimpasto = {{ path = {} }}\nregistry = {{ path = {:?}, package = {:?} }}\ntesting = {{ path = {}, default-features = false{features} }}\ntempfile = \"3\"\n",
            path_literal(&framework.join("crates/impasto"))?,
            format!("../{registry}"),
            format!("impasto-{registry}"),
            path_literal(&framework.join("crates/testing"))?,
        );
        write(&directory.join("Cargo.toml"), &manifest)?;
        write(
            &directory.join("src/main.rs"),
            include_str!("../templates/test_host.rs"),
        )?;
        let workspace_manifest = self.workspace.join("Cargo.toml");
        let mut workspace: toml::Value = toml::from_str(&fs::read_to_string(&workspace_manifest)?)?;
        let members = workspace
            .get_mut("workspace")
            .and_then(|table| table.get_mut("members"))
            .and_then(toml::Value::as_array_mut)
            .context("生成的 Cargo 工作区缺少 members")?;
        let member = toml::Value::String(name.clone());
        if !members.contains(&member) {
            members.push(member);
        }
        write(&workspace_manifest, &toml::to_string(&workspace)?)?;
        let target = toolchain::target_dir(&framework.join("target"))?;
        let mut command = Command::new("cargo");
        command
            .current_dir(&self.workspace)
            .args(["build", "--quiet", "-p", &name])
            .arg("--target-dir")
            .arg(&target);
        toolchain::run(command, "构建应用 Rust 无头测试宿主")?;
        Ok(Some(target.join("debug").join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name
        })))
    }

    pub(crate) fn prune(&mut self, app: &App, linked: &bundler::Bundle) -> Result<()> {
        use std::fmt::Write as _;
        let described = self
            .data
            .modules
            .iter()
            .map(|module| module.owner.as_str())
            .collect::<BTreeSet<_>>();
        let local = app
            .native
            .as_ref()
            .map(|local| cargo_package(&app.root.join(&local.path)))
            .transpose()?
            .map(|(_, name)| name);
        let mut retained = self
            .data
            .modules
            .iter()
            .filter(|module| linked.native_use.contains_key(&module.specifier))
            .map(|module| module.owner.clone())
            .collect::<BTreeSet<_>>();
        retained.extend(
            self.crates
                .iter()
                .filter(|(_, name)| !described.contains(name.as_str()))
                .map(|(_, name)| name.clone()),
        );
        retained.extend(local);
        let (retained, dependencies) = self.data.dependency_closure(retained);
        let inputs = &mut self.data.platform_inputs;
        let permissions = inputs.permissions_for(linked, &dependencies, &retained);
        let permissions_changed = inputs.android_permissions != permissions;
        inputs.android_permissions = permissions;
        let crates_pruned = self.crates.iter().any(|(_, name)| !retained.contains(name));
        self.data
            .modules
            .retain(|module| retained.contains(&module.owner));
        for module in &mut self.data.modules {
            if module.selectable_exports
                && let Some(Some(names)) = linked.native_use.get(&module.specifier)
                && !names.is_empty()
                && names.len() < module.exports.len()
            {
                for name in names {
                    writeln!(self.native_exports, "{}\t{name}", module.specifier)?;
                }
                module.exports.retain(|name| names.contains(name));
                module.export_abi.retain(|name, _| names.contains(name));
                module.typescript.clear();
            }
        }
        let platform = &self.data.platform;
        let registry = self.workspace.join(format!("registry-{platform}"));
        write_runtime_exports(&registry, &self.data.modules)?;
        if !crates_pruned && !permissions_changed {
            return Ok(());
        }
        if crates_pruned {
            self.data.platform_inputs.projects.retain(|project| {
                retained
                    .iter()
                    .any(|name| name.replace('-', "_") == project.gradle_module)
            });
            write_runtime_registry(&registry, &self.crates, Some(&retained))?;
        }
        let framework = toolchain::framework()?;
        crate::sync::platform(app, platform, &framework, &self.data.platform_inputs)
    }

    pub(crate) fn validate_update(&self, installed: &[u8], linked: &bundler::Bundle) -> Result<()> {
        self.data.validate_update(installed, linked)
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

pub(crate) fn cargo_package(root: &Path) -> Result<(PathBuf, String)> {
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
    let framework = toolchain::framework()?;
    for (name, root) in crate::dependencies::declared(&app.root, &framework)? {
        if root.join("plugin.toml").is_file() || root.starts_with(framework.join("packages")) {
            continue;
        }
        let package: JavaScriptPackage =
            serde_json::from_slice(&fs::read(root.join("package.json"))?)?;
        for (key, entry) in package.exports {
            let specifier = if key == "." {
                name.clone()
            } else if let Some(subpath) = key.strip_prefix("./") {
                format!("{name}/{subpath}")
            } else {
                bail!("依赖 {name} 的 exports 键无效: {key}");
            };
            validate_specifier(&specifier)?;
            result
                .entries
                .insert(specifier, under(&root, Path::new(&entry))?);
        }
    }
    for alias in ordered_plugins(&app.plugins)? {
        let dependency = &app.plugins[alias];
        let root = app
            .root
            .join(&dependency.path)
            .canonicalize()
            .with_context(|| format!("插件 {alias} 路径不存在"))?;
        let package: JavaScriptPackage =
            serde_json::from_slice(&fs::read(root.join("package.json"))?)?;
        let specifier = package.name.context("插件 package.json 缺少 name")?;
        if !valid_package_name(&specifier) || !impasto::abi::valid_specifier(&specifier) {
            bail!("无效的插件包名 {specifier:?}");
        }
        let plugin: Plugin = toml::from_str(&fs::read_to_string(root.join("plugin.toml"))?)
            .with_context(|| format!("解析插件 {alias} 的 plugin.toml"))?;
        if plugin.plugin.platforms.is_empty()
            || plugin
                .plugin
                .platforms
                .iter()
                .any(|target| !matches!(target.as_str(), "macos" | "ios" | "android"))
        {
            bail!("插件 {alias} 的 targets 无效");
        }
        let enabled = plugin.plugin.platforms.iter().any(|item| item == target);
        let (cargo_root, cargo_name) = cargo_package(&under(&root, &plugin.rust.path)?)?;
        let expected = crate_name(&specifier);
        if cargo_name != expected {
            bail!("插件 {specifier} 的 Rust crate 应命名为 {expected}，当前为 {cargo_name}");
        }
        if result
            .installed
            .insert(cargo_name.clone(), cargo_root.clone())
            .is_some()
        {
            bail!("多个 npm 插件映射到 Rust crate {cargo_name}");
        }
        for (key, entry) in package.exports {
            let name = if key == "." {
                specifier.clone()
            } else if let Some(subpath) = key.strip_prefix("./") {
                format!("{specifier}/{subpath}")
            } else {
                bail!("插件 {specifier} 的 exports 键无效: {key}");
            };
            validate_specifier(&name)?;
            if enabled {
                let entry = under(&root, Path::new(&entry))?;
                if result.entries.insert(name.clone(), entry).is_some() {
                    bail!("重复的插件入口 {name}");
                }
            } else {
                result.disabled_entries.insert(name);
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
        let platform = match target {
            "macos" => plugin.macos,
            "ios" => plugin.ios,
            "android" => plugin.android,
            _ => bail!("未知插件目标 {target}"),
        };
        result
            .platform
            .add_project(platform, &root, target, &cargo_name)
            .with_context(|| format!("合并插件 {alias} 的 {target} 设置"))?;
        result.allowed.insert(
            cargo_name.clone(),
            dependency
                .dependencies
                .iter()
                .map(|name| crate_name(name))
                .collect(),
        );
        result.crates.push((cargo_root, cargo_name));
    }
    if let Some(native) = &app.native {
        let (path, name) = cargo_package(&under(&app.root, &native.path)?)?;
        if result.installed.contains_key(&name) || name == "impasto" {
            bail!("应用 Rust crate 名称 {name} 与已安装的插件冲突");
        }
        result
            .allowed
            .insert(name.clone(), result.installed.keys().cloned().collect());
        result.crates.push((path, name));
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

fn validate_specifier(specifier: &str) -> Result<()> {
    if !impasto::abi::valid_specifier(specifier)
        || matches!(
            specifier,
            "scene" | "reactive" | "#reactive" | "settings" | "solid-js" | "app.js"
        )
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
        if manifest.is_file() && !manifest.with_file_name("plugin.toml").is_file() {
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
    fn permissions_for(
        &self,
        linked: &bundler::Bundle,
        dependencies: &BTreeSet<String>,
        retained: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        self.permission_rules
            .iter()
            .filter(|(owner, _)| retained.contains(*owner))
            .flat_map(|(owner, rules)| {
                rules.iter().filter_map(move |(permission, methods)| {
                    let required = dependencies.contains(owner)
                        || methods.is_empty()
                        || methods.iter().any(|method| {
                            linked
                                .native_use
                                .get(&method.specifier)
                                .is_some_and(|names| {
                                    names.as_ref().is_none_or(|names| {
                                        names.contains("default")
                                            || names.contains(&method.export)
                                            || method
                                                .object_export
                                                .as_ref()
                                                .is_some_and(|name| names.contains(name))
                                    })
                                })
                        });
                    required.then(|| permission.clone())
                })
            })
            .collect()
    }

    fn resolve_permission_methods(&mut self, modules: &[Descriptor]) -> Result<()> {
        for (owner, rules) in &mut self.permission_rules {
            for (permission, methods) in rules {
                for method in methods {
                    let module = modules
                        .iter()
                        .find(|module| {
                            module.owner == *owner
                                && module.specifier == method.specifier
                                && module.exports.contains(&method.export)
                        })
                        .with_context(|| {
                            format!(
                                "{owner}: {permission} 引用了未声明的原生导出 {}#{}",
                                method.specifier, method.export
                            )
                        })?;
                    method.object_export = module
                        .exports
                        .contains(&module.name)
                        .then(|| module.name.clone());
                }
            }
        }
        Ok(())
    }

    fn add_project(
        &mut self,
        project: PlatformProject,
        root: &Path,
        target: &str,
        cargo_name: &str,
    ) -> Result<()> {
        for (permission, methods) in project.permissions {
            if target != "android"
                || !permission.contains('.')
                || !permission.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                })
            {
                bail!("无效的 Android 权限 {permission}");
            }
            let methods = methods
                .into_iter()
                .map(|method| {
                    let (specifier, export) = method.rsplit_once('#').with_context(|| {
                        format!("{permission} 的方法需要使用 module#export 格式")
                    })?;
                    Ok(PermissionMethod {
                        specifier: specifier.into(),
                        export: export.into(),
                        object_export: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.android_permissions.insert(permission.clone());
            self.permission_rules
                .entry(cargo_name.into())
                .or_default()
                .insert(permission, methods);
        }
        self.usage_description_keys
            .extend(project.usage_description_keys);
        self.required_entitlements
            .extend(project.required_entitlements);
        let Some(path) = project.path else {
            if project.package.is_some()
                || project.module.is_some()
                || project.plugin_class.is_some()
            {
                bail!("平台插件的注册信息需要 path");
            }
            return Ok(());
        };
        let path = under(root, &path)?;
        let package = project.package.context("平台插件需要声明 package")?;
        let plugin_class = project.plugin_class;
        let identifier = |value: &str| {
            value.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                && value
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
        };
        let module = project.module.unwrap_or_else(|| package.clone());
        if plugin_class
            .as_deref()
            .is_some_and(|name| !identifier(name))
            || (target != "android" && plugin_class.is_none())
            || (target == "android" && !package.split('.').all(identifier))
            || (target != "android" && (!identifier(&module) || !valid_package_name(&package)))
        {
            bail!("平台插件包、模块或注册类名称无效");
        }
        let required = if target == "android" {
            "build.gradle.kts"
        } else {
            "Package.swift"
        };
        if !path.join(required).is_file() {
            bail!("缺少 {}", path.join(required).display());
        }
        if self.projects.iter().any(|item| item.package == package) {
            bail!("重复的平台插件包 {package}");
        }
        let gradle_module = cargo_name.replace('-', "_");
        if target == "android"
            && self
                .projects
                .iter()
                .any(|project| project.gradle_module == gradle_module)
        {
            bail!("重复的 Gradle 模块 {gradle_module}");
        }
        self.projects.push(NativeProject {
            path,
            package,
            gradle_module,
            module,
            plugin_class,
        });
        Ok(())
    }
}

#[derive(Default)]
struct Resolved {
    crates: Vec<(PathBuf, String)>,
    allowed: BTreeMap<String, BTreeSet<String>>,
    installed: BTreeMap<String, PathBuf>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform: Platform,
}

pub(crate) fn path_literal(path: &Path) -> Result<String> {
    let path = path.to_str().context("Cargo 路径需要 UTF-8")?;
    Ok(toml::Value::String(path.to_owned()).to_string())
}

pub(crate) fn write(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path)
        .as_deref()
        .is_ok_and(|old| old == content)
    {
        return Ok(());
    }
    use std::io::Write as _;
    let parent = path.parent().context("缺少生成文件目录")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.persist(path)?;
    Ok(())
}

pub fn prepare(app: &App, target: &str, verbose: bool) -> Result<NativeBuild> {
    let mut resolved = resolve(app, target)?;
    let framework = toolchain::framework()?;
    let glue = app.root.join(".impasto");
    let workspace = glue.join("cargo");
    let registry = workspace.join(format!("registry-{target}"));
    let entry = workspace.join(format!("entry-{target}"));
    fs::create_dir_all(&workspace)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(glue.join("plugins.lock"))?;
    lock.lock()?;
    crate::sync::platform(app, target, &framework, &resolved.platform)?;
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
    let mut sources = resolved.installed.clone();
    sources.insert("impasto".into(), framework.join("crates/impasto"));
    for (path, _) in &resolved.crates {
        crate::local_registry::check_registry_dependencies(path, &sources)?;
    }
    let patches = crate::local_registry::configure(&app.root, &workspace, &sources, &framework)?;
    let signature = {
        let mut hasher = std::hash::DefaultHasher::new();
        sources.hash(&mut hasher);
        for (name, dependency) in &app.plugins {
            name.hash(&mut hasher);
            dependency.dependencies.hash(&mut hasher);
        }
        app.native
            .as_ref()
            .map(|native| &native.path)
            .hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };
    write(&registry.join("inputs.hash"), &signature)?;
    let sdk_version = impasto::VERSION;
    let mut package_toml = format!(
        "[package]\nname = \"{registry_name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[[bin]]\nname = \"{descriptor_bin}\"\npath = \"src/bin/descriptors.rs\"\n\n[dependencies]\nimpasto = {{ version = {sdk_version:?}, registry = \"impasto\" }}\nserde_json = \"1\"\n"
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
        writeln!(
            calls,
            "    collect(&mut items, {name:?}, {alias}::register)?;"
        )?;
    }
    write(&registry.join("Cargo.toml"), &package_toml)?;
    write_runtime_registry(&registry, &resolved.crates, None)?;
    write(
        &registry.join("src/bin/descriptors.rs"),
        &format!(
            r#"fn main() -> Result<(), Box<dyn std::error::Error>> {{
    fn collect(items: &mut Vec<serde_json::Value>, owner: &str, register: impasto::Register) -> Result<(), impasto::PluginInitError> {{
    let mut registry = impasto::PluginRegistrar::default();
    register(&mut registry);
    items.extend(registry.describe()?.into_iter().map(|module| serde_json::json!({{
        "name": module.name, "specifier": module.specifier, "exports": module.exports,
        "typescript": module.typescript, "export_signatures": module.export_signatures, "crate": owner,
        "selectable_exports": module.selectable_exports
    }})));
    Ok(())
    }}
    let _ = collect;
    let mut items = Vec::<serde_json::Value>::new();
    let _ = &mut items;
{calls}
    serde_json::to_writer(std::io::stdout().lock(), &items)?;
    Ok(())
}}
"#
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
    let mut members = Vec::new();
    for platform in &app.targets {
        let registry = format!("registry-{platform}");
        let entry = format!("entry-{platform}");
        let manifest = workspace.join(&registry).join("Cargo.toml");
        if manifest.is_file()
            && workspace.join(&entry).join("Cargo.toml").is_file()
            && fs::read_to_string(workspace.join(&registry).join("inputs.hash"))
                .is_ok_and(|hash| hash == signature)
            && registry_sources_exist(&manifest)?
        {
            members.extend([registry, entry]);
        }
    }
    write(
        &workspace.join("Cargo.toml"),
        &format!("[workspace]\nresolver = \"3\"\nmembers = {members:?}\n{profiles}\n{patches}"),
    )?;
    let watch_roots = local_dependencies(&workspace, &framework, &resolved, target)?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&workspace)
        .args([
            "run",
            "--quiet",
            "--locked",
            "-p",
            &registry_name,
            "--bin",
            &descriptor_bin,
        ])
        .arg("--target-dir")
        .arg(toolchain::target_dir(&framework.join("target"))?);
    if target == "macos" && !resolved.platform.projects.is_empty() {
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
    let output = command
        .env_remove("IMPASTO_NATIVE_EXPORTS")
        .output()
        .context("启动 Rust 插件描述导出器")?;
    if !output.status.success() {
        bail!(
            "导出 {target} 插件声明失败\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut modules: Vec<Descriptor> =
        serde_json::from_slice(&output.stdout).context("解析原生模块描述")?;
    let mut used = resolved
        .entries
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    for module in &mut modules {
        validate_specifier(&module.specifier)?;
        if !used.insert(&module.specifier) {
            bail!("插件声明与其他模块冲突: {}", module.specifier);
        }
        if !names.insert(&module.name) {
            bail!("重复的原生模块名称 {}", module.name);
        }
        module.export_abi = std::mem::take(&mut module.export_signatures)
            .into_iter()
            .map(|(name, signature)| {
                let hash = Sha256::digest(signature.as_bytes());
                (name, URL_SAFE_NO_PAD.encode(hash))
            })
            .collect();
    }
    resolved.platform.resolve_permission_methods(&modules)?;
    let dir = glue.join("types").join(target);
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
    let manifest = glue.join(format!("plugins-manifest.{target}.json"));
    let data = Manifest {
        schema_version: 1,
        script_abi: SCRIPT_ABI,
        platform: target.into(),
        modules,
        types,
        entries: resolved.entries,
        disabled_entries: resolved.disabled_entries,
        platform_inputs: resolved.platform,
        watch_roots,
        native_dependencies: resolved.allowed,
    };
    write_runtime_exports(&registry, &data.modules)?;
    write(&manifest, &serde_json::to_string(&data)?)?;
    crate::sync::typescript(&app.root, &framework, &data.entries, &data.types)?;
    Ok(NativeBuild {
        workspace,
        package,
        data,
        crates: resolved.crates,
        native_exports: String::new(),
        _lock: lock,
    })
}

fn write_runtime_registry(
    registry: &Path,
    crates: &[(PathBuf, String)],
    retained: Option<&BTreeSet<String>>,
) -> Result<()> {
    use std::fmt::Write as _;
    let mut code = String::from(
        "pub fn register(registrar: &mut impasto::PluginRegistrar) {\n    let _ = &registrar;\n",
    );
    for (index, (_, name)) in crates.iter().enumerate() {
        if retained.is_none_or(|retained| retained.contains(name)) {
            writeln!(code, "    plugin_{index}::register(registrar);")?;
        }
    }
    code.push_str("}\n");
    write(&registry.join("src/lib.rs"), &code)
}

fn write_runtime_exports(registry: &Path, modules: &[Descriptor]) -> Result<()> {
    let exports = modules
        .iter()
        .map(|module| {
            serde_json::json!({
                "name": module.name,
                "specifier": module.specifier,
                "exports": module.exports,
                "typescript": (!module.typescript.is_empty()).then_some(&module.typescript),
            })
        })
        .collect::<Vec<_>>();
    write(
        &registry.join("exports.json"),
        &serde_json::to_string(&exports)?,
    )
}

fn registry_sources_exist(manifest: &Path) -> Result<bool> {
    // A removed local plugin must not prevent using another platform's registry.
    let registry: toml::Value = toml::from_str(&fs::read_to_string(manifest)?)?;
    Ok(registry
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|dependencies| dependencies.values())
        .filter_map(|dependency| dependency.get("path").and_then(toml::Value::as_str))
        .all(|path| Path::new(path).join("Cargo.toml").is_file()))
}

#[derive(Deserialize)]
struct CargoGraph {
    packages: Vec<CargoPackage>,
    resolve: CargoResolve,
}

#[derive(Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    source: Option<String>,
}

#[derive(Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoNode>,
}

#[derive(Deserialize)]
struct CargoNode {
    id: String,
    dependencies: Vec<String>,
}

fn local_dependencies(
    workspace: &Path,
    framework: &Path,
    input: &Resolved,
    target: &str,
) -> Result<Vec<PathBuf>> {
    let targets: &[&str] = match target {
        "macos" => &["aarch64-apple-darwin", "x86_64-apple-darwin"],
        "ios" => &[
            "aarch64-apple-ios",
            "aarch64-apple-ios-sim",
            "x86_64-apple-ios",
        ],
        "android" => &["aarch64-linux-android", "x86_64-linux-android"],
        _ => bail!("未知 Cargo 目标 {target}"),
    };
    let mut roots = BTreeSet::new();
    for triple in targets {
        let output = Command::new("cargo")
            .current_dir(workspace)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--filter-platform",
                triple,
            ])
            .output()
            .context("解析插件 Cargo 依赖图")?;
        if !output.status.success() {
            bail!(
                "读取 {triple} 插件依赖失败 {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let graph: CargoGraph = serde_json::from_slice(&output.stdout)?;
        validate_cargo_graph(&graph, framework, input, triple)?;
        roots.extend(
            graph
                .packages
                .into_iter()
                .filter(|package| package.source.is_none())
                .filter_map(|package| package.manifest_path.parent().map(Path::to_owned))
                .filter(|path| !path.starts_with(workspace)),
        );
    }
    Ok(roots.into_iter().collect())
}

fn validate_cargo_graph(
    graph: &CargoGraph,
    framework: &Path,
    input: &Resolved,
    triple: &str,
) -> Result<()> {
    for (name, folder) in [
        ("impasto", "crates/impasto"),
        ("javascript", "crates/javascript"),
    ] {
        let expected = framework.join(folder).join("Cargo.toml").canonicalize()?;
        for package in graph.packages.iter().filter(|package| package.name == name) {
            if package.source.is_some() || package.manifest_path.canonicalize()? != expected {
                bail!(
                    "插件必须链接工具链提供的 {name} {}",
                    package.manifest_path.display()
                );
            }
        }
    }

    let packages = graph
        .packages
        .iter()
        .map(|pkg| (pkg.id.as_str(), pkg))
        .collect::<BTreeMap<_, _>>();
    let edges = graph
        .resolve
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), &node.dependencies))
        .collect::<BTreeMap<_, _>>();
    for (root, owner) in &input.crates {
        let manifest = root.join("Cargo.toml").canonicalize()?;
        let package = graph
            .packages
            .iter()
            .find(|pkg| pkg.manifest_path == manifest)
            .with_context(|| format!("Cargo 未解析 {owner} ({triple})"))?;
        let allowed = &input.allowed[owner];
        let mut seen = BTreeSet::new();
        let mut pending = edges[package.id.as_str()]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let dependency = packages[id];
            if let Some(root) = input.installed.get(&dependency.name) {
                if dependency.source.is_some()
                    || dependency.manifest_path.canonicalize()?
                        != root.join("Cargo.toml").canonicalize()?
                {
                    bail!(
                        "{owner} 的 Rust 依赖 {} 必须来自 npm 解析的目录 {}",
                        dependency.name,
                        root.display()
                    );
                }
                if !allowed.contains(&dependency.name) {
                    bail!(
                        "{owner} 的 Rust 依赖 {} 缺少对应的 package.json dependencies 声明 ({triple})",
                        dependency.name
                    );
                }
                if !input.allowed.contains_key(&dependency.name) {
                    bail!(
                        "{owner} 的 Rust 依赖 {} 未启用平台 {triple}",
                        dependency.name
                    );
                }
            } else if dependency.source.is_none()
                && package_is_native_plugin(&dependency.manifest_path)
            {
                bail!(
                    "{owner} 的 Rust 依赖 {} 缺少对应的 npm 插件 ({triple})",
                    dependency.name
                );
            }
            if let Some(children) = edges.get(id) {
                pending.extend(children.iter().map(String::as_str));
            }
        }
    }
    Ok(())
}

fn package_is_native_plugin(manifest: &Path) -> bool {
    manifest
        .parent()
        .and_then(Path::parent)
        .is_some_and(|root| root.join("plugin.toml").is_file())
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
        "macos" => "[lib]\nname = \"impasto_app\"\ncrate-type = [\"staticlib\"]",
        "ios" => "[lib]\nname = \"impasto_app\"\ncrate-type = [\"staticlib\"]",
        "android" => "[lib]\nname = \"impasto\"\ncrate-type = [\"cdylib\"]",
        _ => bail!("未知原生入口 {target}"),
    };
    let config = format!(
        "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n{product}\n\n[dependencies]\nhost = {{ package = \"embedder-{target}\", path = {host}, default-features = false{extra} }}\nplugins = {{ package = {registry_name:?}, path = {registry} }}\n"
    );
    write(&entry.join("Cargo.toml"), &config)?;
    let install = format!(
        "host::install_plugins(plugins::register, include_str!(\"../../registry-{target}/exports.json\"))"
    );
    let (file, source) = match target {
        "macos" => (
            "lib.rs",
            format!(
                "#[unsafe(no_mangle)]\npub extern \"C\" fn impasto_macos_run(open_url: Option<host::OpenUrl>) -> u8 {{\n    if let Err(error) = {install} {{\n        eprintln!(\"{{error}}\");\n        return 1;\n    }}\n    host::exit_status_with_url_handler(open_url)\n}}\n"
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
        .join(".impasto")
        .join(format!("plugins-manifest.{target}.json"));
    path.is_file().then_some(path)
}

/// The watch loop has already checked that the configured native inputs are unchanged.
pub(crate) fn watch_manifest(app: &App, target: &str, verbose: bool) -> Result<Manifest> {
    if let Some(path) = cached_manifest(app, target)
        && let Ok(data) = fs::read(path)
        && let Ok(manifest) = serde_json::from_slice::<Manifest>(&data)
        && manifest.platform == target
        && manifest.schema_version == 1
        && manifest.script_abi == SCRIPT_ABI
    {
        return Ok(manifest);
    }
    Ok(prepare(app, target, verbose)?.data)
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn android_permissions_follow_the_linked_method_or_an_opaque_reference() -> Result<()> {
        let owner = "camera".to_owned();
        let specifier = "@demo/camera/native";
        let permission = "android.permission.CAMERA";
        let mut platform = Platform::default();
        platform.permission_rules.insert(
            owner.clone(),
            BTreeMap::from([(
                permission.to_owned(),
                vec![PermissionMethod {
                    specifier: specifier.to_owned(),
                    export: "open".to_owned(),
                    object_export: None,
                }],
            )]),
        );
        let module = Descriptor {
            name: "Camera".to_owned(),
            specifier: specifier.to_owned(),
            typescript: String::new(),
            exports: vec!["open".to_owned(), "ping".to_owned(), "Camera".to_owned()],
            owner: owner.clone(),
            export_abi: BTreeMap::from([
                ("open".to_owned(), "signature".to_owned()),
                ("ping".to_owned(), "ping".to_owned()),
            ]),
            export_signatures: BTreeMap::new(),
            selectable_exports: false,
        };
        platform.resolve_permission_methods(std::slice::from_ref(&module))?;
        let manifest = Manifest {
            schema_version: 1,
            script_abi: SCRIPT_ABI,
            platform: "android".to_owned(),
            modules: vec![module],
            types: Vec::new(),
            entries: BTreeMap::new(),
            disabled_entries: BTreeSet::new(),
            platform_inputs: platform,
            watch_roots: Vec::new(),
            native_dependencies: BTreeMap::new(),
        };
        let retained = BTreeSet::from([owner]);
        let bundle = |export: Option<&str>| bundler::Bundle {
            code: String::new(),
            warnings: Vec::new(),
            source_map: None,
            native_use: BTreeMap::from([(
                specifier.to_owned(),
                export.map(|export| BTreeSet::from([export.to_owned()])),
            )]),
        };
        let required = |export| {
            manifest
                .platform_inputs
                .permissions_for(&bundle(export), &BTreeSet::new(), &retained)
        };
        assert!(required(Some("ping")).is_empty());
        assert!(required(Some("open")).contains(permission));
        assert!(required(Some("Camera")).contains(permission));
        assert!(required(None).contains(permission));

        let installed = serde_json::to_vec(&InstalledContract {
            schema_version: 1,
            script_abi: SCRIPT_ABI,
            platform: "android".to_owned(),
            modules: vec![InstalledModule {
                specifier: specifier.to_owned(),
                export_abi: manifest.modules[0].export_abi.clone(),
            }],
            android_permissions: BTreeSet::new(),
        })?;
        manifest.validate_update(&installed, &bundle(Some("ping")))?;
        let error = manifest.validate_update(&installed, &bundle(Some("open")));
        assert!(error.is_err_and(|error| error.to_string().contains(permission)));
        Ok(())
    }
}
