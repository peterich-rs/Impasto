use crate::{
    app::App,
    build::{self, BuildOptions},
    toolchain,
};
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub struct RunOptions {
    pub device: Option<String>,
    pub release: bool,
    pub verbose: bool,
    pub trace: bool,
    pub watch: bool,
}
pub fn run(options: RunOptions) -> Result<()> {
    let mut app = App::current()?;
    let device = options
        .device
        .as_deref()
        .unwrap_or(if cfg!(target_os = "macos") {
            "macos"
        } else {
            "android"
        });
    let settings = BuildOptions {
        debug: !options.release,
        verbose: options.verbose,
        trace: options.trace,
    };
    if device == "ios" || device.starts_with("ios:") {
        return crate::ios::run(app, &settings, device.strip_prefix("ios:"), options.watch);
    }
    if options.watch {
        bail!("--watch 仅适用于 iOS 调试运行");
    }
    if device != "macos" {
        let adb = || {
            let mut command = Command::new("adb");
            if device != "android" {
                command.args(["-s", device]);
            }
            command
        };
        let connected = || {
            adb().arg("get-state").output().is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "device"
            })
        };
        let mut snapshot = files(&app)?;
        let mut installed = None;
        let mut installed_manifest: Option<Vec<u8>> = None;
        loop {
            let source_only = settings.debug
                && installed
                    .as_ref()
                    .is_some_and(|previous| script_only(&app, previous, &snapshot));
            let artifact = if source_only {
                build::bundle_update(
                    &app,
                    &settings,
                    "android",
                    &app.root.join("build/android/update"),
                    installed_manifest.as_deref(),
                )
                .map(|path| build::PlatformArtifact {
                    path,
                    plugin_manifest: None,
                })
            } else {
                build::apk(&app, &settings)
            };
            match artifact {
                Ok(artifact) => {
                    let revision = SystemTime::now()
                        .duration_since(UNIX_EPOCH)?
                        .as_nanos()
                        .to_string();
                    let mut waiting = false;
                    while files(&app)? == snapshot {
                        if installed.is_some() && !connected() {
                            if !waiting {
                                eprintln!("impasto: Android 设备已断开，等待重新连接");
                                waiting = true;
                            }
                            std::thread::sleep(Duration::from_millis(500));
                            continue;
                        }
                        let deployed = if source_only {
                            update_android(&adb, &app.android_id(), &revision, &artifact.path)
                        } else {
                            install_android(&adb, &app.android_id(), &artifact.path, settings.debug)
                        };
                        match deployed {
                            Err(_) if installed.is_some() && !connected() => continue,
                            Err(error) if installed.is_some() => {
                                eprintln!("impasto: 更新未完成。{error:#}");
                            }
                            Err(error) => return Err(error),
                            Ok(()) => {
                                if installed.is_none() {
                                    eprintln!("impasto: 正在监听应用文件。Ctrl-C 退出");
                                }
                                installed = Some(snapshot.clone());
                                if !source_only {
                                    installed_manifest = artifact.plugin_manifest;
                                }
                            }
                        }
                        break;
                    }
                }
                Err(error) if installed.is_some() => {
                    eprintln!("impasto: 更新构建失败，设备继续运行上一个版本。{error:#}");
                }
                Err(error) => return Err(error),
            }
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let next = files(&app)?;
                if next != snapshot {
                    snapshot = next;
                    match App::current() {
                        Ok(updated) => {
                            app = updated;
                            break;
                        }
                        Err(error) => eprintln!("impasto: 应用配置无效，等待修正。{error:#}"),
                    }
                }
            }
        }
    }
    app.require("macos")?;
    let mut snapshot = files(&app)?;
    let application = build::macos(&app, &settings)?;
    let mut child = launch_macos(&app, &application, options.trace)?;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("应用退出 ({status})");
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
        let next = files(&app)?;
        if next == snapshot {
            continue;
        }
        snapshot = next;
        let next = App::current().and_then(|app| {
            let application = build::macos(&app, &settings)?;
            Ok((app, application))
        });
        match next {
            Ok((next_app, application)) => {
                child.kill()?;
                child.wait()?;
                child = launch_macos(&next_app, &application, options.trace)?;
                app = next_app;
                snapshot = files(&app)?;
            }
            Err(error) => eprintln!("impasto: 更新构建失败，继续运行上一个版本。{error:#}"),
        }
    }
}

fn launch_macos(app: &App, application: &Path, trace: bool) -> Result<std::process::Child> {
    let mut command = Command::new(application.join("Contents/MacOS/impasto"));
    command.current_dir(&app.root);
    if trace {
        command.env("IMPASTO_TRACE", "1");
    }
    Ok(command.spawn()?)
}

pub(crate) type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

pub(crate) fn script_only(app: &App, previous: &Snapshot, current: &Snapshot) -> bool {
    let unchanged = |path: &Path| {
        previous
            .iter()
            .filter(|(file, _)| file.starts_with(path))
            .eq(current.iter().filter(|(file, _)| file.starts_with(path)))
    };
    let native_unchanged = crate::plugins::watched_paths(app).into_iter().all(|path| {
        let is_native =
            |file: &Path| file.starts_with(&path) && !crate::plugins::is_wrapper_source(file);
        previous
            .iter()
            .filter(|(file, _)| is_native(file))
            .eq(current.iter().filter(|(file, _)| is_native(file)))
    });
    native_unchanged
        && ["assets", "app.toml", "package.json"]
            .into_iter()
            .all(|name| unchanged(&app.root.join(name)))
}

fn install_android(
    adb: &impl Fn() -> Command,
    package: &str,
    apk: &Path,
    debug: bool,
) -> Result<()> {
    let devtools = if debug {
        crate::devtools::config()?
    } else {
        None
    };
    let mut install = adb();
    install.args(["install", "-r"]).arg(apk);
    toolchain::run(install, "安装 APK")?;
    let mut start = adb();
    start.args(["shell", "am", "start"]);
    if let Some((port, token)) = devtools {
        crate::devtools::configure_android(adb, package, port, &token)?;
        start.arg("-S");
    }
    start
        .arg("-n")
        .arg(format!("{package}/dev.impasto.host.ImpastoActivity"));
    toolchain::run(start, "启动 Android 应用")
}

fn update_android(
    adb: &impl Fn() -> Command,
    package: &str,
    revision: &str,
    source: &Path,
) -> Result<()> {
    let file = format!("files/impasto-debug/v1-{revision}");
    let receipt = format!("{file}.result");
    let read_result = || {
        adb()
            .args(["shell", "run-as", package, "cat", &receipt])
            .output()
    };
    let mut status = read_result()?;
    if !status.status.success() {
        let mut upload = adb();
        upload.args(["shell", "-T"]).arg(format!(
            "run-as {package} sh -c 'umask 077 && mkdir -p files/impasto-debug && cat > {file}.upload && mv {file}.upload {file}.js'",
        ));
        upload.stdin(Stdio::from(fs::File::open(source)?));
        toolchain::run(upload, "发送 Android 调试脚本")?;
        let mut start = adb();
        start
            .args(["shell", "am", "start", "--activity-single-top", "-n"])
            .arg(format!("{package}/dev.impasto.host.ImpastoActivity"))
            .args(["--es", "impasto.debug.update", revision]);
        toolchain::run(start, "请求 Android 调试更新")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while !status.status.success() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
            status = read_result()?;
        }
    }
    if !status.status.success() {
        bail!("设备未确认调试更新 {revision}");
    }
    let response = String::from_utf8(status.stdout)?;
    let _ = adb()
        .args(["shell", "run-as", package, "rm", "-f", &receipt])
        .status();
    if let Some(message) = response.strip_prefix("error\n") {
        bail!("脚本验证失败，仍在运行上一版本。{message}");
    }
    if response.trim() != "ok" {
        bail!("设备返回了未知的调试更新结果");
    }
    eprintln!("impasto: 调试更新 {revision} 已应用");
    Ok(())
}

pub(crate) fn files(app: &App) -> Result<Snapshot> {
    let mut out = BTreeMap::new();
    for name in [
        if app.gallery { "src" } else { "lib" },
        "assets",
        "app.toml",
        "package.json",
    ] {
        snapshot_path(&app.root.join(name), &mut out)?;
    }
    for path in crate::plugins::watched_paths(app) {
        snapshot_path(&path, &mut out)?;
    }
    if app.gallery {
        snapshot_path(&toolchain::framework()?.join("packages"), &mut out)?;
    }
    Ok(out)
}

pub(crate) fn snapshot_path(
    path: &Path,
    out: &mut BTreeMap<PathBuf, (SystemTime, u64)>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let meta = fs::metadata(path)?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !["node_modules", "build", "dist", "target", ".git"]
                .iter()
                .any(|n| entry.file_name() == *n)
            {
                snapshot_path(&entry.path(), out)?;
            }
        }
    } else {
        out.insert(path.to_owned(), (meta.modified()?, meta.len()));
    }
    Ok(())
}
