//! A distribution carries its Node runtime, compiler scripts, packages and
//! native hosts next to impasto. Checkout fallback is explicit in --verbose.
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
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
pub fn tool(script: &str, verbose: bool) -> Result<Command> {
    if let Some(root) = distribution() {
        let node = root.join(if cfg!(windows) { "node.exe" } else { "node" });
        let script = root.join("framework/tools").join(script);
        if !node.is_file() || !script.is_file() {
            bail!("CLI 发布包缺少 Node 或 {}", script.display());
        }
        if verbose {
            eprintln!("impasto: 内置 {} {}", node.display(), script.display());
        }
        let mut command = Command::new(node);
        command.arg(script);
        return Ok(command);
    }
    let root =
        crate::framework_root().context("安装包缺少工具链，且当前二进制没有可用的源码检出")?;
    if verbose {
        eprintln!(
            "impasto: 回退到本机 node {}",
            root.join("tools").join(script).display()
        );
    }
    let mut command = Command::new("node");
    command.arg(root.join("tools").join(script));
    Ok(command)
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
