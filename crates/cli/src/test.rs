use crate::{app::App, plugins, toolchain};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) struct Options {
    pub trace: bool,
    pub gpu: bool,
    pub update: bool,
    pub watch: bool,
    pub filter: Option<String>,
}

pub fn test(
    bundle: Option<PathBuf>,
    framework_suite: bool,
    platform: Option<&str>,
    options: Options,
) -> Result<()> {
    if framework_suite {
        let root = toolchain::framework()?;
        return run_project(&root, &root, None, &options, None, None);
    }
    let root = std::env::current_dir()?;
    let app = if !root.join("app.toml").is_file() && root.join("plugin.toml").is_file() {
        App::for_plugin(root)?
    } else {
        App::current()?
    };
    let snapshots = options.gpu.then(|| testing::SnapshotOptions {
        root: app.root.join("tests/__snapshots__"),
        update: options.update,
    });
    if options.gpu && !cfg!(feature = "headless-gpu") {
        bail!("impasto test --gpu 需要启用 headless-gpu 特性");
    }
    if let Some(bundle) = bundle.as_ref() {
        if bundle
            .extension()
            .is_some_and(|extension| extension == "bin")
        {
            if options.watch || options.filter.is_some() {
                bail!("--watch 与 --filter 需要测试源码");
            }
            return run_bytecode(bundle);
        }
        if bundle
            .extension()
            .is_some_and(|extension| extension == "js")
        {
            if options.watch {
                bail!("--watch 需要测试源码入口");
            }
            let code = fs::read_to_string(bundle)?;
            let sandbox = tempfile::tempdir()?;
            let report = testing::run_filtered(
                &code,
                &bundle.to_string_lossy(),
                modules(sandbox.path())?,
                snapshots,
                Some(app.assets().to_path_buf()),
                options.filter,
            )?;
            print_report(bundle, &report, options.trace);
            return Ok(());
        }
    }
    let framework = toolchain::framework()?;
    let target = platform
        .or_else(|| app.targets.first().map(String::as_str))
        .context("no test target")?;
    app.require(target)?;
    let native = plugins::prepare(&app, target, false)?;
    run_project(
        &app.root,
        &framework,
        Some((&app, &native)),
        &options,
        snapshots.as_ref(),
        bundle,
    )
}

fn entries(root: &Path, app: Option<&App>, single: Option<&Path>) -> Result<Vec<PathBuf>> {
    if let Some(entry) = single {
        return Ok(vec![root.join(entry)]);
    }
    let mut entries = Vec::new();
    if let Some(app) = app {
        collect(&root.join("src"), &mut entries)?;
        collect(&root.join("tests"), &mut entries)?;
        if entries.is_empty() {
            entries.push(app.entry.clone());
        }
    } else {
        collect(&root.join("packages"), &mut entries)?;
    }
    entries.sort();
    Ok(entries)
}

type Inputs = BTreeMap<PathBuf, BTreeSet<PathBuf>>;
type TestLinkers = Mutex<BTreeMap<PathBuf, bundler::WatchLinker>>;

fn run_project(
    root: &Path,
    framework: &Path,
    native: Option<(&App, &plugins::NativeBuild)>,
    options: &Options,
    snapshots: Option<&testing::SnapshotOptions>,
    single: Option<PathBuf>,
) -> Result<()> {
    let app = native.map(|(app, _)| app);
    let native = native.map(|(_, native)| native);
    let runner = native
        .map(|native| native.test_runner(framework, snapshots.is_some()))
        .transpose()?
        .flatten();
    let compiler_cache = bundler::ModuleCache::default();
    let linkers = options.watch.then(TestLinkers::default);
    let mut changes = Vec::new();
    let mut inputs = Inputs::new();
    let mut known = entries(root, app, single.as_deref())?;
    let mut selected = known.clone();
    let scope = |inputs: &Inputs| {
        let mut scope = crate::watch::Scope::new(
            root,
            if app.is_some() {
                &["src", "tests", "assets", "locales"]
            } else {
                &["src", "tests", "assets", "locales", "packages"]
            },
            &["tsconfig.json", "app.toml"],
        );
        scope.extend(inputs.values().flatten().chain(single.iter()));
        scope
    };
    let mut watch = if options.watch {
        let (events, rx) = crate::dev::events()?;
        Some((crate::watch::ProjectWatch::new(scope(&inputs), events)?, rx))
    } else {
        None
    };
    loop {
        if let Err(error) = run_sources(
            &TestBatch {
                root,
                framework,
                native,
                runner: runner.as_deref(),
                options,
                snapshots,
                compiler_cache: &compiler_cache,
                linkers: linkers.as_ref(),
                changes: &changes,
            },
            &selected,
            &mut inputs,
        ) {
            if !options.watch {
                return Err(error);
            }
            eprintln!("{error:#}");
        }
        let Some((watcher, events)) = &mut watch else {
            return Ok(());
        };
        watcher.set_scope(scope(&inputs))?;

        selected.clear();
        while selected.is_empty() {
            let changed = match events.recv()? {
                crate::dev::Event::Files(batch) => watcher.reconcile(batch)?,
                crate::dev::Event::Warning(message) => {
                    eprintln!("impasto: {message}");
                    continue;
                }
                crate::dev::Event::Shutdown => return Ok(()),
                _ => continue,
            };
            if changed.is_empty() {
                continue;
            }
            let rescan_all = changed.iter().any(|path| {
                path.starts_with(root.join("locales"))
                    || path.file_name().is_some_and(|name| {
                        matches!(name.to_str(), Some("app.toml" | "tsconfig.json"))
                    })
            });
            let all = entries(root, app, single.as_deref())?;
            inputs.retain(|entry, _| all.contains(entry));
            selected.extend(
                all.iter()
                    .filter(|entry| {
                        rescan_all
                            || !known.contains(entry)
                            || changed.contains(entry)
                            || inputs
                                .get(*entry)
                                .is_none_or(|deps| changed.iter().any(|path| deps.contains(path)))
                    })
                    .cloned(),
            );
            known = all;
            changes = changed;
        }
    }
}

struct TestBatch<'a> {
    root: &'a Path,
    framework: &'a Path,
    native: Option<&'a plugins::NativeBuild>,
    runner: Option<&'a Path>,
    options: &'a Options,
    snapshots: Option<&'a testing::SnapshotOptions>,
    compiler_cache: &'a bundler::ModuleCache,
    linkers: Option<&'a TestLinkers>,
    changes: &'a [PathBuf],
}

fn run_sources(batch: &TestBatch<'_>, entries: &[PathBuf], inputs: &mut Inputs) -> Result<()> {
    let TestBatch {
        root,
        framework,
        native,
        runner,
        options,
        snapshots,
        compiler_cache,
        linkers,
        changes,
    } = *batch;
    let trace = options.trace;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallel| parallel.get())
        .min(4)
        .min(entries.len());
    let next = AtomicUsize::new(0);
    let results = std::thread::scope(|scope| {
        let workers = (0..workers)
            .map(|_| {
                let next = &next;
                scope.spawn(move || {
                    let mut reports = Vec::new();
                    while let Some(entry) = entries.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let mut dependencies = None;
                        let mut test = || {
                            let sandbox = tempfile::tempdir()?;
                            let entry = root.join(entry);
                            let modules = if runner.is_some() {
                                Vec::new()
                            } else {
                                modules(sandbox.path())?
                            };
                            let mut link_options = bundler::BundleOptions {
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
                            testing::install(&mut link_options);
                            link_options
                                .externals
                                .insert("impasto:test/fixtures".to_owned());
                            let linked = if let Some(linkers) = linkers {
                                let mut linkers = linkers
                                    .lock()
                                    .map_err(|_| anyhow::anyhow!("test linker unavailable"))?;
                                let linker = linkers.entry(entry.clone()).or_default();
                                linker.set_changes(Some(changes.to_vec()));
                                linker.bundle_with_cache(link_options, Some(compiler_cache))?
                            } else {
                                bundler::bundle_with_cache(link_options, Some(compiler_cache))?
                            };
                            dependencies = Some(
                                linked
                                    .inputs
                                    .into_iter()
                                    .filter(|path| path.is_absolute())
                                    .collect(),
                            );
                            let code = linked.code;
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
                                command.env_remove("IMPASTO_TEST_FILTER");
                                if let Some(filter) = &options.filter {
                                    command.env("IMPASTO_TEST_FILTER", filter);
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
                            let report = testing::run_filtered(
                                &code,
                                &entry.to_string_lossy(),
                                modules,
                                snapshots,
                                Some(root.join("assets")),
                                options.filter.clone(),
                            )?;
                            Ok(testing::format_report(&entry, &report, trace))
                        };
                        let report = test();
                        reports.push((entry, dependencies, report));
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
    results.sort_by_key(|(entry, _, _)| *entry);
    let mut failures = Vec::new();
    for (entry, dependencies, result) in results {
        if let Some(dependencies) = dependencies {
            inputs.insert(entry.clone(), dependencies);
        } else {
            inputs.remove(entry);
        }
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
        impasto::AppPaths::isolated(root)?,
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
