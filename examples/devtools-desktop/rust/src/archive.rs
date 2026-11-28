use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::io::{Seek, Write};
use std::path::Path;

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use impasto::ImpastoType;
use serde::{Deserialize, Serialize};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::analyzer::treemap::Bundle;
use crate::store::Recording;

use crate::DesktopError;

const SESSION_SCHEMA_VERSION: u32 = 2;

const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(ImpastoType)]
pub struct ArtifactSummary {
    pub name: String,
    pub kind: String,
    pub bytes: f64,
    pub packets: Option<u32>,
}

pub(crate) enum Artifact {
    Session(Box<Recording>, Option<Vec<u8>>),
    Bundle(Bundle),
    Symbols(devtools_symbols::SymbolTable),
    Trace(Vec<u8>),
}

#[derive(Serialize)]
struct SessionRef<'a> {
    schema_version: u32,
    session: &'a Recording,
}

#[derive(Deserialize)]
struct Session {
    schema_version: u32,
    session: Recording,
}

pub(crate) fn read(path: &Path, file: File) -> Result<(ArtifactSummary, Artifact), DesktopError> {
    let size = file.metadata()?.len();
    if size > MAX_ARTIFACT_BYTES {
        return Err(DesktopError::new("Artifact exceeds the 128 MiB limit"));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Artifact");
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let (kind, packets, artifact) = match extension {
        "pftrace" => {
            let bytes = read_limited(file)?;
            let packets = trace_packets(bytes.as_slice())?;
            ("Perfetto trace", Some(packets), Artifact::Trace(bytes))
        }
        "zip" => {
            let (recording, trace) = read_zip(file)?;
            (
                "Session recording",
                None,
                Artifact::Session(Box::new(recording), trace),
            )
        }
        "impasto-session" => {
            let mut file = BufReader::new(file);
            let bytes = if file.fill_buf()?.starts_with(&[0x1f, 0x8b]) {
                read_limited(GzDecoder::new(file))?
            } else {
                read_limited(file)?
            };
            (
                "Session recording",
                None,
                Artifact::Session(Box::new(parse_session(&bytes)?), None),
            )
        }
        "json" => (
            "Bundle size report",
            None,
            Artifact::Bundle(Bundle::parse(&read_limited(file)?)?),
        ),
        "impsym" | "sym" | "map" => (
            "JavaScript source map",
            None,
            Artifact::Symbols(
                devtools_symbols::SymbolTable::load(path).map_err(DesktopError::new)?,
            ),
        ),
        _ => return Err(DesktopError::new("Unsupported artifact")),
    };
    Ok((
        ArtifactSummary {
            name: name.to_owned(),
            kind: kind.to_owned(),
            bytes: f64::from(u32::try_from(size).unwrap_or(u32::MAX)),
            packets,
        },
        artifact,
    ))
}

fn parse_session(bytes: &[u8]) -> Result<Recording, DesktopError> {
    let Session {
        schema_version,
        mut session,
    } = serde_json::from_slice(bytes)?;
    if schema_version != SESSION_SCHEMA_VERSION {
        return Err(DesktopError::new("Unsupported session schema"));
    }
    session.clip();
    Ok(session)
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>, DesktopError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(DesktopError::new(
            "Expanded artifact exceeds the 128 MiB limit",
        ));
    }
    Ok(bytes)
}

fn read_zip(file: impl Read + Seek) -> Result<(Recording, Option<Vec<u8>>), DesktopError> {
    let mut zip = ZipArchive::new(file).map_err(DesktopError::new)?;
    if zip.is_empty() || zip.len() > 2 {
        return Err(DesktopError::new("Invalid session archive entries"));
    }
    let mut session = None;
    let mut trace = None;
    let mut size = 0;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(DesktopError::new)?;
        size = u64::saturating_add(size, entry.size());
        if size > MAX_ARTIFACT_BYTES || !entry.is_file() {
            return Err(DesktopError::new(
                "Invalid session archive size or entry type",
            ));
        }
        match entry.name() {
            "session.json" if session.is_none() => {
                session = Some(parse_session(&read_limited(entry)?)?)
            }
            "capture.pftrace" if trace.is_none() => {
                let bytes = read_limited(entry)?;
                trace_packets(bytes.as_slice())?;
                trace = Some(bytes);
            }
            _ => return Err(DesktopError::new("Unexpected session archive entry")),
        }
    }
    Ok((
        session.ok_or_else(|| DesktopError::new("Archive is missing session.json"))?,
        trace,
    ))
}

pub(crate) fn write_session(
    file: File,
    recording: &Recording,
    trace: Option<&[u8]>,
    format: &str,
) -> Result<(), DesktopError> {
    let session = SessionRef {
        schema_version: SESSION_SCHEMA_VERSION,
        session: recording,
    };
    let max = MAX_ARTIFACT_BYTES.saturating_sub(trace.map_or(0, |bytes| bytes.len() as u64));
    if format == "zip" {
        let mut zip = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("session.json", options)
            .map_err(DesktopError::new)?;
        serde_json::to_writer(
            Limited {
                writer: &mut zip,
                left: max,
            },
            &session,
        )?;
        if let Some(trace) = trace {
            zip.start_file("capture.pftrace", options)
                .map_err(DesktopError::new)?;
            zip.write_all(trace)?;
        }
        zip.finish().map_err(DesktopError::new)?;
    } else {
        let mut gzip = GzEncoder::new(file, Compression::fast());
        serde_json::to_writer(
            Limited {
                writer: &mut gzip,
                left: MAX_ARTIFACT_BYTES,
            },
            &session,
        )?;
        gzip.finish()?;
    }
    Ok(())
}

struct Limited<W> {
    writer: W,
    left: u64,
}

impl<W: Write> Write for Limited<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.left {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Session exceeds the 128 MiB archive limit",
            ));
        }
        let count = self.writer.write(bytes)?;
        self.left -= count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
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
    if packets == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Empty Perfetto trace",
        ));
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
    fn round_trips_gzip_and_zip_and_rejects_unexpected_entries() -> Result<(), DesktopError> {
        let mut session = Recording::default();
        session.frames.update(
            false,
            vec![devtools_protocol::FrameRecord {
                id: 9,
                revision: 3,
                ..Default::default()
            }],
        );
        let trace = [10, 1, 20];
        let file = tempfile::tempfile()?;
        write_session(file.try_clone()?, &session, Some(&trace), "zip")?;
        let (restored, payload) = read_zip(file)?;
        assert_eq!(restored.frames.get(9).map(|frame| frame.revision), Some(3));
        assert_eq!(payload.as_deref(), Some(trace.as_slice()));
        let mut file = tempfile::tempfile()?;
        write_session(file.try_clone()?, &session, None, "gzip")?;
        file.rewind()?;
        let restored = parse_session(&read_limited(GzDecoder::new(file))?)?;
        assert_eq!(restored.frames.entries().len(), 1);

        let file = tempfile::tempfile()?;
        let mut zip = ZipWriter::new(file);
        zip.start_file("../session.json", SimpleFileOptions::default())
            .map_err(DesktopError::new)?;
        zip.write_all(b"{}")?;
        assert!(read_zip(zip.finish().map_err(DesktopError::new)?).is_err());
        Ok(())
    }

    #[test]
    fn counts_packets_and_rejects_truncation() -> io::Result<()> {
        assert_eq!(trace_packets(&[10, 1, 20, 10, 2, 25, 30][..])?, 2);
        assert!(trace_packets(&[10, 2, 20][..]).is_err());
        Ok(())
    }
}
