use crate::app::validate_name;
use anyhow::{Result, bail};
use std::{fs, path::Path};
const MAIN: &str = "import { boot } from \"@impasto/solid-renderer\";\n\nexport default function Main() {\n  return <view />;\n}\n\nboot(Main);\n";
pub fn create(name: &str) -> Result<()> {
    validate_name(name)?;
    let root = Path::new(name);
    if root.exists() {
        bail!("{} 已存在", root.display());
    }
    fs::create_dir(root)?;
    fs::create_dir(root.join("lib"))?;
    fs::create_dir(root.join("assets"))?;
    fs::write(
        root.join("app.toml"),
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ntargets = [\"macos\", \"android\"]\n\n[permissions]\n"
        ),
    )?;
    fs::write(
        root.join("package.json"),
        format!("{{\n  \"name\": \"{name}\",\n  \"private\": true,\n  \"type\": \"module\"\n}}\n"),
    )?;
    fs::write(root.join("lib/main.tsx"), MAIN)?;
    Ok(())
}
