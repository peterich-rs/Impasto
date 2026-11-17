//! `impasto packages get`. Forwards to `pnpm install`. This is not a package
//! manager of its own.

use std::process::Command;

pub fn get(args: &[String]) -> anyhow::Result<()> {
    let status = Command::new("pnpm")
        .arg("install")
        .args(args)
        .status()
        .map_err(|error| anyhow::anyhow!("pnpm: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("pnpm install failed ({status})"))
    }
}
