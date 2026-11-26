use crate::{app::App, plugins, toolchain};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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

pub fn analyze(platform: Option<&str>, tsc: Option<&Path>) -> Result<()> {
    let app = App::current()?;
    let platform =
        plugins::standalone(&app, platform)?.or_else(|| app.targets.first().map(String::as_str));
    let native = platform
        .map(|target| plugins::prepare(&app, target, false))
        .transpose()?;
    let tsc = tsc.map_or_else(crate::typescript::executable, |path| {
        Ok(PathBuf::from(path))
    })?;
    let mut command = Command::new(tsc);
    let config = app.root.join("tsconfig.json");
    let config = if config.is_file() {
        config
    } else {
        app.root.join(".impasto/tsconfig.generated.json")
    };
    command.arg("--project").arg(config).arg("--noEmit");
    toolchain::run(command, "TypeScript 检查")?;
    crate::build::link_source(
        &app.root,
        &app.entry,
        &crate::build::BuildOptions {
            debug: true,
            hot: None,
            verbose: false,
            trace: false,
        },
        native.as_ref().map(plugins::NativeBuild::linker_manifest),
        None,
    )?;
    Ok(())
}
