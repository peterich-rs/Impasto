use crate::ios::IosConfig;
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
    #[serde(default)]
    android: AndroidConfig,
    #[serde(default)]
    ios: IosConfig,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidConfig {
    pub application_id: Option<String>,
    pub version_code: Option<u32>,
    #[serde(default)]
    pub abis: Vec<String>,
    #[serde(default)]
    pub deep_links: Vec<AndroidLink>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidLink {
    pub scheme: String,
    pub host: String,
    pub path_prefix: Option<String>,
}

impl AndroidConfig {
    fn validate(&self) -> Result<()> {
        if let Some(id) = &self.application_id
            && (!id.contains('.')
                || !id.split('.').all(|part| {
                    part.starts_with(|c: char| c.is_ascii_alphabetic())
                        && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                }))
        {
            bail!("android.application_id 必须是以点分隔的 Android 包名");
        }
        if self
            .version_code
            .is_some_and(|code| !(1..=2_100_000_000).contains(&code))
        {
            bail!("android.version_code 必须在 1 到 2100000000 之间");
        }
        if self
            .abis
            .iter()
            .any(|abi| !matches!(abi.as_str(), "arm64-v8a" | "x86_64"))
        {
            bail!("android.abis 只能包含 arm64-v8a 和 x86_64");
        }
        for link in &self.deep_links {
            if !link.scheme.starts_with(|c: char| c.is_ascii_lowercase())
                || !link.scheme.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.')
                })
                || link.host.is_empty()
                || !link
                    .host
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-'))
                || link.path_prefix.as_ref().is_some_and(|path| {
                    !path.starts_with('/') || path.chars().any(char::is_control)
                })
            {
                bail!("android.deep_links 需要小写 scheme、host 和以 / 开头的可选 path_prefix");
            }
        }
        for permission in &self.permissions {
            if !permission.contains('.')
                || !permission.split('.').all(|part| {
                    part.starts_with(|c: char| c.is_ascii_alphabetic())
                        && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
            {
                bail!("android.permissions 需要完整的 Android 权限名称");
            }
        }
        Ok(())
    }
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
    pub android: AndroidConfig,
    pub ios: IosConfig,
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
                targets: vec!["macos".into(), "android".into(), "ios".into()],
                entry: framework.join("apps/gallery-solid/src/quotes.tsx"),
                gallery: true,
                android: AndroidConfig::default(),
                ios: IosConfig::default(),
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
        manifest.android.validate()?;
        manifest.ios.validate()?;
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
            if name == "native-plugins"
                && match permission {
                    Permission::Enabled(enabled) => *enabled,
                    Permission::Targets(targets) => targets != &["ios"],
                }
            {
                bail!(r#"启用 native-plugins 需要显式设置为 ["ios"]"#);
            }
            if ![
                "clipboard",
                "linking",
                "file-system",
                "async-storage",
                "app-state",
                "permissions",
                "native-plugins",
                "devtools",
                "http",
                "websocket",
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
            android: manifest.android,
            ios: manifest.ios,
            permissions: manifest.permissions,
        })
    }
    pub fn require(&self, target: &str) -> Result<()> {
        if !self.targets.iter().any(|t| t == target) {
            bail!("app.toml 没有启用目标 {target}");
        }
        Ok(())
    }

    pub fn android_id(&self) -> String {
        self.android
            .application_id
            .clone()
            .unwrap_or_else(|| format!("dev.impasto.{}", self.name.replace('-', "_")))
    }
    pub fn modules(&self, target: &str) -> String {
        if self.gallery {
            if target == "ios" {
                return crate::ios::MODULES.join(",");
            }
            return "clipboard,linking,file-system,async-storage,app-state,permissions,devtools,http,websocket"
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
    pub fn assets(&self) -> PathBuf {
        if self.gallery {
            crate::framework_root().map_or_else(
                || self.root.join("assets"),
                |r| r.join("crates/embedder/macos/assets"),
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
