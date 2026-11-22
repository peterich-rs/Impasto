use crate::{app::App, toolchain};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct BuildOptions {
    pub debug: bool,
    pub verbose: bool,
    pub trace: bool,
}
pub fn build(
    target: &str,
    options: BuildOptions,
    ios_simulator: bool,
    export_options: Option<&Path>,
) -> Result<()> {
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
            bundle(&app, &options)?;
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
pub fn bundle(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    bundle_for(app, options, None)
}

fn bundle_for(app: &App, options: &BuildOptions, target: Option<&str>) -> Result<PathBuf> {
    bundle_in(app, options, target, &app.root.join("build"))
}

pub(crate) fn bundle_in(
    app: &App,
    options: &BuildOptions,
    target: Option<&str>,
    directory: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let esm = directory.join("app.js");
    let bytecode = directory.join("app.bin");
    let map = directory.join("app.js.map");
    let mut command = pack_command(&app.entry, &esm, options)?;
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
    let bytes = engine::js::compile_with_registry(&code, "app.js", true, registry)?;
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

pub fn apk(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    android(app, options, AndroidArtifact::Apk, true)
}

fn android(
    app: &App,
    options: &BuildOptions,
    artifact: AndroidArtifact,
    local_install: bool,
) -> Result<PathBuf> {
    app.require("android")?;
    let framework = toolchain::framework()?;
    let bundle = bundle_for(app, options, Some("android"))?;
    let generated = app.root.join("build/android");
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
        .filter(|name| name.as_str() != "android.permission.INTERNET")
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|name| format!("    <uses-permission android:name=\"{name}\" />\n"))
        .collect();
    let xml = fs::read_to_string(&manifest)?
        .replace("impasto counter", &app.name)
        .replace(
            "    <application",
            &format!("{permissions}    <application"),
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
    Ok(artifact)
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
    let bundle = bundle_for(app, options, Some("macos"))?;
    let project = app.root.join("build/macos");
    fs::create_dir_all(&project)?;
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    };
    let prepared = Command::new("python3")
        .arg(framework.join("tools/impeller_sdk.py"))
        .args(["prepare", target])
        .env("PYTHONIOENCODING", "UTF-8")
        .output()
        .context("启动 Impeller SDK 准备工具")?;
    if !prepared.status.success() {
        bail!("{}", String::from_utf8_lossy(&prepared.stderr).trim());
    }
    let sdk = PathBuf::from(std::str::from_utf8(&prepared.stdout)?.trim());
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(framework.join("Cargo.toml"))
        .args(["-p", "embedder-macos", "--features", "impeller"])
        .env("IMPASTO_IMPELLER_SDK_DIR", &sdk);
    if !options.debug {
        command.arg("--release");
    }
    toolchain::run(command, "构建 macOS 宿主")?;
    let application = project.join(format!("{}.app", app.name));
    let contents = application.join("Contents");
    let resources = contents.join("Resources");
    if contents.exists() {
        fs::remove_dir_all(&contents)?;
    }
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Frameworks"))?;
    fs::create_dir_all(&resources)?;
    let profile = if options.debug { "debug" } else { "release" };
    fs::copy(
        framework.join(format!("target/{profile}/macos")),
        contents.join("MacOS/impasto"),
    )?;
    fs::copy(
        bundle,
        resources.join(if options.debug { "app.js" } else { "app.bin" }),
    )?;
    copy_tree(&app.assets(), &resources.join("assets"), &[])?;
    fs::write(resources.join("app.modules"), app.modules("macos"))?;
    let library = sdk.join("lib/libimpeller.dylib");
    fs::copy(&library, contents.join("Frameworks/libimpeller.dylib"))
        .with_context(|| format!("复制 {}", library.display()))?;
    fs::copy(sdk.join("LICENSE"), resources.join("Impeller-LICENSE"))?;
    fs::write(
        contents.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>impasto</string><key>CFBundleIdentifier</key><string>dev.impasto.{}</string><key>CFBundleName</key><string>{}</string><key>CFBundleShortVersionString</key><string>{}</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>",
            app.name.replace('-', "_"),
            app.name,
            app.version
        ),
    )?;
    let mut sign = Command::new("codesign");
    sign.args(["--force", "--deep", "--sign", "-"])
        .arg(&application);
    toolchain::run(sign, "本地签名 macOS 应用")?;
    println!("{}", application.display());
    Ok(application)
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
