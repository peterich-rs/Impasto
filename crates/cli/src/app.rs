use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    version: String,
    targets: Vec<String>,
    #[serde(default)]
    permissions: BTreeMap<String, Permission>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Permission {
    Enabled(bool),
    Targets(Vec<String>),
}

pub struct App {
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    pub targets: Vec<String>,
    pub entry: PathBuf,
    pub gallery: bool,
    permissions: BTreeMap<String, Permission>,
}
impl App {
    pub fn current() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        if cwd.join("app.toml").is_file() {
            return Self::load(&cwd);
        }
        if let Some(framework) = crate::framework_root().filter(|root| root == &cwd) {
            return Ok(Self {
                root: framework.join("apps/gallery-solid"),
                name: "gallery".into(),
                version: "0.1.0".into(),
                targets: vec!["macos".into(), "android".into()],
                entry: framework.join("apps/gallery-solid/src/quotes.tsx"),
                gallery: true,
                permissions: BTreeMap::new(),
            });
        }
        bail!("当前目录没有 app.toml");
    }
    pub fn load(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let path = root.join("app.toml");
        let manifest: Manifest = toml::from_str(
            &std::fs::read_to_string(&path).with_context(|| format!("读取 {}", path.display()))?,
        )
        .context("解析 app.toml")?;
        validate_name(&manifest.name)?;
        if manifest.version.split('.').count() != 3
            || !manifest
                .version
                .split('.')
                .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            bail!("version 必须是 major.minor.patch");
        }
        if manifest.targets.is_empty()
            || manifest
                .targets
                .iter()
                .any(|t| !["macos", "android", "ios"].contains(&t.as_str()))
        {
            bail!("targets 只能包含 macos、android、ios");
        }
        let entry = root.join("lib/main.tsx");
        if !entry.is_file() {
            bail!("缺少 {}", entry.display());
        }
        for (name, permission) in &manifest.permissions {
            if ![
                "clipboard",
                "linking",
                "file-system",
                "async-storage",
                "app-state",
                "permissions",
                "devtools",
            ]
            .contains(&name.as_str())
            {
                bail!("未知能力模块 {name}");
            }
            if let Permission::Targets(targets) = permission
                && targets.iter().any(|t| !manifest.targets.contains(t))
            {
                bail!("{name} 的目标不在 targets 中");
            }
        }
        Ok(Self {
            root,
            name: manifest.name,
            version: manifest.version,
            targets: manifest.targets,
            entry,
            gallery: false,
            permissions: manifest.permissions,
        })
    }
    pub fn require(&self, target: &str) -> Result<()> {
        if !self.targets.iter().any(|t| t == target) {
            bail!("app.toml 没有启用目标 {target}");
        }
        Ok(())
    }
    pub fn modules(&self, target: &str) -> String {
        if self.gallery {
            return "clipboard,linking,file-system,async-storage,app-state,permissions,devtools"
                .into();
        }
        self.permissions
            .iter()
            .filter(|(_, p)| match p {
                Permission::Enabled(on) => *on,
                Permission::Targets(targets) => targets.iter().any(|t| t == target),
            })
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
    pub fn output(&self, debug: bool) -> PathBuf {
        self.root.join(if debug {
            "build/app.js"
        } else {
            "build/app.bin"
        })
    }
    pub fn assets(&self) -> PathBuf {
        if self.gallery {
            crate::framework_root().map_or_else(
                || self.root.join("assets"),
                |r| r.join("crates/macos/assets"),
            )
        } else {
            self.root.join("assets")
        }
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.as_bytes()[0].is_ascii_lowercase()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        bail!("应用名必须以小写字母开头，且只含小写字母、数字、-、_");
    }
    Ok(())
}
