use crate::ios::IosConfig;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    version: String,
    targets: Vec<String>,
    assets: Option<PathBuf>,
    #[serde(default)]
    android: AndroidConfig,
    #[serde(default)]
    ios: IosConfig,
    #[serde(default)]
    macos: MacConfig,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default)]
    plugins: BTreeMap<String, PluginOptions>,
    #[serde(default, rename = "theme")]
    _theme: toml::Table,
    #[serde(default, rename = "i18n")]
    _i18n: toml::Table,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PluginOptions {
    pub enabled: Option<BTreeSet<String>>,
    pub android_bluetooth_never_for_location: bool,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacConfig {
    pub window: Option<MacWindowConfig>,
    pub usage_descriptions: BTreeMap<String, String>,
    pub entitlements: BTreeMap<String, bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacWindowConfig {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub min_width: Option<f64>,
    pub min_height: Option<f64>,
    pub title: Option<String>,
    pub autosave_name: Option<String>,
}

impl MacWindowConfig {
    fn validate(&self) -> Result<()> {
        for (key, size) in [
            ("width", self.width),
            ("height", self.height),
            ("min_width", self.min_width),
            ("min_height", self.min_height),
        ] {
            if size.is_some_and(|size| !(200.0..=8192.0).contains(&size)) {
                bail!("macos.window.{key} 必须在 200 到 8192 点之间");
            }
        }
        if self
            .min_width
            .is_some_and(|size| size > self.width.unwrap_or(360.0))
            || self
                .min_height
                .is_some_and(|size| size > self.height.unwrap_or(640.0))
        {
            bail!("macos.window 的最小尺寸不能超过初始尺寸");
        }
        if self.autosave_name.as_ref().is_some_and(|name| {
            name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        }) {
            bail!("macos.window.autosave_name 只能包含字母、数字、点、下划线和连字符");
        }
        if self
            .title
            .as_ref()
            .is_some_and(|title| title.contains('\0'))
        {
            bail!("macos.window.title 不能包含 NUL");
        }
        Ok(())
    }
}

impl MacConfig {
    fn validate(&self) -> Result<()> {
        if let Some(window) = &self.window {
            window.validate()?;
        }
        for (name, value) in &self.usage_descriptions {
            if !name.starts_with("NS")
                || !name.ends_with("UsageDescription")
                || !name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || value.trim().is_empty()
            {
                bail!(
                    "macos.usage_descriptions 的 {name} 需要有效的 UsageDescription 键和非空说明"
                );
            }
        }
        validate_entitlements("macos", &self.entitlements)
    }
}

pub(crate) fn validate_entitlements(
    target: &str,
    entitlements: &BTreeMap<String, bool>,
) -> Result<()> {
    for name in entitlements.keys() {
        if !name.starts_with("com.apple.")
            || name.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            bail!("无效的 {target} entitlement {name}");
        }
    }
    Ok(())
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
pub struct App {
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    pub targets: Vec<String>,
    pub entry: PathBuf,
    assets: PathBuf,
    pub android: AndroidConfig,
    pub ios: IosConfig,
    pub macos: MacConfig,
    pub native: Option<crate::plugins::Local>,
    pub plugins: BTreeMap<String, crate::plugins::Dependency>,
    pub(crate) plugin_options: BTreeMap<String, PluginOptions>,
    pub sources: BTreeMap<String, PathBuf>,
}
impl App {
    pub(crate) fn entry_for(&self, target: Option<&str>) -> PathBuf {
        target
            .map(|target| self.root.join(format!("src/main.{target}.tsx")))
            .filter(|entry| entry.is_file())
            .unwrap_or_else(|| self.entry.clone())
    }

    pub fn current() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        if cwd.join("app.toml").is_file() {
            return Self::load(&cwd);
        }
        bail!("当前目录没有 app.toml。请进入应用目录或指定 --project <目录>");
    }
    pub(crate) fn for_plugin(root: PathBuf) -> Result<Self> {
        let root = root.canonicalize()?;
        let (native, targets) = crate::plugins::plugin_crate(&root)?;
        let discovered = crate::plugins::discover(&root, None)?;
        let package: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("package.json"))?)?;
        let entry = root.join(package["exports"]["."].as_str().context("插件缺少主导出")?);
        Ok(Self {
            assets: root.join("assets"),
            root,
            name: String::new(),
            version: impasto::VERSION.to_owned(),
            targets,
            entry,
            android: AndroidConfig::default(),
            ios: IosConfig::default(),
            macos: MacConfig::default(),
            native: Some(native),
            plugins: discovered.plugins,
            plugin_options: BTreeMap::new(),
            sources: discovered.sources,
        })
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
        manifest.macos.validate()?;
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
        let entry = root.join("src/main.tsx");
        if !entry.is_file() {
            bail!("缺少 {}", entry.display());
        }
        let discovered = crate::plugins::discover(&root, None)?;
        for name in manifest.plugins.keys() {
            if !manifest.dependencies.contains_key(name) || !discovered.plugins.contains_key(name) {
                bail!("[plugins.{name:?}] 需要在 [dependencies] 声明对应的原生插件");
            }
        }
        let native = root
            .join("rust/Cargo.toml")
            .is_file()
            .then(|| crate::plugins::Local {
                path: "rust".into(),
            });
        Ok(Self {
            assets: root.join(manifest.assets.unwrap_or_else(|| PathBuf::from("assets"))),
            root,
            name: manifest.name,
            version: manifest.version,
            targets: manifest.targets,
            entry,
            android: manifest.android,
            ios: manifest.ios,
            macos: manifest.macos,
            native,
            plugins: discovered.plugins,
            plugin_options: manifest.plugins,
            sources: discovered.sources,
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
    pub fn modules(&self, target: &str) -> Result<String> {
        let mut modules = Vec::new();
        for (name, dependency) in &self.plugins {
            let Some(name) = name.strip_prefix("@impasto/") else {
                continue;
            };
            let (_, platforms) = crate::plugins::plugin_crate(&dependency.path)?;
            if platforms.iter().any(|platform| platform == target) {
                modules.push(name);
            }
        }
        Ok(modules.join(","))
    }

    pub fn assets(&self) -> &Path {
        &self.assets
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

#[cfg(test)]
mod mac_window_tests {
    use super::*;

    #[test]
    fn validates_autosaved_content_size() -> Result<()> {
        let config: MacConfig = toml::from_str(
            r#"
            [window]
            width = 1360
            height = 860.0
            min_width = 840
            autosave_name = "impasto.devtools.main"
        "#,
        )?;
        config.validate()?;

        let invalid: MacConfig = toml::from_str("[window]\nwidth = 600\nmin_width = 720")?;
        assert!(invalid.validate().is_err());
        let invalid: MacConfig = toml::from_str("[window]\nautosave_name = 'a/b'")?;
        assert!(invalid.validate().is_err());
        Ok(())
    }
}
