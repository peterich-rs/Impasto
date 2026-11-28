//! iOS development transport selection and devicectl file fallback.

use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, bail};
use dev_protocol::{Frame, Kind, Receipt};

use super::{RunDevice, build_artifact, bundle_id};
use crate::{
    app::App,
    build::BuildOptions,
    dev::{self, Delivery, Event, FileDelivery, Running, Stream, Target},
    toolchain,
};

pub(super) struct Launch<'a> {
    pub(super) token: &'a str,
    pub(super) address: Option<SocketAddrV4>,
    pub(super) port: Option<u16>,
    pub(super) files: bool,
    pub(super) verbose: bool,
}

impl Target for RunDevice {
    fn platform(&self) -> &'static str {
        "ios"
    }

    fn extra_inputs(&self, app: &App) -> Result<Vec<PathBuf>> {
        let mut paths = [&app.ios.privacy_manifest, &app.ios.launch_storyboard]
            .into_iter()
            .flatten()
            .map(|file| app.root.join(file))
            .collect::<Vec<_>>();
        let framework = toolchain::framework()?;
        if framework == app.root || app.root.starts_with(framework.join("examples")) {
            paths.extend(
                [
                    "crates/embedder/ios/src",
                    "crates/embedder/ios/host",
                    "crates/embedder/ios/include",
                    "crates/embedder/ios/template",
                    "crates/embedder/common/src",
                ]
                .into_iter()
                .map(|path| framework.join(path))
                .filter(|path| path.is_dir()),
            );
        }
        Ok(paths)
    }

    fn launch(
        &self,
        app: &App,
        options: &BuildOptions,
        events: &mpsc::Sender<Event>,
        attach: bool,
    ) -> Result<Running> {
        let artifact = build_artifact(app, options, self.platform())?;
        let token = dev::token()?;
        let package = bundle_id(app);
        let mut launch = Launch {
            token: &token,
            address: None,
            port: None,
            files: false,
            verbose: options.verbose,
        };
        let channel = match self {
            Self::Simulator(_) => {
                let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
                let std::net::SocketAddr::V4(address) = listener.local_addr()? else {
                    bail!("无效的模拟器调试地址");
                };
                launch.address = Some(address);
                Delivery::Socket(dev::Channel::start(
                    move || {
                        listener
                            .accept()
                            .map(|(socket, _)| Stream::Tcp(socket))
                            .map_err(Into::into)
                    },
                    Some(Box::new(move || {
                        let _ = TcpStream::connect(address);
                    })),
                    token.clone(),
                    package,
                    events.clone(),
                )?)
            }
            Self::Physical(udid) => {
                let files = Files::new(udid.clone(), package, token.clone())?;
                #[cfg(target_os = "macos")]
                if let Some(usb_id) = self.usb_identifier(udid) {
                    let port = 20_000 + u16::from_str_radix(&token[..4], 16)? % 30_000;
                    launch.port = Some(port);
                    let app_id = files.package.clone();
                    let verbose = options.verbose;
                    let mut attempts = 0;
                    let mut current_port = port;
                    let channel = dev::Channel::start(
                        move || {
                            let device = crate::usbmuxd::devices()?
                                .into_iter()
                                .find(|device| crate::usbmuxd::matches(device, &usb_id))
                                .context("iOS USB 设备未连接")?;
                            match crate::usbmuxd::connect(device.id, current_port) {
                                Ok(stream) => {
                                    attempts = 0;
                                    Ok(Stream::Unix(stream))
                                }
                                Err(error) => {
                                    attempts += 1;
                                    if attempts % 4 == 0
                                        && let Ok(Some(bytes)) = files.read("port")
                                        && let Ok(value) = std::str::from_utf8(&bytes)
                                        && let Ok(port) = value.trim().parse()
                                    {
                                        current_port = port;
                                    }
                                    if attempts % 16 == 0 {
                                        let _ = super::launch_physical(
                                            &files.udid,
                                            &files.package,
                                            &Launch {
                                                token: &files.token,
                                                address: None,
                                                port: Some(port),
                                                files: false,
                                                verbose,
                                            },
                                        )
                                        .output();
                                    }
                                    Err(error)
                                }
                            }
                        },
                        None,
                        token.clone(),
                        app_id,
                        events.clone(),
                    )?;
                    self.install_and_launch(&artifact.path, app, options, Some(&launch), attach)?;
                    return Ok(running(artifact, Delivery::Socket(channel)));
                }

                eprintln!("impasto: iOS 使用 devicectl 文件调试通道");
                launch.files = true;
                Delivery::Files(Box::new(files))
            }
        };
        self.install_and_launch(&artifact.path, app, options, Some(&launch), attach)?;
        Ok(running(artifact, channel))
    }
}

fn running(artifact: crate::build::PlatformArtifact, channel: Delivery) -> Running {
    Running {
        manifest: artifact.plugin_manifest,
        source: fs::read_to_string(artifact.path.join("app.js")).ok(),
        channel,
        child: None,
    }
}

#[cfg(target_os = "macos")]
impl RunDevice {
    fn usb_identifier(&self, udid: &str) -> Option<String> {
        let devices = crate::usbmuxd::devices().ok()?;
        if let Some(device) = devices
            .iter()
            .find(|device| crate::usbmuxd::matches(device, udid))
        {
            return Some(device.serial.clone());
        }
        // CoreDevice also accepts its own UUID. Resolve it to the hardware UDID.
        let device = super::physical_devices().ok()?.into_iter().find(|device| {
            device.identifier.eq_ignore_ascii_case(udid) || device.udid.eq_ignore_ascii_case(udid)
        })?;
        devices
            .iter()
            .find(|connected| crate::usbmuxd::matches(connected, &device.udid))
            .map(|connected| connected.serial.clone())
    }
}

struct Files {
    udid: String,
    package: String,
    token: String,
    directory: tempfile::TempDir,
    last_log: u64,
}

impl Files {
    fn new(udid: String, package: String, token: String) -> Result<Self> {
        Ok(Self {
            udid,
            package,
            token,
            directory: tempfile::tempdir()?,
            last_log: 0,
        })
    }

    fn copy(&self, direction: &str) -> Command {
        let mut command = Command::new("xcrun");
        command.args([
            "devicectl",
            "device",
            "copy",
            direction,
            "--device",
            &self.udid,
            "--domain-type",
            "appDataContainer",
            "--domain-identifier",
            &self.package,
            "--timeout",
            "3",
        ]);
        command
    }

    fn remote(&self, file: &str) -> String {
        format!("Library/Caches/ImpastoUpdates/{}/{file}", self.token)
    }

    fn send(&self, local: &std::path::Path, remote: &str) -> Result<()> {
        let mut copy = self.copy("to");
        copy.arg("--source")
            .arg(local)
            .args(["--destination", &self.remote(remote)]);
        toolchain::run(copy, "发送 iOS 文件调试请求")
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.directory.path().join("download");
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let output = self
            .copy("from")
            .args(["--source", &self.remote(name)])
            .arg("--destination")
            .arg(&path)
            .output()?;
        if output.status.success() {
            return Ok(Some(fs::read(path)?));
        }
        Ok(None)
    }
}

impl FileDelivery for Files {
    fn exchange(&mut self, frame: &Frame, reply: Kind) -> Result<Receipt> {
        let path = self.directory.path().join("upload");
        fs::write(&path, dev::file_request(&self.token, frame)?)?;
        self.send(&path, &format!("request-{}", frame.seq))?;
        fs::write(&path, [])?;
        self.send(&path, &format!("ready-{}", frame.seq))?;
        let deadline = Instant::now() + Duration::from_secs(12);
        let name = format!("reply-{}", frame.seq);
        while Instant::now() < deadline {
            if let Some(bytes) = self.read(&name)? {
                return dev::file_receipt(&bytes, frame.seq, reply, &mut self.last_log);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        bail!("iOS 设备未确认调试请求 {}", frame.seq)
    }
}
