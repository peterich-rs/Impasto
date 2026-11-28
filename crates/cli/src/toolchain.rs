//! Locate the framework sources and run platform build tools.
use anyhow::{Context, Result, bail};
use std::cell::Cell;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

thread_local! {
    static BUILD_DIAGNOSTICS: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn with_build_diagnostics<T>(build: impl FnOnce() -> T) -> T {
    let previous = BUILD_DIAGNOSTICS.replace(true);
    let result = build();
    BUILD_DIAGNOSTICS.set(previous);
    result
}

pub fn run(mut command: Command, action: &str) -> Result<()> {
    if BUILD_DIAGNOSTICS.get() {
        return run_with_diagnostics(command, action);
    }
    let status = command
        .status()
        .with_context(|| format!("{action}: 启动 {}", command.get_program().to_string_lossy()))?;
    if !status.success() {
        bail!("{action} 失败 ({status})");
    }
    Ok(())
}

fn run_with_diagnostics(mut command: Command, action: &str) -> Result<()> {
    const LIMIT: usize = 24 * 1024;
    let mut child = command
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{action}: 启动 {}", command.get_program().to_string_lossy()))?;
    let mut stderr = child.stderr.take().context("读取构建诊断")?;
    let mut tail = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let length = stderr.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        let _ = std::io::stderr().write_all(&chunk[..length]);
        let excess = (tail.len() + length).saturating_sub(LIMIT);
        tail.drain(..excess);
        tail.extend_from_slice(&chunk[..length]);
    }
    let status = child.wait()?;
    if !status.success() {
        bail!(
            "{action} 失败 ({status})\n{}",
            String::from_utf8_lossy(&tail).trim()
        );
    }
    Ok(())
}
