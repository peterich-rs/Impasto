//! Locate the framework sources and run platform build tools.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn distribution() -> Option<PathBuf> {
    let root = std::env::current_exe()
        .ok()?
        .parent()?
        .join("impasto-tools");
    root.is_dir().then_some(root)
}
pub fn framework() -> Result<PathBuf> {
    if let Some(root) = distribution()
        .map(|p| p.join("framework"))
        .filter(|p| p.join("Cargo.toml").is_file())
    {
        return Ok(root);
    }
    crate::framework_root().context("安装包缺少 framework；请重新生成 CLI 发布包")
}
pub fn target_dir(default: &Path) -> Result<PathBuf> {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) => Ok(std::env::current_dir()?.join(path)),
        None => Ok(default.to_owned()),
    }
}

pub fn run(mut command: Command, action: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("{action}: 启动 {}", command.get_program().to_string_lossy()))?;
    if !status.success() {
        bail!("{action} 失败 ({status})");
    }
    Ok(())
}
