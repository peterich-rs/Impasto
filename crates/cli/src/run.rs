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
    if device == "ios" {
        bail!("iOS 宿主尚未实现，无法运行 iOS 应用");
    }
    let android = device != "macos";
    let settings = BuildOptions {
        debug: !options.release,
        verbose: options.verbose,
        trace: options.trace,
    };
    if android {
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
        loop {
            let source_only = settings.debug
                && installed
                    .as_ref()
                    .is_some_and(|previous| script_only(&app, previous, &snapshot));
            let artifact = if source_only {
                build::bundle(&app, &settings)
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
                            update_android(&adb, &app.android_id(), &revision, &artifact)
                        } else {
                            install_android(&adb, &app.android_id(), &artifact)
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
    loop {
        let application = build::macos(&app, &settings)?;
        let executable = application.join("Contents/MacOS/impasto");
        let mut command = Command::new(executable);
        command.current_dir(&app.root);
        if options.trace {
            command.env("IMPASTO_TRACE", "1");
        }
        let mut child = command.spawn()?;
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("应用退出 ({status})");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
            let next = files(&app)?;
            if next != snapshot {
                snapshot = next;
                child.kill()?;
                child.wait()?;
                app = App::current()?;
                break;
            }
        }
    }
}
type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

fn script_only(app: &App, previous: &Snapshot, current: &Snapshot) -> bool {
    let unchanged = |path: &Path| {
        previous
            .iter()
            .filter(|(file, _)| file.starts_with(path))
            .eq(current.iter().filter(|(file, _)| file.starts_with(path)))
    };
    ["assets", "app.toml", "package.json"]
        .into_iter()
        .all(|name| unchanged(&app.root.join(name)))
}

fn install_android(adb: &impl Fn() -> Command, package: &str, apk: &Path) -> Result<()> {
    let mut install = adb();
    install.args(["install", "-r"]).arg(apk);
    toolchain::run(install, "安装 APK")?;
    let mut start = adb();
    start
        .args(["shell", "am", "start", "-n"])
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

fn files(app: &App) -> Result<Snapshot> {
    fn visit(path: &Path, out: &mut BTreeMap<PathBuf, (SystemTime, u64)>) -> Result<()> {
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
                    visit(&entry.path(), out)?;
                }
            }
        } else {
            out.insert(path.to_owned(), (meta.modified()?, meta.len()));
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    for name in [
        if app.gallery { "src" } else { "lib" },
        "assets",
        "app.toml",
        "package.json",
    ] {
        visit(&app.root.join(name), &mut out)?;
    }
    if app.gallery {
        visit(&toolchain::framework()?.join("packages"), &mut out)?;
    }
    Ok(out)
}
