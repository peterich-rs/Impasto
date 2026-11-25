use crate::{
    app::App,
    plugins::{self, NativeBuild},
    toolchain,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

pub struct BuildOptions {
    pub debug: bool,
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
) -> Result<()> {
    if platform.is_some() && target != "bundle" {
        bail!("--platform 仅适用于 build bundle");
    }
    if ios_simulator && target != "ios" {
        bail!("--ios-simulator 仅适用于 build ios");
    }
    if export_options.is_some() && target != "ipa" {
        bail!("--export-options 仅适用于 build ipa");
    }
    if options.debug && matches!(target, "ios-archive" | "ipa" | "ios-xcframework") {
        bail!("{target} 仅提供 release 产物");
    }
    let app = App::current()?;
    match target {
        "bundle" => {
            let platform = plugins::standalone(&app, platform)?;
            let native = platform
                .map(|target| plugins::prepare(&app, target, options.verbose))
                .transpose()?
                .flatten();
            let mut directory = app.root.join("build");
            if let Some(platform) = platform {
                directory = directory
                    .join(platform)
                    .join("bundle")
                    .join(profile(&options));
            }
            bundle_in(&app, &options, platform, &directory, native.as_ref())?;
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
            if plugins::configured(&app) {
                bail!("build ios-xcframework 暂不支持带 Rust 插件的应用");
            }
            let sdk = crate::ios::xcframework(&app, &options)?;
            println!("{}", sdk.display());
        }
        _ => bail!(
            "未知构建目标 {target}，可选 bundle、apk、appbundle、aar、macos、ios、ios-archive、ipa、ios-xcframework"
        ),
    }
    Ok(())
}
pub fn bundle(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    bundle_in(app, options, None, &app.root.join("build"), None)
}

pub(crate) fn bundle_update(
    app: &App,
    options: &BuildOptions,
    target: &str,
    directory: &Path,
    installed: Option<&[u8]>,
) -> Result<PathBuf> {
    let manifest = installed
        .map(|bytes| -> Result<_> {
            let mut file = tempfile::NamedTempFile::new()?;
            file.write_all(bytes)?;
            Ok(file)
        })
        .transpose()?;
    pack_bundle(
        app,
        options,
        Some(target),
        directory,
        manifest.as_ref().map(|file| file.path()),
        &[],
    )
}

pub(crate) fn manifest_snapshot(native: Option<&NativeBuild>) -> Result<Option<Vec<u8>>> {
    Ok(native
        .map(|native| fs::read(&native.manifest))
        .transpose()?)
}

pub(crate) fn bundle_in(
    app: &App,
    options: &BuildOptions,
    target: Option<&str>,
    directory: &Path,
    native: Option<&NativeBuild>,
) -> Result<PathBuf> {
    pack_bundle(
        app,
        options,
        target,
        directory,
        native.map(|build| build.manifest.as_path()),
        native.map_or(&[], NativeBuild::descriptors),
    )
}

fn pack_bundle(
    app: &App,
    options: &BuildOptions,
    target: Option<&str>,
    directory: &Path,
    manifest: Option<&Path>,
    descriptors: &[plugins::Descriptor],
) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let esm = directory.join("app.js");
    let bytecode = directory.join("app.bin");
    let map = directory.join("app.js.map");
    let mut command = pack_command(&app.entry, &esm, options)?;
    if let Some(manifest) = manifest {
        command.arg("--plugins-manifest").arg(manifest);
    }
    if matches!(target, Some("android" | "ios")) && !options.debug {
        command.arg("--source-map");
    }
    toolchain::run(command, "打包 TSX")?;
    if options.debug {
        if bytecode.is_file() {
            fs::remove_file(bytecode)?;
        }
        return Ok(esm);
    }
    let code = fs::read_to_string(&esm)?;
    let registry = compile_registry(app, target);
    let exports = descriptors
        .iter()
        .map(|module| {
            (
                module.specifier.as_str(),
                module
                    .exports
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let exports = exports
        .iter()
        .map(|(specifier, names)| (*specifier, names.as_slice()))
        .collect::<Vec<_>>();
    let bytes = engine::js::compile_with_native_exports(&code, "app.js", true, registry, &exports)?;
    fs::write(&bytecode, bytes)?;
    fs::remove_file(esm)?;
    if map.is_file() && !matches!(target, Some("android" | "ios")) {
        fs::remove_file(map)?;
    }
    Ok(bytecode)
}

fn compile_registry(app: &App, target: Option<&str>) -> engine::session::ModuleRegistry {
    let modules = embedder::first_party_modules();
    let Some(target) = target else {
        // A standalone bundle can be linked by a caller supplying its own
        // module registry. The platform package checks its own capabilities.
        return engine::session::ModuleRegistry::new(modules);
    };
    let enabled = app.modules(target);
    if !matches!(target, "macos" | "android" | "ios") {
        return engine::session::ModuleRegistry::new(Vec::new());
    }
    let modules = modules
        .into_iter()
        .filter(|module| {
            let Some(package) = embedder::package_folder(module.specifier) else {
                return false;
            };
            enabled.split(',').any(|name| name == package)
        })
        .collect();
    engine::session::ModuleRegistry::new(modules)
}

pub fn pack(entry: &Path, output: &Path, options: &BuildOptions) -> Result<()> {
    toolchain::run(pack_command(entry, output, options)?, "打包 TSX")
}

fn pack_command(entry: &Path, output: &Path, options: &BuildOptions) -> Result<Command> {
    let mut command = toolchain::tool("pack.mjs", options.verbose)?;
    command.arg(entry).arg(output);
    if !options.debug {
        command.arg("--release");
    }
    Ok(command)
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
    let native = plugins::prepare(app, "android", options.verbose)?;
    let linked = native.as_ref().filter(|native| native.links_native());
    if linked.is_some() && matches!(artifact, AndroidArtifact::Library) {
        bail!("build aar 暂不支持带 Rust 插件的应用");
    }
    let bundle = bundle_in(
        app,
        options,
        Some("android"),
        &app.root.join("build/android/bundle").join(profile(options)),
        native.as_ref(),
    )?;
    let generated = app.root.join("build/android");
    // Replace the generated source sets so removed or renamed host files
    // cannot survive an upgrade. Keep Gradle output and caches reusable.
    for source_set in ["app/src", "host/src"] {
        let old = generated.join(source_set);
        if old.exists() {
            fs::remove_dir_all(&old).with_context(|| format!("清理 {}", old.display()))?;
        }
    }
    copy_tree(
        &framework.join("crates/embedder/android/apk"),
        &generated,
        &["build", ".gradle", "local.properties"],
    )?;
    let gradle = generated.join("app/build.gradle.kts");
    let text=fs::read_to_string(&gradle)?
        .replace("rootProject.projectDir.resolve(\"../../../..\").canonicalFile",&format!("file({})",kotlin(&framework)))
        .replace("repoRoot.resolve(\"apps/gallery-solid/dist/quotes.js\")",&format!("file({})",kotlin(&bundle)))
        .replace("repoRoot.resolve(\"crates/embedder/macos/assets/logo.png\")",&format!("file({})",kotlin(&framework.join("crates/embedder/macos/assets/logo.png"))))
        .replace("applicationId = \"dev.impasto.host\"",&format!("applicationId = \"{}\"",app.android_id()))
        .replace("        minSdk = 24",&format!("        versionName = \"{}\"\n        versionCode = {}\n        minSdk = 24",app.version, app.android.version_code.unwrap_or(1)))
        .replace("script.copyTo(root.resolve(\"quotes.js\"), overwrite = true)","script.copyTo(root.resolve(if (script.extension == \"bin\") \"app.bin\" else \"app.js\"), overwrite = true)");
    let text = if let Some(native) = linked {
        let inputs = native.inputs();
        let sources = inputs
            .sources
            .iter()
            .map(|path| {
                format!(
                    "    sourceSets.getByName(\"main\").java.srcDir(file({}))",
                    kotlin(path)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let dependencies = inputs
            .gradle_dependencies
            .iter()
            .map(|entry| format!("    implementation({entry:?})"))
            .collect::<Vec<_>>()
            .join("\n");
        text.replace("android {", &format!("android {{\n{sources}"))
            .replace(
                "dependencies {",
                &format!("dependencies {{\n{dependencies}"),
            )
            .replace(
                "minSdk = 24",
                &format!("minSdk = {}", inputs.min_sdk.unwrap_or(24).max(24)),
            )
    } else {
        text
    };
    fs::write(&gradle, text)?;
    let stage = generated.join("app/src/main/app-assets");
    for profile in ["debug", "release"] {
        let old = generated.join(format!(
            "app/build/generated/assets/stageApkAssets{}/",
            if profile == "debug" {
                "Debug"
            } else {
                "Release"
            }
        ));
        if old.exists() {
            fs::remove_dir_all(old)?;
        }
    }
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(stage.join("images"))?;
    copy_tree(&app.assets(), &stage.join("images"), &[])?;
    fs::write(stage.join("app.modules"), app.modules("android"))?;
    let trace = stage.join("app.trace");
    if options.trace {
        fs::write(trace, "1")?;
    } else if trace.exists() {
        fs::remove_file(trace)?;
    }
    let text = fs::read_to_string(&gradle)?.replace(
        "assets.setSrcDirs(emptyList<String>())",
        "assets.setSrcDirs(listOf(\"src/main/app-assets\"))",
    );
    fs::write(&gradle, text)?;
    let manifest = generated.join("app/src/main/AndroidManifest.xml");
    let mut filters = String::new();
    for link in &app.android.deep_links {
        let path = link
            .path_prefix
            .as_deref()
            .map_or_else(String::new, |prefix| {
                format!(" android:pathPrefix=\"{}\"", escape_xml(prefix))
            });
        filters.push_str(&format!(
            "            <intent-filter>\n                <action android:name=\"android.intent.action.VIEW\" />\n                <category android:name=\"android.intent.category.DEFAULT\" />\n                <category android:name=\"android.intent.category.BROWSABLE\" />\n                <data android:scheme=\"{}\" android:host=\"{}\"{path} />\n            </intent-filter>\n",
            link.scheme, link.host,
        ));
    }
    let permissions: String = app
        .android
        .permissions
        .iter()
        .chain(
            native
                .iter()
                .flat_map(|native| native.inputs().permissions.iter()),
        )
        .filter(|name| name.as_str() != "android.permission.INTERNET")
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|name| {
            format!(
                "    <uses-permission android:name=\"{}\" />\n",
                escape_xml(name)
            )
        })
        .collect();
    let features = native
        .iter()
        .flat_map(|native| &native.inputs().uses_features)
        .map(|feature| {
            format!(
                "    <uses-feature android:name=\"{}\" android:required=\"{}\" />\n",
                escape_xml(&feature.name),
                feature.required
            )
        })
        .collect::<String>();
    let xml = fs::read_to_string(&manifest)?
        .replace("impasto counter", &app.name)
        .replace(
            "    <application",
            &format!("{permissions}{features}    <application"),
        )
        .replace(
            "        </activity>",
            &format!("{filters}        </activity>"),
        );
    fs::write(manifest, xml)?;
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
        generated.join("gradlew")
    };
    let mut command = Command::new(gradle);
    let task = match artifact {
        AndroidArtifact::Apk => ":app:assemble",
        AndroidArtifact::Bundle => ":app:bundle",
        AndroidArtifact::Library => ":host:assemble",
    };
    command
        .arg("-p")
        .arg(&generated)
        .arg(format!("{task}{mode}"));
    if let Some(native) = linked {
        command.arg(format!(
            "-PimpastoCargoManifest={}",
            native.workspace.join("Cargo.toml").display()
        ));
        command.arg(format!("-PimpastoCargoPackage={}", native.package));
    }
    if !app.android.abis.is_empty() {
        command.arg(format!("-PimpastoAbis={}", app.android.abis.join(",")));
    }
    if local_install {
        command.arg("-PimpastoLocalSigning=true");
    }
    toolchain::run(command, "构建 Android 应用")?;
    let profile = mode.to_lowercase();
    let artifact = match artifact {
        AndroidArtifact::Apk => {
            gradle_apk(&generated.join("app/build/outputs/apk").join(&profile))?
        }
        AndroidArtifact::Bundle => generated.join(format!(
            "app/build/outputs/bundle/{profile}/app-{profile}.aab"
        )),
        AndroidArtifact::Library => {
            generated.join(format!("host/build/outputs/aar/host-{profile}.aar"))
        }
    };
    if !artifact.is_file() {
        bail!("Gradle 没有生成 {}", artifact.display());
    }
    println!("{}", artifact.display());
    Ok(PlatformArtifact {
        path: artifact,
        plugin_manifest: manifest_snapshot(native.as_ref())?,
    })
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
pub fn macos(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    app.require("macos")?;
    if !cfg!(target_os = "macos") {
        bail!("macos 构建需要 macOS");
    }
    let framework = toolchain::framework()?;
    let native = plugins::prepare(app, "macos", options.verbose)?;
    let bundle = bundle_in(
        app,
        options,
        Some("macos"),
        &app.root.join("build/macos/bundle").join(profile(options)),
        native.as_ref(),
    )?;
    let project = app.root.join("build/macos");
    fs::create_dir_all(&project)?;
    let mut command = Command::new("cargo");
    let linked = native.as_ref().filter(|native| native.links_native());
    let (workspace, package, executable) =
        linked.map_or((framework.as_path(), "embedder-macos", "macos"), |native| {
            (
                native.workspace.as_path(),
                native.package.as_str(),
                "impasto-app",
            )
        });
    command
        .current_dir(workspace)
        .arg(if linked.is_some() { "rustc" } else { "build" })
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .args(["--locked", "-p", package]);
    if !options.debug {
        command.arg("--release");
    }
    if let Some(native) = linked {
        command.arg("--");
        add_macos_linker_inputs(&mut command, native, &project)?;
    }
    toolchain::run(command, "构建 macOS 宿主")?;
    let application = project.join(format!("{}.app", app.name));
    let staging = tempfile::tempdir_in(&project)?;
    let built = staging.path().join(format!("{}.app", app.name));
    let contents = built.join("Contents");
    let resources = contents.join("Resources");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(&resources)?;
    let profile = if options.debug { "debug" } else { "release" };
    fs::copy(
        workspace.join("target").join(profile).join(executable),
        contents.join("MacOS/impasto"),
    )?;
    fs::copy(
        bundle,
        resources.join(if options.debug { "app.js" } else { "app.bin" }),
    )?;
    copy_tree(&app.assets(), &resources.join("assets"), &[])?;
    fs::write(resources.join("app.modules"), app.modules("macos"))?;
    for notice in ["LICENSE", "NOTICE"] {
        fs::copy(framework.join(notice), resources.join(notice))?;
    }
    let mut plist_entries = app
        .macos
        .usage_descriptions
        .iter()
        .map(|(name, value)| format!("<key>{name}</key><string>{}</string>", escape_xml(value)))
        .collect::<String>();
    if let Some(minimum) = native
        .as_ref()
        .and_then(|native| native.inputs().min_version.as_deref())
    {
        use std::fmt::Write as _;
        write!(
            plist_entries,
            "<key>LSMinimumSystemVersion</key><string>{}</string>",
            escape_xml(minimum)
        )?;
    }
    fs::write(
        contents.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>impasto</string><key>CFBundleIdentifier</key><string>dev.impasto.{}</string><key>CFBundleName</key><string>{}</string><key>CFBundleShortVersionString</key><string>{}</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/>{plist_entries}</dict></plist>",
            app.name.replace('-', "_"),
            app.name,
            app.version
        ),
    )?;
    let mut sign = Command::new("codesign");
    sign.args(["--force", "--deep", "--sign", "-"]);
    if !app.macos.entitlements.is_empty() {
        let path = project.join("Impasto.entitlements");
        let entries = app
            .macos
            .entitlements
            .iter()
            .map(|(name, value)| format!("<key>{name}</key><{value}/>"))
            .collect::<String>();
        fs::write(&path, crate::ios::plist_xml(&entries))?;
        sign.arg("--entitlements").arg(path);
    }
    sign.arg(&built);
    toolchain::run(sign, "本地签名 macOS 应用")?;
    let previous = staging.path().join("previous.app");
    if application.exists() {
        fs::rename(&application, &previous)?;
    }
    if let Err(error) = fs::rename(built, &application) {
        if previous.exists() {
            fs::rename(previous, &application).context("恢复上一个 macOS 应用包")?;
        }
        return Err(error.into());
    }
    println!("{}", application.display());
    Ok(application)
}
fn add_macos_linker_inputs(
    command: &mut Command,
    native: &NativeBuild,
    project: &Path,
) -> Result<()> {
    let inputs = native.inputs();
    if !inputs.sources.is_empty() {
        command.arg("--check-cfg=cfg(impasto_metadata)");
    }
    if let Some(minimum) = &inputs.min_version {
        command.env("MACOSX_DEPLOYMENT_TARGET", minimum);
    }
    let mut swift = Vec::new();
    for (index, source) in inputs.sources.iter().enumerate() {
        match source.extension().and_then(|part| part.to_str()) {
            Some("swift") => swift.push(source),
            Some("m" | "c") => {
                let object = project.join(format!("plugin-{index}.o"));
                let mut clang = Command::new("xcrun");
                clang
                    .args(["--sdk", "macosx", "clang", "-c"])
                    .arg(source)
                    .arg("-o")
                    .arg(&object);
                if let Some(minimum) = &inputs.min_version {
                    clang.env("MACOSX_DEPLOYMENT_TARGET", minimum);
                }
                toolchain::run(clang, "编译 macOS 插件桥接源码")?;
                command
                    .arg("-C")
                    .arg(format!("link-arg={}", object.display()));
            }
            _ => bail!("macOS 插件源码格式无效: {}", source.display()),
        }
    }
    if !swift.is_empty() {
        let library = project.join("libImpastoPlugins.a");
        let mut swiftc = Command::new("xcrun");
        swiftc
            .args([
                "--sdk",
                "macosx",
                "swiftc",
                "-parse-as-library",
                "-emit-library",
                "-static",
                "-module-name",
                "ImpastoPlugins",
            ])
            .arg("-o")
            .arg(&library)
            .args(swift);
        if let Some(minimum) = &inputs.min_version {
            swiftc.env("MACOSX_DEPLOYMENT_TARGET", minimum);
        }
        toolchain::run(swiftc, "编译 macOS Swift 插件桥接源码")?;
        command
            .arg("-C")
            .arg(format!("link-arg={}", library.display()));
        command.args(["-C", "link-arg=-L/usr/lib/swift"]);
    }
    for framework in &inputs.frameworks {
        command.arg("-l").arg(format!("framework={framework}"));
    }
    for framework in &inputs.weak_frameworks {
        command
            .args(["-C", "link-arg=-weak_framework", "-C"])
            .arg(format!("link-arg={framework}"));
    }
    Ok(())
}

fn kotlin(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
    )
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
