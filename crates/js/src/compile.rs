//! Compiles IQJS bytecode against explicit module declarations.
//!
//! Static imports are linked against the same catalog the runtime can use.
//! No source module body or native method runs during compilation.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Module, Runtime};

use crate::ModuleCatalog;
use crate::bytecode::{self, BytecodeError, CompiledBytecode};
use crate::modules::{normalize, unresolved};

mod link;

/// QuickJS loader installation for legacy hosts and trusted extensions.
pub trait CompileEnvironment {
    fn prepare(&self, runtime: &Runtime);

    #[doc(hidden)]
    fn validate_module(
        &self,
        _ctx: &Ctx<'_>,
        _name: &str,
        _payload: &[u8],
    ) -> Result<(), BytecodeError> {
        Ok(())
    }
}

/// A compile environment with no host or framework modules.
pub struct EmptyCompile;

impl CompileEnvironment for EmptyCompile {
    fn prepare(&self, _runtime: &Runtime) {}
}

/// Compiles a program with the catalog's source and host declarations.
/// Dynamic specifiers are resolved by the runtime when the import executes.
pub fn compile(
    source: &str,
    name: &str,
    module: bool,
    catalog: &ModuleCatalog,
) -> Result<CompiledBytecode, BytecodeError> {
    compile_with_extensions(source, name, module, catalog, &[])
}

/// Adds the declarations of host-specific native extensions. The supplied
/// descriptions contain names only, so compiling cannot invoke the extension.
pub fn compile_with_extensions(
    source: &str,
    name: &str,
    module: bool,
    catalog: &ModuleCatalog,
    extensions: &[(&str, &[&str])],
) -> Result<CompiledBytecode, BytecodeError> {
    let mut links = link::Modules::default();
    for native in catalog.registry().iter() {
        links.insert(
            native.specifier.to_owned(),
            link::ModuleLinks::native(native.export_names()),
        );
    }
    for (specifier, exports) in extensions {
        catalog
            .check_available(specifier)
            .map_err(|error| BytecodeError::Script(error.to_string()))?;
        let specifier = (*specifier).to_owned();
        let described = link::ModuleLinks::native(exports.iter().copied());
        if described.export_count() != exports.len() {
            return Err(BytecodeError::Script(format!(
                "module {specifier:?} has duplicate exports"
            )));
        }
        if links.insert(specifier.clone(), described).is_some() {
            return Err(BytecodeError::Script(format!(
                "module {specifier:?} is already registered"
            )));
        }
    }
    if module {
        if !js_host::valid_specifier(name) {
            return Err(BytecodeError::Script(format!(
                "invalid module specifier {name:?}"
            )));
        }
        if links.contains_key(name)
            || catalog
                .sources
                .get(name)
                .is_some_and(|registered| registered != source)
        {
            return Err(BytecodeError::Script(format!(
                "module {name:?} is already registered"
            )));
        }
    }
    let state = Rc::new(CompileState {
        root: module.then(|| name.to_owned()),
        sources: Arc::clone(&catalog.sources),
        links: RefCell::new(links),
    });
    bytecode::compile(source, name, module, &CatalogEnvironment(state))
}

struct CompileState {
    root: Option<String>,
    sources: Arc<BTreeMap<String, String>>,
    links: RefCell<link::Modules>,
}

struct CatalogEnvironment(Rc<CompileState>);

impl CompileEnvironment for CatalogEnvironment {
    fn prepare(&self, runtime: &Runtime) {
        runtime.set_loader(
            CatalogResolver(Rc::clone(&self.0)),
            CatalogLoader(Rc::clone(&self.0)),
        );
    }

    fn validate_module(
        &self,
        ctx: &Ctx<'_>,
        name: &str,
        payload: &[u8],
    ) -> Result<(), BytecodeError> {
        let root = link::read(ctx, payload)?;
        let mut links = self.0.links.borrow_mut();
        links.insert(name.to_owned(), root);
        link::validate(name, &links)
    }
}

struct CatalogResolver(Rc<CompileState>);

impl Resolver for CatalogResolver {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let resolved = normalize(base, name).ok_or_else(|| unresolved(ctx, name))?;
        if self.0.root.as_deref() == Some(&resolved)
            || self.0.sources.contains_key(&resolved)
            || self.0.links.borrow().contains_key(&resolved)
        {
            Ok(resolved)
        } else {
            Err(unresolved(ctx, &resolved))
        }
    }
}

struct CatalogLoader(Rc<CompileState>);

impl Loader for CatalogLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        if let Some(source) = self.0.sources.get(name) {
            let module = Module::declare(ctx.clone(), name, source.as_str())?;
            let payload = bytecode::write_module(&module)?;
            let links = link::read(ctx, &payload)
                .map_err(|error| rquickjs::Exception::throw_syntax(ctx, &error.to_string()))?;
            self.0.links.borrow_mut().insert(name.to_owned(), links);
            return Ok(module);
        }

        if self.0.links.borrow().contains_key(name) {
            // Static linkage uses the description below. Nothing in the
            // dummy module body can perform a host operation.
            return Module::declare(ctx.clone(), name, "export {};");
        }
        Err(unresolved(ctx, name))
    }
}
