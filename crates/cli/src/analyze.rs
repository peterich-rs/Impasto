use crate::{app::App, plugins, toolchain};
use anyhow::{Context as _, Result, bail};
use std::{fs, path::Path, process::Command};

pub(crate) fn standard(tsc: Option<&Path>) -> Result<()> {
    let tsc = tsc.map_or_else(crate::typescript::executable, |path| Ok(path.to_path_buf()))?;
    let sandbox = tempfile::tempdir()?;
    let declarations = sandbox.path().join("standard.d.ts");
    let fixture = sandbox.path().join("standard.ts");
    fs::write(
        &declarations,
        javascript::standard::StandardProfile::new().declarations(),
    )?;
    fs::write(
        &fixture,
        concat!(
            include_str!("../../javascript/tests/fixtures/standard.ts"),
            "\nfunction checkHostPackages() {\n",
            "  // @ts-expect-error AbortSignal uses the controller or its factory.\n",
            "  new AbortSignal();\n",
            "  // @ts-expect-error fetch lives in @impasto/http.\n",
            "  fetch('https://example.test/');\n",
            "  // @ts-expect-error WebSocket lives in @impasto/websocket.\n",
            "  new WebSocket('wss://example.test/');\n",
            "}\n",
        ),
    )?;
    let mut command = Command::new(tsc);
    command
        .args([
            "--strict", "--noEmit", "--target", "es2022", "--lib", "es2022",
        ])
        .args([declarations, fixture]);
    toolchain::run(command, "JavaScript 标准库 TypeScript 检查")
}

pub fn analyze(
    platform: Option<&str>,
    tsc: Option<&Path>,
    all_targets: bool,
    deny_warnings: bool,
) -> Result<()> {
    let root = std::env::current_dir()?;
    let plugin = !root.join("app.toml").is_file() && root.join("plugin.toml").is_file();
    let app = if plugin {
        App::for_plugin(root)?
    } else {
        App::current()?
    };
    let targets = if all_targets {
        app.targets.iter().map(String::as_str).collect::<Vec<_>>()
    } else {
        vec![
            platform
                .or_else(|| app.targets.first().map(String::as_str))
                .context("没有分析目标")?,
        ]
    };
    let tsc = tsc.map_or_else(crate::typescript::executable, |path| Ok(path.to_path_buf()))?;
    let framework = toolchain::framework()?;
    let compiler_cache = bundler::ModuleCache::default();
    let config = app.root.join("tsconfig.json");
    let config = if config.is_file() {
        config
    } else if plugin {
        app.root.join(".impasto/tsconfig.plugin.json")
    } else {
        app.root.join(".impasto/tsconfig.generated.json")
    };
    let entry = if plugin {
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(app.root.join("package.json"))?)?;
        app.root
            .join(package["exports"]["."].as_str().context("插件缺少主导出")?)
    } else {
        app.entry.clone()
    };
    let mut errors = Vec::new();
    let mut checked_rust = false;
    for target in targets {
        let native = match app
            .require(target)
            .and_then(|()| plugins::prepare(&app, target, false))
        {
            Ok(native) => native,
            Err(error) => {
                errors.push(format!("{target}: {error:#}"));
                continue;
            }
        };
        if !checked_rust && let Some(native) = &app.native {
            let mut cargo = Command::new("cargo");
            cargo
                .current_dir(&app.root)
                .args(["check", "--all-targets", "--manifest-path"])
                .arg(app.root.join(&native.path).join("Cargo.toml"))
                .env("CARGO_TARGET_DIR", app.root.join("build/analysis/target"));
            if let Err(error) = toolchain::run(cargo, "Rust 检查") {
                errors.push(format!("Rust: {error:#}"));
            }
            checked_rust = true;
        }
        let mut command = Command::new(&tsc);
        command
            .current_dir(&app.root)
            .arg("--project")
            .arg(&config)
            .arg("--noEmit");
        if let Err(error) = toolchain::run(command, "TypeScript 检查") {
            errors.push(format!("{target}: {error:#}"));
        }
        match bundler::bundle_with_cache(
            bundler::BundleOptions {
                root: app.root.clone(),
                entry: entry.clone(),
                framework: framework.clone(),
                plugins: Some(native.linker_manifest()),
                ..Default::default()
            },
            Some(&compiler_cache),
        ) {
            Ok(linked) => {
                let warnings = linked
                    .warnings
                    .iter()
                    .filter(|warning| {
                        let Some(source) = warning.lines().next().map(Path::new) else {
                            return true;
                        };
                        !source.starts_with(&framework) || source.starts_with(&app.root)
                    })
                    .collect::<Vec<_>>();
                for warning in &warnings {
                    eprintln!("{target}: {warning}");
                }
                if deny_warnings && !warnings.is_empty() {
                    errors.push(format!("{target}: {} 条编译警告", warnings.len()));
                }
            }
            Err(error) => errors.push(format!("{target}: {error:#}")),
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(())
}
