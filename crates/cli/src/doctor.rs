//! Inspect local prerequisites and report fixes without invoking a build.

use std::path::PathBuf;
use std::process::Command;

pub(crate) fn run() {
    check(
        "Rust",
        output("rustc", &["--version"]),
        "从 https://rustup.rs 安装 Rust",
    );
    let targets = output("rustup", &["target", "list", "--installed"]).unwrap_or_default();
    let mut expected = vec!["aarch64-linux-android", "x86_64-linux-android"];
    if cfg!(target_os = "macos") {
        expected.extend(["aarch64-apple-ios", "aarch64-apple-ios-sim"]);
    }
    for target in expected {
        let installed = targets
            .split(" · ")
            .any(|line| line == target)
            .then_some("installed".into());
        check(
            target,
            installed,
            &format!("运行 rustup target add {target}"),
        );
    }
    check(
        "adb",
        output("adb", &["version"]),
        "安装 Android SDK Platform-Tools 并加入 PATH",
    );
    let sdk = std::env::var_os("ANDROID_HOME")
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .or_else(default_sdk)
        .filter(|path| path.is_dir());
    check(
        "Android SDK",
        sdk.as_ref().map(|path| path.display().to_string()),
        "设置 ANDROID_HOME",
    );
    let ndk = sdk.as_ref().and_then(|path| {
        let path = path.join("ndk");
        std::fs::read_dir(&path)
            .ok()?
            .flatten()
            .any(|entry| entry.path().join("source.properties").is_file())
            .then(|| path.display().to_string())
    });
    check(
        "Android NDK",
        ndk,
        "在 Android Studio SDK Manager 安装 NDK (Side by side)",
    );
    let connected = crate::devices::android().ok().map(|devices| {
        let ready = devices
            .iter()
            .filter(|device| device.state == "device")
            .count();
        format!("{ready} ready / {} detected", devices.len())
    });
    check("Android devices", connected, "连接设备并授权 USB 调试");

    #[cfg(target_os = "macos")]
    apple();
}

fn default_sdk() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join(if cfg!(target_os = "macos") {
        "Library/Android/sdk"
    } else {
        "Android/Sdk"
    }))
}

#[cfg(target_os = "macos")]
fn apple() {
    check(
        "Xcode",
        output("xcodebuild", &["-version"]),
        "安装 Xcode 并用 xcode-select 选择它",
    );
    let simulator = crate::ios::simulators()
        .ok()
        .filter(|devices| !devices.is_empty())
        .map(|devices| format!("{} available", devices.len()));
    check(
        "iOS Simulator",
        simulator,
        "在 Xcode Settings > Components 安装运行时并创建模拟器",
    );
    let signing = output("security", &["find-identity", "-v", "-p", "codesigning"])
        .filter(|identities| identities.contains("Apple Development"));
    let app = crate::app::App::current().ok();
    let signing = signing.and_then(|_| match app {
        Some(app) => app
            .ios
            .development_team
            .map(|team| format!("{team} · Apple Development certificate available")),
        None => Some("Apple Development certificate available".to_owned()),
    });
    check(
        "Apple signing",
        signing,
        "在 Xcode 添加 Apple Development 证书并配置 app.toml 中的 ios.development_team",
    );
    check(
        "usbmuxd",
        crate::usbmuxd::devices()
            .ok()
            .map(|devices| format!("{} USB devices", devices.len())),
        "插入已信任的 iPhone 并确认 /var/run/usbmuxd 可访问",
    );
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" · ")
    })
}

fn check(name: &str, result: Option<String>, fix: &str) {
    match result {
        Some(value) if !value.is_empty() => println!("OK      {name}  {value}"),
        _ => println!("MISSING {name}  {fix}"),
    }
}
