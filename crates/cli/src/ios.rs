use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

#[path = "ios_delivery.rs"]
mod delivery;
#[path = "ios_watch.rs"]
mod watch;
pub(crate) use delivery::{archive, export_ipa, xcframework};

use crate::{
    app::App,
    build::{BuildOptions, bundle_in, copy_tree, escape_xml},
    toolchain,
};

pub(crate) const MODULES: &[&str] = &[
    "app-state",
    "async-storage",
    "clipboard",
    "devtools",
    "file-system",
    "linking",
    "permissions",
];

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IosConfig {
    pub(crate) bundle_id: Option<String>,
    pub(crate) build_number: Option<u32>,
    pub(crate) development_team: Option<String>,
    provisioning_profile: Option<String>,
    signing_certificate: Option<String>,
    #[serde(default)]
    url_schemes: Vec<String>,
    #[serde(default)]
    associated_domains: Vec<String>,
    #[serde(default)]
    usage_descriptions: BTreeMap<String, String>,
    #[serde(default)]
    device_families: Vec<u8>,
    pub(crate) privacy_manifest: Option<PathBuf>,
    pub(crate) launch_storyboard: Option<PathBuf>,
    #[serde(default)]
    share_window_state: bool,
    #[serde(default)]
    present_on_external_display: bool,
}

impl IosConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(id) = &self.bundle_id
            && (!id.contains('.')
                || !id.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                }))
        {
            bail!("ios.bundle_id 需要以点分隔，只能包含 ASCII 字母、数字和连字符");
        }
        if self.build_number == Some(0) {
            bail!("ios.build_number 必须大于零");
        }
        if let Some(team) = &self.development_team
            && (team.len() != 10
                || !team
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()))
        {
            bail!("ios.development_team 需要 10 位大写字母或数字");
        }
        if self.signing_certificate.is_some() && self.provisioning_profile.is_none() {
            bail!("ios.signing_certificate 需要同时配置 ios.provisioning_profile");
        }
        for (key, value) in [
            ("provisioning_profile", &self.provisioning_profile),
            ("signing_certificate", &self.signing_certificate),
        ] {
            if let Some(value) = value
                && (value.trim().is_empty()
                    || value.contains("//")
                    || value
                        .chars()
                        .any(|ch| ch.is_control() || matches!(ch, '$' | '"' | '\\')))
            {
                bail!("ios.{key} 包含无效的 Xcode 配置值");
            }
        }
        for scheme in &self.url_schemes {
            if !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                || !scheme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
            {
                bail!("ios.url_schemes 需要合法的 URL scheme");
            }
        }
        for domain in &self.associated_domains {
            if !associated_domain(domain) {
                bail!("ios.associated_domains 包含无效条目 {domain}");
            }
        }
        for (key, text) in &self.usage_descriptions {
            if !key.starts_with("NS")
                || !key.ends_with("UsageDescription")
                || !key.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || text.trim().is_empty()
                || text
                    .chars()
                    .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
            {
                bail!("ios.usage_descriptions 中的 {key} 需要有效的 UsageDescription 键和非空说明");
            }
        }
        if self
            .device_families
            .iter()
            .any(|family| !matches!(family, 1 | 2))
            || self.device_families.len() > 2
            || self
                .device_families
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            bail!("ios.device_families 可包含每个编号一次，1 代表 iPhone，2 代表 iPad");
        }
        for (path, suffix, field) in [
            (&self.privacy_manifest, "xcprivacy", "privacy_manifest"),
            (&self.launch_storyboard, "storyboard", "launch_storyboard"),
        ] {
            if let Some(path) = path
                && path.extension() != Some(suffix.as_ref())
            {
                bail!("ios.{field} 需要 .{suffix} 文件");
            }
        }
        Ok(())
    }
}

fn associated_domain(domain: &str) -> bool {
    let Some((service, host)) = domain.split_once(':') else {
        return false;
    };
    let host = match host.split_once("?mode=") {
        Some((host, "developer" | "managed" | "developer+managed")) => host,
        Some(_) => return false,
        None => host,
    };
    let host = host.strip_prefix("*.").unwrap_or(host);
    matches!(
        service,
        "applinks" | "webcredentials" | "activitycontinuation" | "appclips"
    ) && host.contains('.')
        && host.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum IosPlatform {
    Device,
    Simulator,
}

impl IosPlatform {
    fn sdk(self) -> &'static str {
        match self {
            Self::Device => "iphoneos",
            Self::Simulator => "iphonesimulator",
        }
    }

    fn target(self) -> Result<(&'static str, &'static str)> {
        match (self, std::env::consts::ARCH) {
            (Self::Device, _) => Ok(("aarch64-apple-ios", "arm64")),
            (Self::Simulator, "aarch64") => Ok(("aarch64-apple-ios-sim", "arm64")),
            (Self::Simulator, "x86_64") => Ok(("x86_64-apple-ios", "x86_64")),
            (Self::Simulator, arch) => bail!("iOS 模拟器构建不支持主机架构 {arch}"),
        }
    }
}

fn require_platform(app: &App) -> Result<()> {
    app.require("ios")?;
    if !cfg!(target_os = "macos") {
        bail!("iOS 构建和运行需要 macOS 与 Xcode");
    }
    Ok(())
}

pub(crate) fn build(app: &App, options: &BuildOptions, platform: IosPlatform) -> Result<PathBuf> {
    let prepared = prepare_application(app, options, platform)?;
    let mut command = prepared.xcodebuild();
    command.arg("build");
    toolchain::run(command, "构建 iOS 应用")?;
    let application = prepared.path.join(format!(
        "xcode/Build/Products/{}-{}/Impasto.app",
        prepared.configuration,
        platform.sdk()
    ));
    verify_application(&application, !options.debug)?;
    if platform == IosPlatform::Device {
        verify_signature(&application)?;
    }
    Ok(application)
}

struct PreparedApplication {
    path: PathBuf,
    configuration: &'static str,
    platform: IosPlatform,
    architecture: &'static str,
    verbose: bool,
    _lock: fs::File,
}

impl PreparedApplication {
    fn xcodebuild(&self) -> Command {
        let mut command = Command::new("xcodebuild");
        command
            .current_dir(&self.path)
            .args(["-project", "Impasto.xcodeproj", "-scheme", "Impasto"])
            .args(["-configuration", self.configuration])
            .args(["-sdk", self.platform.sdk(), "-destination"])
            .arg(match self.platform {
                IosPlatform::Device => "generic/platform=iOS",
                IosPlatform::Simulator => "generic/platform=iOS Simulator",
            })
            .args(["-derivedDataPath", "xcode"])
            .arg(format!("ARCHS={}", self.architecture));
        if !self.verbose {
            command.arg("-quiet");
        }
        if self.platform == IosPlatform::Simulator {
            command.arg("CODE_SIGNING_ALLOWED=NO");
        }
        command
    }
}

fn prepare_application(
    app: &App,
    options: &BuildOptions,
    platform: IosPlatform,
) -> Result<PreparedApplication> {
    require_platform(app)?;
    if platform == IosPlatform::Device && app.ios.development_team.is_none() {
        bail!("构建 iOS 真机应用需配置 app.toml 的 ios.development_team；模拟器用 --ios-simulator");
    }
    let modules = app.modules("ios");
    for module in modules.split(',').filter(|name| !name.is_empty()) {
        if !MODULES.contains(&module) && module != "native-plugins" {
            bail!("iOS 宿主尚未实现 {module} 模块");
        }
    }
    let framework = toolchain::framework()?;
    let host = framework.join("crates/embedder/ios");
    let (target, architecture) = platform.target()?;
    let profile = if options.debug { "debug" } else { "release" };
    let configuration = if options.debug { "Debug" } else { "Release" };
    let base = app.root.join("build/ios");
    let path = base.join(platform.sdk()).join(profile);
    let lock = lock_directory(&path)?;

    let bundle = bundle_in(app, options, Some("ios"), &path.join("bundle"))?;
    let rust_target_dir = base.join("target");
    let sdk = impeller_sdk(&framework, target)?;
    let native_flags = build_rust(&framework, &rust_target_dir, target, &sdk, options)?;
    let library = rust_target_dir
        .join(target)
        .join(profile)
        .join("libembedder_ios.a");
    let script = if options.debug { "app.js" } else { "app.bin" };
    stage(
        &path,
        &host,
        app,
        BuildInputs {
            bundle: &bundle,
            library: &library,
            sdk: &sdk,
            script,
            modules: &modules,
            native_flags: &native_flags,
        },
    )?;
    Ok(PreparedApplication {
        path,
        configuration,
        platform,
        architecture,
        verbose: options.verbose,
        _lock: lock,
    })
}

fn lock_directory(path: &Path) -> Result<fs::File> {
    fs::create_dir_all(path)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join(".build.lock"))?;
    lock.lock().context("锁定 iOS 工程产物")?;
    Ok(lock)
}

fn verify_application(application: &Path, release: bool) -> Result<()> {
    let script = if release { "app.bin" } else { "app.js" };
    if [
        "Info.plist",
        "Impasto",
        "PrivacyInfo.xcprivacy",
        "app.modules",
        script,
    ]
    .iter()
    .any(|name| !application.join(name).is_file())
    {
        bail!("没有生成完整的 iOS 应用 {}", application.display());
    }
    if release && application.join("app.js").exists() {
        bail!("iOS Release 应用包含调试脚本");
    }
    Ok(())
}

fn verify_signature(application: &Path) -> Result<()> {
    let mut command = Command::new("codesign");
    command.args(["--verify", "--strict"]).arg(application);
    toolchain::run(command, "校验 iOS 应用签名")
}

fn impeller_sdk(framework: &Path, target: &str) -> Result<PathBuf> {
    let output = Command::new("python3")
        .arg(framework.join("tools/impeller_sdk.py"))
        .args(["prepare", target])
        .env("PYTHONIOENCODING", "UTF-8")
        .output()
        .context("启动 Impeller SDK 准备工具")?;
    Ok(PathBuf::from(
        require_output(output, "准备 iOS Impeller SDK")?.trim(),
    ))
}

fn build_rust(
    framework: &Path,
    target_dir: &Path,
    target: &str,
    sdk: &Path,
    options: &BuildOptions,
) -> Result<String> {
    // rustc bundles the native static archives into libembedder_ios.a. The
    // Swift application's link step still needs the system frameworks/libraries.
    // Read plain diagnostics even when the caller forces Cargo colors in CI.
    let mut command = Command::new("cargo");
    command
        .current_dir(framework)
        .args([
            "rustc",
            "--color",
            "never",
            "--locked",
            "-p",
            "embedder-ios",
            "--target",
            target,
        ])
        .arg("--target-dir")
        .arg(target_dir)
        .env_remove("SDKROOT")
        .env("IPHONEOS_DEPLOYMENT_TARGET", "15.0")
        .env("IMPASTO_IMPELLER_SDK_DIR", sdk);
    if !options.debug {
        // Xcode 27's strip can leave host proc-macro dylibs unloadable by dyld.
        command.args([
            "--release",
            "--config",
            "profile.release.build-override.strip = \"none\"",
        ]);
    }
    command.args(["--", "--print", "native-static-libs"]);
    let output = command.output().context("启动 iOS 宿主的 Cargo 构建")?;
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!("构建 iOS Rust 宿主失败 ({})\n{diagnostics}", output.status);
    }
    if options.verbose {
        eprint!("{diagnostics}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let flags = diagnostics
        .lines()
        .chain(stdout.lines())
        .find_map(|line| {
            line.split_once("native-static-libs:")
                .map(|(_, flags)| flags.trim())
        })
        .context("Cargo 未报告 iOS 静态宿主所需的系统链接参数")?;
    if !flags.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'-' | b'_' | b'+' | b'.')
    }) {
        bail!("Cargo 返回的 iOS 静态链接参数格式无效");
    }
    // This is diagnostic output from our Cargo invocation. Xcode accepts the
    // space-separated -l flags and -framework NAME pairs reported by rustc.
    Ok(flags.to_owned())
}

fn bundle_id(app: &App) -> String {
    app.ios
        .bundle_id
        .clone()
        .unwrap_or_else(|| format!("dev.impasto.{}", app.name.replace('_', "-")))
}

struct BuildInputs<'a> {
    bundle: &'a Path,
    library: &'a Path,
    sdk: &'a Path,
    script: &'a str,
    modules: &'a str,
    native_flags: &'a str,
}

fn swift_sources(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(directory)?
        .map(|result| result.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.extension().and_then(|part| part.to_str()) == Some("swift"));
    files.sort();
    Ok(files)
}

fn add_swift_sources(mut pbx: String, files: &[PathBuf]) -> Result<String> {
    let mut buildfiles = String::new();
    let mut references = String::new();
    let mut group = String::new();
    let mut sources = String::new();
    for (index, path) in files.iter().enumerate() {
        use std::fmt::Write as _;

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("iOS Swift 文件名无效")?;
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            bail!("iOS Swift 文件名包含不支持的字符 {name}");
        }
        let file = format!("B2{index:022X}");
        let build = format!("B1{index:022X}");
        writeln!(
            buildfiles,
            "\t\t{build} = {{isa = PBXBuildFile; fileRef = {file}; }};"
        )?;
        writeln!(
            references,
            "\t\t{file} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = \"host/{name}\"; sourceTree = \"<group>\"; }};"
        )?;
        writeln!(group, "\t\t\t\t{file},")?;
        writeln!(sources, "\t\t\t\t{build},")?;
    }
    for (token, value) in [
        ("@HOST_SWIFT_BUILDFILES@", buildfiles),
        ("@HOST_SWIFT_REFERENCES@", references),
        ("@HOST_SWIFT_GROUP@", group),
        ("@HOST_SWIFT_SOURCES@", sources),
    ] {
        pbx = pbx.replace(token, value.trim_end());
    }
    Ok(pbx)
}

fn plist_json(path: &Path) -> Result<Value> {
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(path)
        .output()?;
    let data = require_output(output, &format!("读取 {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("解析 {}", path.display()))
}

fn write_privacy_manifest(template: &Path, custom: Option<&Path>, output: &Path) -> Result<()> {
    let Some(custom) = custom else {
        fs::copy(template, output)?;
        return Ok(());
    };
    let base = plist_json(template)?;
    let mut manifest = plist_json(custom)?;
    let manifest = manifest
        .as_object_mut()
        .context("iOS 隐私清单需要 plist 字典")?;
    for key in ["NSPrivacyTracking", "NSPrivacyCollectedDataTypes"] {
        manifest.entry(key).or_insert_with(|| base[key].clone());
    }
    let entries = manifest
        .entry("NSPrivacyAccessedAPITypes")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("NSPrivacyAccessedAPITypes 需要数组")?;
    let category_key = "NSPrivacyAccessedAPIType";
    let reason_key = "NSPrivacyAccessedAPITypeReasons";
    for required in base["NSPrivacyAccessedAPITypes"]
        .as_array()
        .context("内置 iOS 隐私清单不完整")?
    {
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry[category_key] == required[category_key])
        {
            let reasons = entry
                .get_mut(reason_key)
                .and_then(Value::as_array_mut)
                .context("iOS 隐私清单缺少 NSPrivacyAccessedAPITypeReasons 数组")?;
            for reason in required[reason_key]
                .as_array()
                .context("内置 iOS 隐私清单不完整")?
            {
                if !reasons.contains(reason) {
                    reasons.push(reason.clone());
                }
            }
        } else {
            entries.push(required.clone());
        }
    }
    let path = output.with_extension("json");
    fs::write(&path, serde_json::to_vec(manifest)?)?;
    let mut convert = Command::new("plutil");
    convert
        .args(["-convert", "xml1", "-o"])
        .arg(output)
        .arg("--")
        .arg(&path);
    let result = toolchain::run(convert, "生成 iOS 隐私清单");
    let cleanup = fs::remove_file(path);
    result?;
    cleanup?;
    Ok(())
}

fn write_associated_domains(project: &Path, domains: &[String]) -> Result<&'static str> {
    if domains.is_empty() {
        return Ok("");
    }
    let domains = domains
        .iter()
        .map(|domain| format!("<string>{}</string>", escape_xml(domain)))
        .collect::<String>();
    fs::write(
        project.join("Impasto.entitlements"),
        plist_xml(&format!(
            "<key>com.apple.developer.associated-domains</key><array>{domains}</array>"
        )),
    )?;
    Ok("CODE_SIGN_ENTITLEMENTS = Impasto.entitlements\n")
}

fn plist_xml(entries: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>{entries}</dict></plist>\n"
    )
}

fn stage(project: &Path, host: &Path, app: &App, inputs: BuildInputs<'_>) -> Result<()> {
    let xcode = project.join("Impasto.xcodeproj");
    fs::create_dir_all(xcode.join("xcshareddata/xcschemes"))?;
    let template = host.join("template");
    let pbx = fs::read_to_string(template.join("project.pbxproj"))?;
    let swift = swift_sources(&host.join("host"))?;
    fs::write(
        xcode.join("project.pbxproj"),
        add_swift_sources(pbx, &swift)?.replace("@APP_SCRIPT@", inputs.script),
    )?;
    fs::copy(
        template.join("Impasto.xcscheme"),
        xcode.join("xcshareddata/xcschemes/Impasto.xcscheme"),
    )?;
    copy_tree(&host.join("host"), &project.join("host"), &[])?;
    if let Some(storyboard) = &app.ios.launch_storyboard {
        let storyboard = app.root.join(storyboard);
        fs::copy(&storyboard, project.join("host/LaunchScreen.storyboard"))
            .with_context(|| format!("复制 iOS 启动画面 {}", storyboard.display()))?;
    }
    fs::create_dir_all(project.join("native"))?;
    fs::copy(
        host.join("include/impasto_ios.h"),
        project.join("native/impasto_ios.h"),
    )?;
    fs::copy(inputs.library, project.join("native/libembedder_ios.a"))?;

    let resources = project.join("resources");
    if resources.exists() {
        fs::remove_dir_all(&resources)?;
    }
    fs::create_dir(&resources)?;
    copy_tree(&app.assets(), &resources.join("assets"), &[])?;
    fs::write(resources.join("app.modules"), inputs.modules)?;
    fs::copy(inputs.bundle, resources.join(inputs.script))?;
    fs::copy(
        inputs.sdk.join("LICENSE"),
        resources.join("Impeller-LICENSE"),
    )?;
    let font_notice = fs::read_to_string(host.join("../android/assets/fonts/NOTICE"))?;
    let license = fs::read_to_string(host.join("../../../LICENSE"))?;
    fs::write(
        resources.join("Roboto-NOTICE"),
        format!("{font_notice}\n{license}"),
    )?;

    write_privacy_manifest(
        &template.join("PrivacyInfo.xcprivacy"),
        app.ios
            .privacy_manifest
            .as_ref()
            .map(|path| app.root.join(path))
            .as_deref(),
        &resources.join("PrivacyInfo.xcprivacy"),
    )?;
    let mut plist = fs::read_to_string(template.join("Info.plist"))?;
    let url_types = if app.ios.url_schemes.is_empty() {
        String::new()
    } else {
        let schemes = app
            .ios
            .url_schemes
            .iter()
            .map(|scheme| format!("<string>{scheme}</string>"))
            .collect::<String>();
        format!(
            "<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLSchemes</key><array>{schemes}</array></dict></array>"
        )
    };
    plist = plist.replace("@DISPLAY_NAME@", &escape_xml(&app.name));
    plist = plist.replace("<!-- @URL_TYPES@ -->", &url_types);
    let usage = app
        .ios
        .usage_descriptions
        .iter()
        .map(|(key, text)| format!("<key>{key}</key><string>{}</string>", escape_xml(text)))
        .collect::<String>();
    plist = plist.replace("<!-- @USAGE_DESCRIPTIONS@ -->", &usage);
    if app.ios.share_window_state {
        plist = plist.replace(
            "<!-- @SHARED_WINDOW_STATE@ -->",
            "<key>ImpastoSharedWindowState</key><true/>",
        );
    }
    if app.ios.present_on_external_display {
        plist = plist.replace(
            "<!-- @EXTERNAL_DISPLAY@ -->",
            "<key>ImpastoPresentOnExternalDisplay</key><true/>",
        );
    }
    fs::write(project.join("Info.plist"), plist)?;
    let domains = write_associated_domains(project, &app.ios.associated_domains)?;
    let signing = app.ios.provisioning_profile.as_ref().map_or_else(
        || "CODE_SIGN_STYLE = Automatic\n".to_owned(),
        |profile| {
            let certificate = app.ios.signing_certificate.as_deref().unwrap_or("Apple Development");
            format!("CODE_SIGN_STYLE = Manual\nPROVISIONING_PROFILE_SPECIFIER = {profile}\nCODE_SIGN_IDENTITY = {certificate}\n")
        }
    );
    let families = match app.ios.device_families.as_slice() {
        [] => "1,2".to_owned(),
        values => values
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(","),
    };
    let build_number = app.ios.build_number.unwrap_or(1);
    let team = app.ios.development_team.as_deref().unwrap_or_default();
    fs::write(
        project.join("BuildSettings.xcconfig"),
        format!(
            "PRODUCT_BUNDLE_IDENTIFIER = {}\nMARKETING_VERSION = {}\nCURRENT_PROJECT_VERSION = {build_number}\nDEVELOPMENT_TEAM = {team}\nTARGETED_DEVICE_FAMILY = {families}\n{signing}{domains}OTHER_LDFLAGS = $(inherited) {}\n",
            bundle_id(app),
            app.version,
            inputs.native_flags,
        ),
    )?;
    Ok(())
}

#[derive(Deserialize)]
struct SimulatorList {
    devices: BTreeMap<String, Vec<Simulator>>,
}

#[derive(Deserialize)]
struct Simulator {
    udid: String,
    state: String,
}

enum RunDevice {
    Physical(String),
    Simulator(Simulator),
}

fn choose_device(requested: Option<&str>) -> Result<RunDevice> {
    if let Some(id) = requested
        && (id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            || id.starts_with('-'))
    {
        bail!("请使用 -d ios:<UDID> 选择有效的 iOS 设备");
    }
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output()
        .context("启动 simctl")?;
    let list: SimulatorList =
        serde_json::from_str(&require_output(output, "读取可用 iOS 模拟器")?)?;
    let mut simulators = list
        .devices
        .into_iter()
        .rev()
        .filter(|(runtime, _)| runtime.starts_with("com.apple.CoreSimulator.SimRuntime.iOS-"))
        .flat_map(|(_, devices)| devices);
    if let Some(id) = requested {
        if let Some(device) = simulators.find(|device| device.udid.eq_ignore_ascii_case(id)) {
            return Ok(RunDevice::Simulator(device));
        }
        let probe = Command::new("xcrun")
            .args(["devicectl", "device", "info", "details", "--device", id])
            .output()
            .context("启动 devicectl")?;
        require_output(probe, "确认 iOS 真机可用")?;
        return Ok(RunDevice::Physical(id.to_owned()));
    }
    let devices: Vec<_> = simulators.collect();
    let mut booted = devices.iter().filter(|device| device.state == "Booted");
    let preferred = booted.next().map(|device| device.udid.as_str());
    if booted.next().is_some() {
        bail!("有多个运行中的 iOS 模拟器，请使用 -d ios:<UDID> 指定目标");
    }
    let preferred = preferred.map(str::to_owned);
    let selected = devices
        .into_iter()
        .find(|device| preferred.as_ref().is_none_or(|id| *id == device.udid))
        .context("找不到 iOS 模拟器；在 Xcode 安装 iOS 运行时，或用 -d ios:<UDID> 选择真机")?;
    Ok(RunDevice::Simulator(selected))
}

impl RunDevice {
    fn platform(&self) -> IosPlatform {
        match self {
            Self::Simulator(_) => IosPlatform::Simulator,
            Self::Physical(_) => IosPlatform::Device,
        }
    }

    fn ready(&self) -> Result<()> {
        if let Self::Simulator(device) = self {
            let mut command = Command::new("xcrun");
            command.args(["simctl", "bootstatus", &device.udid, "-b"]);
            toolchain::run(command, "等待 iOS 模拟器启动")?;
        }
        Ok(())
    }

    fn install_and_launch(
        &self,
        application: &Path,
        package: &str,
        options: &BuildOptions,
        development_token: Option<&str>,
    ) -> Result<()> {
        let mut install = Command::new("xcrun");
        let mut launch = Command::new("xcrun");
        match self {
            Self::Simulator(device) => {
                install.args(["simctl", "install", &device.udid]);
                launch.args(["simctl", "launch", "--terminate-running-process"]);
                if development_token.is_none() && (options.verbose || options.trace) {
                    launch.arg("--console");
                }
                if let Some(token) = development_token {
                    launch.env("SIMCTL_CHILD_IMPASTO_DEV_SESSION", token);
                }
                launch.args([&device.udid, package]);
            }
            Self::Physical(udid) => {
                install.args(["devicectl", "device", "install", "app", "--device", udid]);
                launch.args([
                    "devicectl",
                    "device",
                    "process",
                    "launch",
                    "--device",
                    udid,
                    "--terminate-existing",
                ]);
                if development_token.is_none() && (options.verbose || options.trace) {
                    launch.arg("--console");
                }
                if let Some(token) = development_token {
                    launch.env("DEVICECTL_CHILD_IMPASTO_DEV_SESSION", token);
                }
                launch.arg(package);
            }
        }
        install.arg(application);
        toolchain::run(install, "安装 iOS 应用")?;
        toolchain::run(launch, "启动 iOS 应用")
    }
}

pub(crate) fn run(
    app: App,
    options: &BuildOptions,
    requested: Option<&str>,
    watch: bool,
) -> Result<()> {
    require_platform(&app)?;
    let device = choose_device(requested)?;
    device.ready()?;
    if watch {
        return watch::run(app, options, device);
    }
    let package = bundle_id(&app);
    let path = build(&app, options, device.platform())?;
    device.install_and_launch(&path, &package, options, None)
}

fn require_output(output: Output, action: &str) -> Result<String> {
    if !output.status.success() {
        bail!(
            "{action}失败 ({})\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("{action}返回了无效的 UTF-8"))
}
