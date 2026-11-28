//! Generates platform workspaces, Rust glue, and native module declarations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};

use super::cargo::local_dependencies;
use super::resolution::{resolve, validate_specifier};
use super::{Descriptor, Manifest, NativeBuild, SCRIPT_ABI, path_literal, write};
use crate::{app::App, toolchain};

pub fn prepare(app: &App, target: &str, verbose: bool) -> Result<NativeBuild> {
    let mut resolved = resolve(app, target)?;
    let framework = toolchain::framework()?;
    let glue = app.root.join(".impasto");
    let workspace = glue.join("cargo");
    let registry = workspace.join(format!("registry-{target}"));
    let entry = workspace.join(format!("entry-{target}"));
    fs::create_dir_all(&workspace)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(glue.join("plugins.lock"))?;
    lock.lock()?;
    crate::sync::platform(app, target, &framework, &resolved.platform)?;
    fs::create_dir_all(registry.join("src/bin"))?;
    fs::create_dir_all(entry.join("src"))?;
    if !workspace.join("Cargo.lock").is_file() {
        fs::copy(framework.join("Cargo.lock"), workspace.join("Cargo.lock"))?;
    }
    let registry_name = format!("impasto-registry-{target}");
    use std::hash::{Hash as _, Hasher as _};
    let mut hash = std::hash::DefaultHasher::new();
    (app.root.as_path(), target).hash(&mut hash);
    let descriptor_bin = format!("impasto-descriptors-{:016x}", hash.finish());
    let package = format!("impasto-app-{target}");
    let framework_manifest: toml::Value =
        toml::from_str(&fs::read_to_string(framework.join("Cargo.toml"))?)?;
    let profiles = framework_manifest
        .get("profile")
        .map(|profile| toml::to_string(&BTreeMap::from([("profile", profile)])))
        .transpose()?
        .unwrap_or_default();
    if resolved
        .crates
        .iter()
        .any(|(_, name)| name == "impasto-intl")
    {
        let locales = intl_locales(app)?;
        let blob = impasto_intl::datagen::export_data(&locales).map_err(anyhow::Error::msg)?;
        write_blob(&registry.join("src/intl.postcard.deflate"), &blob)?;
        resolved
            .rust_features
            .entry("impasto-intl".into())
            .or_default()
            .insert("embedded-data".into());
    }
    let mut sources = resolved.installed.clone();
    sources.insert("impasto".into(), framework.join("crates/impasto"));
    for (path, _) in &resolved.crates {
        crate::local_registry::check_registry_dependencies(path, &sources)?;
    }
    let patches = crate::local_registry::configure(&app.root, &workspace, &sources, &framework)?;
    let signature = {
        let mut hasher = std::hash::DefaultHasher::new();
        sources.hash(&mut hasher);
        resolved.rust_features.hash(&mut hasher);
        for (name, dependency) in &app.plugins {
            name.hash(&mut hasher);
            dependency.dependencies.hash(&mut hasher);
        }
        app.native
            .as_ref()
            .map(|native| &native.path)
            .hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };
    write(&registry.join("inputs.hash"), &signature)?;
    let sdk_version = impasto::VERSION;
    let mut package_toml = format!(
        "[package]\nname = \"{registry_name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[[bin]]\nname = \"{descriptor_bin}\"\npath = \"src/bin/descriptors.rs\"\n\n[dependencies]\nimpasto = {{ version = {sdk_version:?}, registry = \"impasto\" }}\nserde_json = \"1\"\n"
    );
    let mut calls = String::new();
    for (index, (path, name)) in resolved.crates.iter().enumerate() {
        use std::fmt::Write as _;
        let alias = format!("plugin_{index}");
        let features = resolved
            .rust_features
            .get(name)
            .map_or_else(String::new, |features| {
                let values = features.iter().cloned().map(toml::Value::String).collect();
                format!(", features = {}", toml::Value::Array(values))
            });
        writeln!(
            package_toml,
            "{alias} = {{ package = {name:?}, path = {}{features} }}",
            path_literal(path)?
        )?;
        writeln!(
            calls,
            "    collect(&mut items, {name:?}, {alias}::register)?;"
        )?;
    }
    write(&registry.join("Cargo.toml"), &package_toml)?;
    write_runtime_registry(&registry, &resolved.crates, None)?;
    write(
        &registry.join("src/bin/descriptors.rs"),
        &format!(
            r#"fn main() -> Result<(), Box<dyn std::error::Error>> {{
    fn collect(items: &mut Vec<serde_json::Value>, owner: &str, register: impasto::Register) -> Result<(), impasto::PluginInitError> {{
    let mut registry = impasto::PluginRegistrar::default();
    register(&mut registry);
    items.extend(registry.describe()?.into_iter().map(|module| serde_json::json!({{
        "name": module.name, "specifier": module.specifier, "exports": module.exports,
        "typescript": module.typescript, "export_signatures": module.export_signatures, "crate": owner,
        "selectable_exports": module.selectable_exports
    }})));
    Ok(())
    }}
    let _ = collect;
    let mut items = Vec::<serde_json::Value>::new();
    let _ = &mut items;
{calls}
    serde_json::to_writer(std::io::stdout().lock(), &items)?;
    Ok(())
}}
"#
        ),
    )?;
    write_entry(
        &entry,
        &registry,
        &framework,
        target,
        &registry_name,
        &package,
    )?;
    let mut members = Vec::new();
    for platform in &app.targets {
        let registry = format!("registry-{platform}");
        let entry = format!("entry-{platform}");
        let manifest = workspace.join(&registry).join("Cargo.toml");
        if manifest.is_file()
            && workspace.join(&entry).join("Cargo.toml").is_file()
            && fs::read_to_string(workspace.join(&registry).join("inputs.hash"))
                .is_ok_and(|hash| hash == signature)
            && registry_sources_exist(&manifest)?
        {
            members.extend([registry, entry]);
        }
    }
    write(
        &workspace.join("Cargo.toml"),
        &format!("[workspace]\nresolver = \"3\"\nmembers = {members:?}\n{profiles}\n{patches}"),
    )?;
    let watch_roots = local_dependencies(&workspace, &framework, &resolved, target)?;
    let mut modules = export_descriptors(
        &workspace,
        &framework,
        target,
        &registry_name,
        &descriptor_bin,
        !resolved.platform.projects.is_empty(),
        verbose,
    )?;
    validate_descriptors(&mut modules, &resolved.entries)?;
    resolved.platform.resolve_permission_methods(&modules)?;
    let dir = glue.join("types").join(target);
    fs::create_dir_all(&dir)?;
    let mut types = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        let path = dir.join(format!("module-{index}.d.ts"));
        let text = format!(
            "// Generated by Impasto.\ndeclare module {:?} {{\n{}\n}}\n",
            module.specifier,
            module.typescript.replace("export declare ", "export ")
        );
        write(&path, &text)?;
        types.push(path);
    }
    for path in fs::read_dir(&dir)? {
        let path = path?.path();
        if path.is_file() && !types.contains(&path) {
            fs::remove_file(path)?;
        }
    }
    let manifest = glue.join(format!("plugins-manifest.{target}.json"));
    let data = Manifest {
        schema_version: 1,
        script_abi: SCRIPT_ABI,
        platform: target.into(),
        modules,
        types,
        entries: resolved.entries,
        disabled_entries: resolved.disabled_entries,
        platform_inputs: resolved.platform,
        watch_roots,
        native_dependencies: resolved.allowed,
    };
    write_runtime_exports(&registry, &data.modules)?;
    write(&manifest, &serde_json::to_string(&data)?)?;
    crate::sync::typescript(&app.root, &framework, &data.entries, &data.types)?;
    Ok(NativeBuild {
        workspace,
        package,
        data,
        crates: resolved.crates,
        native_exports: String::new(),
        _lock: lock,
    })
}

fn export_descriptors(
    workspace: &Path,
    framework: &Path,
    target: &str,
    registry_name: &str,
    descriptor_bin: &str,
    has_platform_projects: bool,
    verbose: bool,
) -> Result<Vec<Descriptor>> {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .args([
            "run",
            "--quiet",
            "--locked",
            "-p",
            registry_name,
            "--bin",
            descriptor_bin,
        ])
        .arg("--target-dir")
        .arg(toolchain::target_dir(&framework.join("target"))?);
    if target == "macos" && has_platform_projects {
        // The host exporter can type-check the same macOS declarations without
        // linking the application's Swift or Objective-C objects.
        let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_else(|_| {
            std::env::var("RUSTFLAGS")
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("\x1f")
        });
        let flags = [
            flags.as_str(),
            "--cfg",
            "impasto_metadata",
            "--check-cfg",
            "cfg(impasto_metadata)",
        ]
        .into_iter()
        .filter(|flag| !flag.is_empty())
        .collect::<Vec<_>>()
        .join("\x1f");
        command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    if verbose {
        eprintln!("导出 {target} 插件声明: {command:?}");
    }
    let output = command
        .env_remove("IMPASTO_NATIVE_EXPORTS")
        .output()
        .context("启动 Rust 插件描述导出器")?;
    if !output.status.success() {
        bail!(
            "导出 {target} 插件声明失败\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).context("解析原生模块描述")
}

fn validate_descriptors(
    modules: &mut [Descriptor],
    entries: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let mut used = entries.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    for module in modules {
        validate_specifier(&module.specifier)?;
        if !used.insert(&module.specifier) {
            bail!("插件声明与其他模块冲突: {}", module.specifier);
        }
        if !names.insert(&module.name) {
            bail!("重复的原生模块名称 {}", module.name);
        }
        module.export_abi = std::mem::take(&mut module.export_signatures)
            .into_iter()
            .map(|(name, signature)| {
                let hash = Sha256::digest(signature.as_bytes());
                (name, URL_SAFE_NO_PAD.encode(hash))
            })
            .collect();
    }
    Ok(())
}

fn intl_locales(app: &App) -> Result<Vec<String>> {
    let configured = app
        .plugin_options
        .get("@impasto/intl")
        .and_then(|options| options.locales.as_ref());
    let mut locales = BTreeSet::from(["en".to_owned()]);
    if let Some(configured) = configured {
        locales.extend(configured.iter().cloned());
    } else {
        let directory = app.root.join("locales");
        if directory.is_dir() {
            for file in fs::read_dir(directory)? {
                let path = file?.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                    locales.insert(
                        path.file_stem()
                            .and_then(|name| name.to_str())
                            .context("语种名称需要 UTF-8")?
                            .to_owned(),
                    );
                }
            }
        }
    }
    Ok(locales.into_iter().collect())
}

fn write_blob(path: &Path, blob: &[u8]) -> Result<()> {
    use std::io::Write as _;

    if fs::read(path).is_ok_and(|old| old == blob) {
        return Ok(());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("缺少 ICU 数据目录")?)?;
    temporary.write_all(blob)?;
    temporary.persist(path)?;
    Ok(())
}

pub(super) fn write_runtime_registry(
    registry: &Path,
    crates: &[(PathBuf, String)],
    retained: Option<&BTreeSet<String>>,
) -> Result<()> {
    use std::fmt::Write as _;
    let mut code = String::from(
        "pub fn register(registrar: &mut impasto::PluginRegistrar) {\n    let _ = &registrar;\n",
    );
    for (index, (_, name)) in crates.iter().enumerate() {
        if retained.is_none_or(|retained| retained.contains(name)) {
            if name == "impasto-intl" {
                writeln!(
                    code,
                    "    plugin_{index}::register_with_data(registrar, include_bytes!(\"intl.postcard.deflate\"));"
                )?;
            } else {
                writeln!(code, "    plugin_{index}::register(registrar);")?;
            }
        }
    }
    code.push_str("}\n");
    write(&registry.join("src/lib.rs"), &code)
}

pub(super) fn write_runtime_exports(registry: &Path, modules: &[Descriptor]) -> Result<()> {
    let exports = modules
        .iter()
        .map(|module| {
            serde_json::json!({
                "name": module.name,
                "specifier": module.specifier,
                "exports": module.exports,
                "typescript": (!module.typescript.is_empty()).then_some(&module.typescript),
            })
        })
        .collect::<Vec<_>>();
    write(
        &registry.join("exports.json"),
        &serde_json::to_string(&exports)?,
    )
}

fn registry_sources_exist(manifest: &Path) -> Result<bool> {
    // A removed local plugin must not prevent using another platform's registry.
    let registry: toml::Value = toml::from_str(&fs::read_to_string(manifest)?)?;
    Ok(registry
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|dependencies| dependencies.values())
        .filter_map(|dependency| dependency.get("path").and_then(toml::Value::as_str))
        .all(|path| Path::new(path).join("Cargo.toml").is_file()))
}

fn write_entry(
    entry: &Path,
    registry: &Path,
    framework: &Path,
    target: &str,
    registry_name: &str,
    package: &str,
) -> Result<()> {
    let host = path_literal(&framework.join("crates/embedder").join(target))?;
    let registry = path_literal(registry)?;
    let extra = if target == "android" {
        ", features = [\"app-entry\"]"
    } else {
        ""
    };
    let product = match target {
        "macos" => "[lib]\nname = \"impasto_app\"\ncrate-type = [\"staticlib\"]",
        "ios" => "[lib]\nname = \"impasto_app\"\ncrate-type = [\"staticlib\"]",
        "android" => "[lib]\nname = \"impasto\"\ncrate-type = [\"cdylib\"]",
        _ => bail!("未知原生入口 {target}"),
    };
    let config = format!(
        "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n{product}\n\n[dependencies]\nhost = {{ package = \"embedder-{target}\", path = {host}, default-features = false{extra} }}\nplugins = {{ package = {registry_name:?}, path = {registry} }}\n"
    );
    write(&entry.join("Cargo.toml"), &config)?;
    let install = format!(
        "host::install_plugins(plugins::register, include_str!(\"../../registry-{target}/exports.json\"))"
    );
    let (file, source) = match target {
        "macos" => (
            "lib.rs",
            format!(
                r#"fn run(open_url: Option<host::OpenUrl>, callbacks: Option<host::Callbacks>) -> u8 {{
    if let Err(error) = {install} {{
        eprintln!("{{error}}");
        return 1;
    }}
    host::exit_status_with_callbacks(open_url, callbacks)
}}

#[unsafe(no_mangle)]
pub extern "C" fn impasto_macos_run(open_url: Option<host::OpenUrl>) -> u8 {{
    run(open_url, None)
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_macos_run_host(callbacks: *const host::Callbacks) -> u8 {{
    // SAFETY: the AppKit host retains the callbacks and their context until run returns.
    run(None, unsafe {{ callbacks.as_ref() }}.copied())
}}
"#
            ),
        ),
        "ios" => (
            "lib.rs",
            format!(
                "#[unsafe(no_mangle)]\npub extern \"C\" fn impasto_install_app_plugins() -> bool {{\n    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();\n    *INSTALLED.get_or_init(|| {install}.is_ok())\n}}\n"
            ),
        ),
        "android" => ("lib.rs", android_entry(&install)?),
        _ => bail!("未知原生入口 {target}"),
    };
    write(&entry.join("src").join(file), &source)
}

fn android_entry(install: &str) -> Result<String> {
    use quote::{ToTokens, quote};
    use std::fmt::Write as _;
    let entry = syn::parse_file(include_str!(
        "../../../embedder/android/src/app/entrypoints.rs"
    ))?;
    let mut source = format!(
        r#"use host::app_jni::*;

#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> jint {{
    if {install}.is_ok() {{ 0x0001_0006 }} else {{ -1 }}
}}
"#
    );
    for item in entry.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        let mut signature = function.sig;
        let name = &signature.ident;
        if !name.to_string().starts_with("Java_") {
            continue;
        }
        let name = name.clone();
        let mut params = Vec::new();
        for param in &mut signature.inputs {
            let syn::FnArg::Typed(param) = param else {
                bail!("JNI receiver is invalid")
            };
            let syn::Pat::Ident(pat) = &mut *param.pat else {
                bail!("JNI parameter is invalid")
            };
            pat.mutability = None;
            params.push(pat.ident.clone());
        }
        writeln!(
            source,
            "{}",
            quote! {
                #[unsafe(no_mangle)]
                pub #signature { host::app_jni::#name(#(#params),*) }
            }
            .to_token_stream()
        )?;
    }
    Ok(source)
}
