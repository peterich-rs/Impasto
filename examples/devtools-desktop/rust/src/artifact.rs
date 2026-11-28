use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use impasto::ImpastoType;

use crate::DesktopError;

const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(ImpastoType)]
pub struct ArtifactSummary {
    pub name: String,
    pub kind: String,
    pub bytes: f64,
    pub packets: Option<u32>,
}

pub(crate) fn read(path: &Path, file: File) -> Result<ArtifactSummary, DesktopError> {
    let size = file.metadata()?.len();
    if size > MAX_ARTIFACT_BYTES {
        return Err(DesktopError::new("artifact exceeds the size limit"));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Artifact");
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let (kind, packets) = match ext {
        "pftrace" => ("Perfetto trace", Some(trace_packets(BufReader::new(file))?)),
        "impasto-session" | "json" => {
            serde_json::from_reader::<_, serde::de::IgnoredAny>(BufReader::new(file))
                .map_err(DesktopError::new)?;
            ("JSON archive", None)
        }
        "impsym" | "map" => ("JavaScript source map", None),
        _ => return Err(DesktopError::new("unsupported artifact")),
    };
    Ok(ArtifactSummary {
        name: name.to_owned(),
        kind: kind.to_owned(),
        bytes: f64::from(u32::try_from(size).unwrap_or(u32::MAX)),
        packets,
    })
}

fn trace_packets(mut reader: impl BufRead) -> io::Result<u32> {
    let mut packets = 0u32;
    while !reader.fill_buf()?.is_empty() {
        if varint(&mut reader)? != 10 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Perfetto packet tag",
            ));
        }
        let len = varint(&mut reader)?;
        if len > MAX_ARTIFACT_BYTES
            || io::copy(&mut reader.by_ref().take(len), &mut io::sink())? != len
        {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated Perfetto packet",
            ));
        }
        packets = packets.saturating_add(1);
    }
    Ok(packets)
}

fn varint(reader: &mut impl Read) -> io::Result<u64> {
    let mut result = 0;
    for shift in (0..64).step_by(7) {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        let byte = byte[0];
        if shift == 63 && byte > 1 {
            break;
        }
        result |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(result);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid packet length",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_packets_and_rejects_truncation() -> io::Result<()> {
        assert_eq!(trace_packets(&[10, 1, 20, 10, 2, 25, 30][..])?, 2);
        assert!(trace_packets(&[10, 2, 20][..]).is_err());
        Ok(())
    }
}
