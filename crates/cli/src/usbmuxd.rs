//! Client for the local Apple USB multiplexing daemon.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use plist::{Dictionary, Value};

const SOCKET: &str = "/var/run/usbmuxd";
const MAX_MESSAGE: u32 = 1024 * 1024;
const HEADER: u32 = 16;
const PROTOCOL: u32 = 1;
const PLIST_PACKET: u32 = 8;

#[derive(Debug)]
pub(crate) struct Device {
    pub(crate) id: u32,
    pub(crate) serial: String,
}

fn daemon() -> io::Result<UnixStream> {
    let stream = UnixStream::connect(SOCKET)?;
    let timeout = Some(Duration::from_secs(2));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    Ok(stream)
}

fn request(stream: &mut UnixStream, kind: &str, extra: Dictionary) -> Result<Value> {
    let mut message = extra;
    message.insert("MessageType".into(), kind.into());
    message.insert(
        "ClientVersionString".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    message.insert("ProgName".into(), "impasto".into());
    message.insert("kLibUSBMuxVersion".into(), Value::Integer(3.into()));
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &message)?;
    let size = HEADER + u32::try_from(bytes.len())?;
    for word in [size, PROTOCOL, PLIST_PACKET, 1] {
        stream.write_all(&word.to_le_bytes())?;
    }
    stream.write_all(&bytes)?;
    let len = read_word(stream)?;
    let version = read_word(stream)?;
    let id = read_word(stream)?;
    let tag = read_word(stream)?;
    ensure!(
        (HEADER..=MAX_MESSAGE).contains(&len)
            && version == PROTOCOL
            && id == PLIST_PACKET
            && tag == 1,
        "无效的 usbmuxd 响应帧"
    );
    let mut data = vec![0; (len - HEADER) as usize];
    stream.read_exact(&mut data)?;
    Ok(Value::from_reader_xml(data.as_slice())?)
}

fn read_word(stream: &mut UnixStream) -> io::Result<u32> {
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

pub(crate) fn devices() -> Result<Vec<Device>> {
    let reply = request(&mut daemon()?, "ListDevices", Dictionary::new())?;
    let devices = reply
        .as_dictionary()
        .and_then(|reply| reply.get("DeviceList"))
        .and_then(Value::as_array)
        .context("usbmuxd 未返回设备列表")?;
    Ok(devices
        .iter()
        .filter_map(|entry| {
            let properties = entry.as_dictionary()?.get("Properties")?.as_dictionary()?;
            if properties.get("ConnectionType")?.as_string()? != "USB" {
                return None;
            }
            Some(Device {
                id: u32::try_from(properties.get("DeviceID")?.as_unsigned_integer()?).ok()?,
                serial: properties.get("SerialNumber")?.as_string()?.to_owned(),
            })
        })
        .collect())
}

pub(crate) fn matches(device: &Device, udid: &str) -> bool {
    let normalize = |text: &str| {
        text.bytes()
            .filter(|&byte| byte != b'-')
            .map(|byte| byte.to_ascii_lowercase())
            .collect::<Vec<_>>()
    };
    normalize(&device.serial) == normalize(udid)
}

pub(crate) fn connect(device_id: u32, port: u16) -> Result<UnixStream> {
    let mut stream = daemon()?;
    let mut parameters = Dictionary::new();
    parameters.insert("DeviceID".into(), Value::Integer(device_id.into()));
    parameters.insert("PortNumber".into(), Value::Integer(port.to_be().into()));
    let reply = request(&mut stream, "Connect", parameters)?;
    let code = reply
        .as_dictionary()
        .and_then(|value| value.get("Number"))
        .and_then(Value::as_unsigned_integer);
    ensure!(code == Some(0), "usbmuxd 无法打开设备调试端口 ({code:?})");
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::{Device, matches};

    #[test]
    fn compares_hardware_ids_across_apple_tool_formats() {
        let device = Device {
            id: 1,
            serial: "00008130-ABCD1234".into(),
        };
        assert!(matches(&device, "00008130abcd1234"));
        assert!(!matches(&device, "00008130abcd1235"));
    }
}
