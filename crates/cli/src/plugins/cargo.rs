//! Validation of generated plugin Cargo dependency graphs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::Resolved;

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

pub(super) fn local_dependencies(
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
