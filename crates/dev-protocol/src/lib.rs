//! Wire types shared by the development CLI and debug embedders.

use std::io::{self, Read, Write};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

/// Maximum frame payload, including message-specific headers.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
/// Version advertised by the handshake and every frame header.
pub const VERSION: u16 = 2;

const HEADER_LEN: usize = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 0x01,
    HelloAck = 0x02,
    SourceUpdate = 0x10,
    UpdateReceipt = 0x11,
    AssetUpdate = 0x12,
    AssetRemove = 0x13,
    AssetReceipt = 0x14,
    ErrorOverlayShow = 0x20,
    ErrorOverlayClear = 0x21,
    LogBatch = 0x30,
    Ping = 0x40,
    Pong = 0x41,
}

impl TryFrom<u8> for Kind {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Ok(match value {
            0x01 => Self::Hello,
            0x02 => Self::HelloAck,
            0x10 => Self::SourceUpdate,
            0x11 => Self::UpdateReceipt,
            0x12 => Self::AssetUpdate,
            0x13 => Self::AssetRemove,
            0x14 => Self::AssetReceipt,
            0x20 => Self::ErrorOverlayShow,
            0x21 => Self::ErrorOverlayClear,
            0x30 => Self::LogBatch,
            0x40 => Self::Ping,
            0x41 => Self::Pong,
            _ => return Err(invalid("unknown development message")),
        })
    }
}

#[derive(Debug)]
pub struct Frame {
    pub kind: Kind,
    pub seq: u64,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; HEADER_LEN];
        reader.read_exact(&mut header)?;
        if &header[..4] != b"IDEV" || u16::from_be_bytes([header[4], header[5]]) != VERSION {
            return Err(invalid("development protocol version mismatch"));
        }
        let kind = Kind::try_from(header[6])?;
        let seq = u64::from_be_bytes(header[7..15].try_into().map_err(io::Error::other)?);
        let len = u32::from_be_bytes(header[15..].try_into().map_err(io::Error::other)?) as usize;
        if len > MAX_PAYLOAD {
            return Err(invalid("development message exceeds the payload limit"));
        }
        let mut payload = vec![0; len];
        reader.read_exact(&mut payload)?;
        Ok(Self { kind, seq, payload })
    }

    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(invalid("development message exceeds the payload limit"));
        }
        let mut header = [0; HEADER_LEN];
        header[..4].copy_from_slice(b"IDEV");
        header[4..6].copy_from_slice(&VERSION.to_be_bytes());
        header[6] = self.kind as u8;
        header[7..15].copy_from_slice(&self.seq.to_be_bytes());
        header[15..].copy_from_slice(
            &u32::try_from(self.payload.len())
                .map_err(io::Error::other)?
                .to_be_bytes(),
        );
        writer.write_all(&header)?;
        writer.write_all(&self.payload)
    }

    pub fn json(kind: Kind, seq: u64, payload: &impl Serialize) -> io::Result<Self> {
        Ok(Self {
            kind,
            seq,
            payload: serde_json::to_vec(payload)?,
        })
    }

    pub fn decode<'a, T: Deserialize<'a>>(&'a self) -> io::Result<T> {
        serde_json::from_slice(&self.payload).map_err(Into::into)
    }

    pub fn source(seq: u64, patch: bool, code: &str) -> io::Result<Self> {
        if code.len() > MAX_PAYLOAD - 9 {
            return Err(invalid("development source exceeds the payload limit"));
        }
        let mut payload = Vec::with_capacity(9 + code.len());
        payload.extend(seq.to_be_bytes());
        payload.push(if patch { 1 } else { 2 });
        payload.extend(code.as_bytes());
        Ok(Self {
            kind: Kind::SourceUpdate,
            seq,
            payload,
        })
    }

    pub fn decode_source(&self) -> io::Result<(bool, &str)> {
        let rest = self.revision_payload()?;
        let (mode, code) = rest
            .split_first()
            .ok_or_else(|| invalid("missing source mode"))?;
        let patch = match mode {
            1 => true,
            2 => false,
            _ => return Err(invalid("unknown source mode")),
        };
        Ok((patch, std::str::from_utf8(code).map_err(io::Error::other)?))
    }

    pub fn asset(seq: u64, path: &str, bytes: Option<Vec<u8>>) -> io::Result<Self> {
        validate_asset_path(path)?;
        let mut payload = seq.to_be_bytes().to_vec();
        let kind = if let Some(bytes) = bytes {
            if bytes.len().saturating_add(path.len()) > MAX_PAYLOAD - 10 {
                return Err(invalid("development asset exceeds the payload limit"));
            }
            let len = u16::try_from(path.len()).map_err(io::Error::other)?;
            payload.reserve(2 + path.len() + bytes.len());
            payload.extend(len.to_be_bytes());
            payload.extend(path.as_bytes());
            payload.extend(bytes);
            Kind::AssetUpdate
        } else {
            payload.extend(path.as_bytes());
            Kind::AssetRemove
        };
        Ok(Self { kind, seq, payload })
    }

    pub fn decode_asset(&self) -> io::Result<(&str, Option<&[u8]>)> {
        let rest = self.revision_payload()?;
        let (path, bytes) = if self.kind == Kind::AssetUpdate {
            let (len, rest) = rest
                .split_at_checked(2)
                .ok_or_else(|| invalid("missing asset path length"))?;
            let count = u16::from_be_bytes([len[0], len[1]]) as usize;
            let (path, bytes) = rest
                .split_at_checked(count)
                .ok_or_else(|| invalid("truncated asset path"))?;
            (path, Some(bytes))
        } else {
            (rest, None)
        };
        let path = std::str::from_utf8(path).map_err(io::Error::other)?;
        validate_asset_path(path)?;
        Ok((path, bytes))
    }

    fn revision_payload(&self) -> io::Result<&[u8]> {
        let (revision, rest) = self
            .payload
            .split_at_checked(8)
            .ok_or_else(|| invalid("missing update revision"))?;
        if revision != self.seq.to_be_bytes() {
            return Err(invalid("update revision does not match its frame"));
        }
        Ok(rest)
    }
}

#[derive(Deserialize, Serialize)]
pub struct Hello<'a> {
    pub token: &'a str,
    pub protocol: u16,
    pub cli_version: &'a str,
}

#[derive(Deserialize, Serialize)]
pub struct HelloAck {
    pub ok: bool,
    pub platform: String,
    pub app_id: String,
    pub capabilities: Vec<String>,
    pub process: u64,
    pub instance: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Receipt {
    pub revision: u64,
    pub ok: bool,
    pub applied: String,
    pub apply_ms: u64,
    pub error: Option<String>,
}

impl Receipt {
    pub fn new(revision: u64, apply_ms: u64, result: Result<&str, String>) -> Self {
        let (applied, error) = match result {
            Ok(applied) => (applied, None),
            Err(error) => ("rejected", Some(error)),
        };
        Self {
            revision,
            ok: error.is_none(),
            applied: applied.to_owned(),
            apply_ms,
            error,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ErrorOverlay {
    pub source: String,
    pub title: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub snippet: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogEntry {
    pub seq: u64,
    pub ts_ms: u64,
    pub level: String,
    pub target: String,
    pub message: String,
    pub frame: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LogBatch {
    pub entries: Vec<LogEntry>,
    #[serde(default)]
    pub dropped: u64,
}

pub fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn validate_asset_path(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || !Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid("invalid relative asset path"));
    }
    Ok(())
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[cfg(test)]
mod tests {
    use super::{Frame, HEADER_LEN, Kind, MAX_PAYLOAD};

    #[test]
    fn asset_frames_round_trip_and_reject_traversal() {
        let frame = Frame::asset(42, "images/icon.png", Some(vec![0, 255])).unwrap();
        let mut bytes = Vec::new();
        frame.write(&mut bytes).unwrap();
        assert_eq!(&bytes[..7], b"IDEV\0\x02\x12");
        let decoded = Frame::read(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded.kind, Kind::AssetUpdate);
        assert_eq!(
            decoded.decode_asset().unwrap(),
            ("images/icon.png", Some(&[0, 255][..]))
        );
        assert!(Frame::asset(42, "images/../escape", None).is_err());
        bytes[HEADER_LEN - 4..HEADER_LEN]
            .copy_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
        assert!(Frame::read(&mut bytes.as_slice()).is_err());
    }
}
