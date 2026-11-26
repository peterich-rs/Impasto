// The CLI builds this executable on demand for application-local Rust plugins.

use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("missing project root")?);
    let entry = PathBuf::from(args.next().ok_or("missing test entry")?);
    let script = PathBuf::from(args.next().ok_or("missing test bundle")?);
    let trace = args.next().is_some_and(|value| value == "trace");
    let snapshots = args.next().map(|path| testing::SnapshotOptions {
        root: PathBuf::from(path),
        update: args.next().is_some_and(|value| value == "update"),
    });
    let sandbox = tempfile::tempdir()?;
    let mut registrar = impasto::PluginRegistrar::default();
    registry::register(&mut registrar);
    let context = impasto::PluginContext::new(
        impasto::Runtime::default(),
        impasto::AppPaths::new(Some(sandbox.path().to_path_buf()), None, None),
        |_| false,
    );
    let mut modules = registrar.build(&context)?;
    modules.push(testing::fixture_module(&root, &entry)?);
    let report = testing::run_filtered(
        &std::fs::read_to_string(script)?, &entry.to_string_lossy(), modules, snapshots,
        Some(root.join("assets")),
        env::var("IMPASTO_TEST_FILTER").ok(),
    )?;
    println!("{}", testing::format_report(&entry, &report, trace));
    Ok(())
}
