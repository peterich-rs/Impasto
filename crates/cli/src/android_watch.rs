//! ADB Smart Socket and run-as transports for the Android debug host.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use dev_protocol::{Frame, Kind, Receipt};

use crate::dev::{self, FileDelivery, Stream};

pub(crate) fn adb(serial: &str) -> Command {
    let mut command = Command::new("adb");
    command.args(["-s", serial]);
    command
}

pub(crate) fn uses_file_transport() -> bool {
    std::env::var_os("ADB_SERVER_SOCKET").is_some()
        || std::env::var_os("ANDROID_ADB_SERVER_ADDRESS").is_some()
}

pub(crate) fn open(serial: &str, package: &str) -> Result<Stream> {
    let port = std::env::var("ANDROID_ADB_SERVER_PORT")
        .ok()
        .map(|port| port.parse::<u16>())
        .transpose()?
        .unwrap_or(5037);
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into();
    let timeout = Duration::from_secs(1);
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    for request in [
        format!("host:transport:{serial}"),
        format!("localabstract:impasto-dev-v2-{package}"),
    ] {
        let length = u16::try_from(request.len())?;
        write!(stream, "{length:04x}{request}")?;
        let mut response = [0; 4];
        stream.read_exact(&mut response)?;
        ensure!(&response == b"OKAY", "ADB 无法连接 Android 调试服务");
    }
    Ok(Stream::Tcp(stream))
}

pub(crate) fn configure_session(
    serial: &str,
    package: &str,
    token: Option<&str>,
    verbose: bool,
) -> Result<()> {
    let mut command = adb(serial);
    command.args(["shell", "-T", "run-as", package, "sh", "-c",
        "'umask 077; rm -rf files/impasto-debug; mkdir -p files/impasto-debug && cat > files/impasto-debug/session'"]);
    let session = token.map_or_else(String::new, |token| {
        format!("{token}\n{}\n", u8::from(verbose))
    });
    send_input(command, session.as_bytes()).map(|_| ())
}

fn send_input(mut command: Command, bytes: &[u8]) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let sent = child
        .stdin
        .take()
        .context("ADB stdin")
        .and_then(|mut input| input.write_all(bytes).map_err(Into::into));
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "ADB 文件调试命令失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    sent?;
    Ok(output.stdout)
}

pub(crate) struct Files {
    serial: String,
    package: String,
    token: String,
    last_log: u64,
}

impl Files {
    pub(crate) fn new(serial: String, package: String, token: String) -> Self {
        Self {
            serial,
            package,
            token,
            last_log: 0,
        }
    }
}

impl FileDelivery for Files {
    fn exchange(&mut self, frame: &Frame, reply: Kind) -> Result<Receipt> {
        let name = format!("files/impasto-debug/request-{}", frame.seq);
        let response = format!("files/impasto-debug/reply-{}", frame.seq);
        let mut command = adb(&self.serial);
        command.args(["shell", "-T", "run-as", &self.package, "sh", "-c"]).arg(format!(
            "'umask 077; cat > {name}.upload && mv {name}.upload {name} || exit 1;              i=0; until [ -f {response} ] || [ \"$i\" -ge 60 ]; do sleep 0.2; i=$((i+1)); done;              cat {response} && rm -f {response}'",
        ));
        let bytes = send_input(command, &dev::file_request(&self.token, frame)?)?;
        dev::file_receipt(&bytes, frame.seq, reply, &mut self.last_log)
    }
}
