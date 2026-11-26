use anyhow::{Context, Result};
use std::{fs, path::Path};
pub fn clean() -> Result<()> {
    let app = crate::app::App::current()?;
    clean_in(&app.root)
}
pub fn clean_in(root: &Path) -> Result<()> {
    let cargo = root.join(".cargo/config.toml");
    if fs::read_to_string(&cargo)
        .is_ok_and(|contents| contents.starts_with(crate::local_registry::GENERATED))
    {
        fs::remove_file(cargo)?;
    }
    for name in ["build", "dist", ".impasto"] {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(&path)?,
            Ok(meta) if meta.is_dir() => {
                fs::remove_dir_all(&path).with_context(|| format!("删除 {}", path.display()))?
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
