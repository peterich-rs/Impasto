//! Dependency discovery, topological ordering, and target resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{
    Dependency, Discovery, JavaScriptPackage, Local, Plugin, Resolved, cargo_package, under,
};
use crate::{app::App, toolchain};

pub(crate) fn discover(root: &Path, refresh: Option<Option<&str>>) -> Result<Discovery> {
    let framework = toolchain::framework()?;
    let root = root.canonicalize()?;
    let mut resolver = crate::dependencies::Resolver::new(&root, &framework, refresh)?;
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
        let explicit = resolver.dependencies(&owner)?;
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
    ordered_plugins(&found)?;
    Ok(Discovery {
        plugins: found,
        sources: resolver.finish()?,
    })
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

pub(super) fn resolve(app: &App, target: &str) -> Result<Resolved> {
    let mut result = Resolved::default();
    resolve_source_entries(app, &mut result.entries)?;
    let ordered = ordered_plugins(&app.plugins)?;
    let required = dependency_features(app, target, &ordered)?;
    for alias in ordered {
        resolve_plugin(app, target, alias, required.get(alias), &mut result)?;
    }
    result.platform.resolve_permission_attributes()?;
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
    validate_platform(app, target, &result.platform)?;
    Ok(result)
}

fn resolve_source_entries(app: &App, entries: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    let framework = toolchain::framework()?;
    for (name, root) in &app.sources {
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
            entries.insert(specifier, under(root, Path::new(&entry))?);
        }
    }
    Ok(())
}

fn dependency_features(
    app: &App,
    target: &str,
    ordered: &[&str],
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut required = BTreeMap::<String, BTreeSet<String>>::new();
    for &alias in ordered.iter().rev() {
        let dependency = &app.plugins[alias];
        let manifest = app.root.join(&dependency.path).join("plugin.toml");
        let plugin: Plugin = toml::from_str(&fs::read_to_string(&manifest)?)
            .with_context(|| format!("解析 {}", manifest.display()))?;
        if !plugin.plugin.platforms.iter().any(|name| name == target) {
            continue;
        }
        let project = match target {
            "ios" => &plugin.ios,
            "android" => &plugin.android,
            "macos" => &plugin.macos,
            _ => bail!("未知插件目标 {target}"),
        };
        let mut features = project
            .select_features(target, app, app.plugin_options.get(alias))?
            .unwrap_or_default();
        if let Some(extra) = required.get(alias) {
            features.extend(extra.iter().cloned());
        }
        let dependencies = project.dependency_features.iter().chain(
            features
                .iter()
                .filter_map(|name| project.features.get(name))
                .flat_map(|feature| &feature.dependency_features),
        );
        for (name, features) in dependencies {
            if !dependency.dependencies.contains(name) {
                bail!("插件 {alias} 请求了未声明依赖 {name} 的特性");
            }
            required
                .entry(name.clone())
                .or_default()
                .extend(features.iter().cloned());
        }
    }
    Ok(required)
}

fn resolve_plugin(
    app: &App,
    target: &str,
    alias: &str,
    required: Option<&BTreeSet<String>>,
    result: &mut Resolved,
) -> Result<()> {
    let dependency = &app.plugins[alias];
    let root = app
        .root
        .join(&dependency.path)
        .canonicalize()
        .with_context(|| format!("插件 {alias} 路径不存在"))?;
    let package: JavaScriptPackage = serde_json::from_slice(&fs::read(root.join("package.json"))?)?;
    let specifier = package.name.context("插件 package.json 缺少 name")?;
    if !valid_package_name(&specifier) || !impasto::abi::valid_specifier(&specifier) {
        bail!("无效的插件包名 {specifier:?}");
    }
    let plugin: Plugin = toml::from_str(&fs::read_to_string(root.join("plugin.toml"))?)
        .with_context(|| format!("解析插件 {alias} 的 plugin.toml"))?;
    let options = app.plugin_options.get(alias);
    if let Some(features) = options.and_then(|options| options.enabled.as_ref()) {
        for feature in features {
            if !plugin.rust.optional_features.contains_key(feature)
                && ![&plugin.ios, &plugin.macos, &plugin.android]
                    .iter()
                    .any(|project| project.features.contains_key(feature))
            {
                bail!("插件 {alias} 没有特性 {feature}");
            }
        }
    }
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
        // Hosts resolve the canonical /native binding through the Rust descriptors.
        // Package exports may provide a fallback for tooling outside Impasto.
        if key == "./native" {
            continue;
        }
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
        return Ok(());
    }
    let mut features = plugin.rust.features;
    if let Some(enabled) = options.and_then(|options| options.enabled.as_ref()) {
        for feature in enabled {
            if let Some(cargo) = plugin.rust.optional_features.get(feature) {
                features.extend(cargo.iter().cloned());
            }
        }
    }
    if !features.is_empty() {
        result.rust_features.insert(cargo_name.clone(), features);
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
    let mut selected = platform.select_features(target, app, options)?;
    if let Some(required) = required {
        for feature in required {
            if !platform.features.contains_key(feature) {
                bail!("依赖 {cargo_name} 没有 {target} 特性 {feature}");
            }
        }
        selected
            .get_or_insert_with(BTreeSet::new)
            .extend(required.iter().cloned());
    }
    result
        .platform
        .add_project(platform, &root, target, &cargo_name, options, selected)
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
    Ok(())
}

fn validate_platform(app: &App, target: &str, platform: &super::Platform) -> Result<()> {
    let usage = match target {
        "ios" => Some(&app.ios.usage_descriptions),
        "macos" => Some(&app.macos.usage_descriptions),
        _ => None,
    };
    if let Some(usage) = usage {
        for key in &platform.usage_description_keys {
            if !usage.contains_key(key) {
                bail!("缺少 [{target}.usage_descriptions].{key}");
            }
        }
    }
    let entitlements = match target {
        "ios" => Some(&app.ios.entitlements),
        "macos" => Some(&app.macos.entitlements),
        _ => None,
    };
    if let Some(entitlements) = entitlements {
        for key in &platform.required_entitlements {
            if entitlements.get(key) != Some(&true) {
                bail!("插件需要 [{target}.entitlements] 中启用 {key}");
            }
        }
    }
    Ok(())
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

pub(super) fn validate_specifier(specifier: &str) -> Result<()> {
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
