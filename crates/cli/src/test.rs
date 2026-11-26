use crate::{app::App, plugins, toolchain};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

pub fn test(
    bundle: Option<PathBuf>,
    framework_suite: bool,
    platform: Option<&str>,
    trace: bool,
    gpu: bool,
    update: bool,
) -> Result<()> {
    if framework_suite {
        let root = toolchain::framework()?;
        let mut entries = Vec::new();
        collect(&root.join("packages"), &mut entries)?;
        entries.sort();
        return run_sources(&root, &root, None, trace, None, entries);
    }
    let app = App::current()?;
    let snapshots = gpu.then(|| testing::SnapshotOptions {
        root: app.root.join("tests/__snapshots__"),
        update,
    });
    if gpu && !cfg!(feature = "headless-gpu") {
        bail!("impasto test --gpu 需要启用 headless-gpu 特性");
    }
    if let Some(bundle) = bundle {
        if bundle
            .extension()
            .is_some_and(|extension| extension == "bin")
        {
            return run_bytecode(&bundle);
        }
        if bundle
            .extension()
            .is_some_and(|extension| extension == "js")
        {
            let code = fs::read_to_string(&bundle)?;
            let sandbox = tempfile::tempdir()?;
            let report = testing::run(
                &code,
                &bundle.to_string_lossy(),
                modules(sandbox.path())?,
                snapshots,
            )?;
            print_report(&bundle, &report, trace);
            return Ok(());
        }
        return test_sources(&app, platform, trace, snapshots.as_ref(), vec![bundle]);
    }
    let mut entries = Vec::new();
    collect(&app.root.join("src"), &mut entries)?;
    collect(&app.root.join("tests"), &mut entries)?;
    if entries.is_empty() {
        entries.push(app.entry.clone());
    }
    entries.sort();
    test_sources(&app, platform, trace, snapshots.as_ref(), entries)
}

fn test_sources(
    app: &App,
    platform: Option<&str>,
    trace: bool,
    snapshots: Option<&testing::SnapshotOptions>,
    entries: Vec<PathBuf>,
) -> Result<()> {
    let framework = toolchain::framework()?;
    let target = platform
        .or_else(|| app.targets.first().map(String::as_str))
        .context("no test target")?;
    app.require(target)?;
    let native = plugins::prepare(app, target, false)?;
    run_sources(
        &app.root,
        &framework,
        Some(&native),
        trace,
        snapshots,
        entries,
    )
}

fn run_sources(
    root: &Path,
    framework: &Path,
    native: Option<&plugins::NativeBuild>,
    trace: bool,
    snapshots: Option<&testing::SnapshotOptions>,
    entries: Vec<PathBuf>,
) -> Result<()> {
    let runner = native
        .map(|native| native.test_runner(framework, snapshots.is_some()))
        .transpose()?
        .flatten();
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallel| parallel.get())
        .min(4)
        .min(entries.len());
    let next = AtomicUsize::new(0);
    let results = std::thread::scope(|scope| {
        let workers = (0..workers)
            .map(|_| {
                let entries = &entries;
                let runner = runner.as_deref();
                let next = &next;
                scope.spawn(move || {
                    let mut reports = Vec::new();
                    while let Some(entry) = entries.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let test = || {
                            let sandbox = tempfile::tempdir()?;
                            let entry = root.join(entry);
                            let modules = if runner.is_some() {
                                Vec::new()
                            } else {
                                modules(sandbox.path())?
                            };
                            let mut options = bundler::BundleOptions {
                                root: root.to_path_buf(),
                                entry: entry.clone(),
                                framework: framework.to_path_buf(),
                                plugins: native.map(plugins::NativeBuild::linker_manifest),
                                externals: modules
                                    .iter()
                                    .map(|module| module.specifier.to_owned())
                                    .collect(),
                                ..Default::default()
                            };
                            testing::install(&mut options);
                            options.externals.insert("impasto:test/fixtures".to_owned());
                            let code = bundler::bundle(options)?.code;
                            let snapshots = snapshots.cloned().map(|mut options| {
                                if let Ok(relative) = entry.strip_prefix(root) {
                                    options.root.push(relative);
                                } else if let Some(file) = entry.file_name() {
                                    options.root.push(file);
                                }
                                options
                            });
                            if let Some(runner) = runner {
                                let script = sandbox.path().join("test.js");
                                fs::write(&script, code)?;
                                let mut command = Command::new(runner);
                                command.arg(root).arg(&entry).arg(script).arg(if trace {
                                    "trace"
                                } else {
                                    "plain"
                                });
                                if let Some(snapshot) = snapshots {
                                    command.arg(snapshot.root).arg(if snapshot.update {
                                        "update"
                                    } else {
                                        "compare"
                                    });
                                }
                                let output =
                                    command.output().context("运行应用 Rust 无头测试宿主")?;
                                if !output.status.success() {
                                    bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
                                }
                                return Ok(String::from_utf8(output.stdout)?.trim_end().to_owned());
                            }
                            let mut modules = modules;
                            modules.push(testing::fixture_module(root, &entry)?);
                            let report =
                                testing::run(&code, &entry.to_string_lossy(), modules, snapshots)?;
                            Ok(testing::format_report(&entry, &report, trace))
                        };
                        reports.push((entry, test()));
                    }
                    reports
                })
            })
            .collect::<Vec<_>>();
        let mut results = Vec::new();
        for worker in workers {
            results.extend(
                worker
                    .join()
                    .map_err(|_| anyhow::anyhow!("test worker panicked"))?,
            );
        }
        Ok::<_, anyhow::Error>(results)
    })?;
    let mut results = results;
    results.sort_by_key(|(entry, _)| *entry);
    let mut failures = Vec::new();
    for (entry, result) in results {
        match result {
            Ok(report) => println!("{report}"),
            Err(error) => failures.push(format!("{}: {error:#}", entry.display())),
        }
    }
    if !failures.is_empty() {
        bail!("{}", failures.join("\n"));
    }
    println!("{} 个无头测试入口通过", entries.len());
    Ok(())
}

fn modules(root: &Path) -> Result<Vec<engine::session::Module>> {
    let mut registrar = impasto::PluginRegistrar::default();
    impasto_bundled_plugins::register(&mut registrar);
    let context = impasto::PluginContext::new(
        impasto::Runtime::default(),
        impasto::AppPaths::new(Some(root.to_path_buf()), None, None),
        |_| false,
    );
    registrar
        .build(&context)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn print_report(path: &Path, report: &testing::Report, trace: bool) {
    println!("{}", testing::format_report(path, report, trace));
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && !["build", "node_modules", "target", ".impasto"]
                .iter()
                .any(|name| entry.file_name() == *name)
        {
            collect(&entry.path(), out)?;
        } else if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".test.ts") || name.ends_with(".test.tsx"))
        {
            out.push(entry.path());
        }
    }
    Ok(())
}
#[expect(
    unsafe_code,
    reason = "loads a trusted local build artifact selected by the caller"
)]
fn run_bytecode(bundle: &Path) -> Result<()> {
    let bytes = fs::read(bundle)?;
    let sandbox = tempfile::tempdir()?;
    let config = engine::session::ModuleConfig {
        registry: engine::session::ModuleRegistry::new(modules(sandbox.path())?).into(),
        ..engine::session::ModuleConfig::disconnected()
    };
    let name = bundle.to_string_lossy();
    let log: engine::js::ConsoleLog = Box::new(|level, line| {
        if level >= 3 {
            eprintln!("{line}");
        }
    });
    // SAFETY: the caller supplies an unmodified locally compiled IQJS file.
    let mut session = unsafe {
        engine::js::open_trusted_bytecode(
            &bytes,
            &name,
            engine::text::MonoShaper::default(),
            log,
            config,
        )
    }?;
    session.metrics(&engine::session::Metrics::new(360.0, 640.0, 2.0));
    for step in 0..4 {
        session.frame(engine::session::FrameTime::at(f64::from(step) * 16.0));
    }
    if let Some(error) = session
        .take_script_error()
        .filter(|error| error != &engine::Error::NoRoot.to_string())
    {
        bail!("{error}");
    }
    if let Some(fault) = session
        .take_faults()
        .iter()
        .find(|fault| fault.message != engine::Error::NoRoot.to_string())
    {
        bail!("{}: {}", fault.source, fault.message);
    }
    Ok(())
}

#[cfg(test)]
mod hot_gallery;
