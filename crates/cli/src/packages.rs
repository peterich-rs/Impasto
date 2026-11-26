//! Update app source and native Cargo dependencies.

pub fn get() -> anyhow::Result<()> {
    crate::sync::run(None)
}

/// Adds a bundled Impasto dependency and optionally exposes its Rust crate.
pub fn add(package: &str, rust: bool) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use std::fs;
    use toml_edit::{DocumentMut, InlineTable, Item, Value};

    if !crate::plugins::valid_package_name(package) {
        bail!("无效的 Impasto 包名 {package:?}");
    }
    let cwd = std::env::current_dir()?;
    let plugin = cwd.join("plugin.toml").is_file() && !cwd.join("app.toml").is_file();
    let app = if plugin {
        crate::app::App::for_plugin(cwd)?
    } else {
        crate::app::App::current()?
    };
    let framework = crate::toolchain::framework()?;
    let bundled = crate::dependencies::builtin(&framework, package)?;
    if rust && !bundled.join("plugin.toml").is_file() {
        bail!("{package} 未提供 Impasto Rust 插件");
    }
    let manifest = app
        .root
        .join(if plugin { "plugin.toml" } else { "app.toml" });
    let mut document = fs::read_to_string(&manifest)?.parse::<DocumentMut>()?;
    document["dependencies"][package] = toml_edit::value("builtin");
    crate::plugins::write(&manifest, &document.to_string())?;
    if rust {
        let dependencies = crate::plugins::discover(&app.root)?;
        let dependency = dependencies
            .get(package)
            .with_context(|| format!("{package} 未提供 Impasto Rust 插件"))?;
        let (local, _) = crate::plugins::plugin_crate(&dependency.path)?;
        let root = dependency.path.join(local.path);
        let (_, name) = crate::plugins::cargo_package(&root)?;
        let version = crate::local_registry::package_version(&root)?;
        if app.native.is_none() {
            crate::create::rust_crate(&app)?;
        }
        let local_rust = app.native.as_ref().map_or_else(
            || app.root.join("rust"),
            |native| app.root.join(&native.path),
        );
        let manifest = local_rust.join("Cargo.toml");
        let mut document = fs::read_to_string(&manifest)?.parse::<DocumentMut>()?;
        let mut spec = InlineTable::new();
        spec.insert("version", Value::from(version));
        spec.insert("registry", Value::from("impasto"));
        document["dependencies"][&name] = Item::Value(Value::InlineTable(spec));
        crate::plugins::write(&manifest, &document.to_string())?;
    }
    let app = if plugin {
        crate::app::App::for_plugin(app.root)?
    } else {
        crate::app::App::load(&app.root)?
    };
    crate::sync::sync(&app, None)
}
