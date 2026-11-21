use crate::{
    app::App,
    build::{self, BuildOptions},
};
use anyhow::{Result, bail};
use std::{fs, path::Path};
pub fn test(bundle: Option<std::path::PathBuf>) -> Result<()> {
    let app = App::current()?;
    if let Some(bundle) = bundle {
        return run_bundle(&bundle, &app);
    }
    let mut entries = Vec::new();
    collect(
        &app.root.join(if app.gallery { "src" } else { "lib" }),
        &mut entries,
    )?;
    let output = app.root.join("build/tests");
    fs::create_dir_all(&output)?;
    if entries.is_empty() {
        let bundle = build::bundle(
            &app,
            &BuildOptions {
                debug: true,
                verbose: false,
                trace: false,
            },
        )?;
        return run_bundle(&bundle, &app);
    }
    for (index, entry) in entries.iter().enumerate() {
        let bundle = output.join(format!("{index}.js"));
        build::pack(
            entry,
            &bundle,
            &BuildOptions {
                debug: true,
                verbose: false,
                trace: false,
            },
        )?;
        run_bundle(&bundle, &app)?;
    }
    println!("{} 个无头测试入口通过", entries.len());
    Ok(())
}
fn collect(path: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect(&entry.path(), out)?;
        } else if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".test.ts") || name.ends_with(".test.tsx"))
        {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(())
}
#[expect(
    unsafe_code,
    reason = "loads a trusted local build artifact selected by the caller"
)]
fn run_bundle(bundle: &Path, app: &App) -> Result<()> {
    let bytes = fs::read(bundle)?;
    let modules = app
        .modules("macos")
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let config = session::ModuleConfig {
        registry: std::sync::Arc::new(
            session::ModuleRegistry::new(embedder::first_party_modules()),
        ),
        ..session::ModuleConfig::disconnected()
    }
    .restrict(&modules);
    let name = bundle.to_string_lossy();
    let log: javascript::ui::ConsoleLog = Box::new(|level, line| {
        if level >= 3 {
            eprintln!("{line}");
        }
    });
    let mut session = if bundle.extension().is_some_and(|ext| ext == "bin") {
        // SAFETY: `impasto test` loads a path the user passed. A `.bin` file is
        // treated as an envelope from this toolchain's `compile` (`impasto build`).
        // The envelope does not prove the payload is valid bytecode.
        unsafe {
            javascript::ui::open_trusted_bytecode_with_routed_io(
                &bytes,
                &name,
                uicore::MonoShaper::default(),
                log,
                None,
                config,
            )
        }
    } else {
        javascript::ui::open_program_with_routed_io(
            &bytes,
            &name,
            uicore::MonoShaper::default(),
            log,
            None,
            config,
        )
    }?;
    session.metrics(&session::Metrics::new(360.0, 640.0, 2.0));
    for step in 0..4 {
        session.frame(session::FrameTime::at(f64::from(step) * 16.0));
    }
    if let Some(error) = session
        .take_script_error()
        .filter(|error| error != &uicore::Error::NoRoot.to_string())
    {
        bail!("{error}");
    }
    if let Some(fault) = session
        .take_faults()
        .iter()
        .find(|fault| fault.message != uicore::Error::NoRoot.to_string())
    {
        bail!("{}: {}", fault.source, fault.message);
    }
    Ok(())
}
