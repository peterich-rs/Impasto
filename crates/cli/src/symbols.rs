//! Release source map sidecars and QuickJS location lookup.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

use anyhow::{Context, Result};
use flate2::{Compression, write::GzEncoder};
use oxc_sourcemap::SourceMap;
use serde::{Deserialize, Serialize};

mod trace;

const FORMAT: &[u8] = b"IMPSYM\x01";
const CHUNK_FORMAT: &[u8] = b"IMPSYM\x02";

type FunctionRange = (String, [u32; 4]);

#[derive(Serialize, Deserialize)]
struct FunctionNames {
    #[serde(default, rename = "x_impasto_functions")]
    functions: Vec<Vec<FunctionRange>>,
}

pub(crate) fn write(path: &Path, source_map: &str) -> Result<()> {
    write_data(path, FORMAT, &strip_symbols(source_map)?)
}

pub(crate) fn write_chunks(
    path: &Path,
    source_map: &str,
    chunks: &BTreeMap<String, bundler::ChunkOutput>,
) -> Result<()> {
    let mut maps = BTreeMap::from([("app.js", strip_symbols(source_map)?)]);
    for (name, chunk) in chunks {
        if let Some(map) = &chunk.source_map {
            maps.insert(name, strip_symbols(map)?);
        }
    }
    write_data(path, CHUNK_FORMAT, &serde_json::to_string(&maps)?)
}

fn strip_symbols(source_map: &str) -> Result<String> {
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
    Ok(data)
}

fn write_data(path: &Path, format: &[u8], data: &str) -> Result<()> {
    let mut output =
        tempfile::NamedTempFile::new_in(path.parent().context("symbol file has no directory")?)?;
    output.write_all(format)?;
    let mut compressed = GzEncoder::new(output.as_file_mut(), Compression::default());
    compressed.write_all(data.as_bytes())?;
    compressed.finish()?;
    output.persist(path)?;
    Ok(())
}

pub(crate) fn symbolicate(
    path: &Path,
    position: Option<(u32, u32)>,
    stack: Option<&Path>,
    trace: Option<(&Path, &Path)>,
) -> Result<()> {
    let table = devtools_symbols::SymbolTable::load(path)?;
    let resolver = table.resolver();
    let remap = |input: &str| resolver.remap(input);
    if let Some((input, output)) = trace {
        return trace::symbolicate(input, output, &remap);
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Some(path) = stack {
        let mut input = BufReader::new(fs::File::open(path)?);
        let mut row = String::new();
        while input.read_line(&mut row)? > 0 {
            out.write_all(remap(&row).as_bytes())?;
            row.clear();
        }
    } else if let Some((line, column)) = position {
        let (location, _) = resolver
            .lookup("app.js", line, column)
            .context("no matching generated location (line and column are 1-based)")?;
        writeln!(out, "{location}")?;
    }
    Ok(())
}

pub(crate) fn browser_index(path: &Path) -> Result<Vec<u8>> {
    devtools_symbols::SymbolTable::load(path)?.browser_index()
}
