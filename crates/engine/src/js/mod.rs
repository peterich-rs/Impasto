//! JavaScript UI sessions and their source and bytecode entry points.

use crate::text::Shaper;

use javascript::runtime::TrustedBytecode;

mod compiler;
mod error;
mod text_runs;

mod player;

pub use player::JsSession;

pub use compiler::{compile, compile_with_native_exports, compile_with_registry};
pub use error::Error;
pub use javascript::compiler::CompiledBytecode;

/// Console callback. Level 3 is an error. Other levels are informational.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Loads `script` and returns the session the host pumps each frame.
///
/// `name` is the filename the interpreter reports for this script. `log` receives
/// console lines. Level 3 is an error. Other levels are informational.
pub fn open(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
) -> Result<Box<dyn crate::session::Session>, Error> {
    open_with_config(
        script,
        name,
        shaper,
        log,
        crate::session::ModuleConfig::disconnected(),
    )
}

/// Opens a session with the supplied native module configuration.
pub fn open_with_config(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    Ok(Box::new(JsSession::open_with(
        script, name, shaper, log, config,
    )?))
}

/// Loads UTF-8 source. Bytecode uses [`open_compiled`] or [`open_trusted_bytecode`].
pub fn open_program(
    source: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    Ok(Box::new(JsSession::open_source(
        source, name, shaper, log, config,
    )?))
}

/// Loads an envelope that [`compile`] just produced in this process.
pub fn open_compiled(
    program: &CompiledBytecode,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    Ok(Box::new(JsSession::open_trusted(
        TrustedBytecode::from_compiled(program),
        name,
        shaper,
        log,
        config,
    )?))
}

/// Loads an IQJS envelope from a trusted file.
///
/// # Safety
/// `bytes` must be an unmodified envelope written by this toolchain's `compile`,
/// such as `impasto build`. The envelope checks the pin, the mode tag, and that
/// the payload is non-empty. It does not prove the payload is valid bytecode.
/// Untrusted bytes may cause undefined behavior in `Module::load` and `JS_ReadObject`.
pub unsafe fn open_trusted_bytecode(
    bytes: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    // SAFETY: the caller guarantees these bytes came from this toolchain's `compile`.
    let program = unsafe { TrustedBytecode::from_bytes(bytes) };
    Ok(Box::new(JsSession::open_trusted(
        program, name, shaper, log, config,
    )?))
}

#[cfg(test)]
mod component_tests;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "engine behavior assertions")]
mod tests;
