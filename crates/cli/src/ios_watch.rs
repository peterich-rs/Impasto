//! Explicitly enabled updates over the app container's development channel.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{RunDevice, build, bundle_id, delivery::session_id, require_output};
use crate::{
    app::App,
    build::{BuildOptions, bundle_in},
    run::{Snapshot, files, script_only, snapshot_path},
    toolchain,
};

const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_UPDATE_BYTES: usize = 16 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const ACK_TIMEOUT: Duration = Duration::from_secs(12);

struct DevelopmentSession {
    token: String,
    output: PathBuf,
    package: String,
    revision: u64,
}

impl DevelopmentSession {
    fn launch(app: &App, options: &BuildOptions, device: &RunDevice) -> Result<Self> {
        let token = session_id()?;
        let output = app.root.join("build/ios/development").join(&token);
        fs::create_dir_all(&output)?;
        let session = Self {
            token,
            output,
            package: bundle_id(app),
            revision: 0,
        };
        let path = build(app, options, device.platform())?;
        device.install_and_launch(&path, &session.package, options, Some(&session.token))?;
        Ok(session)
    }

    fn update(&mut self, app: &App, options: &BuildOptions, device: &RunDevice) -> Result<()> {
        let source = bundle_in(app, options, Some("ios"), &self.output.join("bundle"))?;
        let size = fs::metadata(&source)?.len();
        if size > MAX_SOURCE_BYTES as u64 {
            bail!("调试脚本超过 {MAX_SOURCE_BYTES} 字节");
        }
        let source = fs::read_to_string(&source)?;
        self.revision = self.revision.checked_add(1).context("iOS 更新序号用尽")?;
        let data = serde_json::to_vec(&Update {
            token: &self.token,
            revision: self.revision,
            source: &source,
        })?;
        if data.len() > MAX_UPDATE_BYTES {
            bail!("调试更新传输包超过 {MAX_UPDATE_BYTES} 字节");
        }
        let path = self.output.join("update.json");
        fs::write(&path, data)?;

        loop {
            await_connection(device);
            match self.send_and_wait(device, &path) {
                Ok(()) => return Ok(()),
                Err(_) if !device.connected() => continue,
                Err(error) => return Err(error),
            }
        }
    }

    fn send_and_wait(&self, device: &RunDevice, path: &Path) -> Result<()> {
        let channel = self.channel(device)?;
        let deadline = Instant::now() + ACK_TIMEOUT;
        while !channel.ready(&self.token)? {
            if Instant::now() > deadline {
                bail!("iOS 应用未建立调试会话，请确认其正在前台运行");
            }
            thread::sleep(POLL_INTERVAL);
        }
        channel.write_update(path)?;
        let deadline = Instant::now() + ACK_TIMEOUT;
        loop {
            if let Some(bytes) = channel.read("receipt.json")?
                && let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes)
                && receipt.token == self.token
                && receipt.revision == self.revision
            {
                if let Some(error) = receipt.error {
                    bail!(
                        "iOS 调试更新 {} 被拒绝，保留上一次运行版本。{error}",
                        self.revision
                    );
                }
                eprintln!("impasto: iOS 调试更新 {} 已应用", self.revision);
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!(
                    "iOS 应用未确认调试更新 {}，请确认其正在前台运行",
                    self.revision
                );
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn channel<'a>(&'a self, device: &'a RunDevice) -> Result<Channel<'a>> {
        let remote = format!("Library/Caches/ImpastoUpdates/{}", self.token);
        match device {
            RunDevice::Simulator(device) => {
                let output = Command::new("xcrun")
                    .args([
                        "simctl",
                        "get_app_container",
                        &device.udid,
                        &self.package,
                        "data",
                    ])
                    .output()?;
                let data = PathBuf::from(require_output(output, "读取 iOS 模拟器应用容器")?.trim());
                Ok(Channel::Local(data.join(remote)))
            }
            RunDevice::Physical(udid) => Ok(Channel::Remote {
                udid,
                package: &self.package,
                directory: remote,
                local: &self.output,
            }),
        }
    }
}

impl RunDevice {
    fn connected(&self) -> bool {
        let mut probe = Command::new("xcrun");
        match self {
            Self::Simulator(device) => {
                probe.args(["simctl", "getenv", &device.udid, "HOME"]);
            }
            Self::Physical(udid) => {
                probe.args([
                    "devicectl",
                    "device",
                    "info",
                    "details",
                    "--device",
                    udid,
                    "--timeout",
                    "3",
                ]);
            }
        }
        probe
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn await_connection(device: &RunDevice) {
    if !device.connected() {
        eprintln!("impasto: iOS 设备已断开，等待重新连接");
        loop {
            thread::sleep(Duration::from_secs(1));
            if device.connected() {
                eprintln!("impasto: iOS 设备已重新连接");
                break;
            }
        }
    }
}

#[derive(Serialize)]
struct Update<'a> {
    token: &'a str,
    revision: u64,
    source: &'a str,
}

#[derive(Deserialize)]
struct Receipt {
    token: String,
    revision: u64,
    error: Option<String>,
}

enum Channel<'a> {
    Local(PathBuf),
    Remote {
        udid: &'a str,
        package: &'a str,
        directory: String,
        local: &'a Path,
    },
}

impl Channel<'_> {
    fn ready(&self, token: &str) -> Result<bool> {
        Ok(self
            .read("ready")?
            .is_some_and(|bytes| bytes == token.as_bytes()))
    }

    fn write_update(&self, path: &Path) -> Result<()> {
        match self {
            Self::Local(directory) => {
                let staging = directory.join("upload.json");
                fs::copy(path, &staging)?;
                fs::rename(staging, directory.join("update.json"))?;
            }
            Self::Remote {
                udid,
                package,
                directory,
                ..
            } => {
                let mut command = copy_command(udid, package, "to");
                command.arg("--source").arg(path);
                command
                    .arg("--destination")
                    .arg(format!("{directory}/update.json"));
                toolchain::run(command, "发送 iOS 真机调试更新")?;
            }
        }
        Ok(())
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self {
            Self::Local(directory) => read_file(&directory.join(name)),
            Self::Remote {
                udid,
                package,
                directory,
                local,
            } => {
                let path = local.join(name);
                // Avoid a stale local receipt when devicectl reports missing remote data.
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                let mut command = copy_command(udid, package, "from");
                command.args(["--source", &format!("{directory}/{name}")]);
                command.arg("--destination").arg(&path);
                let output = command.output().context("启动 devicectl 文件接收")?;
                if output.status.success() {
                    read_file(&path)
                } else {
                    Ok(None)
                }
            }
        }
    }
}

fn copy_command(udid: &str, package: &str, direction: &str) -> Command {
    let mut command = Command::new("xcrun");
    command.args([
        "devicectl",
        "device",
        "copy",
        direction,
        "--device",
        udid,
        "--domain-type",
        "appDataContainer",
        "--domain-identifier",
        package,
        "--timeout",
        "3",
    ]);
    command
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, PartialEq, Eq)]
struct AppSnapshot {
    written: Snapshot,
    native: Snapshot,
}

impl AppSnapshot {
    fn read(app: &App) -> Result<Self> {
        let mut native = Snapshot::new();
        for file in [&app.ios.privacy_manifest, &app.ios.launch_storyboard]
            .into_iter()
            .flatten()
        {
            snapshot_path(&app.root.join(file), &mut native)?;
        }
        let framework = toolchain::framework()?;
        for directory in [
            "Cargo.toml",
            "Cargo.lock",
            "crates/embedder/ios/Cargo.toml",
            "crates/embedder/ios/host",
            "crates/embedder/ios/include",
            "crates/embedder/ios/src",
            "crates/embedder/ios/template",
            "crates/embedder/common/Cargo.toml",
            "crates/embedder/common/src",
        ] {
            snapshot_path(&framework.join(directory), &mut native)?;
        }
        Ok(Self {
            written: files(app)?,
            native,
        })
    }
}

pub(super) fn run(mut app: App, options: &BuildOptions, device: RunDevice) -> Result<()> {
    let mut snapshot = AppSnapshot::read(&app)?;
    let mut session = DevelopmentSession::launch(&app, options, &device)?;
    let mut installed = snapshot.clone();
    eprintln!("impasto: 正在监听 iOS 应用文件。Ctrl-C 退出");
    loop {
        thread::sleep(POLL_INTERVAL);
        let next = AppSnapshot::read(&app)?;
        if next == snapshot {
            continue;
        }
        snapshot = next;
        app = match App::current() {
            Ok(app) => app,
            Err(error) => {
                eprintln!("impasto: 应用配置无效，等待修正。{error:#}");
                continue;
            }
        };
        snapshot = AppSnapshot::read(&app)?;
        let result = deploy(&mut session, &app, options, &device, &installed, &snapshot);
        match result {
            Ok(()) => installed = snapshot.clone(),
            Err(error) => eprintln!("impasto: iOS 更新未完成。{error:#}"),
        }
    }
}

fn deploy(
    session: &mut DevelopmentSession,
    app: &App,
    options: &BuildOptions,
    device: &RunDevice,
    installed: &AppSnapshot,
    current: &AppSnapshot,
) -> Result<()> {
    if installed.native == current.native && script_only(app, &installed.written, &current.written)
    {
        return session.update(app, options, device);
    }
    await_connection(device);
    loop {
        match DevelopmentSession::launch(app, options, device) {
            Ok(next) => {
                *session = next;
                return Ok(());
            }
            Err(_) if !device.connected() => await_connection(device),
            Err(error) => return Err(error),
        }
    }
}
