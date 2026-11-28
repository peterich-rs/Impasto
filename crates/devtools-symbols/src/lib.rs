//! Offline resolution of generated QuickJS positions using source maps and Impasto sidecars.

use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Read},
    path::Path,
};

use anyhow::{Result, bail};
use flate2::read::GzDecoder;
use oxc_sourcemap::{SourceMap, Token};
use serde::Deserialize;

const FORMAT: &[u8] = b"IMPSYM\x01";
const CHUNK_FORMAT: &[u8] = b"IMPSYM\x02";
const MAX_SOURCE_MAP: u64 = 128 * 1024 * 1024;

type FunctionRange = (String, [u32; 4]);

#[derive(Deserialize)]
struct FunctionNames {
    #[serde(default, rename = "x_impasto_functions")]
    functions: Vec<Vec<FunctionRange>>,
}

/// A source map or chunked release symbols sidecar.
pub struct SymbolTable(ModuleSymbols);

impl SymbolTable {
    /// Loads a bounded plain source map or a compressed Impasto sidecar.
    pub fn load(path: &Path) -> Result<Self> {
        read(path).map(Self)
    }

    /// Loads an in-memory source map for the main script.
    pub fn from_source_map(json: &str) -> Result<Self> {
        if json.len() as u64 > MAX_SOURCE_MAP {
            bail!("source map exceeds the symbolication size limit");
        }
        Ok(Self(BTreeMap::from([(
            "app.js".to_owned(),
            parse_map(json)?,
        )])))
    }

    /// Builds a position index once for a series of stack lookups.
    pub fn resolver(&self) -> Resolver<'_> {
        Resolver(
            self.0
                .iter()
                .map(|(name, (map, names))| (name.as_str(), Symbols::new(map, names)))
                .collect(),
        )
    }

    /// Encodes the map and names for the lightweight browser client.
    pub fn browser_index(&self) -> Result<Vec<u8>> {
        let index = self
            .0
            .iter()
            .map(|(name, (map, names))| {
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
                (
                    name,
                    (
                        map.get_sources().collect::<Vec<_>>(),
                        mappings,
                        &names.functions,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Ok(serde_json::to_vec(&index)?)
    }
}

/// A borrowed lookup index for an open symbol table.
pub struct Resolver<'a>(BTreeMap<&'a str, Symbols<'a>>);

impl Resolver<'_> {
    /// Looks up a one-based generated location.
    pub fn lookup(&self, module: &str, line: u32, column: u32) -> Option<(String, Option<&str>)> {
        self.0.get(module)?.lookup(line, column)
    }

    /// Resolves recognizable QuickJS stack frames, preserving unmapped lines.
    pub fn remap(&self, stack: &str) -> String {
        remap_stack(stack, |name, line, column| self.lookup(name, line, column))
    }
}

fn parse_map(json: &str) -> Result<(SourceMap<'static>, FunctionNames)> {
    Ok((
        SourceMap::from_json_string(json)?.into_owned(),
        serde_json::from_str(json)?,
    ))
}

type ModuleSymbols = BTreeMap<String, (SourceMap<'static>, FunctionNames)>;

fn read(path: &Path) -> Result<ModuleSymbols> {
    let mut file = BufReader::new(fs::File::open(path)?);
    let mut header = [0; FORMAT.len()];
    file.read_exact(&mut header)?;
    let mut json = String::new();
    if header == FORMAT || header == CHUNK_FORMAT {
        GzDecoder::new(file)
            .take(MAX_SOURCE_MAP + 1)
            .read_to_string(&mut json)?;
    } else {
        header
            .as_slice()
            .chain(file)
            .take(MAX_SOURCE_MAP + 1)
            .read_to_string(&mut json)?;
    }
    if json.len() as u64 > MAX_SOURCE_MAP {
        bail!("source map exceeds the symbolication size limit");
    }

    if header != CHUNK_FORMAT {
        return Ok(BTreeMap::from([("app.js".to_owned(), parse_map(&json)?)]));
    }
    serde_json::from_str::<BTreeMap<String, String>>(&json)?
        .into_iter()
        .map(|(name, map)| parse_map(&map).map(|map| (name, map)))
        .collect()
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
}

fn remap_stack<'a>(
    stack: &str,
    lookup: impl Fn(&str, u32, u32) -> Option<(String, Option<&'a str>)>,
) -> String {
    let mut output = String::with_capacity(stack.len());
    for row in stack.split_inclusive('\n') {
        if let Some((start, end, module, line, column)) = stack_position(row)
            && let Some((original, function)) = lookup(module, line, column)
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
fn stack_position(row: &str) -> Option<(usize, usize, &str, u32, u32)> {
    let frame = row.trim_start().strip_prefix("at ")?;
    let base = row.len() - frame.len();
    let frame = if let Some((_, file)) = frame.rsplit_once('(') {
        file
    } else {
        frame
    };
    let start = base + (row[base..].len() - frame.len());
    let (module, location) = frame.split_once(':')?;
    let module = match module {
        "app.js" | "app.bin" => "app.js",
        other if other.starts_with("chunks/") && other.ends_with(".js") => other,
        _ => return None,
    };
    let (line, columns) = location.split_once(':')?;
    let column_len = columns.bytes().take_while(u8::is_ascii_digit).count();
    let end = row.len() - columns.len() + column_len;
    Some((
        start,
        end,
        module,
        line.parse().ok()?,
        columns[..column_len].parse().ok()?,
    ))
}
