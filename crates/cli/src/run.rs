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
    process::Command,
    time::{Duration, SystemTime},
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
        bail!("crates/ios 不存在，无法运行 iOS 应用");
    }
    let android = device != "macos";
    let settings = BuildOptions {
        debug: !options.release,
        verbose: options.verbose,
        trace: options.trace,
    };
    if android {
        let mut snapshot = files(&app)?;
        loop {
            let apk = build::apk(&app, &settings)?;
            let mut adb = Command::new("adb");
            if device != "android" {
                adb.args(["-s", device]);
            }
            adb.arg("install").arg("-r").arg(apk);
            toolchain::run(adb, "安装 APK")?;
            let mut adb = Command::new("adb");
            if device != "android" {
                adb.args(["-s", device]);
            }
            adb.args(["shell", "am", "start", "-n"]).arg(format!(
                "dev.impasto.{}/dev.impasto.host.HostActivity",
                app.name.replace('-', "_")
            ));
            toolchain::run(adb, "启动 Android 应用")?;
            eprintln!("impasto: 监听 lib/ 和 assets/，修改后重新构建安装；Ctrl-C 退出");
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let next = files(&app)?;
                if next != snapshot {
                    snapshot = next;
                    app = App::current()?;
                    break;
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
fn files(app: &App) -> Result<BTreeMap<PathBuf, (SystemTime, u64)>> {
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
