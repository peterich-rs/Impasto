//! Release source map sidecars and QuickJS location lookup.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
};

use anyhow::{Context, Result, bail};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use oxc_sourcemap::{SourceMap, Token};
use serde::{Deserialize, Serialize};

mod trace;

const FORMAT: &[u8] = b"IMPSYM\x01";
const MAX_SOURCE_MAP: u64 = 128 * 1024 * 1024;

type FunctionRange = (String, [u32; 4]);

#[derive(Serialize, Deserialize)]
struct FunctionNames {
    #[serde(default, rename = "x_impasto_functions")]
    functions: Vec<Vec<FunctionRange>>,
}

pub(crate) fn write(path: &Path, source_map: &str) -> Result<()> {
    let mut source_map = SourceMap::from_json_string(source_map)?;
    let functions = source_map
        .get_sources()
        .zip(source_map.get_source_contents())
        .map(|(path, source)| {
            source.map_or_else(Vec::new, |text| {
                bundler::function_symbols(Path::new(path), text)
            })
        })
        .collect();
    source_map.set_source_contents(Vec::new());
    let mut data = source_map.to_json_string();
    // Store original names as a standard source map extension. Older readers
    // can still read the positions while sourcesContent remains stripped.
    let mut extension = serde_json::to_string(&FunctionNames { functions })?;
    data.pop();
    extension.replace_range(..1, ",");
    data.push_str(&extension);
    let mut output =
        tempfile::NamedTempFile::new_in(path.parent().context("symbol file has no directory")?)?;
    output.write_all(FORMAT)?;
    let mut compressed = GzEncoder::new(output.as_file_mut(), Compression::default());
    compressed.write_all(data.as_bytes())?;
    compressed.finish()?;
    output.persist(path)?;
    Ok(())
}

fn read(path: &Path) -> Result<(SourceMap<'static>, FunctionNames)> {
    let mut file = BufReader::new(fs::File::open(path)?);
    let mut header = [0; FORMAT.len()];
    file.read_exact(&mut header)?;
    if header != FORMAT {
        bail!("unsupported Impasto symbol table");
    }
    let mut json = String::new();
    GzDecoder::new(file)
        .take(MAX_SOURCE_MAP + 1)
        .read_to_string(&mut json)?;
    if json.len() as u64 > MAX_SOURCE_MAP {
        bail!("source map exceeds the symbolication size limit");
    }
    Ok((
        SourceMap::from_json_string(&json)?.into_owned(),
        serde_json::from_str(&json)?,
    ))
}

struct Symbols<'a> {
    map: &'a SourceMap<'a>,
    positions: Vec<&'a [Token]>,
    names: &'a FunctionNames,
}

impl<'a> Symbols<'a> {
    fn new(map: &'a SourceMap<'a>, names: &'a FunctionNames) -> Self {
        Self {
            map,
            positions: map.generate_lookup_table(),
            names,
        }
    }

    fn lookup(&self, line: u32, column: u32) -> Option<(String, Option<&str>)> {
        let target = self.map.lookup_source_view_token(
            &self.positions,
            line.checked_sub(1)?,
            column.checked_sub(1)?,
        )?;
        let source = target.get_source()?;
        let original = format!(
            "{source}:{}:{}",
            target.get_src_line() + 1,
            target.get_src_col() + 1
        );
        let position = (target.get_src_line(), target.get_src_col());
        let function = target
            .get_source_id()
            .and_then(|id| self.names.functions.get(id as usize))
            .and_then(|ranges| {
                ranges
                    .iter()
                    .rev()
                    .find(|(_, [first_line, first_col, last_line, last_col])| {
                        (*first_line, *first_col) <= position && position < (*last_line, *last_col)
                    })
            })
            .map(|(name, _)| name.as_str());
        Some((original, function))
    }

    fn stack(&self, stack: &str) -> String {
        let mut output = String::with_capacity(stack.len());
        for row in stack.split_inclusive('\n') {
            if let Some((start, end, line, column)) = stack_position(row)
                && let Some((original, function)) = self.lookup(line, column)
            {
                let prefix = &row[..start];
                if let Some(name) = function
                    && prefix.ends_with('(')
                {
                    let indent = prefix.len() - prefix.trim_start().len();
                    output.push_str(&prefix[..indent]);
                    output.push_str("at ");
                    output.push_str(name);
                    output.push_str(" (");
                } else {
                    output.push_str(prefix);
                }
                output.push_str(&original);
                output.push_str(&row[end..]);
            } else {
                output.push_str(row);
            }
        }
        output
    }
}

pub(crate) fn symbolicate(
    path: &Path,
    position: Option<(u32, u32)>,
    stack: Option<&Path>,
    trace: Option<(&Path, &Path)>,
) -> Result<()> {
    let (map, names) = read(path)?;
    let symbols = Symbols::new(&map, &names);
    if let Some((input, output)) = trace {
        return trace::symbolicate(input, output, &symbols);
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Some(path) = stack {
        let mut input = BufReader::new(fs::File::open(path)?);
        let mut row = String::new();
        while input.read_line(&mut row)? > 0 {
            out.write_all(symbols.stack(&row).as_bytes())?;
            row.clear();
        }
    } else if let Some((line, column)) = position {
        let (location, _) = symbols
            .lookup(line, column)
            .context("no matching generated location (line and column are 1-based)")?;
        writeln!(out, "{location}")?;
    }
    Ok(())
}

pub(crate) fn browser_index(path: &Path) -> Result<Vec<u8>> {
    let (map, names) = read(path)?;
    let mappings = map
        .get_tokens()
        .map(|token| {
            [
                token.get_dst_line(),
                token.get_dst_col(),
                token.get_source_id().unwrap_or(u32::MAX),
                token.get_src_line(),
                token.get_src_col(),
            ]
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_vec(&(
        map.get_sources().collect::<Vec<_>>(),
        mappings,
        names.functions,
    ))?)
}

fn stack_position(row: &str) -> Option<(usize, usize, u32, u32)> {
    let frame = row.trim_start().strip_prefix("at ")?;
    let base = row.len() - frame.len();
    let frame = if let Some((_, file)) = frame.rsplit_once('(') {
        file
    } else {
        frame
    };
    let start = base + (row[base..].len() - frame.len());
    let location = frame
        .strip_prefix("app.js:")
        .or_else(|| frame.strip_prefix("app.bin:"))?;
    let (line, columns) = location.split_once(':')?;
    let column_len = columns.bytes().take_while(u8::is_ascii_digit).count();
    let end = row.len() - columns.len() + column_len;
    Some((
        start,
        end,
        line.parse().ok()?,
        columns[..column_len].parse().ok()?,
    ))
}
