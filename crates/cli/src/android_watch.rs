//! Debug updates over ADB to the application's local development socket.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const READY: u8 = 1;

pub(super) struct Channel {
    stream: TcpStream,
    open: bool,
}

impl Channel {
    pub(super) fn open(adb: &impl Fn() -> Command, package: &str) -> Result<Self> {
        if std::env::var_os("ADB_SERVER_SOCKET").is_some()
            || std::env::var_os("ANDROID_ADB_SERVER_ADDRESS").is_some()
        {
            bail!("使用当前 ADB 服务器配置的文件更新通道");
        }
        let port = std::env::var("ANDROID_ADB_SERVER_PORT")
            .ok()
            .map(|port| port.parse::<u16>())
            .transpose()?
            .unwrap_or(5037);
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into();
        let serial = adb()
            .arg("get-serialno")
            .output()
            .context("读取 Android 设备序列号")?;
        if !serial.status.success() {
            bail!("ADB 无法选择调试设备 ({})", serial.status);
        }
        let serial = std::str::from_utf8(&serial.stdout)?.trim();
        let transport = format!("host:transport:{serial}");
        let service = format!("localabstract:impasto-hot-v1-{package}");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(150))
                && open_service(&mut stream, &transport, &service).is_ok()
            {
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                return Ok(Self { stream, open: true });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        bail!("Android 调试通道未就绪");
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// Returns false when the host has not accepted the request and the caller
    /// should wake the activity through the file-based delivery route.
    pub(super) fn send(&mut self, revision: &str, source: &str) -> Result<bool> {
        let revision = revision.parse::<u64>()?;
        let length = u32::try_from(source.len())?;
        let mut header = [0; 12];
        header[..8].copy_from_slice(&revision.to_be_bytes());
        header[8..].copy_from_slice(&length.to_be_bytes());
        let response = self.exchange(&header, source.as_bytes());
        match response {
            Err(error) => {
                self.open = false;
                Err(error)
            }
            Ok(None) => Ok(false),
            Ok(Some(message)) if message == "ok" => {
                eprintln!("impasto: 调试更新 {revision} 已应用");
                Ok(true)
            }
            Ok(Some(message)) => {
                bail!(
                    "脚本验证失败，仍在运行上一版本。{}",
                    message.trim_start_matches("error\n")
                );
            }
        }
    }

    fn exchange(&mut self, header: &[u8], source: &[u8]) -> Result<Option<String>> {
        self.stream.write_all(header)?;
        self.stream.write_all(source)?;
        let mut length = [0; 4];
        self.stream.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length);
        if length == 0 {
            return Ok(None);
        }
        if length > 1024 * 1024 {
            bail!("Android 调试响应过大");
        }
        let mut message = vec![0; length as usize];
        self.stream.read_exact(&mut message)?;
        Ok(Some(String::from_utf8(message)?))
    }
}

fn open_service(stream: &mut TcpStream, transport: &str, service: &str) -> Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(150)))?;
    stream.set_write_timeout(Some(Duration::from_millis(150)))?;
    // The ADB smart socket switches this connection to the selected device,
    // then to the device's local service. Closing it releases the route.
    for request in [transport, service] {
        let length = u16::try_from(request.len())?;
        write!(stream, "{length:04x}{request}")?;
        let mut response = [0; 4];
        stream.read_exact(&mut response)?;
        if &response != b"OKAY" {
            bail!("ADB 无法连接 Android 调试服务");
        }
    }
    let mut greeting = [0];
    stream.read_exact(&mut greeting)?;
    if greeting[0] != READY {
        bail!("Android 调试协议不匹配");
    }
    Ok(())
}
