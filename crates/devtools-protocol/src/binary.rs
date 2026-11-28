//! Fixed-width binary message framing.

use std::{error::Error, fmt};

use crate::{MAX_BINARY_PAYLOAD, PROTOCOL_VERSION_V2};

pub const BINARY_MAGIC: [u8; 4] = *b"IMDT";
pub const BINARY_FLAG_ZSTD: u16 = 1;
pub const BINARY_HEADER_LEN: usize = 20;

/// Payload interpretation, independent of the originating RPC.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryMsgType {
    RpcResponse = 1,
    TraceExport = 2,
    NetBody = 3,
    ViewSnapshot = 4,
    FrameRepaints = 5,
}

/// Parsed header of a single complete WebSocket binary message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BinaryEnvelopeHeader {
    pub msg_type: BinaryMsgType,
    pub flags: u16,
    pub request_id: u64,
    pub payload_len: u32,
}

/// Invalid envelope or unsupported wire encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryError {
    Header,
    MessageType,
    Flags,
    PayloadLength,
}

impl fmt::Display for BinaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Header => "invalid binary envelope header",
            Self::MessageType => "unknown binary message type",
            Self::Flags => "unsupported binary envelope flags",
            Self::PayloadLength => "invalid binary payload length",
        })
    }
}

impl Error for BinaryError {}

impl BinaryEnvelopeHeader {
    /// Encodes the header in wire order. The caller appends exactly `payload_len` bytes.
    pub fn encode(self) -> [u8; BINARY_HEADER_LEN] {
        let mut bytes = [0; BINARY_HEADER_LEN];
        bytes[..4].copy_from_slice(&BINARY_MAGIC);
        bytes[4] = PROTOCOL_VERSION_V2;
        bytes[5] = self.msg_type as u8;
        bytes[6..8].copy_from_slice(&self.flags.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        bytes[16..].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes
    }

    /// Parses a complete message without allocating or copying its payload.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), BinaryError> {
        let (header, payload) = bytes
            .split_at_checked(BINARY_HEADER_LEN)
            .ok_or(BinaryError::Header)?;
        let &[
            b'I',
            b'M',
            b'D',
            b'T',
            PROTOCOL_VERSION_V2,
            kind,
            f0,
            f1,
            a,
            b,
            c,
            d,
            e,
            f,
            g,
            h,
            l0,
            l1,
            l2,
            l3,
        ] = header
        else {
            return Err(BinaryError::Header);
        };
        let msg_type = match kind {
            1 => BinaryMsgType::RpcResponse,
            2 => BinaryMsgType::TraceExport,
            3 => BinaryMsgType::NetBody,
            4 => BinaryMsgType::ViewSnapshot,
            5 => BinaryMsgType::FrameRepaints,
            _ => return Err(BinaryError::MessageType),
        };
        let flags = u16::from_le_bytes([f0, f1]);
        if flags & !BINARY_FLAG_ZSTD != 0 {
            return Err(BinaryError::Flags);
        }
        let request_id = u64::from_le_bytes([a, b, c, d, e, f, g, h]);
        let payload_len = u32::from_le_bytes([l0, l1, l2, l3]);
        if payload.len() != payload_len as usize || payload.len() > MAX_BINARY_PAYLOAD {
            return Err(BinaryError::PayloadLength);
        }
        Ok((
            Self {
                msg_type,
                flags,
                request_id,
                payload_len,
            },
            payload,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_wire_bytes_and_payload_length() {
        let header = BinaryEnvelopeHeader {
            msg_type: BinaryMsgType::NetBody,
            flags: BINARY_FLAG_ZSTD,
            request_id: 0x0807_0605_0403_0201,
            payload_len: 2,
        };
        assert_eq!(
            header.encode(),
            *b"IMDT\x02\x03\x01\0\x01\x02\x03\x04\x05\x06\x07\x08\x02\0\0\0"
        );
        let mut frame = header.encode().to_vec();
        frame.extend([11, 22]);
        assert_eq!(
            BinaryEnvelopeHeader::decode(&frame),
            Ok((header, &[11, 22][..]))
        );
        assert_eq!(
            BinaryEnvelopeHeader::decode(&frame[..21]),
            Err(BinaryError::PayloadLength)
        );
    }
}
