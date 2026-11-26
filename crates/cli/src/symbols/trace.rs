//! Enrich Perfetto TrackEvents while preserving other fields and packet types.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

const MAX_PACKET_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn symbolicate(
    input: &Path,
    output: &Path,
    remap: &impl Fn(&str) -> String,
) -> Result<()> {
    let mut input = BufReader::new(File::open(input)?);
    let directory = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        let mut packet = Vec::new();
        while !input.fill_buf()?.is_empty() {
            // Trace.packet, wire type length-delimited.
            ensure!(
                varint(&mut input)? == 10,
                "expected a Perfetto trace packet"
            );
            let len = usize::try_from(varint(&mut input)?)?;
            ensure!(len <= MAX_PACKET_BYTES, "Perfetto packet exceeds 64 MiB");
            packet.resize(len, 0);
            input.read_exact(&mut packet)?;
            let rewritten = annotate_packet(&packet, remap)?;
            delimited(&mut writer, 1, rewritten.as_deref().unwrap_or(&packet))?;
        }
        writer.flush()?;
    }
    temporary.persist(output)?;
    Ok(())
}

fn annotate_packet(bytes: &[u8], remap: &impl Fn(&str) -> String) -> Result<Option<Vec<u8>>> {
    let mut input = bytes;
    while !input.is_empty() {
        let start = bytes.len() - input.len();
        let (tag, value) = field(&mut input)?;
        if tag == 11
            && let Some(event) = value
            && let Some(stack) = source_stack(event, remap)?
        {
            let mut annotation = Vec::new();
            delimited(&mut annotation, 10, b"js_source_stack")?;
            delimited(&mut annotation, 6, stack.as_bytes())?;
            let mut augmented = event.to_vec();
            delimited(&mut augmented, 4, &annotation)?;
            let mut packet = bytes[..start].to_vec();
            delimited(&mut packet, 11, &augmented)?;
            packet.extend_from_slice(input);
            return Ok(Some(packet));
        }
    }
    Ok(None)
}

fn source_stack(mut event: &[u8], remap: &impl Fn(&str) -> String) -> Result<Option<String>> {
    let mut stack = None;
    while !event.is_empty() {
        let (tag, value) = field(&mut event)?;
        let Some(mut annotation) = value.filter(|_| tag == 4) else {
            continue;
        };
        let mut name = None;
        let mut text = None;
        while !annotation.is_empty() {
            let (tag, value) = field(&mut annotation)?;
            match tag {
                10 => name = value,
                6 => text = value,
                _ => {}
            }
        }
        match name {
            Some(b"js_source_stack") => return Ok(None),
            Some(b"js_stack") => stack = text,
            _ => {}
        }
    }
    let Some(stack) = stack else { return Ok(None) };
    let generated = std::str::from_utf8(stack)?;
    let original = remap(generated);
    Ok((original != generated).then_some(original))
}

fn field<'a>(bytes: &mut &'a [u8]) -> Result<(u64, Option<&'a [u8]>)> {
    let key = varint(bytes)?;
    let length = match key & 7 {
        0 => {
            varint(bytes)?;
            return Ok((key >> 3, None));
        }
        1 => 8,
        2 => usize::try_from(varint(bytes)?)?,
        5 => 4,
        _ => bail!("invalid Perfetto protobuf wire type"),
    };
    let (value, remaining) = bytes
        .split_at_checked(length)
        .context("truncated Perfetto field")?;
    *bytes = remaining;
    Ok((key >> 3, (key & 7 == 2).then_some(value)))
}

fn varint(reader: &mut impl Read) -> Result<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let mut byte = [0; 1];
        reader.read_exact(&mut byte)?;
        let byte = byte[0];
        ensure!(shift != 63 || byte <= 1, "invalid protobuf varint");
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    bail!("invalid protobuf varint")
}

fn delimited(writer: &mut impl Write, tag: u64, bytes: &[u8]) -> Result<()> {
    for mut value in [(tag << 3) | 2, u64::try_from(bytes.len())?] {
        while value >= 128 {
            writer.write_all(&[value.to_le_bytes()[0] | 128])?;
            value >>= 7;
        }
        writer.write_all(&[value.to_le_bytes()[0]])?;
    }
    writer.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use oxc_sourcemap::SourceMap;

    use super::*;

    #[test]
    fn enriches_quickjs_frames_without_replacing_original_trace_payloads() -> Result<()> {
        let map = SourceMap::from_json_string(
            r#"{"version":3,"sources":["src/widget.tsx"],"names":[],"mappings":"AACA"}"#,
        )?;
        let names = super::super::FunctionNames {
            functions: vec![vec![("OriginalWidget".into(), [1, 0, 3, 0])]],
        };
        let symbols = super::super::Symbols::new(&map, &names);
        let remap = |input: &str| {
            super::super::remap_stack(input, |_, line, column| symbols.lookup(line, column))
        };
        let mut annotation = Vec::new();
        delimited(&mut annotation, 10, b"js_stack")?;
        delimited(
            &mut annotation,
            6,
            b"    at Widget (app.js:1:2)\n    at native (native)\n",
        )?;
        let mut event = Vec::new();
        delimited(&mut event, 4, &annotation)?;
        delimited(&mut event, 73, b"unrecognized event field")?;
        let mut packet = Vec::new();
        delimited(&mut packet, 11, &event)?;
        delimited(&mut packet, 99, b"unrecognized packet field")?;

        let mapped = annotate_packet(&packet, &remap)?.context("expected a mapped packet")?;
        for preserved in [
            event.as_slice(),
            b"unrecognized packet field",
            b"    at OriginalWidget (src/widget.tsx:2:1)\n    at native (native)\n",
        ] {
            assert!(
                mapped
                    .windows(preserved.len())
                    .any(|slice| slice == preserved)
            );
        }
        assert!(annotate_packet(&mapped, &remap)?.is_none());
        Ok(())
    }
}
