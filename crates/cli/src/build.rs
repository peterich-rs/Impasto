use crate::{
    app::App,
    plugins::{self, NativeBuild},
    toolchain,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct BuildOptions {
    pub debug: bool,
    pub hot: Option<std::cell::RefCell<bundler::WatchLinker>>,
    pub verbose: bool,
    pub trace: bool,
}

pub(crate) struct PlatformArtifact {
    pub path: PathBuf,
    pub plugin_manifest: Option<Vec<u8>>,
}

fn profile(options: &BuildOptions) -> &'static str {
    if options.debug { "debug" } else { "release" }
}
pub fn build(
    target: &str,
    platform: Option<&str>,
    options: BuildOptions,
    ios_simulator: bool,
    export_options: Option<&Path>,
    installed_manifest: Option<&Path>,
    size_request: crate::size::Request<'_>,
) -> Result<()> {
    if size_request.enabled() && target != "bundle" {
        bail!("--analyze-size 仅适用于 build bundle");
    }
    if platform.is_some() && target != "bundle" {
        bail!("--platform 仅适用于 build bundle");
    }
    if ios_simulator && target != "ios" {
        bail!("--ios-simulator 仅适用于 build ios");
    }
    if export_options.is_some() && target != "ipa" {
        bail!("--export-options 仅适用于 build ipa");
    }
    if installed_manifest.is_some() && target != "bundle" {
        bail!("--installed-manifest 仅适用于 build bundle");
    }
    if options.debug && matches!(target, "ios-archive" | "ipa" | "ios-xcframework") {
        bail!("{target} 仅提供 release 产物");
    }
    let app = App::current()?;
    match target {
        "bundle" => {
            let platform = plugins::standalone(&app, platform)?;
            let descriptor_target = platform.or_else(|| app.targets.first().map(String::as_str));
            let native = descriptor_target
                .map(|target| plugins::prepare(&app, target, options.verbose))
                .transpose()?;
            let mut directory = app.root.join("build");
            if let Some(platform) = platform {
                directory = directory
                    .join(platform)
                    .join("bundle")
                    .join(profile(&options));
            }
            let manifest = native.as_ref().map(NativeBuild::linker_manifest);
            let linked = link_source(
                &app.root,
                &app.entry_for(descriptor_target),
                &options,
                manifest.clone(),
            )?;
            if let Some(installed) = installed_manifest {
                native
                    .as_ref()
                    .context("缺少当前目标的原生契约")?
                    .validate_update(&fs::read(installed)?, &linked)?;
            }
            let contract = native
                .as_ref()
                .map(|native| native.snapshot_for(&linked))
                .transpose()?;
            let exports = manifest.map_or_else(Vec::new, owned_exports);
            let output_platform = if installed_manifest.is_some() {
                descriptor_target
            } else {
                platform
            };
            write_bundle(
                &options,
                output_platform,
                &directory,
                exports,
                linked,
                size_request.enabled().then_some(crate::size::Output {
                    request: size_request,
                    app: &app.root,
                }),
            )?;
            if let Some(contract) = contract {
                fs::write(directory.join("plugins-manifest.json"), contract)?;
            }
        }
        "apk" => {
            android(&app, &options, AndroidArtifact::Apk, false)?;
        }
        "aab" | "appbundle" => {
            android(&app, &options, AndroidArtifact::Bundle, false)?;
        }
        "aar" => {
            android(&app, &options, AndroidArtifact::Library, false)?;
        }
        "macos" => {
            macos(&app, &options)?;
        }
        "ios" => {
            let platform = if ios_simulator {
                crate::ios::IosPlatform::Simulator
            } else {
                crate::ios::IosPlatform::Device
            };
            let application = crate::ios::build(&app, &options, platform)?;
            println!("{}", application.display());
        }
        "ios-archive" => {
            let archive = crate::ios::archive(&app, &options)?;
            println!("{}", archive.display());
        }
        "ipa" => {
            let export = export_options.context("build ipa 需要 --export-options <plist>")?;
            let ipa = crate::ios::export_ipa(&app, &options, export)?;
            println!("{}", ipa.display());
        }
        "ios-xcframework" => {
            let sdk = crate::ios::xcframework(&app, &options)?;
            println!("{}", sdk.display());
        }
        _ => bail!(
            "未知构建目标 {target}，可选 bundle、apk、appbundle、aar、macos、ios、ios-archive、ipa、ios-xcframework"
        ),
    }
    Ok(())
}
pub(crate) fn bundle_update(
    app: &App,
    options: &BuildOptions,
    target: &str,
    installed: Option<&[u8]>,
) -> Result<String> {
    let installed = installed.context("缺少已安装壳的原生契约，请重新构建并安装应用")?;
    let native = plugins::watch_manifest(app, target, options.verbose)?;
    let linked = link_source(
        &app.root,
        &app.entry_for(Some(target)),
        options,
        Some(native.linker_manifest()),
    )?;
    native.validate_update(installed, &linked)?;
    Ok(linked.code)
}

fn owned_exports(manifest: bundler::PluginManifest) -> Vec<(String, Vec<String>)> {
    manifest
        .exports()
        .map(|(module, names)| {
            (
                module.to_owned(),
                names.into_iter().map(str::to_owned).collect(),
            )
        })
        .collect()
}

pub(crate) fn manifest_snapshot(native: Option<&NativeBuild>) -> Result<Option<Vec<u8>>> {
    native.map(NativeBuild::snapshot).transpose()
}

pub(crate) fn bundle_for_host(
    app: &App,
    options: &BuildOptions,
    target: &str,
    directory: &Path,
    native: &mut NativeBuild,
) -> Result<PathBuf> {
    let linked = link_source(
        &app.root,
        &app.entry_for(Some(target)),
        options,
        Some(native.linker_manifest()),
    )?;
    if !options.debug {
        native.prune(app, &linked)?;
    }
    let exports = owned_exports(native.linker_manifest());
    let contract = native.snapshot_for(&linked)?;
    let output = write_bundle(options, Some(target), directory, exports, linked, None)?;
    fs::write(directory.join("plugins-manifest.json"), contract)?;
    Ok(output)
}

fn write_bundle(
    options: &BuildOptions,
    target: Option<&str>,
    directory: &Path,
    exports: Vec<(String, Vec<String>)>,
    linked: bundler::Bundle,
    report: Option<crate::size::Output<'_>>,
) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let esm = directory.join("app.js");
    let bytecode = directory.join("app.bin");
    let map = directory.join("app.js.map");
    let mut sizes = std::collections::BTreeMap::new();
    let output = if options.debug {
        fs::write(&esm, &linked.code)?;
        remove_file_if_exists(&bytecode)?;
        esm
    } else {
        let registry = engine::session::ModuleRegistry::new(Vec::new());
        let names = exports
            .iter()
            .filter_map(|(module, names)| {
                let used = linked.native_use.get(module)?;
                let required = names
                    .iter()
                    .filter(|name| used.as_ref().is_none_or(|used| used.contains(*name)))
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                Some((module.as_str(), required))
            })
            .collect::<Vec<_>>();
        let exports = names
            .iter()
            .map(|(module, names)| (*module, names.as_slice()))
            .collect::<Vec<_>>();
        let bytes = if linked.chunks.is_empty() {
            engine::js::compile_with_native_exports(
                &linked.code,
                "app.js",
                true,
                registry,
                &exports,
            )?
        } else {
            let sources = std::iter::once(("app.js".to_owned(), linked.code.clone()))
                .chain(
                    linked
                        .chunks
                        .iter()
                        .map(|(name, chunk)| (name.clone(), chunk.code.clone())),
                )
                .collect();
            engine::js::compile_archive("app.js", sources, registry, &exports)?
        };
        if report.is_some() {
            sizes.clone_from(bytes.module_sizes());
        }
        fs::write(&bytecode, &bytes)?;
        remove_file_if_exists(&esm)?;
        bytecode
    };
    if !options.debug
        && let Some(source_map) = &linked.source_map
    {
        if linked.chunks.is_empty() {
            crate::symbols::write(&directory.join("app.sym"), source_map)?;
        } else {
            crate::symbols::write_chunks(&directory.join("app.sym"), source_map, &linked.chunks)?;
        }
    } else {
        remove_file_if_exists(&directory.join("app.sym"))?;
    }
    if let Some(source_map) = linked
        .source_map
        .as_ref()
        .filter(|_| options.debug || matches!(target, Some("android" | "ios")))
    {
        fs::write(map, source_map)?;
    } else {
        remove_file_if_exists(&map)?;
    }
    if let Some(report) = report {
        crate::size::write(report, &output, &linked, &sizes)?;
    }
    Ok(output)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn link_source(
    root: &Path,
    entry: &Path,
    options: &BuildOptions,
    plugins: Option<bundler::PluginManifest>,
) -> Result<bundler::Bundle> {
    link_source_mode(
        root,
        entry,
        options,
        plugins,
        Default::default(),
        !options.debug,
    )
}

fn link_source_mode(
    root: &Path,
    entry: &Path,
    options: &BuildOptions,
    plugins: Option<bundler::PluginManifest>,
    defines: std::collections::BTreeMap<String, String>,
    split: bool,
) -> Result<bundler::Bundle> {
    let framework = toolchain::framework()?;
    if options.verbose {
        eprintln!("impasto: Oxc + Rolldown {}", entry.display());
    }
    let link = bundler::BundleOptions {
        root: root.to_path_buf(),
        entry: entry.to_path_buf(),
        framework,
        release: !options.debug,
        split,
        hot: options.hot.is_some(),
        plugins,
        defines,
        ..Default::default()
    };
    let linked = match &options.hot {
        Some(watch) => watch.borrow_mut().bundle(link)?,
        None => bundler::bundle(link)?,
    };
    for warning in &linked.warnings {
        eprintln!("{warning}");
    }
    Ok(linked)
}

fn pack_defines(overrides: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut defines = std::collections::BTreeMap::new();
    for define in overrides {
        let (key, expression) = define
            .split_once('=')
            .context("--define 需要 KEY=JS_EXPR")?;
        let mut chars = key.bytes();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'$' | b'_'))
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'$' | b'_'))
            || expression.trim().is_empty()
        {
            bail!("无效的 --define {}", key);
        }
        if defines
            .insert(key.to_owned(), expression.to_owned())
            .is_some()
        {
            bail!("重复的 --define {}", key);
        }
    }
    Ok(defines)
}

pub(crate) fn pack(
    entry: &Path,
    output: &Path,
    options: &BuildOptions,
    overrides: &[String],
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let entry = cwd.join(entry);
    let defines = pack_defines(overrides)?;
    let linked = link_source_mode(&cwd, &entry, options, None, defines, false)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, linked.code)?;
    if let Some(map) = linked.source_map {
        let mut name = output.as_os_str().to_os_string();
        name.push(".map");
        fs::write(name, map)?;
    }
    Ok(())
}

enum AndroidArtifact {
    Apk,
    Bundle,
    Library,
}

pub fn apk(app: &App, options: &BuildOptions) -> Result<PlatformArtifact> {
    android(app, options, AndroidArtifact::Apk, true)
}

fn android(
    app: &App,
    options: &BuildOptions,
    artifact: AndroidArtifact,
    local_install: bool,
) -> Result<PlatformArtifact> {
    app.require("android")?;
    let framework = toolchain::framework()?;
    crate::create::scaffold(app)?;
    let mut native = plugins::prepare(app, "android", options.verbose)?;

    let bundle = bundle_for_host(
        app,
        options,
        "android",
        &app.root.join("build/android/bundle").join(profile(options)),
        &mut native,
    )?;
    let project = app.root.join("android");
    let resources = app
        .root
        .join(".impasto/android/assets")
        .join(profile(options));
    stage_assets(
        app,
        &framework,
        &resources,
        &bundle,
        "android",
        options.trace,
    )?;
    let mode = if options.debug { "Debug" } else { "Release" };
    // The wrapper pins the default. Developers can explicitly use an installed
    // Gradle to build offline after dependencies have been cached.
    let gradle = if let Some(home) = std::env::var_os("GRADLE_HOME") {
        let executable = PathBuf::from(home).join("bin/gradle");
        if !executable.is_file() {
            bail!("GRADLE_HOME 缺少 {}", executable.display());
        }
        executable
    } else {
        project.join("gradlew")
    };
    let mut command = Command::new(gradle);
    command.env("IMPASTO_NATIVE_EXPORTS", native.native_exports());
    let task = match artifact {
        AndroidArtifact::Apk => ":app:assemble",
        AndroidArtifact::Bundle => ":app:bundle",
        AndroidArtifact::Library => ":impasto-host:assemble",
    };
    command.arg("-p").arg(&project).arg(format!("{task}{mode}"));
    if local_install {
        command.arg("-PimpastoLocalSigning=true");
    }
    let companion_modules = native
        .inputs()
        .projects
        .iter()
        .map(|plugin| plugin.gradle_module.as_str())
        .chain(
            (!native.inputs().android_permissions.is_empty())
                .then_some(crate::sync::ANDROID_PERMISSIONS_MODULE),
        )
        .collect::<Vec<_>>();
    if matches!(artifact, AndroidArtifact::Library) {
        command.arg(format!(":app:cargoImpasto{mode}"));
        for module in &companion_modules {
            command.arg(format!(":{module}:assemble{mode}"));
        }
    }
    toolchain::run(command, "构建 Android 应用")?;
    let profile = mode.to_lowercase();
    let output = app.root.join("build/android/gradle");
    let artifact = match artifact {
        AndroidArtifact::Apk => gradle_apk(&output.join("app/outputs/apk").join(&profile))?,
        AndroidArtifact::Bundle => {
            output.join(format!("app/outputs/bundle/{profile}/app-{profile}.aab"))
        }
        AndroidArtifact::Library => {
            package_aar(&app.root, &output, &resources, mode, &companion_modules)?
        }
    };
    if !artifact.is_file() {
        bail!("Gradle 没有生成 {}", artifact.display());
    }
    println!("{}", artifact.display());
    Ok(PlatformArtifact {
        path: artifact,
        plugin_manifest: manifest_snapshot(Some(&native))?,
    })
}

fn package_aar(
    app: &Path,
    gradle: &Path,
    assets: &Path,
    mode: &str,
    modules: &[&str],
) -> Result<PathBuf> {
    let profile = mode.to_ascii_lowercase();
    let stage = tempfile::tempdir_in(gradle)?;
    let aar = stage.path().join("app.aar");
    fs::copy(
        gradle.join(format!(
            "impasto-host/outputs/aar/impasto-host-{profile}.aar"
        )),
        &aar,
    )?;
    copy_tree(assets, &stage.path().join("assets"), &[])?;
    copy_tree(
        &gradle.join(format!("app/generated/jniLibs/cargoImpasto{mode}")),
        &stage.path().join("jni"),
        &[],
    )?;
    let mut zip = Command::new("zip");
    zip.current_dir(stage.path())
        .args(["-q", "-r", "app.aar", "assets", "jni"]);
    toolchain::run(zip, "打包 Android 宿主 AAR")?;
    let directory = app.join("build/android/aar");
    fs::create_dir_all(&directory)?;
    let mut companions = std::collections::BTreeSet::new();
    for module in modules {
        let filename = format!("{module}-{profile}.aar");
        let source = gradle.join(module).join("outputs/aar").join(&filename);
        let companion = format!("plugin-{filename}");
        fs::copy(&source, stage.path().join(&companion))
            .with_context(|| format!("读取插件 AAR {}", source.display()))?;
        companions.insert(companion);
    }
    for filename in &companions {
        fs::rename(stage.path().join(filename), directory.join(filename))?;
    }
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with("plugin-")
            && name.ends_with(&format!("-{profile}.aar"))
            && !companions.contains(name)
        {
            fs::remove_file(entry.path())?;
        }
    }
    let output = directory.join(format!("app-{profile}.aar"));
    fs::rename(aar, &output)?;
    Ok(output)
}

fn gradle_apk(directory: &Path) -> Result<PathBuf> {
    #[derive(serde::Deserialize)]
    struct Metadata {
        elements: Vec<Output>,
    }
    #[derive(serde::Deserialize)]
    struct Output {
        #[serde(rename = "outputFile")]
        file: PathBuf,
    }
    let metadata = directory.join("output-metadata.json");
    let outputs: Metadata = serde_json::from_slice(&fs::read(&metadata)?)?;
    let [output] = outputs.elements.as_slice() else {
        bail!("Gradle 应生成一个 APK，见 {}", metadata.display());
    };
    if output.file.file_name() != Some(output.file.as_os_str()) {
        bail!("Gradle APK 文件名无效，见 {}", metadata.display());
    }
    Ok(directory.join(&output.file))
}

pub(crate) fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}
pub fn macos(app: &App, options: &BuildOptions) -> Result<PlatformArtifact> {
    app.require("macos")?;
    if !cfg!(target_os = "macos") {
        bail!("macos 构建需要 macOS");
    }
    let framework = toolchain::framework()?;
    crate::create::scaffold(app)?;
    let mut native = plugins::prepare(app, "macos", options.verbose)?;
    crate::ios::verify_runner_privacy(
        &app.root.join("macos/Runner/Info.plist"),
        &native.inputs().usage_description_keys,
    )?;
    crate::ios::verify_runner_entitlements(
        &app.root.join("macos/Runner/Runner.entitlements"),
        &native.inputs().required_entitlements,
    )?;
    let profile = profile(options);
    let project = app.root.join("build/macos").join(profile);
    let bundle = bundle_for_host(app, options, "macos", &project.join("bundle"), &mut native)?;
    let rust_target_dir = toolchain::target_dir(&framework.join("target"))?;
    let archive_lock = crate::ios::lock_directory(&rust_target_dir.join(".impasto-macos"))?;
    let (target, architecture) = match std::env::consts::ARCH {
        "aarch64" => ("aarch64-apple-darwin", "arm64"),
        "x86_64" => ("x86_64-apple-darwin", "x86_64"),
        arch => bail!("macOS 不支持主机架构 {arch}"),
    };
    let flags = crate::ios::build_rust(
        &native.workspace,
        &rust_target_dir,
        target,
        options,
        &native.package,
        "13.0",
        native.native_exports(),
    )?;
    let library = project.join("libimpasto_app.a");
    fs::copy(
        rust_target_dir.join(profile).join("libimpasto_app.a"),
        &library,
    )?;
    drop(archive_lock);
    let resources = project.join("resources");
    stage_assets(app, &framework, &resources, &bundle, "macos", options.trace)?;
    let configuration = if options.debug { "Debug" } else { "Release" };
    let settings = darwin_settings(&library, &resources, &flags)?;
    let config = app
        .root
        .join(".impasto/darwin/macos")
        .join(format!("{configuration}.xcconfig"));
    plugins::write(&config, &settings)?;
    let mut command = Command::new("xcodebuild");
    command
        .current_dir(app.root.join("macos"))
        .env("IMPASTO_ROOT", framework)
        .args(["-project", "Runner.xcodeproj", "-scheme", "Runner"])
        .arg("-destination")
        .arg(format!("platform=macOS,arch={architecture}"))
        .arg(format!("ARCHS={architecture}"))
        .args(["-configuration", configuration, "-derivedDataPath"])
        .arg(project.join("xcode"));
    if let Some(window) = &app.macos.window {
        let source = app.root.join("macos/Runner/Info.plist");
        let generated = project.join("Info.plist");
        configure_window_plist(window, &source, &generated)?;
        let mut override_plist = std::ffi::OsString::from("INFOPLIST_FILE=");
        override_plist.push(generated);
        command.arg(override_plist);
    }
    if !options.verbose {
        command.arg("-quiet");
    }
    command.arg("build");
    toolchain::run(command, "构建 macOS 应用")?;
    let application = project
        .join("xcode/Build/Products")
        .join(configuration)
        .join(format!("{}.app", app.name));
    if !application.join("Contents/MacOS/impasto").is_file() {
        bail!("没有生成完整的 macOS 应用 {}", application.display());
    }
    println!("{}", application.display());
    Ok(PlatformArtifact {
        path: application,
        plugin_manifest: manifest_snapshot(Some(&native))?,
    })
}

fn configure_window_plist(
    window: &crate::app::MacWindowConfig,
    source: &Path,
    output: &Path,
) -> Result<()> {
    let directory = output.parent().context("missing Info.plist directory")?;
    let temp = tempfile::NamedTempFile::new_in(directory)?;
    fs::copy(source, temp.path())?;
    for (key, value, kind) in [
        (
            "ImpastoWindowWidth",
            window.width.map(|value| value.to_string()),
            "-float",
        ),
        (
            "ImpastoWindowHeight",
            window.height.map(|value| value.to_string()),
            "-float",
        ),
        (
            "ImpastoWindowMinWidth",
            window.min_width.map(|value| value.to_string()),
            "-float",
        ),
        (
            "ImpastoWindowMinHeight",
            window.min_height.map(|value| value.to_string()),
            "-float",
        ),
        ("ImpastoWindowTitle", window.title.clone(), "-string"),
        (
            "ImpastoWindowAutosaveName",
            window.autosave_name.clone(),
            "-string",
        ),
    ] {
        if let Some(value) = value {
            let mut plist = Command::new("/usr/bin/plutil");
            plist.args(["-replace", key, kind, &value]).arg(temp.path());
            toolchain::run(plist, "写入 macOS 窗口配置")?;
        }
    }
    temp.persist(output)?;
    Ok(())
}

pub(crate) fn darwin_settings(library: &Path, resources: &Path, flags: &str) -> Result<String> {
    let xcconfig_path = |path: &Path| -> Result<String> {
        // xcconfig parses consecutive slashes as a comment outside a variable expansion.
        let path = path.to_str().context("Xcode 构建路径需要 UTF-8")?;
        if path.contains(['\n', '\r', '"']) {
            bail!("Xcode 构建路径包含不支持的字符 {path:?}");
        }
        Ok(path.to_owned())
    };
    let library = xcconfig_path(library)?;
    let resources = xcconfig_path(resources)?;
    Ok(format!(
        "IMPASTO_RESOURCES = {resources}\nOTHER_LDFLAGS = $(inherited) \"{library}\" {flags}\n"
    ))
}

pub(crate) fn stage_assets(
    app: &App,
    framework: &Path,
    resources: &Path,
    bundle: &Path,
    platform: &str,
    trace: bool,
) -> Result<()> {
    let directory = resources.parent().context("缺少应用资源目录")?;
    fs::create_dir_all(directory)?;
    let staging = tempfile::tempdir_in(directory)?;
    let stage = staging.path().join("resources");
    fs::create_dir_all(&stage)?;
    let script = bundle.file_name().context("缺少应用脚本名")?;
    fs::copy(bundle, stage.join(script))?;
    if let Some(manifest) = bundle
        .parent()
        .map(|parent| parent.join("plugins-manifest.json"))
        && manifest.is_file()
    {
        fs::copy(manifest, stage.join("plugins-manifest.json"))?;
    }
    if bundle
        .extension()
        .is_some_and(|extension| extension == "js")
    {
        let map = bundle.with_extension("js.map");
        if map.is_file() {
            fs::copy(&map, stage.join("app.js.map"))?;
        }
    }
    fs::write(stage.join("app.modules"), app.modules(platform)?)?;
    let asset_directory = if platform == "android" {
        "images"
    } else {
        "assets"
    };
    fs::create_dir(stage.join(asset_directory))?;
    copy_tree(app.assets(), &stage.join(asset_directory), &[])?;
    for notice in ["LICENSE", "NOTICE"] {
        fs::copy(framework.join(notice), stage.join(notice))?;
    }
    if platform == "android" {
        fs::create_dir(stage.join("fonts"))?;
        for file in ["Sans.ttf", "NOTICE"] {
            fs::copy(
                framework
                    .join("crates/embedder/android/assets/fonts")
                    .join(file),
                stage.join("fonts").join(file),
            )?;
        }
        if trace {
            fs::write(stage.join("app.trace"), "1")?;
        }
    }
    let previous = staging.path().join("previous");
    if resources.exists() {
        fs::rename(resources, &previous)?;
    }
    if let Err(error) = fs::rename(stage, resources) {
        if previous.exists() {
            fs::rename(previous, resources).context("恢复上一版应用资源")?;
        }
        return Err(error.into());
    }
    Ok(())
}

pub fn copy_tree(from: &Path, to: &Path, skip: &[&str]) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("读取 {}", from.display()))? {
        let entry = entry?;
        if skip.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, skip)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
