use crate::{app::App, plugins, toolchain};
use anyhow::Result;

pub fn analyze(platform: Option<&str>) -> Result<()> {
    let app = App::current()?;
    let platform = plugins::standalone(&app, platform)?;
    let native = platform
        .map(|target| plugins::prepare(&app, target, false))
        .transpose()?
        .flatten();
    let mut command = toolchain::tool("analyze.mjs", false)?;
    command.arg(&app.root);
    if let Some(native) = &native {
        command.arg(&native.manifest);
    }
    toolchain::run(command, "TypeScript 检查")
}
