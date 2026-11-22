//! UI entry to the shared IQJS envelope.
//!
//! [`compile`] keeps the historical signature and installs the UI module
//! declarations before passing source to the shared compiler.

use rquickjs::Runtime;

use crate::session::ModuleRegistry;
use javascript::compiler::{self, BytecodeError, CompileEnvironment, CompiledBytecode};
use javascript::runtime::ModuleCatalog;

use super::{Error, player};

struct UiCompile;

impl CompileEnvironment for UiCompile {
    fn prepare(&self, runtime: &Runtime) {
        player::prepare_compile(runtime);
    }
}

/// Compiles a script or module into the pinned engine's IQJS envelope.
pub fn compile(source: &str, name: &str, module: bool) -> Result<CompiledBytecode, Error> {
    compiler::bytecode::compile(source, name, module, &UiCompile).map_err(map_bytecode)
}

/// Compiles a UI entry against the extension declarations and the host's
/// actual module registry. The compiler does not instantiate native methods.
pub fn compile_with_registry(
    source: &str,
    name: &str,
    module: bool,
    registry: ModuleRegistry,
) -> Result<CompiledBytecode, Error> {
    let catalog = ModuleCatalog::new(registry).map_err(|error| Error::Script(error.to_string()))?;
    let extensions = player::compile_declarations();
    compiler::compile_with_extensions(source, name, module, &catalog, &extensions)
        .map_err(map_bytecode)
}

pub(crate) fn map_bytecode(error: BytecodeError) -> Error {
    match error {
        BytecodeError::Runtime => Error::Runtime,
        BytecodeError::Script(message) => Error::Script(message),
    }
}
