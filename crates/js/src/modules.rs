//! In-memory modules and native module factories.
//!
//! Names are explicit. This loader does not read the working directory and does
//! not treat `scene`, `reactive`, or `@impasto/` as built-in modules. A host
//! that wants those modules registers them.

use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Module};

/// Declares one native module on the JavaScript thread.
pub trait NativeFactory {
    fn load<'js>(
        &self,
        ctx: &Ctx<'js>,
        name: &str,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>>;
}

pub(crate) struct ModuleTable {
    sources: HashMap<String, String>,
    natives: HashMap<String, Box<dyn NativeFactory>>,
}

impl ModuleTable {
    pub(crate) fn new(
        sources: HashMap<String, String>,
        natives: HashMap<String, Box<dyn NativeFactory>>,
    ) -> Self {
        Self { sources, natives }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.sources.contains_key(name) || self.natives.contains_key(name)
    }

    pub(crate) fn source(&self, name: &str) -> Option<&str> {
        self.sources.get(name).map(String::as_str)
    }

    pub(crate) fn is_native(&self, name: &str) -> bool {
        self.natives.contains_key(name)
    }
}

pub(crate) struct MemoryResolver(pub(crate) Rc<ModuleTable>);

impl Resolver for MemoryResolver {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let resolved = normalize(base, name).ok_or_else(|| unresolved(ctx, name))?;
        if self.0.contains(&resolved) {
            Ok(resolved)
        } else {
            Err(unresolved(ctx, &resolved))
        }
    }
}

pub(crate) struct MemoryLoader(pub(crate) Rc<ModuleTable>);

impl Loader for MemoryLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        if let Some(source) = self.0.source(name) {
            return Module::declare(ctx.clone(), name, source);
        }
        if let Some(native) = self.0.natives.get(name) {
            return native.load(ctx, name);
        }
        Err(unresolved(ctx, name))
    }
}

pub(crate) fn declare_native<'js>(
    table: &ModuleTable,
    ctx: &Ctx<'js>,
    name: &str,
) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
    let native = table
        .natives
        .get(name)
        .ok_or_else(|| unresolved(ctx, name))?;
    native.load(ctx, name)
}

fn unresolved(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    rquickjs::Exception::throw_reference(ctx, &format!("module \"{name}\" is not registered"))
}

/// Join `name` onto `base` when it is relative. Only `/` separators are used.
/// A name that is not registered never becomes a filesystem path.
fn normalize(base: &str, name: &str) -> Option<String> {
    if name.is_empty() || name.contains('\\') || name.contains("://") || name.starts_with('/') {
        return None;
    }
    let combined = if name.starts_with('.') {
        match base.rsplit_once('/') {
            Some((dir, _)) if !dir.is_empty() => format!("{dir}/{name}"),
            _ => name.to_owned(),
        }
    } else {
        name.to_owned()
    };
    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            segment => parts.push(segment),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}
