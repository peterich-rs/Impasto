use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::json;

use super::{
    IosPlatform, build_rust, lock_directory, plist_json, plist_xml, prepare_application,
    require_output, require_platform, swift_sources, verify_application, verify_signature,
};
use crate::{
    app::App,
    build::{BuildOptions, bundle_for_host, copy_tree},
    plugins::{self, NativeBuild},
    toolchain,
};

pub(crate) fn archive(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    let prepared = prepare_application(app, options, IosPlatform::Device)?;
    let output = unique_directory(&prepared.path.join("archives"))?;
    let archive = output.join("Impasto.xcarchive");
    let mut command = prepared.xcodebuild();
    command.arg("-archivePath").arg(&archive).arg("archive");
    toolchain::run(command, "归档 iOS 应用")?;
    let application = archive.join("Products/Applications/Impasto.app");
    verify_application(&application, true)?;
    verify_signature(&application)?;

    let binary = application.join("Impasto");
    let dsym = archive.join("dSYMs/Impasto.app.dSYM");
    let uuids = mach_uuids(&binary)?;
    if mach_uuids(&dsym)? != uuids {
        bail!("iOS 归档和 dSYM 的 Mach-O UUID 不一致");
    }
    let map = prepared.path.join("bundle/app.js.map");
    fs::copy(&map, output.join("app.js.map"))
        .with_context(|| format!("复制 iOS JavaScript 源映射 {}", map.display()))?;
    let plist = plist_json(&application.join("Info.plist"))?;
    fs::write(
        output.join("build.json"),
        serde_json::to_vec_pretty(&json!({
            "archive": "Impasto.xcarchive",
            "bundleId": plist["CFBundleIdentifier"],
            "version": plist["CFBundleShortVersionString"],
            "buildNumber": plist["CFBundleVersion"],
            "machUuids": uuids,
            "symbols": "Impasto.xcarchive/dSYMs/Impasto.app.dSYM",
            "sourceMap": "app.js.map",
        }))?,
    )?;
    Ok(archive)
}

pub(crate) fn export_ipa(
    app: &App,
    options: &BuildOptions,
    export_options: &Path,
) -> Result<PathBuf> {
    let export_options = export_options.canonicalize().context("读取 iOS 导出配置")?;
    let settings = plist_json(&export_options)?;
    if !settings.is_object()
        || settings
            .get("destination")
            .is_some_and(|value| value.as_str() != Some("export"))
    {
        bail!("IPA 导出配置必须是本地导出的 plist 字典，destination 可设为 export");
    }
    let archive = archive(app, options)?;
    let destination = archive
        .parent()
        .context("iOS 归档缺少父目录")?
        .join("export");
    let mut command = Command::new("xcodebuild");
    command
        .arg("-exportArchive")
        .arg("-archivePath")
        .arg(&archive)
        .arg("-exportOptionsPlist")
        .arg(export_options)
        .arg("-exportPath")
        .arg(&destination);
    toolchain::run(command, "导出 iOS IPA")?;
    let mut ipas = fs::read_dir(&destination)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    ipas.retain(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "ipa"));
    if ipas.len() != 1 {
        bail!(
            "导出目录 {} 应包含一份 IPA，实际有 {}",
            destination.display(),
            ipas.len()
        );
    }
    let ipa = ipas.pop().context("未生成 IPA")?;
    let contents = require_output(
        Command::new("unzip").arg("-Z1").arg(&ipa).output()?,
        "读取 IPA 包内容",
    )?;
    for file in [
        "Impasto",
        "Info.plist",
        "app.bin",
        "PrivacyInfo.xcprivacy",
        "_CodeSignature/CodeResources",
    ] {
        if !contents
            .lines()
            .any(|line| line == format!("Payload/Impasto.app/{file}"))
        {
            bail!("IPA 缺少已签名的 iOS 应用内容 {file}");
        }
    }
    Ok(ipa)
}

fn mach_uuids(path: &Path) -> Result<Vec<String>> {
    let command = Command::new("xcrun")
        .args(["dwarfdump", "--uuid"])
        .arg(path)
        .output()
        .context("启动 dwarfdump")?;
    let output = require_output(command, "读取 Mach-O UUID")?;
    let mut uuids = output
        .lines()
        .filter_map(|line| line.strip_prefix("UUID: "))
        .filter_map(|row| row.split_ascii_whitespace().next())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    if uuids.is_empty() {
        bail!("{} 缺少 Mach-O UUID", path.display());
    }
    uuids.sort();
    Ok(uuids)
}

pub(super) fn session_id() -> Result<String> {
    let output = require_output(Command::new("uuidgen").output()?, "生成 iOS 构建标识")?;
    let id = output.trim();
    if id.len() != 36
        || !id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'F' | b'a'..=b'f' | b'-'))
    {
        bail!("uuidgen 返回无效的 iOS 会话标识");
    }
    Ok(id.to_ascii_lowercase())
}

fn unique_directory(base: &Path) -> Result<PathBuf> {
    fs::create_dir_all(base)?;
    let path = base.join(session_id()?);
    fs::create_dir(&path)?;
    Ok(path)
}

#[derive(Clone, Copy)]
struct Slice {
    rust: &'static str,
    swift: &'static str,
    swift_module: &'static str,
    simulator: bool,
}

const DEVICE: Slice = Slice {
    rust: "aarch64-apple-ios",
    swift: "arm64-apple-ios15.0",
    swift_module: "arm64-apple-ios",
    simulator: false,
};
const SIM_ARM: Slice = Slice {
    rust: "aarch64-apple-ios-sim",
    swift: "arm64-apple-ios15.0-simulator",
    swift_module: "arm64-apple-ios-simulator",
    simulator: true,
};
const SIM_INTEL: Slice = Slice {
    rust: "x86_64-apple-ios",
    swift: "x86_64-apple-ios15.0-simulator",
    swift_module: "x86_64-apple-ios-simulator",
    simulator: true,
};

const FRAMEWORK: &str = "ImpastoHost";

struct FrameworkSources {
    swift: Vec<PathBuf>,
    header: PathBuf,
    privacy: PathBuf,
    notices: String,
}

impl FrameworkSources {
    fn stage(framework: &Path, directory: &Path) -> Result<Self> {
        let host = framework.join("crates/embedder/ios");
        let swift = directory.join("swift");
        copy_tree(
            &host.join("host"),
            &swift,
            &["App.swift", "LaunchScreen.storyboard"],
        )?;
        fs::write(
            swift.join("ImpastoRustPlugins.swift"),
            r#"
@_silgen_name("impasto_install_app_plugins")
private func installAppPlugins() -> Bool

public enum ImpastoRustPlugins {
    public static func install() -> Bool { installAppPlugins() }
}
"#,
        )?;
        let header = directory.join("impasto_ios.h");
        fs::copy(host.join("include/impasto_ios.h"), &header)?;
        let privacy = directory.join("PrivacyInfo.xcprivacy");
        fs::copy(host.join("template/PrivacyInfo.xcprivacy"), &privacy)?;
        let font = fs::read_to_string(host.join("../android/assets/fonts/NOTICE"))?;
        let license = fs::read_to_string(framework.join("LICENSE"))?;
        Ok(Self {
            swift: swift_sources(&swift)?,
            header,
            privacy,
            notices: format!(
                "{}\n{font}\n{license}",
                fs::read_to_string(framework.join("NOTICE"))?
            ),
        })
    }
}

pub(crate) fn xcframework(app: &App, options: &BuildOptions) -> Result<PathBuf> {
    require_platform(app)?;
    let framework = toolchain::framework()?;
    let rust_target_dir = toolchain::target_dir(&app.root.join("build/ios/target"))?;
    let output = app.root.join("build/ios/sdk");
    let _lock = lock_directory(&output)?;
    let staging = unique_directory(&output)?;
    let mut native = plugins::prepare(app, "ios", options.verbose)?;
    let app_bundle = bundle_for_host(app, options, "ios", &staging.join("Bundle"), &mut native)?;
    crate::build::stage_assets(
        app,
        &framework,
        &staging.join("AppResources"),
        &app_bundle,
        "ios",
        options.trace,
    )?;
    let temporary = tempfile::tempdir_in(&staging)?;
    let sources = FrameworkSources::stage(&framework, &temporary.path().join("sources"))?;
    let _archive_lock = lock_directory(&rust_target_dir.join(".impasto-ios"))?;
    let (device, mut flags) = build_framework(
        &native,
        &sources,
        &rust_target_dir,
        &temporary.path().join("device"),
        &[DEVICE],
        options,
    )?;
    let (simulator, simulator_flags) = build_framework(
        &native,
        &sources,
        &rust_target_dir,
        &temporary.path().join("simulator"),
        &[SIM_ARM, SIM_INTEL],
        options,
    )?;
    flags.extend(simulator_flags);
    let result = staging.join("ImpastoHost.xcframework");
    let mut command = Command::new("xcodebuild");
    command
        .arg("-create-xcframework")
        .arg("-framework")
        .arg(&device)
        .arg("-framework")
        .arg(&simulator)
        .arg("-output")
        .arg(&result);
    toolchain::run(command, "组装 ImpastoHost.xcframework")?;
    verify_xcframework(&result)?;
    write_integration(app, &staging, &native, &flags)?;
    Ok(result)
}

fn build_framework(
    native: &NativeBuild,
    sources: &FrameworkSources,
    rust_target_dir: &Path,
    output: &Path,
    slices: &[Slice],
    options: &BuildOptions,
) -> Result<(PathBuf, std::collections::BTreeSet<String>)> {
    let bundle = output.join("ImpastoHost.framework");
    let modules = bundle.join("Modules/ImpastoHost.swiftmodule");
    let headers = bundle.join("Headers");
    fs::create_dir_all(&modules)?;
    fs::create_dir_all(&headers)?;
    let header = headers.join("impasto_ios.h");
    fs::copy(&sources.header, &header)?;
    fs::write(
        bundle.join("Modules/module.modulemap"),
        "framework module ImpastoHost {\n  umbrella header \"impasto_ios.h\"\n  export *\n}\n",
    )?;
    fs::copy(&sources.privacy, bundle.join("PrivacyInfo.xcprivacy"))?;
    fs::write(bundle.join("ThirdParty-NOTICE"), &sources.notices)?;
    fs::write(
        bundle.join("Info.plist"),
        plist_xml(
            "<key>CFBundlePackageType</key><string>FMWK</string>\
         <key>CFBundleIdentifier</key><string>dev.impasto.ImpastoHost</string>\
         <key>CFBundleName</key><string>ImpastoHost</string>\
         <key>CFBundleExecutable</key><string>ImpastoHost</string>\
         <key>CFBundleShortVersionString</key><string>0.1.0</string>\
         <key>CFBundleVersion</key><string>1</string>\
         <key>MinimumOSVersion</key><string>15.0</string>",
        ),
    )?;
    let mut binaries = Vec::new();
    let mut flags = std::collections::BTreeSet::new();
    for slice in slices {
        let linker = build_rust(
            &native.workspace,
            rust_target_dir,
            slice.rust,
            options,
            &native.package,
            "15.0",
            native.native_exports(),
        )?;
        let platform = if slice.simulator {
            "iphonesimulator"
        } else {
            "iphoneos"
        };
        let library = rust_target_dir
            .join(slice.rust)
            .join("release/libimpasto_app.a");
        let binary = output.join(slice.swift_module);
        let swift_archive = output.join(format!("{}.a", slice.swift_module));
        let mut command = Command::new("xcrun");
        command
            .args([
                "--sdk",
                platform,
                "swiftc",
                "-target",
                slice.swift,
                "-swift-version",
                "5",
            ])
            .args([
                "-O",
                "-emit-module",
                "-emit-library",
                "-static",
                "-enable-library-evolution",
            ])
            .args(["-module-name", FRAMEWORK, "-emit-module-interface-path"])
            .arg(modules.join(format!("{}.swiftinterface", slice.swift_module)))
            .arg("-emit-module-path")
            .arg(modules.join(format!("{}.swiftmodule", slice.swift_module)))
            .arg("-internal-import-bridging-header")
            .arg(&header)
            .args(&sources.swift)
            .arg("-o")
            .arg(&swift_archive);
        toolchain::run(command, "链接 Swift iOS SDK")?;
        let mut combine = Command::new("xcrun");
        combine
            .args(["libtool", "-static", "-o"])
            .arg(&binary)
            .arg(&swift_archive)
            .arg(&library);
        toolchain::run(combine, "合并 Swift 与 Rust 静态库")?;
        flags.insert(linker);
        binaries.push(binary);
    }
    let binary = bundle.join(FRAMEWORK);
    if let [source] = binaries.as_slice() {
        fs::rename(source, &binary)?;
    } else {
        let mut command = Command::new("xcrun");
        command
            .args(["lipo", "-create"])
            .args(&binaries)
            .arg("-output")
            .arg(&binary);
        toolchain::run(command, "合并 iOS 模拟器架构")?;
    }
    Ok((bundle, flags))
}

fn write_integration(
    app: &App,
    output: &Path,
    native: &NativeBuild,
    flags: &std::collections::BTreeSet<String>,
) -> Result<()> {
    use std::fmt::Write as _;

    let host = output.join("crates/embedder/ios/ImpastoHost");
    fs::create_dir_all(&host)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        "../../../../ImpastoHost.xcframework",
        host.join("ImpastoHost.xcframework"),
    )?;
    #[cfg(not(unix))]
    copy_tree(
        &output.join("ImpastoHost.xcframework"),
        &host.join("ImpastoHost.xcframework"),
        &[],
    )?;
    fs::write(
        host.join("Package.swift"),
        r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpastoHost",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoHost", targets: ["ImpastoHost"])],
    targets: [.binaryTarget(name: "ImpastoHost", path: "ImpastoHost.xcframework")]
)
"#,
    )?;
    let mut packages =
        ".package(name: \"ImpastoHost\", path: \"crates/embedder/ios/ImpastoHost\"),".to_owned();
    let mut products = ".product(name: \"ImpastoHost\", package: \"ImpastoHost\"),".to_owned();
    let mut source = String::from("@_exported import ImpastoHost\n");
    let mut calls = String::new();
    for project in &native.inputs().projects {
        let relative = format!("NativePackages/{}", project.package);
        copy_tree(
            &project.path,
            &output.join(&relative),
            &[".build", ".swiftpm", ".git"],
        )?;
        writeln!(
            packages,
            ".package(name: {:?}, path: {relative:?}),",
            project.package
        )?;
        writeln!(
            products,
            ".product(name: {name:?}, package: {name:?}),",
            name = project.package
        )?;
        writeln!(source, "import {}", project.module)?;
        let class = project
            .plugin_class
            .as_deref()
            .context("缺少 Swift 插件注册类")?;
        writeln!(
            calls,
            "        {}.{class}.register(with: host)",
            project.module
        )?;
    }
    let mut settings = std::collections::BTreeSet::new();
    for linker in flags {
        let mut arguments = linker.split_ascii_whitespace();
        while let Some(argument) = arguments.next() {
            if argument == "-framework"
                && let Some(name) = arguments.next()
            {
                settings.insert(format!(".linkedFramework({name:?})"));
            } else if let Some(name) = argument.strip_prefix("-l") {
                settings.insert(format!(".linkedLibrary({name:?})"));
            }
        }
    }
    let settings = settings.into_iter().collect::<Vec<_>>().join(", ");
    fs::write(
        output.join("Package.swift"),
        format!(
            r#"// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "ImpastoGeneratedPlugins",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoGeneratedPlugins", targets: ["ImpastoGeneratedPlugins"])],
    dependencies: [{packages}],
    targets: [.target(name: "ImpastoGeneratedPlugins", dependencies: [{products}], linkerSettings: [{settings}])]
)
"#
        ),
    )?;
    writeln!(
        source,
        r#"
@MainActor
public enum GeneratedPluginRegistrant {{
    public static func prepare() -> Bool {{ ImpastoRustPlugins.install() }}
    public static func register(with host: ImpastoHost) {{
{calls}    }}
}}
"#
    )?;
    let source_path = output.join("Sources/ImpastoGeneratedPlugins");
    fs::create_dir_all(&source_path)?;
    fs::write(source_path.join("GeneratedPluginRegistrant.swift"), source)?;
    fs::write(
        output.join("integration.json"),
        serde_json::to_vec_pretty(&json!({
            "resources": "AppResources",
            "package": "Package.swift",
            "host": "ImpastoHost.xcframework",
            "nativeLinkFlags": flags,
            "impastoRoot": ".",
            "infoPlist": app.ios.usage_descriptions.iter()
                .filter(|(key, _)| native.inputs().usage_description_keys.contains(*key))
                .collect::<std::collections::BTreeMap<_, _>>(),
        }))?,
    )?;
    Ok(())
}

fn verify_xcframework(framework: &Path) -> Result<()> {
    let plist = plist_json(&framework.join("Info.plist"))?;
    let libraries = plist["AvailableLibraries"]
        .as_array()
        .context("XCFramework 缺少 AvailableLibraries")?;
    for (variant, architectures) in [
        (None, &["arm64"][..]),
        (Some("simulator"), &["arm64", "x86_64"][..]),
    ] {
        let slice = libraries
            .iter()
            .find(|value| {
                value["SupportedPlatform"] == "ios"
                    && value["SupportedPlatformVariant"].as_str() == variant
            })
            .context("XCFramework 缺少所需 iOS 平台切片")?;
        let actual = slice["SupportedArchitectures"]
            .as_array()
            .context("XCFramework 缺少架构列表")?;
        if actual.len() != architectures.len()
            || !architectures
                .iter()
                .all(|arch| actual.contains(&json!(arch)))
        {
            bail!("XCFramework 切片架构不完整 {actual:?}");
        }
        let id = slice["LibraryIdentifier"]
            .as_str()
            .context("XCFramework 缺少切片标识")?;
        let bundle = framework.join(id).join("ImpastoHost.framework");
        if !bundle.join(FRAMEWORK).is_file() || !bundle.join("Headers/impasto_ios.h").is_file() {
            bail!("XCFramework 切片 {id} 缺少二进制或公开头文件");
        }
        let modules = bundle.join("Modules/ImpastoHost.swiftmodule");
        for arch in architectures {
            let suffix = if variant.is_some() { "-simulator" } else { "" };
            if !modules
                .join(format!("{arch}-apple-ios{suffix}.swiftinterface"))
                .is_file()
            {
                bail!("XCFramework 切片 {id} 缺少 {arch} Swift 模块接口");
            }
        }
    }
    Ok(())
}
