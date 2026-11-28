use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::Args;
use devtools_protocol::{DeviceKind, DeviceSessionEntry, LocalSessionEntry};

#[derive(Args)]
pub(crate) struct Options {
    /// Generate the legacy standalone HTML client.
    #[arg(long)]
    html: bool,
    /// Local process ID, android:<serial>, or ios:<UDID>.
    #[arg(long)]
    attach: Option<String>,
    /// Port configured on the target app.
    #[arg(long, default_value_t = 8765, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    /// android, an Android serial, or ios:<USB UDID>. Omit for desktop and iOS Simulator.
    #[arg(short = 'd', long)]
    device: Option<String>,
    /// Local forwarded port. Android otherwise requests an available port.
    #[arg(long, requires = "device", value_parser = clap::value_parser!(u16).range(1..))]
    local_port: Option<u16>,
    /// Path for the standalone HTML client.
    #[arg(long)]
    client: Option<PathBuf>,
    /// Directory containing bundler.js and bundler_bg.wasm from wasm-bindgen --target web.
    #[arg(long)]
    compiler: Option<PathBuf>,
    /// Release symbol sidecar used to resolve captured JavaScript frames.
    #[arg(long)]
    symbols: Option<PathBuf>,
    /// Local app root for Inspector source links.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Editor to open when an Inspector source link is followed.
    #[arg(long, default_value = "vscode", value_parser = ["vscode", "cursor", "zed"])]
    editor: String,
}

pub(crate) fn config() -> Result<Option<(u16, String)>> {
    if std::env::var("IMPASTO_DEVTOOLS").as_deref() == Ok("0") {
        return Ok(None);
    }
    let Some(port) = std::env::var_os("IMPASTO_DEVTOOLS_PORT") else {
        return Ok(None);
    };
    let port = port
        .to_string_lossy()
        .parse::<u16>()
        .context("IMPASTO_DEVTOOLS_PORT")?;
    let token = std::env::var("IMPASTO_DEVTOOLS_TOKEN").context("IMPASTO_DEVTOOLS_TOKEN")?;
    if port == 0
        || !(32..=128).contains(&token.len())
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("DevTools requires a nonzero port and a 32–128 character hexadecimal token");
    }
    Ok(Some((port, token)))
}

pub(crate) fn device_config() -> Result<Option<(u16, String)>> {
    if std::env::var("IMPASTO_DEVTOOLS").as_deref() == Ok("0") {
        return Ok(None);
    }
    if let Some(config) = config()? {
        return Ok(Some(config));
    }
    use std::fmt::Write as _;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("DevTools token: {error}"))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        write!(token, "{byte:02x}")?;
    }
    Ok(Some((8765, token)))
}

pub(crate) fn discovery_root() -> PathBuf {
    std::env::var_os("IMPASTO_DEVTOOLS_DISCOVERY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("impasto-devtools"))
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o700 {
            bail!("DevTools discovery directory must be private");
        }
    }
    if !metadata.is_dir() {
        bail!("Invalid DevTools discovery directory");
    }
    Ok(())
}

pub(crate) fn register_device(
    kind: DeviceKind,
    serial: &str,
    package: &str,
    config: (u16, String),
    project_root: &Path,
    bundle: &Path,
) -> Result<String> {
    let prefix = match kind {
        DeviceKind::Android => "android",
        DeviceKind::Ios => "ios",
    };
    let filename = device_filename(prefix, serial)?;
    let root = discovery_root();
    private_dir(&root)?;
    let directory = root.join("devices");
    private_dir(&directory)?;
    let mut file = tempfile::NamedTempFile::new_in(&directory)?;
    serde_json::to_writer(
        file.as_file_mut(),
        &DeviceSessionEntry {
            device_kind: kind,
            serial: serial.to_owned(),
            package: package.to_owned(),
            remote_port: config.0,
            token: config.1,
            project_root: project_root.to_owned(),
            source_map: ["app.sym", "app.js.map"]
                .into_iter()
                .map(|name| bundle.join(name))
                .find(|path| path.is_file()),
        },
    )?;
    file.flush()?;
    file.persist(directory.join(filename))
        .map_err(|error| error.error)?;
    Ok(format!("{prefix}:{serial}"))
}

fn device_filename(kind: &str, serial: &str) -> Result<String> {
    if !matches!(kind, "android" | "ios")
        || serial.is_empty()
        || !serial
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-'))
    {
        bail!("Invalid DevTools device identifier");
    }
    Ok(format!("{kind}-{serial}.json"))
}

fn local_sessions() -> Result<Vec<LocalSessionEntry>> {
    let directory = discovery_root().join("sessions");
    let files = match std::fs::read_dir(&directory) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut sessions = Vec::new();
    for file in files.flatten() {
        let Some(pid) = file
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if file
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
            || !file.file_type().is_ok_and(|kind| kind.is_file())
        {
            continue;
        }
        let live = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .stdout(Stdio::null())
            .status()
            .map(|status| status.success());
        if matches!(live, Ok(false)) {
            let _ = std::fs::remove_file(file.path());
            continue;
        }
        if let Ok(entry) = read_registration::<LocalSessionEntry>(&file.path())
            && entry.pid == pid
        {
            sessions.push(entry);
        }
    }
    sessions.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    Ok(sessions)
}

fn read_registration<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        bail!("Invalid DevTools registration");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            bail!("DevTools registration must be private");
        }
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub(crate) fn launch_native(attach: Option<&str>) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "The Impasto DevTools desktop app requires macOS. Use --html for the browser client."
        );
    }
    if attach.is_none() {
        // Prune stale records before the client presents its target list.
        let _ = local_sessions();
    }
    let bundled = crate::toolchain::distribution()
        .map(|root| root.join("Impasto DevTools.app"))
        .filter(|app| app.is_dir())
        .or_else(|| {
            let installed = PathBuf::from("/Applications/Impasto DevTools.app");
            installed.is_dir().then_some(installed)
        });
    let application = match bundled {
        Some(path) => path,
        None => {
            let root = crate::toolchain::framework()?.join("examples/devtools-desktop");
            let app =
                crate::app::App::load(&root).context("Load the local DevTools desktop app")?;
            crate::build::macos(
                &app,
                &crate::build::BuildOptions {
                    debug: false,
                    hot: None,
                    verbose: false,
                    trace: false,
                },
            )?
            .path
        }
    };
    let mut command = Command::new("open");
    command.arg("-n");
    for variable in ["PATH", "IMPASTO_DEVTOOLS_DISCOVERY_DIR"] {
        if let Some(value) = std::env::var_os(variable) {
            let mut entry = std::ffi::OsString::from(format!("{variable}="));
            entry.push(value);
            command.arg("--env").arg(entry);
        }
    }
    command.arg(application);
    if let Some(attach) = attach {
        command.args(["--args", "--attach", attach]);
    }
    crate::toolchain::run(command, "Launch Impasto DevTools")
}

pub(crate) fn configure_android(
    adb: &impl Fn() -> Command,
    package: &str,
    port: u16,
    token: &str,
) -> Result<()> {
    let mut child = adb()
        .args(["shell", "-T"])
        .arg(format!(
            "run-as {package} sh -c 'umask 077 && cat > cache/impasto-devtools.tmp && mv cache/impasto-devtools.tmp cache/impasto-devtools'"
        ))
        .stdin(Stdio::piped())
        .spawn()?;
    let write = child
        .stdin
        .take()
        .context("adb stdin")
        .and_then(|mut input| {
            writeln!(input, "{port}\n{token}").context("configure Android DevTools")
        });
    let status = child.wait()?;
    write?;
    if !status.success() {
        bail!("Android DevTools configuration failed ({status})");
    }
    Ok(())
}

pub(crate) fn android_serial() -> Result<String> {
    let output = Command::new("adb").arg("get-serialno").output()?;
    anyhow::ensure!(
        output.status.success(),
        "Select an Android device with -d <serial>"
    );
    let serial = String::from_utf8(output.stdout)?.trim().to_owned();
    device_filename("android", &serial)?;
    Ok(serial)
}

enum Tunnel {
    Ios(std::process::Child),
    Android { serial: Option<String>, port: u16 },
}

fn adb(serial: Option<&str>) -> Command {
    let mut adb = Command::new("adb");
    if let Some(serial) = serial {
        adb.args(["-s", serial]);
    }
    adb
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        match self {
            Self::Ios(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Self::Android { serial, port } => {
                let _ = adb(serial.as_deref())
                    .args(["forward", "--remove", &format!("tcp:{port}")])
                    .status();
            }
        }
    }
}

fn forward(options: &Options) -> Result<(u16, Option<Tunnel>)> {
    let Some(device) = options.device.as_deref() else {
        return Ok((options.port, None));
    };
    if let Some(udid) = device.strip_prefix("ios:") {
        if udid.is_empty() {
            bail!("Expected ios:<USB UDID>");
        }
        let port = options.local_port.unwrap_or(options.port);
        let child = Command::new("iproxy")
            .args(["-s", "127.0.0.1", "-u", udid])
            .arg(format!("{port}:{}", options.port))
            .spawn()
            .context("Start iproxy from libusbmuxd")?;
        return Ok((port, Some(Tunnel::Ios(child))));
    }
    let serial = (device != "android").then(|| device.to_owned());
    let port = options.local_port.unwrap_or(0);
    let output = adb(serial.as_deref())
        .args([
            "forward",
            "--no-rebind",
            &format!("tcp:{port}"),
            &format!("tcp:{}", options.port),
        ])
        .output()
        .context("Start adb forward")?;
    if !output.status.success() {
        bail!(
            "adb forward failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let port = if port == 0 {
        std::str::from_utf8(&output.stdout)?.trim().parse()?
    } else {
        port
    };
    Ok((port, Some(Tunnel::Android { serial, port })))
}

pub(crate) fn run(mut options: Options) -> Result<()> {
    if !options.html {
        let device = if options.attach.is_none() {
            match options.device.as_deref() {
                Some("android") => Some(format!("android:{}", android_serial()?)),
                Some(device) if device.starts_with("ios:") || device.starts_with("android:") => {
                    Some(device.to_owned())
                }
                Some(device) => Some(format!("android:{device}")),
                None => None,
            }
        } else {
            None
        };
        return launch_native(options.attach.as_deref().or(device.as_deref()));
    }
    if let Some(attach) = &options.attach {
        if let Some((kind, serial)) = attach.split_once(':') {
            let path = discovery_root()
                .join("devices")
                .join(device_filename(kind, serial)?);
            let session: DeviceSessionEntry = read_registration(&path)?;
            options.port = session.remote_port;
            options.source_root = Some(session.project_root);
            options.device = Some(match session.device_kind {
                DeviceKind::Android => session.serial,
                DeviceKind::Ios => format!("ios:{}", session.serial),
            });
            eprintln!("DevTools token is stored in {}", path.display());
        } else {
            let pid = attach
                .parse::<u32>()
                .context("expected a local session PID")?;
            let session = local_sessions()?
                .into_iter()
                .find(|session| session.pid == pid)
                .context("DevTools session is unavailable")?;
            options.port = session.port;
            options.source_root = session.project_root;
            eprintln!(
                "DevTools token is stored in {}",
                discovery_root()
                    .join("sessions")
                    .join(format!("{pid}.json"))
                    .display()
            );
        }
    }

    let project = options
        .source_root
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let locations = STANDARD.encode(serde_json::to_vec(&serde_json::json!({
        "project": project.canonicalize()?,
        "framework": crate::toolchain::framework()?,
        "editor": options.editor,
    }))?);
    let compiler = options
        .compiler
        .clone()
        .or_else(|| {
            crate::toolchain::distribution()
                .map(|root| root.join("compiler"))
                .filter(|path| path.join("bundler_bg.wasm").is_file())
        })
        .as_ref()
        .map(|directory| -> Result<_> {
            Ok((
                STANDARD.encode(std::fs::read(directory.join("bundler.js"))?),
                STANDARD.encode(std::fs::read(directory.join("bundler_bg.wasm"))?),
            ))
        })
        .transpose()?
        .unwrap_or_default();
    let symbols = options
        .symbols
        .as_deref()
        .map(crate::symbols::browser_index)
        .transpose()?
        .map(|index| STANDARD.encode(index))
        .unwrap_or_default();
    let (port, tunnel) = forward(&options)?;
    let path = options.client.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("impasto-devtools-{}.html", std::process::id()))
    });
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("Create {}", path.display()))?
        .write_all(
            include_str!("devtools.html")
                .replace("%%PORT%%", &port.to_string())
                .replace("%%COMPILER_JS%%", &compiler.0)
                .replace("%%COMPILER_WASM%%", &compiler.1)
                .replace("%%SYMBOLS%%", &symbols)
                .replace("%%SOURCE_ROOTS%%", &locations)
                .as_bytes(),
        )?;
    println!("Open {} in a browser", path.canonicalize()?.display());
    if tunnel.is_some() {
        println!("Forwarding 127.0.0.1:{port}. Press Enter to close the tunnel.");
        io::stdin().read_line(&mut String::new())?;
    }
    Ok(())
}
