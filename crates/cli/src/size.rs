//! Bundle size accounting. Payload bytes are measured per chunk.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context as _, Result};
use serde::Serialize;

pub(crate) struct Request<'a> {
    pub summary: bool,
    pub path: Option<&'a Path>,
}

impl Request<'_> {
    pub(crate) fn enabled(&self) -> bool {
        self.summary || self.path.is_some()
    }
}

pub(crate) struct Output<'a> {
    pub request: Request<'a>,
    pub app: &'a Path,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Chunk<'a> {
    rendered_javascript_bytes: usize,
    bytecode_payload_bytes: Option<usize>,
    unattributed_bytecode_bytes: Option<usize>,
    startup: bool,
    modules: Vec<Module<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Module<'a> {
    path: &'a str,
    group: &'static str,
    rendered_bytes: usize,
    apportioned_bytecode_bytes: Option<usize>,
    retained_pool_bytes: usize,
    amortized_pool_bytes: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pool<'a> {
    rendered_bytes: usize,
    consumers: &'a std::collections::BTreeSet<String>,
    dedup_saved_bytes: usize,
}

fn chunk<'a>(
    name: &str,
    code: &str,
    modules: &'a BTreeMap<String, usize>,
    sizes: &BTreeMap<String, usize>,
    app: &Path,
    framework: &Path,
) -> Chunk<'a> {
    let sum = modules.values().copied().sum::<usize>();
    let mut used = 0usize;
    let payload = sizes.get(name).copied();
    let modules = modules
        .iter()
        .map(|(path, &size)| {
            let previous = used;
            used += size;
            let apportioned = payload.filter(|_| sum != 0).map(|bytes| {
                let portion = |weight| {
                    usize::try_from((bytes as u128) * (weight as u128) / (sum as u128))
                        .unwrap_or(bytes)
                };
                portion(used) - portion(previous)
            });
            let source = Path::new(path);
            let group = if path.starts_with("impasto:pool/") {
                "staticPool"
            } else if source.extension().is_some_and(|ext| {
                matches!(
                    ext.to_str(),
                    Some("svg" | "wgsl" | "png" | "webp" | "jpg" | "jpeg" | "ttf" | "otf")
                )
            }) {
                "compiledAsset"
            } else if source.starts_with(framework.join("packages")) {
                "framework"
            } else if source.starts_with(app) && !source.starts_with(app.join("node_modules")) {
                "application"
            } else {
                "dependency"
            };
            Module {
                path,
                group,
                rendered_bytes: size,
                apportioned_bytecode_bytes: apportioned,
                retained_pool_bytes: 0,
                amortized_pool_bytes: 0,
            }
        })
        .collect();
    Chunk {
        rendered_javascript_bytes: code.len(),
        bytecode_payload_bytes: payload,
        unattributed_bytecode_bytes: payload.map(|bytes| if sum == 0 { bytes } else { 0 }),
        startup: false,
        modules,
    }
}

pub(crate) fn write(
    output: Output<'_>,
    artifact: &Path,
    linked: &bundler::Bundle,
    sizes: &BTreeMap<String, usize>,
) -> Result<()> {
    let framework = crate::toolchain::framework()?;
    let Output { request, app } = output;
    let mut chunks = BTreeMap::from([(
        "app.js",
        chunk(
            "app.js",
            &linked.code,
            &linked.rendered_modules,
            sizes,
            app,
            &framework,
        ),
    )]);
    for (name, source) in &linked.chunks {
        chunks.insert(
            name.as_str(),
            chunk(
                name,
                &source.code,
                &source.rendered_modules,
                sizes,
                app,
                &framework,
            ),
        );
    }
    let mut pending = vec!["app.js"];
    let mut startup_bytes = 0usize;
    while let Some(name) = pending.pop() {
        let Some(chunk) = chunks.get_mut(name) else {
            continue;
        };
        if chunk.startup {
            continue;
        }
        chunk.startup = true;
        startup_bytes += chunk
            .bytecode_payload_bytes
            .unwrap_or(chunk.rendered_javascript_bytes);
        let imports = if name == "app.js" {
            &linked.entry_imports
        } else {
            &linked.chunks[name].imports
        };
        pending.extend(imports.iter().map(String::as_str));
    }
    let mut pools = BTreeMap::new();
    let mut retained = BTreeMap::<_, usize>::new();
    let mut amortized = BTreeMap::<_, usize>::new();
    for module in chunks
        .values()
        .flat_map(|chunk| &chunk.modules)
        .filter(|module| module.group == "staticPool")
    {
        let Some(consumers) = linked.pool_consumers.get(module.path) else {
            continue;
        };
        let size = module.rendered_bytes;
        let count = consumers.len();
        if count == 1
            && let Some(owner) = consumers.first()
        {
            *retained.entry(owner.as_str()).or_default() += size;
        }
        for (index, consumer) in consumers.iter().enumerate() {
            *amortized.entry(consumer.as_str()).or_default() +=
                size / count + usize::from(index < size % count);
        }
        pools.insert(
            module.path,
            Pool {
                rendered_bytes: size,
                consumers,
                dedup_saved_bytes: size * count.saturating_sub(1),
            },
        );
    }
    for module in chunks.values_mut().flat_map(|chunk| &mut chunk.modules) {
        module.retained_pool_bytes = retained.get(module.path).copied().unwrap_or(0);
        module.amortized_pool_bytes = amortized.get(module.path).copied().unwrap_or(0);
    }
    let mut assets = crate::watch::Snapshot::new();
    crate::watch::scan(&app.join("assets"), &mut assets)?;
    let assets = assets
        .iter()
        .filter_map(|(path, (_, size))| Some((path.strip_prefix(app).ok()?, size)))
        .collect::<BTreeMap<_, _>>();
    let artifact_bytes = fs::metadata(artifact)?.len();
    let file = request.path.map_or_else(
        || artifact.with_file_name("bundle-size.json"),
        Path::to_path_buf,
    );
    if let Some(parent) = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &file,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "artifactBytes": artifact_bytes,
            "startupBytes": startup_bytes,
            "chunks": chunks,
            "pools": pools,
            "externalAssets": assets,
        }))?,
    )?;
    if request.summary {
        println!(
            "Bundle {} · {} bytes",
            artifact
                .file_name()
                .context("artifact filename")?
                .to_string_lossy(),
            artifact_bytes
        );
        println!("{:>10} {:>10}  Chunk", "ESM", "IQJS");
        for (name, chunk) in &chunks {
            println!(
                "{:>10} {:>10}  {name}",
                chunk.rendered_javascript_bytes,
                chunk
                    .bytecode_payload_bytes
                    .map_or_else(|| "–".to_owned(), |bytes| bytes.to_string())
            );
        }
        let mut modules = chunks
            .values()
            .flat_map(|chunk| &chunk.modules)
            .collect::<Vec<_>>();
        modules.sort_by_key(|module| std::cmp::Reverse(module.rendered_bytes));
        println!(
            "{:>10} {:>10} {:>10}  Module",
            "Rendered", "Retained", "Amortized"
        );
        for module in modules.into_iter().take(20) {
            println!(
                "{:>10} {:>10} {:>10}  {}",
                module.rendered_bytes,
                module.retained_pool_bytes,
                module.amortized_pool_bytes,
                module.path
            );
        }
    }
    println!("{}", file.display());
    Ok(())
}
