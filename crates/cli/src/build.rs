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
pub fn build(target: &str, options: BuildOptions) -> Result<()> {
    let app = App::current()?;
    match target {
        "bundle" => {
            bundle(&app, &options)?;
        }
        "apk" => {
            apk(&app, &options)?;
        }
        "macos" => {
            macos(&app, &options)?;
        }
        "ios" => {
            let generated = app.root.join("build/ios");
            fs::create_dir_all(&generated)?;
            fs::write(
                generated.join("README.txt"),
                "crates/ios 不存在；预构建 SDK 没有 iOS 列。此目录不是可安装的工程。\n",
            )?;
            bail!("crates/ios 不存在，不能生成可安装的 iOS 包");
        }
        _ => bail!("未知构建目标 {target}，可选 bundle、apk、macos、ios"),
    }
    Ok(())
}
pub fn bundle(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    bundle_for(app, options, None)
}

fn bundle_for(app: &App, options: &BuildOptions, target: Option<&str>) -> Result<PathBuf> {
    fs::create_dir_all(app.root.join("build"))?;
    let esm = app.root.join("build/app.js");
    pack(&app.entry, &esm, options)?;
    let output = app.output(options.debug);
    if !options.debug {
        let code = fs::read_to_string(&esm)?;
        let registry = compile_registry(app, target);
        let bytes = quickjs::compile_with_registry(&code, "app.js", true, registry)?;
        fs::write(&output, bytes)?;
        fs::remove_file(esm)?;
        let map = app.root.join("build/app.js.map");
        if map.is_file() {
            fs::remove_file(map)?;
        }
    } else {
        let stale = app.output(false);
        if stale.is_file() {
            fs::remove_file(stale)?;
        }
    }
    Ok(output)
}

fn compile_registry(app: &App, target: Option<&str>) -> session::ModuleRegistry {
    let modules = shell::first_party_modules();
    let Some(target) = target else {
        // A standalone bundle can be linked by a caller supplying its own
        // module registry. The platform package checks its own capabilities.
        return session::ModuleRegistry::new(modules);
    };
    let enabled = app.modules(target);
    let modules = modules
        .into_iter()
        .filter(|module| {
            let Some(package) = shell::package_folder(module.specifier) else {
                return false;
            };
            host_has_module(target, package) && enabled.split(',').any(|name| name == package)
        })
        .collect();
    session::ModuleRegistry::new(modules)
}

fn host_has_module(target: &str, package: &str) -> bool {
    match target {
        "macos" => true,
        "android" => package != "permissions",
        _ => false,
    }
}

pub fn pack(entry: &Path, output: &Path, options: &BuildOptions) -> Result<()> {
    let mut command = toolchain::tool("pack.mjs", options.verbose)?;
    command.arg(entry).arg(output);
    if !options.debug {
        command.arg("--release");
    }
    toolchain::run(command, "打包 TSX")
}
pub fn apk(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    app.require("android")?;
    let framework = toolchain::framework()?;
    let bundle = bundle_for(app, options, Some("android"))?;
    let generated = app.root.join("build/android");
    copy_tree(
        &framework.join("crates/android/apk"),
        &generated,
        &["build", ".gradle", "local.properties"],
    )?;
    let gradle = generated.join("app/build.gradle.kts");
    let text=fs::read_to_string(&gradle)?
        .replace("rootProject.projectDir.resolve(\"../../..\").canonicalFile",&format!("file({})",kotlin(&framework)))
        .replace("repoRoot.resolve(\"apps/gallery-solid/dist/quotes.js\")",&format!("file({})",kotlin(&bundle)))
        .replace("repoRoot.resolve(\"crates/macos/assets/logo.png\")",&format!("file({})",kotlin(&framework.join("crates/macos/assets/logo.png"))))
        .replace("applicationId = \"dev.impasto.host\"",&format!("applicationId = \"dev.impasto.{}\"",app.name.replace('-',"_")))
        .replace("        minSdk = 24",&format!("        versionName = \"{}\"\n        versionCode = 1\n        minSdk = 24",app.version))
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
    let xml = fs::read_to_string(&manifest)?.replace("impasto counter", &app.name);
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
    command
        .arg("-p")
        .arg(&generated)
        .arg(format!("assemble{mode}"));
    toolchain::run(command, "构建 APK")?;
    let profile = mode.to_lowercase();
    let artifact = generated.join(format!("app/build/outputs/apk/{profile}/app-{profile}.apk"));
    if !artifact.is_file() {
        bail!("Gradle 没有生成 {}", artifact.display());
    }
    println!("{}", artifact.display());
    Ok(artifact)
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
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(framework.join("Cargo.toml"))
        .args(["-p", "macos"]);
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
    let sdk = framework.join("third_party/impeller-sdk/darwin-arm64/lib/libimpeller.dylib");
    fs::copy(&sdk, contents.join("Frameworks/libimpeller.dylib"))
        .with_context(|| format!("复制 {}", sdk.display()))?;
    let mut rpath = Command::new("install_name_tool");
    rpath
        .args(["-add_rpath", "@executable_path/../Frameworks"])
        .arg(contents.join("MacOS/impasto"));
    toolchain::run(rpath, "设置应用 rpath")?;
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
