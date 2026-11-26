use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::Args;

#[derive(Args)]
pub(crate) struct Options {
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
}

pub(crate) fn config() -> Result<Option<(u16, String)>> {
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

pub(crate) fn run(options: Options) -> Result<()> {
    let compiler = options
        .compiler
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
                .as_bytes(),
        )?;
    println!("Open {} in a browser", path.canonicalize()?.display());
    if tunnel.is_some() {
        println!("Forwarding 127.0.0.1:{port}. Press Enter to close the tunnel.");
        io::stdin().read_line(&mut String::new())?;
    }
    Ok(())
}
