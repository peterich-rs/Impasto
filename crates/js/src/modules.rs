//! In-memory modules and native module factories.
//!
//! Names are explicit. This loader does not read the working directory and does
//! not treat `scene`, `reactive`, or `@impasto/` as built-in modules. A host
//! that wants those modules registers them.

use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Module};

use crate::{CatalogError, ModuleCatalog};

/// Declares one native module on the JavaScript thread.
pub trait NativeFactory {
    fn load<'js>(
        &self,
        ctx: &Ctx<'js>,
        name: &str,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>>;
}

pub(crate) struct ModuleTable {
    catalog: ModuleCatalog,
    natives: HashMap<String, Box<dyn NativeFactory>>,
}

impl ModuleTable {
    pub(crate) fn with_catalog(
        mut catalog: ModuleCatalog,
        sources: HashMap<String, String>,
        natives: HashMap<String, Box<dyn NativeFactory>>,
    ) -> Result<Self, CatalogError> {
        for (name, source) in sources {
            catalog.add_source(name, source)?;
        }
        for name in natives.keys() {
            catalog.check_available(name)?;
        }
        Ok(Self { catalog, natives })
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.catalog.contains(name) || self.natives.contains_key(name)
    }

    pub(crate) fn source(&self, name: &str) -> Option<&str> {
        self.catalog.sources.get(name).map(String::as_str)
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
        declare_native(&self.0, ctx, name)
    }
}

pub(crate) fn declare_native<'js>(
    table: &ModuleTable,
    ctx: &Ctx<'js>,
    name: &str,
) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
    if let Some(native) = table.natives.get(name) {
        return native.load(ctx, name);
    }
    if table.catalog.registry().by_specifier(name).is_some() {
        return crate::quickjs::binder::declare_host_module(ctx, name);
    }
    Err(unresolved(ctx, name))
}

pub(crate) fn unresolved(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    rquickjs::Exception::throw_reference(ctx, &format!("module \"{name}\" is not registered"))
}

/// Join `name` onto `base` when it is relative. Only `/` separators are used.
/// A name that is not registered never becomes a filesystem path.
pub(crate) fn normalize(base: &str, name: &str) -> Option<String> {
    if name.contains(['\\', '\0']) || name.contains("//") || name.chars().any(char::is_control) {
        return None;
    }
    if !name.starts_with("./") && !name.starts_with("../") {
        return js_host::valid_specifier(name).then(|| name.to_owned());
    }
    let combined = match base.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{dir}/{name}"),
        _ => name.to_owned(),
    };
    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            segment => parts.push(segment),
        }
    }
    let resolved = parts.join("/");
    js_host::valid_specifier(&resolved).then_some(resolved)
}
