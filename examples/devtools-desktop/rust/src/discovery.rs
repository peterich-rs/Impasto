use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use devtools_protocol::{DeviceKind, DeviceSessionEntry, LocalSessionEntry};
use impasto::ImpastoType;
use serde::de::DeserializeOwned;

#[derive(Clone, PartialEq, Eq, ImpastoType)]
pub struct TargetSummary {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub pid: Option<u32>,
    pub project_root: Option<String>,
    pub framework_root: Option<String>,
    pub has_source_map: bool,
    pub is_self: bool,
}

#[derive(Clone)]
pub(crate) enum Target {
    Local(LocalSessionEntry),
    Device(DeviceSessionEntry),
}

impl Target {
    pub(crate) fn summary(&self) -> TargetSummary {
        match self {
            Self::Local(entry) => TargetSummary {
                id: entry.pid.to_string(),
                name: entry.app_name.clone(),
                platform: entry.platform.clone(),
                pid: Some(entry.pid),
                project_root: entry
                    .project_root
                    .as_ref()
                    .map(|path| path.display().to_string()),
                framework_root: entry
                    .framework_root
                    .as_ref()
                    .map(|path| path.display().to_string()),
                has_source_map: self.symbol_path().is_some_and(Path::is_file),
                is_self: entry.pid == std::process::id(),
            },
            Self::Device(entry) => {
                let platform = match entry.device_kind {
                    DeviceKind::Android => "android",
                    DeviceKind::Ios => "ios",
                };
                TargetSummary {
                    id: format!("{platform}:{}", entry.serial),
                    name: entry.package.clone(),
                    platform: platform.to_owned(),
                    pid: None,
                    project_root: Some(entry.project_root.display().to_string()),
                    framework_root: None,
                    has_source_map: self.symbol_path().is_some_and(Path::is_file),
                    is_self: false,
                }
            }
        }
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(left), Self::Local(right)) => {
                left.app_name == right.app_name
                    && left.platform == right.platform
                    && left.project_root == right.project_root
            }
            (Self::Device(left), Self::Device(right)) => {
                left.serial == right.serial
                    && left.package == right.package
                    && std::mem::discriminant(&left.device_kind)
                        == std::mem::discriminant(&right.device_kind)
            }
            _ => false,
        }
    }

    pub(crate) fn roots(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        match self {
            Self::Local(entry) => (entry.project_root.clone(), entry.framework_root.clone()),
            Self::Device(entry) => (Some(entry.project_root.clone()), None),
        }
    }

    pub(crate) fn same_registration(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(left), Self::Local(right)) => {
                left.pid == right.pid && left.token == right.token
            }
            (Self::Device(left), Self::Device(right)) => {
                left.token == right.token && self.matches(other)
            }
            _ => false,
        }
    }

    pub(crate) fn connection(&self) -> Result<(u16, &str, Option<Tunnel>), String> {
        let valid = |port, token: &str| {
            port != 0
                && (32..=128).contains(&token.len())
                && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        };
        match self {
            Self::Local(entry) if valid(entry.port, &entry.token) => {
                Ok((entry.port, &entry.token, None))
            }
            Self::Device(entry) if valid(entry.remote_port, &entry.token) => {
                let (port, tunnel) = Tunnel::open(entry)?;
                Ok((port, &entry.token, Some(tunnel)))
            }
            _ => Err("invalid session registration".to_owned()),
        }
    }

    pub(crate) fn symbol_path(&self) -> Option<&Path> {
        match self {
            Self::Local(entry) => entry.source_map.as_deref(),
            Self::Device(entry) => entry.source_map.as_deref(),
        }
    }
}

pub(crate) fn targets() -> Vec<Target> {
    let root = std::env::var_os("IMPASTO_DEVTOOLS_DISCOVERY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("impasto-devtools"));
    if !private_directory(&root) {
        return Vec::new();
    }
    let mut local: Vec<_> =
        registrations::<LocalSessionEntry>(&root.join("sessions"), |path, entry| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u32>().ok())
                == Some(entry.pid)
        })
        .into_iter()
        .filter(|session| {
            Command::new("/bin/ps")
                .args(["-p", &session.pid.to_string(), "-o", "pid="])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .collect();
    local.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    let devices = registrations::<DeviceSessionEntry>(&root.join("devices"), |_, entry| {
        !entry.serial.is_empty()
            && entry.serial.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-')
            })
    });
    local
        .into_iter()
        .map(Target::Local)
        .chain(devices.into_iter().map(Target::Device))
        .collect()
}

fn private_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700)
}

fn registrations<T: DeserializeOwned>(
    directory: &Path,
    validate: impl Fn(&Path, &T) -> bool,
) -> Vec<T> {
    if !private_directory(directory) {
        return Vec::new();
    }
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|file| {
            let path = file.path();
            let metadata = fs::symlink_metadata(&path).ok()?;
            if path.extension()? != "json"
                || !metadata.is_file()
                || metadata.len() > 64 * 1024
                || metadata.permissions().mode() & 0o777 != 0o600
            {
                return None;
            }
            let entry = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            validate(&path, &entry).then_some(entry)
        })
        .collect()
}

fn platform_tool(name: &str) -> PathBuf {
    let candidates = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    candidates
        .into_iter()
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .chain(
            ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
                .into_iter()
                .filter_map(|variable| {
                    std::env::var_os(variable)
                        .map(|root| PathBuf::from(root).join("platform-tools"))
                }),
        )
        .chain(std::env::home_dir().map(|home| home.join("Library/Android/sdk/platform-tools")))
        .map(|root| root.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn tunnel_command(command: &mut Command) -> std::io::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "USB tunnel command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) enum Tunnel {
    Adb { serial: String, port: u16 },
    Iproxy(Child),
}

impl Tunnel {
    fn open(entry: &DeviceSessionEntry) -> Result<(u16, Self), String> {
        let remote = entry.remote_port;
        match entry.device_kind {
            DeviceKind::Android => {
                let output = tunnel_command(Command::new(platform_tool("adb")).args([
                    "-s",
                    &entry.serial,
                    "forward",
                    "--no-rebind",
                    "tcp:0",
                    &format!("tcp:{remote}"),
                ]))
                .map_err(|error| format!("Start adb forward: {error}"))?;
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                }
                let port = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u16>()
                    .map_err(|error| error.to_string())?;
                Ok((
                    port,
                    Self::Adb {
                        serial: entry.serial.clone(),
                        port,
                    },
                ))
            }
            DeviceKind::Ios => {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
                    .map_err(|error| error.to_string())?;
                let port = listener
                    .local_addr()
                    .map_err(|error| error.to_string())?
                    .port();
                drop(listener);
                let child = Command::new(platform_tool("iproxy"))
                    .args(["-s", "127.0.0.1", "-u", &entry.serial])
                    .arg(format!("{port}:{remote}"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("Start iproxy: {error}"))?;
                Ok((port, Self::Iproxy(child)))
            }
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        match self {
            Self::Adb { serial, port } => {
                let _ = tunnel_command(Command::new(platform_tool("adb")).args([
                    "-s",
                    serial,
                    "forward",
                    "--remove",
                    &format!("tcp:{port}"),
                ]));
            }
            Self::Iproxy(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
