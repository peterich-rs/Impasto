//! Resolves plugin inputs and generates target-specific Cargo projects and declarations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{app::App, toolchain};

mod cargo;
mod generated;
mod platform;
mod resolution;

pub use generated::prepare;
use generated::{write_runtime_exports, write_runtime_registry};
pub(crate) use resolution::{crate_name, discover, plugin_crate, valid_package_name};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub path: PathBuf,
}

pub struct Dependency {
    pub path: PathBuf,
    dependencies: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plugin {
    plugin: Package,
    rust: Local,
    #[serde(default)]
    ios: PlatformProject,
    #[serde(default)]
    android: PlatformProject,
    #[serde(default)]
    macos: PlatformProject,
    #[serde(default, rename = "dependencies")]
    _dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    platforms: Vec<String>,
    min_impasto_version: Option<String>,
}

#[derive(Default, Deserialize)]
struct JavaScriptPackage {
    name: Option<String>,
    #[serde(default)]
    exports: BTreeMap<String, String>,
}

pub(crate) struct Discovery {
    pub plugins: BTreeMap<String, Dependency>,
    pub sources: BTreeMap<String, PathBuf>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PlatformProject {
    path: Option<PathBuf>,
    package: Option<String>,
    module: Option<String>,
    plugin_class: Option<String>,
    usage_description_keys: BTreeSet<String>,
    required_entitlements: BTreeSet<String>,
    permissions: BTreeMap<String, Vec<String>>,
    features: BTreeMap<String, PlatformFeature>,
    permission_rules: BTreeMap<String, AndroidPermissionRule>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PlatformFeature {
    product: Option<String>,
    module: Option<String>,
    registrar: Option<String>,
    usage_description_keys: BTreeSet<String>,
    required_entitlements: BTreeSet<String>,
    infer_permissions: BTreeSet<String>,
    default: bool,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AndroidPermissionRule {
    feature: Option<String>,
    features: BTreeSet<String>,
    max_sdk: Option<u32>,
    max_sdk_by_feature: BTreeMap<String, u32>,
    uses_permission_flags: BTreeSet<String>,
}

#[derive(Clone, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AndroidPermissionAttributes {
    pub max_sdk: Option<u32>,
    pub uses_permission_flags: BTreeSet<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SwiftFeature {
    pub product: String,
    pub module: String,
    pub registrar: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct NativeProject {
    pub path: PathBuf,
    pub package: String,
    pub gradle_module: String,
    pub module: String,
    pub plugin_class: Option<String>,
    #[serde(default)]
    pub features: Option<BTreeSet<String>>,
    #[serde(default)]
    pub swift_features: Vec<SwiftFeature>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Platform {
    pub projects: Vec<NativeProject>,
    pub usage_description_keys: BTreeSet<String>,
    pub required_entitlements: BTreeSet<String>,
    permission_rules: BTreeMap<String, BTreeMap<String, Vec<PermissionMethod>>>,
    permission_attribute_rules: BTreeMap<String, BTreeMap<String, AndroidPermissionAttributes>>,
    pub android_permissions: BTreeSet<String>,
    pub android_permission_attributes: BTreeMap<String, AndroidPermissionAttributes>,
}

#[derive(Clone, Deserialize, Serialize)]
struct PermissionMethod {
    specifier: String,
    export: String,
    #[serde(default)]
    object_export: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Descriptor {
    pub name: String,
    pub specifier: String,
    pub typescript: String,
    pub exports: Vec<String>,
    #[serde(default, rename = "crate")]
    owner: String,
    #[serde(default, rename = "exportAbi")]
    export_abi: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    export_signatures: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    selectable_exports: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    schema_version: u32,
    #[serde(default)]
    script_abi: u32,
    platform: String,
    modules: Vec<Descriptor>,
    types: Vec<PathBuf>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform_inputs: Platform,
    #[serde(default)]
    watch_roots: Vec<PathBuf>,
    #[serde(default)]
    native_dependencies: BTreeMap<String, BTreeSet<String>>,
}

impl Manifest {
    pub(crate) fn linker_manifest(&self) -> bundler::PluginManifest {
        bundler::PluginManifest::new(
            self.platform.clone(),
            self.entries.clone(),
            self.disabled_entries.clone(),
            self.modules
                .iter()
                .map(|module| (module.specifier.clone(), module.exports.clone())),
        )
    }

    pub(crate) fn validate_update(&self, installed: &[u8], linked: &bundler::Bundle) -> Result<()> {
        let installed: InstalledContract = serde_json::from_slice(installed)?;
        if installed.schema_version != self.schema_version
            || installed.platform != self.platform
            || installed.script_abi != SCRIPT_ABI
        {
            bail!("已安装壳的原生契约版本或平台不匹配，请重新构建并安装应用");
        }
        let roots = self
            .modules
            .iter()
            .filter(|module| linked.native_use.contains_key(&module.specifier))
            .map(|module| module.owner.clone())
            .collect::<BTreeSet<_>>();
        let (retained, dependencies) = self.dependency_closure(roots);
        let required_permissions =
            self.platform_inputs
                .permissions_for(linked, &dependencies, &retained);
        if let Some(permission) = required_permissions
            .difference(&installed.android_permissions)
            .next()
        {
            bail!("已安装壳缺少 Android 权限 {permission}，请重新构建并安装应用");
        }
        for (specifier, exports) in &linked.native_use {
            let current = self
                .modules
                .iter()
                .find(|item| &item.specifier == specifier);
            let prior = installed
                .modules
                .iter()
                .find(|item| &item.specifier == specifier);
            let (Some(current), Some(prior)) = (current, prior) else {
                bail!("已安装壳缺少原生模块 {specifier}，请重新构建并安装应用");
            };
            let required = exports.as_ref().map_or_else(
                || current.exports.iter().collect::<Vec<_>>(),
                |exports| exports.iter().collect(),
            );
            for name in required {
                let signature = current.export_abi.get(name);
                if signature.is_none() || signature != prior.export_abi.get(name) {
                    bail!("原生接口 {specifier}::{name} 与已安装壳不兼容，请重新构建并安装应用");
                }
            }
        }
        Ok(())
    }

    fn dependency_closure(
        &self,
        mut retained: BTreeSet<String>,
    ) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut pending = retained.iter().cloned().collect::<Vec<_>>();
        let mut dependencies = BTreeSet::new();
        while let Some(owner) = pending.pop() {
            for dependency in self.native_dependencies.get(&owner).into_iter().flatten() {
                dependencies.insert(dependency.clone());
                if retained.insert(dependency.clone()) {
                    pending.push(dependency.clone());
                }
            }
        }
        (retained, dependencies)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledContract {
    schema_version: u32,
    #[serde(default)]
    script_abi: u32,
    platform: String,
    modules: Vec<InstalledModule>,
    #[serde(default)]
    android_permissions: BTreeSet<String>,
}

// Increment when generated JavaScript requires a new scene or runtime binding.
const SCRIPT_ABI: u32 = 4;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledModule {
    specifier: String,
    #[serde(default)]
    export_abi: BTreeMap<String, String>,
}

pub struct NativeBuild {
    pub workspace: PathBuf,
    pub package: String,
    data: Manifest,
    crates: Vec<(PathBuf, String)>,
    native_exports: String,
    _lock: fs::File,
}

impl NativeBuild {
    pub fn inputs(&self) -> &Platform {
        &self.data.platform_inputs
    }

    pub(crate) fn native_exports(&self) -> &str {
        &self.native_exports
    }

    pub fn linker_manifest(&self) -> bundler::PluginManifest {
        self.data.linker_manifest()
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>> {
        let contract = InstalledContract {
            schema_version: self.data.schema_version,
            script_abi: SCRIPT_ABI,
            platform: self.data.platform.clone(),
            android_permissions: self.data.platform_inputs.android_permissions.clone(),
            modules: self
                .data
                .modules
                .iter()
                .map(|module| InstalledModule {
                    specifier: module.specifier.clone(),
                    export_abi: module.export_abi.clone(),
                })
                .collect(),
        };
        Ok(serde_json::to_vec_pretty(&contract)?)
    }

    pub(crate) fn snapshot_for(&self, linked: &bundler::Bundle) -> Result<Vec<u8>> {
        let mut contract: serde_json::Value = serde_json::from_slice(&self.snapshot()?)?;
        let mut chunks = BTreeMap::from([("app.js", &linked.entry_native_use)]);
        chunks.extend(
            linked
                .chunks
                .iter()
                .map(|(name, chunk)| (name.as_str(), &chunk.native_use)),
        );
        contract["nativeUseByChunk"] = serde_json::to_value(chunks)?;
        Ok(serde_json::to_vec_pretty(&contract)?)
    }

    pub(crate) fn test_runner(&self, framework: &Path, gpu: bool) -> Result<Option<PathBuf>> {
        let packages = framework.join("packages");
        if self
            .crates
            .iter()
            .all(|(path, _)| path.starts_with(&packages))
        {
            return Ok(None);
        }
        use std::hash::{Hash as _, Hasher as _};
        let mut hash = std::hash::DefaultHasher::new();
        self.workspace.hash(&mut hash);
        self.data.platform.hash(&mut hash);
        let name = format!("impasto-native-tests-{:016x}", hash.finish());
        let registry = format!("registry-{}", self.data.platform);
        let directory = self.workspace.join(&name);
        fs::create_dir_all(directory.join("src"))?;
        let features = if gpu {
            ", features = [\"headless-gpu\"]"
        } else {
            ""
        };
        let manifest = format!(
            "[package]\nname = {name:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nimpasto = {{ path = {} }}\nregistry = {{ path = {:?}, package = {:?} }}\ntesting = {{ path = {}, default-features = false{features} }}\ntempfile = \"3\"\n",
            path_literal(&framework.join("crates/impasto"))?,
            format!("../{registry}"),
            format!("impasto-{registry}"),
            path_literal(&framework.join("crates/testing"))?,
        );
        write(&directory.join("Cargo.toml"), &manifest)?;
        write(
            &directory.join("src/main.rs"),
            include_str!("../templates/test_host.rs"),
        )?;
        let workspace_manifest = self.workspace.join("Cargo.toml");
        let mut workspace: toml::Value = toml::from_str(&fs::read_to_string(&workspace_manifest)?)?;
        let members = workspace
            .get_mut("workspace")
            .and_then(|table| table.get_mut("members"))
            .and_then(toml::Value::as_array_mut)
            .context("生成的 Cargo 工作区缺少 members")?;
        let member = toml::Value::String(name.clone());
        if !members.contains(&member) {
            members.push(member);
        }
        write(&workspace_manifest, &toml::to_string(&workspace)?)?;
        let target = toolchain::target_dir(&framework.join("target"))?;
        let mut command = Command::new("cargo");
        command
            .current_dir(&self.workspace)
            .args(["build", "--quiet", "-p", &name])
            .arg("--target-dir")
            .arg(&target);
        toolchain::run(command, "构建应用 Rust 无头测试宿主")?;
        Ok(Some(target.join("debug").join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name
        })))
    }

    pub(crate) fn prune(&mut self, app: &App, linked: &bundler::Bundle) -> Result<()> {
        use std::fmt::Write as _;
        let described = self
            .data
            .modules
            .iter()
            .map(|module| module.owner.as_str())
            .collect::<BTreeSet<_>>();
        let local = app
            .native
            .as_ref()
            .map(|local| cargo_package(&app.root.join(&local.path)))
            .transpose()?
            .map(|(_, name)| name);
        let mut retained = self
            .data
            .modules
            .iter()
            .filter(|module| linked.native_use.contains_key(&module.specifier))
            .map(|module| module.owner.clone())
            .collect::<BTreeSet<_>>();
        retained.extend(
            self.crates
                .iter()
                .filter(|(_, name)| !described.contains(name.as_str()))
                .map(|(_, name)| name.clone()),
        );
        retained.extend(local);
        let (retained, dependencies) = self.data.dependency_closure(retained);
        let inputs = &mut self.data.platform_inputs;
        let permissions = inputs.permissions_for(linked, &dependencies, &retained);
        let permissions_changed = inputs.android_permissions != permissions;
        inputs.android_permissions = permissions;
        inputs
            .permission_attribute_rules
            .retain(|owner, _| retained.contains(owner));
        inputs.resolve_permission_attributes()?;
        let crates_pruned = self.crates.iter().any(|(_, name)| !retained.contains(name));
        self.data
            .modules
            .retain(|module| retained.contains(&module.owner));
        for module in &mut self.data.modules {
            if module.selectable_exports
                && let Some(Some(names)) = linked.native_use.get(&module.specifier)
                && !names.is_empty()
                && names.len() < module.exports.len()
            {
                for name in names {
                    writeln!(self.native_exports, "{}\t{name}", module.specifier)?;
                }
                module.exports.retain(|name| names.contains(name));
                module.export_abi.retain(|name, _| names.contains(name));
                module.typescript.clear();
            }
        }
        let platform = &self.data.platform;
        let registry = self.workspace.join(format!("registry-{platform}"));
        write_runtime_exports(&registry, &self.data.modules)?;
        if !crates_pruned && !permissions_changed {
            return Ok(());
        }
        if crates_pruned {
            self.data.platform_inputs.projects.retain(|project| {
                retained
                    .iter()
                    .any(|name| name.replace('-', "_") == project.gradle_module)
            });
            write_runtime_registry(&registry, &self.crates, Some(&retained))?;
        }
        let framework = toolchain::framework()?;
        crate::sync::platform(app, platform, &framework, &self.data.platform_inputs)
    }

    pub(crate) fn validate_update(&self, installed: &[u8], linked: &bundler::Bundle) -> Result<()> {
        self.data.validate_update(installed, linked)
    }
}

pub fn configured(app: &App) -> bool {
    app.native.is_some() || !app.plugins.is_empty()
}

pub fn standalone<'a>(app: &'a App, target: Option<&'a str>) -> Result<Option<&'a str>> {
    if let Some(target) = target {
        app.require(target)?;
        return Ok(Some(target));
    }
    match app.targets.as_slice() {
        _ if !configured(app) => Ok(None),
        [target] => Ok(Some(target)),
        _ => bail!("应用包含 Rust 插件，需要 --platform macos、ios 或 android"),
    }
}

fn under(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.is_absolute() {
        bail!("插件资源必须相对插件目录: {}", relative.display());
    }
    let path = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("插件资源不存在: {}", root.join(relative).display()))?;
    if !path.starts_with(root) {
        bail!("插件资源超出插件目录: {}", relative.display());
    }
    Ok(path)
}

pub(crate) fn cargo_package(root: &Path) -> Result<(PathBuf, String)> {
    let manifest = root.join("Cargo.toml");
    let cfg: toml::Value = toml::from_str(
        &fs::read_to_string(&manifest).with_context(|| format!("读取 {}", manifest.display()))?,
    )?;
    let name = cfg
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} 需要一个 Cargo 包", manifest.display()))?;
    if !name
        .bytes()
        .all(|item| item.is_ascii_alphanumeric() || matches!(item, b'_' | b'-'))
    {
        bail!("Cargo 包名无效: {name}");
    }
    Ok((root.to_owned(), name.to_owned()))
}

#[derive(Default)]
struct Resolved {
    crates: Vec<(PathBuf, String)>,
    allowed: BTreeMap<String, BTreeSet<String>>,
    installed: BTreeMap<String, PathBuf>,
    entries: BTreeMap<String, PathBuf>,
    disabled_entries: BTreeSet<String>,
    platform: Platform,
}

pub(crate) fn path_literal(path: &Path) -> Result<String> {
    let path = path.to_str().context("Cargo 路径需要 UTF-8")?;
    Ok(toml::Value::String(path.to_owned()).to_string())
}

pub(crate) fn write(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path)
        .as_deref()
        .is_ok_and(|old| old == content)
    {
        return Ok(());
    }
    use std::io::Write as _;
    let parent = path.parent().context("缺少生成文件目录")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.persist(path)?;
    Ok(())
}

pub fn watched_paths(app: &App) -> BTreeSet<PathBuf> {
    let mut roots = app
        .native
        .iter()
        .map(|entry| app.root.join(&entry.path))
        .chain(app.plugins.values().map(|entry| app.root.join(&entry.path)))
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect::<BTreeSet<_>>();
    if configured(app) {
        for target in &app.targets {
            if let Some(path) = cached_manifest(app, target)
                && let Ok(data) = fs::read(path)
                && let Ok(manifest) = serde_json::from_slice::<Manifest>(&data)
            {
                roots.extend(manifest.watch_roots);
            }
        }
    }
    roots
}

pub fn is_wrapper_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts" | "tsx" | "js" | "jsx")
    )
}

pub fn cached_manifest(app: &App, target: &str) -> Option<PathBuf> {
    let path = app
        .root
        .join(".impasto")
        .join(format!("plugins-manifest.{target}.json"));
    path.is_file().then_some(path)
}

/// The watch loop has already checked that the configured native inputs are unchanged.
pub(crate) fn watch_manifest(app: &App, target: &str, verbose: bool) -> Result<Manifest> {
    if let Some(path) = cached_manifest(app, target)
        && let Ok(data) = fs::read(path)
        && let Ok(manifest) = serde_json::from_slice::<Manifest>(&data)
        && manifest.platform == target
        && manifest.schema_version == 1
        && manifest.script_abi == SCRIPT_ABI
    {
        return Ok(manifest);
    }
    Ok(prepare(app, target, verbose)?.data)
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    fn permissions_app(config: &str) -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("src"))?;
        fs::write(root.path().join("src/main.tsx"), "")?;
        fs::write(
            root.path().join("app.toml"),
            format!(
                r#"
name = "permission-fixture"
version = "1.0.0"
targets = ["ios", "android"]

[dependencies]
"@impasto/permissions" = "builtin"

{config}
"#
            ),
        )?;
        Ok(root)
    }

    #[test]
    fn swift_registrant_links_only_selected_products_and_requires_their_usage_descriptions()
    -> Result<()> {
        let project = permissions_app(
            r#"
[plugins."@impasto/permissions"]
enabled = ["camera"]

[ios.usage_descriptions]
NSCameraUsageDescription = "Use the camera for scans."
"#,
        )?;
        let app = App::load(project.path())?;
        let framework = toolchain::framework()?;
        let inputs = resolution::resolve(&app, "ios")?.platform;
        crate::sync::platform(&app, "ios", &framework, &inputs)?;
        let root = project
            .path()
            .join(".impasto/darwin/ios/ImpastoGeneratedPlugins");
        let package = fs::read_to_string(root.join("Package.swift"))?;
        let registrant = fs::read_to_string(
            root.join("Sources/ImpastoGeneratedPlugins/GeneratedPluginRegistrant.swift"),
        )?;
        assert!(package.contains("name: \"ImpastoPermissions\""));
        assert!(package.contains("name: \"ImpastoPermissionsCamera\""));
        assert!(!package.contains("ImpastoPermissionsPhotos"));
        assert!(registrant.contains("ImpastoPermissionsCamera.CameraPermissionRegistrar.self"));
        assert!(!registrant.contains("PhotosPermissionRegistrar"));

        let missing = permissions_app(
            r#"
[plugins."@impasto/permissions"]
enabled = ["camera"]
"#,
        )?;
        let error = resolution::resolve(&App::load(missing.path())?, "ios");
        assert!(error.is_err_and(|error| error.to_string().contains("NSCameraUsageDescription")));
        Ok(())
    }

    #[test]
    fn android_generates_selected_permissions_with_max_sdk_and_optional_flags() -> Result<()> {
        for bluetooth_flag in [false, true] {
            let project = permissions_app(&format!(
                r#"
[plugins."@impasto/permissions"]
enabled = ["storage", "bluetoothScan"]
android_bluetooth_never_for_location = {bluetooth_flag}
"#
            ))?;
            let app = App::load(project.path())?;
            let framework = toolchain::framework()?;
            let inputs = resolution::resolve(&app, "android")?.platform;
            crate::sync::platform(&app, "android", &framework, &inputs)?;
            let manifest = fs::read_to_string(
                project
                    .path()
                    .join(".impasto/android/permissions/src/main/AndroidManifest.xml"),
            )?;
            assert!(manifest.contains(r#"android:name="android.permission.WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="32""#));
            assert!(!manifest.contains("android.permission.CAMERA"));
            assert_eq!(manifest.contains("xmlns:tools="), bluetooth_flag);
            assert_eq!(
                manifest.contains(
                    r#"android:usesPermissionFlags="neverForLocation" tools:targetApi="s""#
                ),
                bluetooth_flag
            );
        }
        Ok(())
    }

    #[test]
    fn android_legacy_foreground_location_does_not_enable_unrelated_features() -> Result<()> {
        let project = permissions_app(
            r#"
[android]
permissions = ["android.permission.ACCESS_COARSE_LOCATION"]
"#,
        )?;
        let app = App::load(project.path())?;
        let inputs = resolution::resolve(&app, "android")?.platform;
        assert!(
            inputs
                .android_permissions
                .contains("android.permission.ACCESS_FINE_LOCATION")
        );
        assert!(
            !inputs
                .android_permissions
                .contains("android.permission.ACCESS_BACKGROUND_LOCATION")
        );
        assert!(
            !inputs
                .android_permissions
                .contains("android.permission.BLUETOOTH_SCAN")
        );
        assert!(
            !inputs
                .android_permissions
                .contains("android.permission.NEARBY_WIFI_DEVICES")
        );
        Ok(())
    }

    #[test]
    fn android_permissions_follow_the_linked_method_or_an_opaque_reference() -> Result<()> {
        let owner = "camera".to_owned();
        let specifier = "@demo/camera/native";
        let permission = "android.permission.CAMERA";
        let mut platform = Platform::default();
        platform.permission_rules.insert(
            owner.clone(),
            BTreeMap::from([(
                permission.to_owned(),
                vec![PermissionMethod {
                    specifier: specifier.to_owned(),
                    export: "open".to_owned(),
                    object_export: None,
                }],
            )]),
        );
        let module = Descriptor {
            name: "Camera".to_owned(),
            specifier: specifier.to_owned(),
            typescript: String::new(),
            exports: vec!["open".to_owned(), "ping".to_owned(), "Camera".to_owned()],
            owner: owner.clone(),
            export_abi: BTreeMap::from([
                ("open".to_owned(), "signature".to_owned()),
                ("ping".to_owned(), "ping".to_owned()),
            ]),
            export_signatures: BTreeMap::new(),
            selectable_exports: false,
        };
        platform.resolve_permission_methods(std::slice::from_ref(&module))?;
        let manifest = Manifest {
            schema_version: 1,
            script_abi: SCRIPT_ABI,
            platform: "android".to_owned(),
            modules: vec![module],
            types: Vec::new(),
            entries: BTreeMap::new(),
            disabled_entries: BTreeSet::new(),
            platform_inputs: platform,
            watch_roots: Vec::new(),
            native_dependencies: BTreeMap::new(),
        };
        let retained = BTreeSet::from([owner]);
        let bundle = |export: Option<&str>| bundler::Bundle {
            inputs: Default::default(),
            chunks: Default::default(),
            rendered_modules: Default::default(),
            entry_native_use: Default::default(),
            entry_imports: Vec::new(),
            pool_consumers: Default::default(),
            code: String::new(),
            warnings: Vec::new(),
            source_map: None,
            native_use: BTreeMap::from([(
                specifier.to_owned(),
                export.map(|export| BTreeSet::from([export.to_owned()])),
            )]),
        };
        let required = |export| {
            manifest
                .platform_inputs
                .permissions_for(&bundle(export), &BTreeSet::new(), &retained)
        };
        assert!(required(Some("ping")).is_empty());
        assert!(required(Some("open")).contains(permission));
        assert!(required(Some("Camera")).contains(permission));
        assert!(required(None).contains(permission));

        let installed = serde_json::to_vec(&InstalledContract {
            schema_version: 1,
            script_abi: SCRIPT_ABI,
            platform: "android".to_owned(),
            modules: vec![InstalledModule {
                specifier: specifier.to_owned(),
                export_abi: manifest.modules[0].export_abi.clone(),
            }],
            android_permissions: BTreeSet::new(),
        })?;
        manifest.validate_update(&installed, &bundle(Some("ping")))?;
        let error = manifest.validate_update(&installed, &bundle(Some("open")));
        assert!(error.is_err_and(|error| error.to_string().contains(permission)));
        Ok(())
    }
}
