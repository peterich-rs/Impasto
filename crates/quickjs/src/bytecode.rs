//! UI entry to the shared IQJS envelope.
//!
//! [`compile`] keeps the historical signature and installs the UI module
//! declarations before compilation. The envelope bytes and the compiler live
//! in `js`. Loading the payload remains unsafe: the envelope check does not
//! prove that the bytes are valid bytecode.

use rquickjs::Runtime;

use crate::Error;

pub use js::bytecode::CompiledBytecode;

struct UiCompile;

impl js::quickjs::CompileEnvironment for UiCompile {
    fn prepare(&self, runtime: &Runtime) {
        crate::linked::prepare_compile(runtime);
    }
}

/// Compiles a script or module into the pinned engine's IQJS envelope.
pub fn compile(source: &str, name: &str, module: bool) -> Result<CompiledBytecode, Error> {
    js::bytecode::compile(source, name, module, &UiCompile).map_err(map_bytecode)
}

/// Compiles a UI entry against the extension declarations and the host's
/// actual module registry. The compiler does not instantiate native methods.
pub fn compile_with_registry(
    source: &str,
    name: &str,
    module: bool,
    registry: session::ModuleRegistry,
) -> Result<CompiledBytecode, Error> {
    let catalog = js::ModuleCatalog::new(registry.as_host_registry().clone())
        .map_err(|error| Error::Script(error.to_string()))?;
    let extensions = crate::linked::compile_declarations();
    js::compile::compile_with_extensions(source, name, module, &catalog, &extensions)
        .map_err(map_bytecode)
}

pub(crate) fn map_bytecode(error: js::bytecode::BytecodeError) -> Error {
    match error {
        js::bytecode::BytecodeError::Runtime => Error::Runtime,
        js::bytecode::BytecodeError::Script(message) => Error::Script(message),
    }
}

pub(crate) use js::bytecode::is_module;
pub(crate) use js::quickjs::TrustedBytecode;
