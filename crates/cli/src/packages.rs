//! Update app source and native Cargo dependencies.

pub fn get() -> anyhow::Result<()> {
    crate::sync::run(None)
}

pub(crate) fn remove(package: &str) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::fs;
    use toml_edit::{DocumentMut, Item};

    let (app, manifest) = project()?;
    let mut document = fs::read_to_string(&manifest)?.parse::<DocumentMut>()?;
    let dependencies = document
        .get_mut("dependencies")
        .and_then(Item::as_table_like_mut)
        .context("当前工程没有显式依赖")?;
    dependencies
        .remove(package)
        .with_context(|| format!("未直接声明 {package}"))?;
    let mut cargo = None;
    if let Some(dependency) = app.plugins.get(package)
        && let Some(native) = &app.native
    {
        let (local, _) = crate::plugins::plugin_crate(&dependency.path)?;
        let (_, name) = crate::plugins::cargo_package(&dependency.path.join(local.path))?;
        let path = app.root.join(&native.path).join("Cargo.toml");
        let mut document = fs::read_to_string(&path)?.parse::<DocumentMut>()?;
        if let Some(deps) = document
            .get_mut("dependencies")
            .and_then(Item::as_table_like_mut)
            && deps
                .get(&name)
                .and_then(|dep| dep.get("registry"))
                .and_then(Item::as_str)
                == Some("impasto")
        {
            deps.remove(&name);
            cargo = Some((path, document));
        }
    }
    if let Some((path, document)) = cargo {
        crate::plugins::write(&path, &document.to_string())?;
    }
    crate::plugins::write(&manifest, &document.to_string())?;
    let (app, _) = project()?;
    crate::sync::sync(&app, None)
}

pub(crate) fn update(package: Option<&str>, tag: Option<&str>) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use std::fs;
    use toml_edit::{DocumentMut, Item, value};

    let (mut app, manifest) = project()?;
    let previous = if let Some(tag) = tag {
        let package = package.context("--tag 需要一个直接 Git 依赖")?;
        let previous = fs::read_to_string(&manifest)?;
        let mut document = previous.parse::<DocumentMut>()?;
        let source = document
            .get_mut("dependencies")
            .and_then(|item| item.get_mut(package))
            .and_then(Item::as_table_like_mut)
            .filter(|source| source.contains_key("git"))
            .with_context(|| format!("{package} 不是直接 Git 依赖"))?;
        source.insert("tag", value(tag));
        crate::plugins::write(&manifest, &document.to_string())?;
        Some(previous)
    } else {
        None
    };
    let discovered = match crate::plugins::discover(&app.root, Some(package)) {
        Ok(discovered) => discovered,
        Err(error) => {
            if let Some(previous) = previous {
                crate::plugins::write(&manifest, &previous)?;
            }
            return Err(error);
        }
    };
    app.plugins = discovered.plugins;
    app.sources = discovered.sources;
    crate::sync::sync(&app, None)
}

fn project() -> anyhow::Result<(crate::app::App, std::path::PathBuf)> {
    let root = std::env::current_dir()?;
    if !root.join("app.toml").is_file() && root.join("plugin.toml").is_file() {
        let manifest = root.join("plugin.toml");
        Ok((crate::app::App::for_plugin(root)?, manifest))
    } else {
        let app = crate::app::App::current()?;
        let manifest = app.root.join("app.toml");
        Ok((app, manifest))
    }
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
        let dependencies = crate::plugins::discover(&app.root, None)?.plugins;
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
