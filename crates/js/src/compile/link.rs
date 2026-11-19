//! Reads the static module header written by the pinned QuickJS compiler.
//! QuickJS resolves specifiers in compile-only mode, but normally checks
//! imported names only when it starts evaluating the module. Reading the
//! compiler-owned header lets us check the same links without running code.

use std::collections::{BTreeMap, HashSet};

use rquickjs::{Ctx, qjs};

use crate::bytecode::{BC_VERSION, BytecodeError};
use crate::modules::normalize;

const MODULE_TAG: u8 = 13;
const HEADER_BYTES: usize = 5; // Version and u32 checksum.
const MAX_LINK_DEPTH: usize = 512;

pub(super) type Modules = BTreeMap<String, ModuleLinks>;

#[derive(Default)]
pub(super) struct ModuleLinks {
    requests: Vec<String>,
    exports: BTreeMap<String, Export>,
    stars: Vec<usize>,
    imports: Vec<(u32, Dependency)>,
}

enum Export {
    Local(u32),
    Indirect(Dependency),
    Native,
}

struct Dependency {
    request: usize,
    name: ImportName,
}

enum ImportName {
    Named(String),
    Namespace,
}

impl ModuleLinks {
    pub(super) fn native<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            exports: names
                .into_iter()
                .map(|name| (name.to_owned(), Export::Native))
                .collect(),
            ..Self::default()
        }
    }

    pub(super) fn export_count(&self) -> usize {
        self.exports.len()
    }

    fn dependency<'a>(
        &'a self,
        dependency: &'a Dependency,
    ) -> Result<(&'a str, &'a ImportName), BytecodeError> {
        let target = self.requests.get(dependency.request).ok_or_else(invalid)?;
        Ok((target, &dependency.name))
    }
}

fn invalid() -> BytecodeError {
    BytecodeError::Script("the QuickJS module header does not match the pinned format".into())
}

pub(super) fn read(ctx: &Ctx<'_>, bytes: &[u8]) -> Result<ModuleLinks, BytecodeError> {
    if bytes.first() != Some(&BC_VERSION) {
        return Err(invalid());
    }
    let mut input = Input(bytes.get(HEADER_BYTES..).ok_or_else(invalid)?);
    let atoms = (0..input.number()?)
        .map(|_| input.table_atom())
        .collect::<Result<Vec<_>, _>>()?;
    if input.byte()? != MODULE_TAG {
        return Err(invalid());
    }
    let _module_name = input.number()?;
    let mut module = ModuleLinks::default();
    for _ in 0..input.number()? {
        module.requests.push(input.atom(ctx, &atoms)?);
    }
    for _ in 0..input.number()? {
        let export = match input.byte()? {
            0 => Export::Local(input.number()?),
            1 => Export::Indirect(input.dependency(ctx, &atoms)?),
            _ => return Err(invalid()),
        };
        module.exports.insert(input.atom(ctx, &atoms)?, export);
    }
    for _ in 0..input.number()? {
        module.stars.push(input.number()? as usize);
    }
    for _ in 0..input.number()? {
        let variable = input.number()?;
        let name = input.import_name(ctx, &atoms)?;
        let request = input.number()? as usize;
        module
            .imports
            .push((variable, Dependency { request, name }));
    }
    Ok(module)
}

struct Input<'a>(&'a [u8]);

enum TableAtom<'a> {
    Predefined(u32),
    Latin1(&'a [u8]),
    Utf16(&'a [u8]),
}

impl<'a> Input<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], BytecodeError> {
        let (head, tail) = self.0.split_at_checked(count).ok_or_else(invalid)?;
        self.0 = tail;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, BytecodeError> {
        Ok(self.take(1)?[0])
    }

    fn number(&mut self) -> Result<u32, BytecodeError> {
        let mut number = 0;
        for shift in (0..32).step_by(7) {
            let byte = self.byte()?;
            if shift == 28 && byte > 15 {
                return Err(invalid());
            }
            number |= u32::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(number);
            }
        }
        Err(invalid())
    }

    fn table_atom(&mut self) -> Result<TableAtom<'a>, BytecodeError> {
        let tag = self.byte()?;
        if tag & 128 != 0 {
            let low = u32::from(tag & 63);
            let high = if tag & 64 == 0 { 0 } else { self.number()? };
            if high > qjs::JS_ATOM_END >> 6 {
                return Err(invalid());
            }
            return Ok(TableAtom::Predefined(low | high << 6));
        }
        let length = self.number()?;
        let count = (length >> 1) as usize;
        if length & 1 == 0 {
            return self.take(count).map(TableAtom::Latin1);
        }
        self.take(count.checked_mul(2).ok_or_else(invalid)?)
            .map(TableAtom::Utf16)
    }

    fn atom(&mut self, ctx: &Ctx<'_>, table: &[TableAtom<'_>]) -> Result<String, BytecodeError> {
        atom(ctx, table, self.number()?)
    }

    fn import_name(
        &mut self,
        ctx: &Ctx<'_>,
        table: &[TableAtom<'_>],
    ) -> Result<ImportName, BytecodeError> {
        let encoded = self.number()?;
        if encoded == qjs::JS_ATOM__star_ << 1 {
            Ok(ImportName::Namespace)
        } else {
            atom(ctx, table, encoded).map(ImportName::Named)
        }
    }

    fn dependency(
        &mut self,
        ctx: &Ctx<'_>,
        table: &[TableAtom<'_>],
    ) -> Result<Dependency, BytecodeError> {
        Ok(Dependency {
            request: self.number()? as usize,
            name: self.import_name(ctx, table)?,
        })
    }
}

fn atom(ctx: &Ctx<'_>, table: &[TableAtom<'_>], encoded: u32) -> Result<String, BytecodeError> {
    let index = encoded >> 1;
    if encoded & 1 != 0 {
        Ok(index.to_string())
    } else if index < qjs::JS_ATOM_END {
        predefined_atom(ctx, index)
    } else {
        match table
            .get((index - qjs::JS_ATOM_END) as usize)
            .ok_or_else(invalid)?
        {
            TableAtom::Predefined(index) => predefined_atom(ctx, *index),
            TableAtom::Latin1(bytes) => Ok(bytes.iter().map(|&byte| char::from(byte)).collect()),
            TableAtom::Utf16(bytes) => {
                let (pairs, tail) = bytes.as_chunks::<2>();
                if !tail.is_empty() {
                    return Err(invalid());
                }
                let units = pairs.iter().copied().map(u16::from_le_bytes);
                char::decode_utf16(units)
                    .collect::<Result<String, _>>()
                    .map_err(|_| invalid())
            }
        }
    }
}

fn predefined_atom(ctx: &Ctx<'_>, index: u32) -> Result<String, BytecodeError> {
    if !(1..qjs::JS_ATOM_END).contains(&index) {
        return Err(invalid());
    }
    let pointer = ctx.as_raw().as_ptr();
    // SAFETY: The pinned engine owns all of its predefined atoms, including
    // this numerically checked index. The result is an owned, NUL-terminated
    // string or a null pointer. We copy it before releasing it with QuickJS.
    let text = unsafe {
        let value = qjs::JS_AtomToCStringLen(pointer, std::ptr::null_mut(), index);
        if value.is_null() {
            return Err(invalid());
        }
        let text = std::ffi::CStr::from_ptr(value)
            .to_string_lossy()
            .into_owned();
        qjs::JS_FreeCString(pointer, value);
        text
    };
    Ok(text)
}

pub(super) fn validate(root: &str, modules: &Modules) -> Result<(), BytecodeError> {
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let module = modules.get(name).ok_or_else(invalid)?;
        for request in &module.requests {
            pending.push(target(modules, name, request)?);
        }
        let imported = module.imports.iter().map(|(_, entry)| entry);
        let reexported = module.exports.values().filter_map(|entry| {
            if let Export::Indirect(entry) = entry {
                Some(entry)
            } else {
                None
            }
        });
        for dependency in imported.chain(reexported) {
            let (request, import) = module.dependency(dependency)?;
            let ImportName::Named(import) = import else {
                continue;
            };
            let target = target(modules, name, request)?;
            let found = resolve_export(modules, target, import, &mut HashSet::new(), 0)?;
            if found.is_none() {
                return Err(BytecodeError::Script(format!(
                    "module {target:?} does not export {import:?} imported by {name:?}"
                )));
            }
        }
    }
    Ok(())
}

fn target<'a>(modules: &'a Modules, base: &str, name: &str) -> Result<&'a str, BytecodeError> {
    normalize(base, name)
        .and_then(|resolved| modules.get_key_value(&resolved))
        .map(|(name, _)| name.as_str())
        .ok_or_else(|| {
            BytecodeError::Script(format!(
                "module {name:?} imported by {base:?} is not registered"
            ))
        })
}

#[derive(Eq, PartialEq)]
enum Binding<'a> {
    Local(&'a str, u32),
    Native(&'a str, &'a str),
    Namespace(&'a str),
}

fn resolve_export<'a>(
    modules: &'a Modules,
    source: &'a str,
    name: &'a str,
    visited: &mut HashSet<(&'a str, &'a str)>,
    depth: usize,
) -> Result<Option<Binding<'a>>, BytecodeError> {
    if !visited.insert((source, name)) {
        return Ok(None);
    }
    if depth >= MAX_LINK_DEPTH {
        return Err(BytecodeError::Script(
            "module re-export depth exceeded".into(),
        ));
    }
    let module = modules.get(source).ok_or_else(invalid)?;
    let dependency = match module.exports.get(name) {
        Some(Export::Native) => return Ok(Some(Binding::Native(source, name))),
        Some(Export::Indirect(dep)) => Some(dep),
        Some(Export::Local(index)) => {
            match module
                .imports
                .iter()
                .find(|(variable, _)| variable == index)
            {
                Some((_, dep)) => Some(dep),
                None => return Ok(Some(Binding::Local(source, *index))),
            }
        }
        None => None,
    };
    if let Some(dep) = dependency {
        let (request, import) = module.dependency(dep)?;
        let source = target(modules, source, request)?;
        return match import {
            ImportName::Named(name) => resolve_export(modules, source, name, visited, depth + 1),
            ImportName::Namespace => Ok(Some(Binding::Namespace(source))),
        };
    }
    if name == "default" {
        return Ok(None);
    }
    let mut found = None;
    for &index in &module.stars {
        let request = module.requests.get(index).ok_or_else(invalid)?;
        let target = target(modules, source, request)?;
        if let Some(candidate) = resolve_export(modules, target, name, visited, depth + 1)? {
            if found.as_ref().is_some_and(|prior| *prior != candidate) {
                return Err(BytecodeError::Script(format!(
                    "module {source:?} has an ambiguous export {name:?}"
                )));
            }
            found = Some(candidate);
        }
    }
    Ok(found)
}
