use crate::{app::App, toolchain};
use anyhow::Result;
pub fn analyze() -> Result<()> {
    let app = App::current()?;
    let mut command = toolchain::tool("analyze.mjs", false)?;
    command.arg(&app.root);
    toolchain::run(command, "TypeScript 检查")
}
