//! Package the CLI executable, its framework sources, and the native checker.
use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};

pub(crate) fn write(output: &Path) -> Result<()> {
    let framework = crate::toolchain::framework()?;
    let cwd = env::current_dir()?;
    let output = cwd.join(output);
    let parent = output
        .parent()
        .context("distribution needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let output = parent.join(output.file_name().context("invalid distribution path")?);
    if framework.starts_with(&output)
        || ["crates", "packages"]
            .iter()
            .any(|directory| output.starts_with(framework.join(directory)))
    {
        bail!("distribution cannot replace the framework source");
    }
    let staging = tempfile::tempdir_in(parent)?;
    let root = staging.path().join("dist");
    fs::create_dir_all(root.join("impasto-tools"))?;
    crate::typescript::write_archive(&root.join("impasto-tools/typescript.tgz"))?;
    let exe = env::current_exe()?;
    let binary = if cfg!(windows) {
        "impasto.exe"
    } else {
        "impasto"
    };
    fs::copy(exe, root.join(binary))?;
    let destination = root.join("impasto-tools/framework");
    fs::create_dir_all(&destination)?;
    let skip = [
        "node_modules",
        "target",
        "build",
        "dist",
        ".impasto",
        ".build",
        ".swiftpm",
        ".gradle",
        ".kotlin",
        ".git",
        "local.properties",
        "xcuserdata",
    ];
    for directory in ["packages", "crates"] {
        copy_sources(
            &framework,
            &framework.join(directory),
            &destination.join(directory),
            &skip,
        )?;
    }
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "LICENSE",
        "NOTICE",
    ] {
        fs::copy(framework.join(file), destination.join(file))?;
    }
    let previous = staging.path().join("previous");
    if output.exists() {
        if !output.join(binary).is_file() || !output.join("impasto-tools").is_dir() {
            bail!("{} is not an existing Impasto SDK", output.display());
        }
        fs::rename(&output, &previous)?;
    }
    if let Err(error) = fs::rename(&root, &output) {
        if previous.exists() {
            fs::rename(previous, &output)?;
        }
        return Err(error.into());
    }
    println!("{}", output.display());
    Ok(())
}

fn copy_sources(framework: &Path, source: &Path, destination: &Path, skip: &[&str]) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|excluded| name == *excluded) {
            continue;
        }
        let path = entry.path();
        let target = destination.join(name);
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let link = fs::read_link(&path)?;
            if link.is_absolute() || !path.canonicalize()?.starts_with(framework) {
                bail!(
                    "SDK source link must stay inside the framework: {}",
                    path.display()
                );
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(link, target)?;
            #[cfg(windows)]
            if path.is_dir() {
                std::os::windows::fs::symlink_dir(link, target)?;
            } else {
                std::os::windows::fs::symlink_file(link, target)?;
            }
        } else if kind.is_dir() {
            copy_sources(framework, &path, &target, skip)?;
        } else {
            fs::copy(&path, target).with_context(|| format!("copy {}", path.display()))?;
        }
    }
    Ok(())
}
