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
    build::{BuildOptions, PlatformArtifact, bundle_for_host, escape_xml, manifest_snapshot},
    plugins::{self, NativeBuild},
    toolchain,
};

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
    pub(crate) usage_descriptions: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) entitlements: BTreeMap<String, bool>,
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
        crate::app::validate_entitlements("ios", &self.entitlements)?;
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
    build_artifact(app, options, platform).map(|built| built.path)
}

fn build_artifact(
    app: &App,
    options: &BuildOptions,
    platform: IosPlatform,
) -> Result<PlatformArtifact> {
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
    Ok(PlatformArtifact {
        path: application,
        plugin_manifest: prepared.plugin_manifest,
    })
}

struct PreparedApplication {
    path: PathBuf,
    project: PathBuf,
    framework: PathBuf,
    configuration: &'static str,
    platform: IosPlatform,
    architecture: &'static str,
    verbose: bool,
    plugin_manifest: Option<Vec<u8>>,
    _lock: fs::File,
    _native: NativeBuild,
}

impl PreparedApplication {
    fn xcodebuild(&self) -> Command {
        let mut command = Command::new("xcodebuild");
        command
            .current_dir(&self.project)
            .env("IMPASTO_ROOT", &self.framework)
            .args(["-project", "Runner.xcodeproj", "-scheme", "Runner"])
            .args(["-configuration", self.configuration])
            .args(["-sdk", self.platform.sdk(), "-destination"])
            .arg(match self.platform {
                IosPlatform::Device => "generic/platform=iOS",
                IosPlatform::Simulator => "generic/platform=iOS Simulator",
            })
            .arg("-derivedDataPath")
            .arg(self.path.join("xcode"))
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
    let framework = toolchain::framework()?;
    crate::create::scaffold(app)?;
    let (target, architecture) = platform.target()?;
    let profile = if options.debug { "debug" } else { "release" };
    let configuration = if options.debug { "Debug" } else { "Release" };
    let base = app.root.join("build/ios");
    let path = base.join(platform.sdk()).join(profile);
    let lock = lock_directory(&path)?;
    let mut native = plugins::prepare(app, "ios", options.verbose)?;
    verify_runner_privacy(
        &app.root.join("ios/Runner/Info.plist"),
        &native.inputs().usage_description_keys,
    )?;
    let minimum = "15.0";
    let bundle = bundle_for_host(app, options, "ios", &path.join("bundle"), &mut native)?;
    let rust_target_dir = toolchain::target_dir(&base.join("target"))?;
    let archive_lock = lock_directory(&rust_target_dir.join(".impasto-ios"))?;
    let flags = build_rust(
        &native.workspace,
        &rust_target_dir,
        target,
        options,
        &native.package,
        minimum,
        native.native_exports(),
    )?;
    let library = path.join("libimpasto_app.a");
    fs::copy(
        rust_target_dir
            .join(target)
            .join(profile)
            .join("libimpasto_app.a"),
        &library,
    )?;
    drop(archive_lock);
    let resources = path.join("resources");
    crate::build::stage_assets(app, &framework, &resources, &bundle, "ios", options.trace)?;
    let glue = app.root.join(".impasto/darwin/ios");
    let sdk = platform.sdk();
    let settings = crate::build::darwin_settings(&library, &resources, &flags)?;
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
    let domains = write_entitlements(&glue, &app.ios)?;
    let team = app.ios.development_team.as_deref().unwrap_or_default();
    let sdk_settings = settings
        .lines()
        .map(|line| line.replacen(" = ", &format!("[sdk={sdk}*] = "), 1) + "\n")
        .collect::<String>();
    plugins::write(
        &glue.join(format!("{configuration}.{sdk}.xcconfig")),
        &sdk_settings,
    )?;
    plugins::write(
        &glue.join(format!("{configuration}.xcconfig")),
        &format!(
            "DEVELOPMENT_TEAM = {team}\nTARGETED_DEVICE_FAMILY = {families}\n{signing}{domains}#include? \"{configuration}.iphoneos.xcconfig\"\n#include? \"{configuration}.iphonesimulator.xcconfig\"\n",
        ),
    )?;
    let plugin_manifest = manifest_snapshot(Some(&native))?;
    Ok(PreparedApplication {
        path,
        project: app.root.join("ios"),
        framework,
        configuration,
        platform,
        architecture,
        verbose: options.verbose,
        plugin_manifest,
        _lock: lock,
        _native: native,
    })
}

pub(crate) fn lock_directory(path: &Path) -> Result<fs::File> {
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

pub(crate) fn build_rust(
    framework: &Path,
    target_dir: &Path,
    target: &str,
    options: &BuildOptions,
    package: &str,
    minimum: &str,
    native_exports: &str,
) -> Result<String> {
    // rustc bundles the native static archives into libembedder_ios.a. The
    // Swift application's link step still needs the system frameworks/libraries.
    // Read plain diagnostics even when the caller forces Cargo colors in CI.
    let mut command = Command::new("cargo");
    command
        .current_dir(framework)
        .args(["rustc", "--color", "never", "--locked", "-p", package])
        .arg("--target-dir")
        .arg(target_dir)
        .env_remove("SDKROOT")
        .env("IMPASTO_NATIVE_EXPORTS", native_exports)
        .env(
            if target.ends_with("apple-darwin") {
                "MACOSX_DEPLOYMENT_TARGET"
            } else {
                "IPHONEOS_DEPLOYMENT_TARGET"
            },
            minimum,
        );
    if !target.ends_with("apple-darwin") {
        command.args(["--target", target]);
    }
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

pub(crate) fn bundle_id(app: &App) -> String {
    app.ios
        .bundle_id
        .clone()
        .unwrap_or_else(|| format!("dev.impasto.{}", app.name.replace('_', "-")))
}

fn swift_sources(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(directory)?
        .map(|result| result.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.extension().and_then(|part| part.to_str()) == Some("swift"));
    files.sort();
    Ok(files)
}

pub(crate) fn verify_runner_privacy(
    plist: &Path,
    keys: &std::collections::BTreeSet<String>,
) -> Result<()> {
    if !keys.is_empty() {
        let document = plist_json(plist)?;
        for key in keys {
            if !document[key]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            {
                bail!("{} 缺少插件所需的用途说明 {key}", plist.display());
            }
        }
    }
    Ok(())
}

pub(crate) fn verify_runner_entitlements(
    plist: &Path,
    keys: &std::collections::BTreeSet<String>,
) -> Result<()> {
    if !keys.is_empty() {
        let document = plist_json(plist)?;
        for key in keys {
            if document[key].as_bool() != Some(true) {
                bail!("{} 缺少插件所需的 entitlement {key}", plist.display());
            }
        }
    }
    Ok(())
}

fn plist_json(path: &Path) -> Result<Value> {
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(path)
        .output()?;
    let data = require_output(output, &format!("读取 {}", path.display()))?;
    serde_json::from_str(&data).with_context(|| format!("解析 {}", path.display()))
}

pub(crate) fn write_privacy_manifest(
    template: &Path,
    custom: Option<&Path>,
    output: &Path,
) -> Result<()> {
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

fn write_entitlements(project: &Path, config: &IosConfig) -> Result<&'static str> {
    if config.associated_domains.is_empty() && config.entitlements.is_empty() {
        return Ok("");
    }
    let domains = config
        .associated_domains
        .iter()
        .map(|domain| format!("<string>{}</string>", escape_xml(domain)))
        .collect::<String>();
    let mut entries = String::new();
    if !domains.is_empty() {
        entries.push_str(&format!(
            "<key>com.apple.developer.associated-domains</key><array>{domains}</array>"
        ));
    }
    for (key, enabled) in &config.entitlements {
        entries.push_str(&format!("<key>{key}</key><{enabled}/>"));
    }
    fs::write(project.join("Impasto.entitlements"), plist_xml(&entries))?;
    Ok("CODE_SIGN_ENTITLEMENTS = $(SRCROOT)/../.impasto/darwin/ios/Impasto.entitlements\n")
}

pub(crate) fn plist_xml(entries: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>{entries}</dict></plist>\n"
    )
}

pub(crate) fn app_plist(app: &App, host: &Path) -> Result<String> {
    let mut plist = fs::read_to_string(host.join("template/Info.plist"))?;
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
    Ok(plist)
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
        app: &App,
        options: &BuildOptions,
        development_token: Option<&str>,
        attach: bool,
    ) -> Result<()> {
        let package = bundle_id(app);
        let mut install = Command::new("xcrun");
        let mut launch = Command::new("xcrun");
        let mut device_config = None;
        if options.debug {
            let (prefix, config) = match self {
                Self::Simulator(_) => {
                    let root = crate::devtools::discovery_root();
                    crate::devtools::private_dir(&root)?;
                    launch.env("SIMCTL_CHILD_IMPASTO_DEVTOOLS_DISCOVERY_DIR", root);
                    launch.env(
                        "SIMCTL_CHILD_IMPASTO_DEVTOOLS_SOURCE_MAP",
                        application.join("app.js.map"),
                    );
                    ("SIMCTL_CHILD", crate::devtools::config()?)
                }
                Self::Physical(_) => ("DEVICECTL_CHILD", crate::devtools::device_config()?),
            };
            if std::env::var("IMPASTO_DEVTOOLS").as_deref() == Ok("0") {
                launch.env(format!("{prefix}_IMPASTO_DEVTOOLS"), "0");
            }
            launch.env(format!("{prefix}_IMPASTO_DEVTOOLS_APP_NAME"), &app.name);
            launch.env(format!("{prefix}_IMPASTO_DEVTOOLS_PROJECT"), &app.root);
            launch.env(
                format!("{prefix}_IMPASTO_DEVTOOLS_FRAMEWORK"),
                toolchain::framework()?,
            );
            if let Some((port, token)) = config {
                launch.env(format!("{prefix}_IMPASTO_DEVTOOLS_PORT"), port.to_string());
                launch.env(format!("{prefix}_IMPASTO_DEVTOOLS_TOKEN"), &token);
                device_config = Some((port, token));
            }
        }
        match self {
            Self::Simulator(device) => {
                install.args(["simctl", "install", &device.udid]);
                launch.args(["simctl", "launch", "--terminate-running-process"]);
                if !attach && development_token.is_none() && (options.verbose || options.trace) {
                    launch.arg("--console");
                }
                if let Some(token) = development_token {
                    launch.env("SIMCTL_CHILD_IMPASTO_DEV_SESSION", token);
                }
                launch.args([&device.udid, &package]);
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
                if !attach && development_token.is_none() && (options.verbose || options.trace) {
                    launch.arg("--console");
                }
                if let Some(token) = development_token {
                    launch.env("DEVICECTL_CHILD_IMPASTO_DEV_SESSION", token);
                }
                launch.arg(&package);
            }
        }
        install.arg(application);
        toolchain::run(install, "安装 iOS 应用")?;
        let needs_output = attach && matches!(self, Self::Simulator(_));
        let output = if needs_output {
            Some(require_output(launch.output()?, "启动 iOS 应用")?)
        } else {
            toolchain::run(launch, "启动 iOS 应用")?;
            None
        };
        let target = match self {
            Self::Physical(udid) => device_config
                .map(|config| {
                    crate::devtools::register_device(
                        devtools_protocol::DeviceKind::Ios,
                        udid,
                        &package,
                        config,
                        &app.root,
                    )
                })
                .transpose()?,
            Self::Simulator(_) => output.as_deref().and_then(|text| {
                text.lines()
                    .find_map(|line| {
                        let (name, pid) = line.split_once(':')?;
                        (name.trim() == package)
                            .then_some(pid.trim())
                            .filter(|pid| pid.parse::<u32>().is_ok())
                    })
                    .map(str::to_owned)
            }),
        };
        if attach {
            crate::devtools::launch_native(Some(
                target
                    .as_deref()
                    .context("iOS DevTools session unavailable")?,
            ))?;
        }
        Ok(())
    }
}

pub(crate) fn run(
    app: App,
    options: &BuildOptions,
    requested: Option<&str>,
    watch: bool,
    attach: bool,
) -> Result<()> {
    require_platform(&app)?;
    let device = choose_device(requested)?;
    device.ready()?;
    if watch {
        return watch::run(app, options, device, attach);
    }
    let path = build(&app, options, device.platform())?;
    device.install_and_launch(&path, &app, options, None, attach)
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
