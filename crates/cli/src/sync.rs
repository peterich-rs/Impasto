//! Synchronizes generated platform imports and TypeScript contracts for an app.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::{
    app::App,
    plugins::{self, Platform, write},
};

pub(crate) const ANDROID_PERMISSIONS_MODULE: &str = "impasto-generated-permissions";

pub fn run(platform: Option<&str>) -> Result<()> {
    let root = std::env::current_dir()?;
    if !root.join("app.toml").is_file() && root.join("plugin.toml").is_file() {
        return sync(&App::for_plugin(root)?, platform);
    }
    let app = App::current()?;
    crate::create::scaffold(&app)?;
    sync(&app, platform)
}

pub(crate) fn sync(app: &App, target: Option<&str>) -> Result<()> {
    if let Some(target) = target {
        app.require(target)?;
        plugins::prepare(app, target, false)?;
    } else {
        for target in &app.targets {
            plugins::prepare(app, target, false)?;
        }
    }
    Ok(())
}

pub(crate) fn typescript(
    app: &Path,
    framework: &Path,
    entries: &BTreeMap<String, PathBuf>,
    types: &[PathBuf],
) -> Result<()> {
    let packages = framework.join("packages");
    let mut paths = Map::new();
    for directory in fs::read_dir(&packages)? {
        let root = directory?.path();
        let manifest = root.join("package.json");
        if !manifest.is_file() || root.join("plugin.toml").is_file() {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(manifest)?)?;
        if let (Some(name), Some(exports)) = (value["name"].as_str(), value["exports"].as_object())
        {
            for (key, entry) in exports {
                if let Some(entry) = entry.as_str() {
                    let suffix = if key == "." {
                        ""
                    } else {
                        key.trim_start_matches('.')
                    };
                    paths.insert(format!("{name}{suffix}"), json!([root.join(entry)]));
                }
            }
        }
    }
    paths.insert(
        "solid-js".into(),
        json!([packages.join("reactive/src/api.d.ts")]),
    );
    paths.insert(
        "solid-js/jsx-runtime".into(),
        json!([packages.join("solid-renderer/src/jsx.ts")]),
    );
    paths.extend(
        entries
            .iter()
            .map(|(name, path)| (name.clone(), json!([path]))),
    );
    let [token_types, i18n_types, shader_types] = development_types(app)?;
    let shader_root = app.join(".impasto/types/shaders");
    let files = types
        .iter()
        .cloned()
        .chain([
            packages.join("reactive/src/modules.d.ts"),
            packages.join("solid-renderer/src/jsx.ts"),
            framework.join("crates/testing/src/test.d.ts"),
            framework.join("crates/bundler/assets.d.ts"),
            shader_types,
            token_types,
            i18n_types,
        ])
        .collect::<Vec<_>>();
    let config = json!({
        "compilerOptions": {
            "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",
            "jsx": "preserve", "jsxImportSource": "solid-js", "noEmit": true,
            "allowImportingTsExtensions": true, "strict": true, "skipLibCheck": true,
            "types": [], "paths": paths,
            "allowArbitraryExtensions": true, "rootDirs": [app, &shader_root],
        },
        "files": files,
        "include": ["../src/**/*"],
    });
    let file = if app.join("plugin.toml").is_file() {
        ".impasto/tsconfig.plugin.json"
    } else {
        ".impasto/tsconfig.generated.json"
    };
    write(&app.join(file), &serde_json::to_string_pretty(&config)?)
}

pub(crate) fn refresh_development_types(app: &App, paths: &[PathBuf]) -> Result<()> {
    let root = &app.root;
    let types = root.join(".impasto/types");
    let theme = paths.iter().any(|path| path == &root.join("src/theme.ts"));
    let locales = paths
        .iter()
        .any(|path| path.starts_with(root.join("locales")));
    let shaders = paths.iter().any(|path| {
        path.extension()
            .is_some_and(|extension| extension == "wgsl")
    });
    if theme || locales || shaders {
        fs::create_dir_all(&types)?;
    }
    if theme {
        write(&types.join("tokens.d.ts"), &theme_types(root)?)?;
    }
    if locales {
        write(
            &types.join("i18n.d.ts"),
            &bundler::i18n::declarations(root)?,
        )?;
    }
    if shaders {
        write_shader_types(root, &types)?;
    }
    Ok(())
}

fn development_types(app: &Path) -> Result<[PathBuf; 3]> {
    let types = app.join(".impasto/types");
    fs::create_dir_all(&types)?;
    let token_types = types.join("tokens.d.ts");
    let i18n_types = types.join("i18n.d.ts");
    write(&token_types, &theme_types(app)?)?;
    write(&i18n_types, &bundler::i18n::declarations(app)?)?;
    Ok([token_types, i18n_types, write_shader_types(app, &types)?])
}

fn write_shader_types(app: &Path, types: &Path) -> Result<PathBuf> {
    let shader_types = types.join("shaders.d.ts");
    let shaders = bundler::shader_types(app)?;
    write(&shader_types, &shaders.ambient)?;
    let shader_root = types.join("shaders");
    remove_stale_shader_types(&shader_root, &shader_root, &shaders.modules)?;
    for (path, declaration) in &shaders.modules {
        let path = shader_root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write(&path, declaration)?;
    }
    Ok(shader_types)
}

fn theme_types(app: &Path) -> Result<String> {
    let tokens = bundler::ThemeTokens::load(app)?;
    let mut output = r#"// Generated by Impasto.
import type { Theme } from "@impasto/solid-ui";
type Keys<T> = Extract<keyof T, string>;
type ThemeColors = Record<Keys<Theme["color"]> | "foreground" | "white" | "black" | "transparent", true>;
type ThemeLengths = Record<
  `space.${Keys<Theme["space"]>}` | `radius.${Keys<Theme["radius"]>}`
  | `shape.${Keys<Theme["shape"]>}` | `fontSize.${Keys<Theme["fontSize"]>}`
  | `typography.${Keys<Theme["typography"]>}.size`
  | `radius.${"none" | "sm" | "md" | "lg" | "xl" | "2xl" | "full"}`
  | `text.${"xs" | "sm" | "base" | "lg" | "xl" | "2xl" | "3xl" | "4xl"}`, true>;
declare global {
"#
    .to_owned();
    for (kind, base, names) in [
        ("Color", "ThemeColors", tokens.colors),
        ("Length", "ThemeLengths", tokens.lengths),
    ] {
        writeln!(output, "  interface Impasto{kind}Tokens extends {base} {{")?;
        for name in names {
            writeln!(output, "    {}: true;", serde_json::to_string(&name)?)?;
        }
        output.push_str("  }\n");
    }
    output.push_str("}\n");
    Ok(output)
}

fn remove_stale_shader_types(
    shader_root: &Path,
    directory: &Path,
    declarations: &BTreeMap<PathBuf, String>,
) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_stale_shader_types(shader_root, &path, declarations)?;
        } else if path.to_string_lossy().ends_with(".d.wgsl.ts")
            && !declarations.contains_key(path.strip_prefix(shader_root)?)
            && fs::read_to_string(&path)?.starts_with("// Generated by Impasto.\n")
        {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub(crate) fn platform(app: &App, target: &str, framework: &Path, inputs: &Platform) -> Result<()> {
    match target {
        "android" => android(app, framework, inputs),
        "ios" | "macos" => darwin(app, target, framework, inputs),
        _ => anyhow::bail!("未知平台 {target}"),
    }
}

fn android(app: &App, framework: &Path, inputs: &Platform) -> Result<()> {
    let directory = app.root.join(".impasto/android");
    fs::create_dir_all(directory.join("kotlin"))?;
    let mut settings = String::new();
    let mut calls = String::new();
    let mut modules = Vec::new();
    for plugin in &inputs.projects {
        let name = format!(":{}", plugin.gradle_module);
        let path = serde_json::to_string(&plugin.path)?;
        writeln!(
            settings,
            "include({name:?})\nproject({name:?}).projectDir = file({})",
            path.replace('$', "\\$")
        )?;
        if let Some(class) = &plugin.plugin_class {
            if let Some(features) = &plugin.features {
                let features = features
                    .iter()
                    .map(|name| format!("{name:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    calls,
                    "        {}.{class}.register(host, enabled = setOf({features}))",
                    plugin.package
                )?;
            } else {
                writeln!(calls, "        {}.{class}.register(host)", plugin.package)?;
            }
        }
        modules.push(name);
    }
    if !inputs.android_permissions.is_empty() {
        let name = format!(":{ANDROID_PERMISSIONS_MODULE}");
        let project = directory.join("permissions");
        let manifest = project.join("src/main");
        fs::create_dir_all(&manifest)?;
        write(
            &project.join("build.gradle.kts"),
            r#"plugins { id("com.android.library") }
android {
    namespace = "dev.impasto.generated.permissions"
    compileSdk = 35
    defaultConfig { minSdk = 24 }
}
"#,
        )?;
        let xml = android_permission_manifest(inputs)?;
        write(&manifest.join("AndroidManifest.xml"), &xml)?;
        let path = serde_json::to_string(&project)?;
        writeln!(
            settings,
            "include({name:?})\nproject({name:?}).projectDir = file({})",
            path.replace('$', "\\$")
        )?;
        modules.push(name);
    }
    write(
        &directory.join("settings-plugins.settings.gradle.kts"),
        &settings,
    )?;
    write(
        &directory.join("kotlin/GeneratedPluginRegistrant.kt"),
        &format!(
            "package dev.impasto.runner\n\nimport dev.impasto.host.ImpastoHost\n\ninternal object GeneratedPluginRegistrant {{\n    @Suppress(\"UNUSED_PARAMETER\")\n    fun register(host: ImpastoHost) {{\n{calls}    }}\n}}\n"
        ),
    )?;
    let props = [
        (
            "framework",
            framework.to_str().context("SDK 路径需要 UTF-8")?.to_owned(),
        ),
        ("plugin.modules", modules.join(",")),
        ("applicationId", app.android_id()),
        ("appName", app.name.clone()),
        ("versionName", app.version.clone()),
        (
            "versionCode",
            app.android.version_code.unwrap_or(1).to_string(),
        ),
        ("abis", app.android.abis.join(",")),
    ]
    .into_iter()
    .fold(String::new(), |mut source, (key, value)| {
        source.push_str(key);
        source.push('=');
        source.push_str(
            &value
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\r', "\\r"),
        );
        source.push('\n');
        source
    });
    write(&directory.join("plugins.properties"), &props)?;
    let local = app.root.join("android/local.properties");
    if local.parent().is_some_and(Path::is_dir) {
        let mut lines = match fs::read_to_string(&local) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.starts_with("impasto.sdk="))
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        if !lines.iter().any(|line| line.starts_with("sdk.dir="))
            && let Some(sdk) =
                std::env::var_os("ANDROID_HOME").or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        {
            lines.push(format!(
                "sdk.dir={}",
                sdk.to_string_lossy().replace('\\', "\\\\")
            ));
        }
        lines.push(format!(
            "impasto.sdk={}",
            framework.display().to_string().replace('\\', "\\\\")
        ));
        write(&local, &(lines.join("\n") + "\n"))?;
    }
    Ok(())
}

fn android_permission_manifest(inputs: &Platform) -> Result<String> {
    let flagged = inputs.android_permissions.iter().any(|name| {
        inputs
            .android_permission_attributes
            .get(name)
            .is_some_and(|rule| !rule.uses_permission_flags.is_empty())
    });
    let mut xml =
        String::from("<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"");
    if flagged {
        xml.push_str(" xmlns:tools=\"http://schemas.android.com/tools\"");
    }
    xml.push_str(">\n");
    for permission in &inputs.android_permissions {
        write!(xml, "    <uses-permission android:name={permission:?}")?;
        if let Some(rule) = inputs.android_permission_attributes.get(permission) {
            if let Some(max_sdk) = rule.max_sdk {
                write!(xml, " android:maxSdkVersion=\"{max_sdk}\"")?;
            }
            if !rule.uses_permission_flags.is_empty() {
                let flags = rule
                    .uses_permission_flags
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join("|");
                write!(
                    xml,
                    " android:usesPermissionFlags={flags:?} tools:targetApi=\"s\""
                )?;
            }
        }
        xml.push_str(" />\n");
    }
    xml.push_str("</manifest>\n");
    Ok(xml)
}

fn darwin(app: &App, target: &str, framework: &Path, inputs: &Platform) -> Result<()> {
    let dir = app.root.join(".impasto/darwin").join(target);
    let package = dir.join("ImpastoGeneratedPlugins");
    let sources = package.join("Sources/ImpastoGeneratedPlugins");
    fs::create_dir_all(&sources)?;
    let aliases = dir.join("packages");
    fs::create_dir_all(&aliases)?;
    let host = framework
        .join("crates/embedder")
        .join(target)
        .join("ImpastoHost");
    let host_path = swift_string(host.to_str().context("SwiftPM 宿主路径需要 UTF-8")?);
    let mut packages = format!("        .package(path: {host_path}),\n");
    let mut products = String::from(
        "                .product(name: \"ImpastoHost\", package: \"ImpastoHost\"),\n",
    );
    let mut imports = String::from("@_exported import ImpastoHost\n");
    let mut calls = String::new();
    let mut startup = String::new();
    for plugin in &inputs.projects {
        let alias = aliases.join(&plugin.package);
        symlink(&plugin.path, &alias)?;
        let path = swift_string(alias.to_str().context("SwiftPM 插件路径需要 UTF-8")?);
        let identity = swift_string(&plugin.package);
        let product = swift_string(&plugin.package);
        writeln!(packages, "        .package(path: {path}),")?;
        writeln!(
            products,
            "                .product(name: {product}, package: {identity}),"
        )?;
        writeln!(imports, "import {}", plugin.module)?;
        let class = plugin
            .plugin_class
            .as_deref()
            .context("缺少 Swift 插件注册类")?;
        if let Some(prepare) = &plugin.prepare {
            writeln!(startup, "        {}.{class}.{prepare}()", plugin.module)?;
        }
        if let Some(features) = &plugin.features {
            let mut feature_products = std::collections::BTreeSet::new();
            let mut feature_modules = std::collections::BTreeSet::new();
            let mut registrars = Vec::new();
            for feature in &plugin.swift_features {
                if feature_products.insert(&feature.product) {
                    let product = swift_string(&feature.product);
                    writeln!(
                        products,
                        "                .product(name: {product}, package: {identity}),"
                    )?;
                }
                if feature_modules.insert(&feature.module) {
                    writeln!(imports, "import {}", feature.module)?;
                }
                registrars.push(format!("{}.{}.self", feature.module, feature.registrar));
            }
            let names = features
                .iter()
                .map(|name| swift_string(name))
                .collect::<Vec<_>>()
                .join(", ");
            let registrars = registrars.join(", ");
            writeln!(
                calls,
                "        {}.{class}.register(with: host, enabled: [{names}], registrars: [{registrars}])",
                plugin.module
            )?;
        } else {
            writeln!(
                calls,
                "        {}.{class}.register(with: host)",
                plugin.module
            )?;
        }
    }
    for entry in fs::read_dir(&aliases)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink()
            && !inputs
                .projects
                .iter()
                .any(|plugin| entry.file_name() == plugin.package.as_str())
        {
            fs::remove_file(entry.path())?;
        }
    }
    let platform = if target == "ios" {
        ".iOS(.v15)"
    } else {
        ".macOS(.v13)"
    };
    write(
        &package.join("Package.swift"),
        &format!(
            r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpastoGeneratedPlugins",
    platforms: [{platform}],
    products: [.library(name: "ImpastoGeneratedPlugins", targets: ["ImpastoGeneratedPlugins"])],
    dependencies: [
{packages}    ],
    targets: [
        .target(
            name: "ImpastoGeneratedPlugins",
            dependencies: [
{products}            ]
        ),
    ]
)
"#
        ),
    )?;
    let install = if target == "ios" {
        "\n@_silgen_name(\"impasto_install_app_plugins\")\nprivate func installRustPlugins() -> Bool\n"
    } else {
        ""
    };
    let prepare = if target == "ios" {
        format!(
            "    public static func prepare() -> Bool {{\n{startup}        return installRustPlugins()\n    }}\n\n"
        )
    } else {
        String::new()
    };
    write(
        &sources.join("GeneratedPluginRegistrant.swift"),
        &format!(
            "{imports}{install}\n@MainActor\npublic enum GeneratedPluginRegistrant {{\n{prepare}    public static func register(with host: ImpastoHost) {{\n{calls}    }}\n}}\n"
        ),
    )?;
    let (bundle_id, build_number, signing) = if target == "ios" {
        (
            crate::ios::bundle_id(app),
            app.ios.build_number.unwrap_or(1),
            "",
        )
    } else {
        (
            format!("dev.impasto.{}", app.name.replace('_', "-")),
            1,
            "CODE_SIGN_IDENTITY = -\n",
        )
    };
    write(
        &dir.join("Generated.xcconfig"),
        &format!(
            "PRODUCT_BUNDLE_IDENTIFIER = {bundle_id}\nMARKETING_VERSION = {}\nCURRENT_PROJECT_VERSION = {build_number}\n{signing}",
            app.version,
        ),
    )
}

fn swift_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[cfg(unix)]
fn symlink(source: &Path, alias: &Path) -> Result<()> {
    if fs::read_link(alias)
        .as_deref()
        .is_ok_and(|current| current == source)
    {
        return Ok(());
    }
    if fs::symlink_metadata(alias).is_ok() {
        fs::remove_file(alias)?;
    }
    std::os::unix::fs::symlink(source, alias)?;
    Ok(())
}

#[cfg(windows)]
fn symlink(source: &Path, alias: &Path) -> Result<()> {
    if fs::read_link(alias)
        .as_deref()
        .is_ok_and(|current| current == source)
    {
        return Ok(());
    }
    if fs::symlink_metadata(alias).is_ok() {
        fs::remove_dir(alias)?;
    }
    std::os::windows::fs::symlink_dir(source, alias)?;
    Ok(())
}
