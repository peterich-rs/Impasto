use crate::{
    app::App,
    build::{self, BuildOptions},
    toolchain,
};
use anyhow::{Context, Result, bail};
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
    pub devtools: bool,
}
pub fn run(options: RunOptions) -> Result<()> {
    anyhow::ensure!(
        !options.devtools || std::env::var("IMPASTO_DEVTOOLS").as_deref() != Ok("0"),
        "--devtools requires the DevTools agent to be enabled"
    );
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
        hot: (options.watch && !options.release).then(Default::default),
        verbose: options.verbose,
        trace: options.trace,
    };
    let watch_interval = Duration::from_millis(if settings.hot.is_some() { 10 } else { 250 });
    if device == "ios" || device.starts_with("ios:") {
        return crate::ios::run(
            app,
            &settings,
            device.strip_prefix("ios:"),
            options.watch,
            options.devtools,
        );
    }
    if device != "macos" {
        let serial = if device == "android" {
            let output = Command::new("adb").arg("get-serialno").output()?;
            anyhow::ensure!(
                output.status.success(),
                "Select an Android device with -d <serial>"
            );
            String::from_utf8(output.stdout)?.trim().to_owned()
        } else {
            device.to_owned()
        };
        let adb = || {
            let mut command = Command::new("adb");
            command.args(["-s", &serial]);
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
        let mut installed_source: Option<String> = None;
        let mut installed_activity: Option<String> = None;
        let mut channel: Option<crate::android_watch::Channel> = None;
        loop {
            let source_only = settings.debug
                && installed
                    .as_ref()
                    .is_some_and(|previous| script_only(&app, previous, &snapshot));
            let deployment = if source_only {
                if let Some(installed) = &installed {
                    set_watch_changes(&settings, installed, &snapshot);
                }
                build::bundle_update(&app, &settings, "android", installed_manifest.as_deref())
                    .map(AndroidDeployment::Script)
            } else {
                build::apk(&app, &settings).map(AndroidDeployment::Apk)
            };
            match deployment {
                Ok(deployment) => {
                    if installed.is_none() {
                        snapshot = files(&app)?;
                    }
                    let revision = SystemTime::now()
                        .duration_since(UNIX_EPOCH)?
                        .as_nanos()
                        .to_string();
                    let mut waiting = false;
                    while files(&app)? == snapshot {
                        let deployed = match &deployment {
                            AndroidDeployment::Script(source) => {
                                let activity = installed_activity
                                    .as_deref()
                                    .context("缺少已安装的 Android 启动入口")?;
                                let patch = installed_source
                                    .as_deref()
                                    .and_then(|before| bundler::delta(before, source));
                                let package = app.android_id();
                                let source = patch.as_deref().unwrap_or(source);
                                if !channel
                                    .as_ref()
                                    .is_some_and(crate::android_watch::Channel::is_open)
                                {
                                    channel =
                                        crate::android_watch::Channel::open(&adb, &package).ok();
                                }
                                channel
                                    .as_mut()
                                    .map(|channel| channel.send(&revision, source))
                                    .transpose()
                                    .and_then(|delivered| {
                                        if delivered == Some(true) {
                                            Ok(None)
                                        } else {
                                            update_android(
                                                &adb, &package, activity, &revision, source,
                                            )
                                            .map(|()| None)
                                        }
                                    })
                            }
                            AndroidDeployment::Apk(artifact) => {
                                let config = if settings.debug {
                                    crate::devtools::device_config()?
                                } else {
                                    None
                                };
                                install_android(
                                    &adb,
                                    &app.android_id(),
                                    &artifact.path,
                                    config.as_ref(),
                                )
                                .and_then(|activity| {
                                    if let Some(config) = config {
                                        let attach = crate::devtools::register_device(
                                            devtools_protocol::DeviceKind::Android,
                                            &serial,
                                            &app.android_id(),
                                            config,
                                            &app.root,
                                        )?;
                                        if options.devtools && installed.is_none() {
                                            crate::devtools::launch_native(Some(&attach))?;
                                        }
                                    }
                                    Ok(Some(activity))
                                })
                            }
                        };
                        match deployed {
                            Err(_) if installed.is_some() && !connected() => {
                                channel = None;
                                if !waiting {
                                    eprintln!("impasto: Android 设备已断开，等待重新连接");
                                    waiting = true;
                                }
                                std::thread::sleep(Duration::from_millis(500));
                                continue;
                            }
                            Err(error) if installed.is_some() => {
                                eprintln!("impasto: 更新未完成。{error:#}");
                            }
                            Err(error) => return Err(error),
                            Ok(activity) => {
                                let first = installed.is_none();
                                installed = Some(snapshot.clone());
                                match deployment {
                                    AndroidDeployment::Script(source) => {
                                        installed_source = Some(source)
                                    }
                                    AndroidDeployment::Apk(artifact) => {
                                        channel = crate::android_watch::Channel::open(
                                            &adb,
                                            &app.android_id(),
                                        )
                                        .ok();
                                        installed_manifest = artifact.plugin_manifest;
                                        installed_activity = activity;
                                        installed_source = fs::read_to_string(
                                            app.root.join("build/android/bundle/debug/app.js"),
                                        )
                                        .ok();
                                    }
                                }
                                if first {
                                    eprintln!("impasto: 正在监听应用文件。Ctrl-C 退出");
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
                std::thread::sleep(watch_interval);
                let next = files(&app)?;
                if next != snapshot {
                    snapshot = next;
                    if installed
                        .as_ref()
                        .is_some_and(|prior| script_only(&app, prior, &snapshot))
                    {
                        break;
                    }
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
    let mut application = build::macos(&app, &settings)?;
    let mailbox = settings.debug.then(tempfile::tempdir).transpose()?;
    let update = mailbox.as_ref().map(|dir| dir.path().join("app.js"));
    let mut child = launch_macos(&app, &application.path, options.trace, update.as_deref())?;
    if options.devtools {
        crate::devtools::launch_native(Some(&child.id().to_string()))?;
    }
    let mut snapshot = files(&app)?;
    let mut installed_source = macos_source(&application.path, settings.hot.is_some());
    let mut installed = snapshot.clone();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("应用退出 ({status})");
            }
            return Ok(());
        }
        std::thread::sleep(watch_interval);
        let next = files(&app)?;
        if next == snapshot {
            continue;
        }
        snapshot = next;
        if let Some(update) = &update
            && script_only(&app, &installed, &snapshot)
        {
            set_watch_changes(&settings, &installed, &snapshot);
            let result = build::bundle_update(
                &app,
                &settings,
                "macos",
                application.plugin_manifest.as_deref(),
            )
            .and_then(|source| {
                let patch = installed_source
                    .as_deref()
                    .and_then(|before| bundler::delta(before, &source));
                publish_source(patch.as_deref().unwrap_or(&source), update)?;
                Ok(source)
            });
            match result {
                Ok(source) => {
                    installed_source = Some(source);
                    installed = snapshot.clone();
                }
                Err(error) => eprintln!("impasto: 源码更新失败。{error:#}"),
            }
            continue;
        }
        let next_app = match App::current() {
            Ok(next_app) => next_app,
            Err(error) => {
                eprintln!("impasto: 应用配置无效，等待修正。{error:#}");
                continue;
            }
        };
        match build::macos(&next_app, &settings) {
            Ok(next) => {
                child.kill()?;
                child.wait()?;
                if let Some(update) = &update
                    && update.is_file()
                {
                    fs::remove_file(update)?;
                }
                child = launch_macos(&next_app, &next.path, options.trace, update.as_deref())?;
                installed_source = macos_source(&next.path, settings.hot.is_some());
                app = next_app;
                application = next;
                snapshot = files(&app)?;
                installed = snapshot.clone();
            }
            Err(error) => eprintln!("impasto: 更新构建失败，继续运行上一个版本。{error:#}"),
        }
    }
}

fn macos_source(application: &Path, hot: bool) -> Option<String> {
    hot.then(|| fs::read_to_string(application.join("Contents/Resources/app.js")).ok())
        .flatten()
}

fn publish_source(source: &str, output: &Path) -> Result<()> {
    use std::io::Write as _;
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid source mailbox"))?;
    let revision = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    writeln!(file, "{revision}")?;
    file.write_all(source.as_bytes())?;
    file.persist(output)?;
    let receipt = parent.join(format!("{revision}.result"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(result) = fs::read_to_string(&receipt) {
            fs::remove_file(receipt)?;
            if result == "ok" {
                eprintln!("impasto: macOS 调试更新 {revision} 已应用");
                return Ok(());
            }
            bail!("{result}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    bail!("macOS 宿主未确认调试更新 {revision}");
}

fn launch_macos(
    app: &App,
    application: &Path,
    trace: bool,
    update: Option<&Path>,
) -> Result<std::process::Child> {
    let mut command = Command::new(application.join("Contents/MacOS/impasto"));
    command.current_dir(&app.root);
    if trace {
        command.env("IMPASTO_TRACE", "1");
    }
    if let Some(update) = update {
        command.env("IMPASTO_SOURCE_MAILBOX", update);
    }
    command.env("IMPASTO_DEVTOOLS_APP_NAME", &app.name);
    command.env("IMPASTO_DEVTOOLS_PROJECT", &app.root);
    command.env("IMPASTO_DEVTOOLS_FRAMEWORK", toolchain::framework()?);
    command.env(
        "IMPASTO_DEVTOOLS_SOURCE_MAP",
        application.join("Contents/Resources/app.js.map"),
    );
    Ok(command.spawn()?)
}

pub(crate) type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

pub(crate) fn set_watch_changes(options: &BuildOptions, previous: &Snapshot, current: &Snapshot) {
    let Some(watch) = &options.hot else {
        return;
    };
    let changes = previous.keys().eq(current.keys()).then(|| {
        current
            .iter()
            .filter(|(path, stamp)| previous.get(*path) != Some(*stamp))
            .map(|(path, _)| path.clone())
            .collect()
    });
    watch.borrow_mut().set_changes(changes);
}

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
    let assets_unchanged = previous
        .iter()
        .filter(|(file, _)| file.starts_with(app.assets()))
        .eq(current
            .iter()
            .filter(|(file, _)| file.starts_with(app.assets())));
    native_unchanged
        && assets_unchanged
        && [
            "assets",
            "app.toml",
            "impasto.lock",
            "package.json",
            "pnpm-lock.yaml",
            "package-lock.json",
            "android",
            "ios",
            "macos",
        ]
        .into_iter()
        .all(|name| unchanged(&app.root.join(name)))
}

fn install_android(
    adb: &impl Fn() -> Command,
    package: &str,
    apk: &Path,
    devtools: Option<&(u16, String)>,
) -> Result<String> {
    let mut install = adb();
    install.args(["install", "-r"]).arg(apk);
    toolchain::run(install, "安装 APK")?;
    let resolved = adb()
        .args([
            "shell",
            "cmd",
            "package",
            "resolve-activity",
            "--brief",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
            "-p",
            package,
        ])
        .output()?;
    let activity = String::from_utf8(resolved.stdout)?;
    let activity = activity
        .lines()
        .last()
        .map(str::trim)
        .filter(|activity| {
            resolved.status.success()
                && activity
                    .split_once('/')
                    .is_some_and(|(name, class)| name == package && !class.is_empty())
        })
        .ok_or_else(|| anyhow::anyhow!("找不到 {package} 的 Android 启动入口"))?;
    let mut start = adb();
    start.args(["shell", "am", "start", "-S"]);
    if let Some((port, token)) = devtools {
        crate::devtools::configure_android(adb, package, *port, token)?;
    }
    start.args(["-n", activity]);
    toolchain::run(start, "启动 Android 应用")?;
    Ok(activity.to_owned())
}

fn update_android(
    adb: &impl Fn() -> Command,
    package: &str,
    activity: &str,
    revision: &str,
    source: &str,
) -> Result<()> {
    use std::io::Write as _;
    let file = format!("files/impasto-debug/v1-{revision}");
    let receipt = format!("{file}.result");
    let read_result = || {
        adb()
            .args(["shell", "run-as", package, "cat", &receipt])
            .output()
    };
    let mut upload = adb();
    upload.args(["shell", "-T"]).arg(format!(
        "run-as {package} sh -c 'if [ ! -f {receipt} ]; then \
         umask 077 && mkdir -p files/impasto-debug && cat > {file}.upload && \
         mv {file}.upload {file}.js || exit 1; fi; \
         i=0; while [ ! -f {receipt} ] && [ \"$i\" -lt 40 ]; do sleep 0.01; i=$((i+1)); done; \
         [ -f {receipt} ] || exit 3; cat {receipt} && rm -f {receipt}'",
    ));
    let mut child = upload
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let written = child
        .stdin
        .take()
        .context("adb stdin")
        .and_then(|mut input| {
            input.write_all(source.as_bytes())?;
            Ok(())
        });
    let mut status = child.wait_with_output()?;
    if status.status.success() {
        written?;
    }
    if status.status.code() == Some(3) {
        // The first request may race the activity's mailbox watcher.
        let mut start = adb();
        start
            .args(["shell", "am", "start", "--activity-single-top"])
            .args(["-n", activity])
            .args(["--es", "impasto.debug.update", revision]);
        toolchain::run(start, "请求 Android 调试更新")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        status = read_result()?;
        while !status.status.success() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(15));
            status = read_result()?;
        }
        if status.status.success() {
            let _ = adb()
                .args(["shell", "run-as", package, "rm", "-f", &receipt])
                .status();
        }
    }
    if !status.status.success() {
        bail!(
            "设备未确认调试更新 {revision}。{}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    let response = String::from_utf8(status.stdout)?;
    if let Some(message) = response.strip_prefix("error\n") {
        bail!("脚本验证失败，仍在运行上一版本。{message}");
    }
    if response.trim() != "ok" {
        bail!("设备返回了未知的调试更新结果");
    }
    eprintln!("impasto: 调试更新 {revision} 已应用");
    Ok(())
}

enum AndroidDeployment {
    Apk(build::PlatformArtifact),
    Script(String),
}

pub(crate) fn files(app: &App) -> Result<Snapshot> {
    let mut out = BTreeMap::new();
    for name in [
        "src",
        "assets",
        "locales",
        "app.toml",
        "package.json",
        "pnpm-lock.yaml",
        "package-lock.json",
        "android",
        "ios",
        "macos",
    ] {
        snapshot_path(&app.root.join(name), &mut out)?;
    }
    for path in crate::plugins::watched_paths(app) {
        snapshot_path(&path, &mut out)?;
    }
    snapshot_path(app.assets(), &mut out)?;
    for path in app.sources.values() {
        snapshot_path(path, &mut out)?;
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
            if ![
                "node_modules",
                "build",
                "dist",
                "target",
                ".git",
                ".impasto",
                ".gradle",
                ".kotlin",
                ".build",
                ".swiftpm",
                "xcuserdata",
                "local.properties",
            ]
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
