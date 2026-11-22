//! JavaScript UI sessions and their source and bytecode entry points.

use crate::text::Shaper;

use javascript::runtime::TrustedBytecode;

mod compiler;
mod error;
mod text_runs;

mod player;

pub use player::JsSession;

pub use compiler::{compile, compile_with_registry};
pub use error::Error;
pub use javascript::compiler::CompiledBytecode;

/// Console callback. Level 3 is an error. Other levels are informational.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Nonblocking admission into the host's generation-aware transport.
pub type RoutedIoSubmit =
    Box<dyn Fn(crate::session::IoJob) -> Result<(), Box<crate::session::IoJob>> + Send>;

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
    open_with_io(
        script,
        name,
        shaper,
        log,
        None,
        crate::session::ModuleConfig::disconnected(),
    )
}

/// Opens a session and installs the host sink for [`crate::session::IoJob`].
pub fn open_with_io(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(crate::session::IoJob) + Send>>,
    host: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    {
        let mut session = JsSession::open_with(script, name, shaper, log, None, host)?;
        if let Some(io) = io {
            session.install_io(io);
        }
        Ok(Box::new(session))
    }
}

/// Loads UTF-8 source. Bytecode uses [`open_compiled_with_io`] or
/// [`open_trusted_bytecode_with_io`].
pub fn open_program_with_io(
    source: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(crate::session::IoJob) + Send>>,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    finish_session(
        JsSession::open_source(source, name, shaper, log, None, config)?,
        io,
    )
}

/// Loads an envelope that [`compile`] just produced in this process.
pub fn open_compiled_with_io(
    program: &CompiledBytecode,
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(crate::session::IoJob) + Send>>,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    finish_session(
        JsSession::open_trusted(
            TrustedBytecode::from_compiled(program),
            name,
            shaper,
            log,
            None,
            config,
        )?,
        io,
    )
}

/// Loads an IQJS envelope from a trusted file.
///
/// # Safety
/// `bytes` must be an unmodified envelope written by this toolchain's `compile`,
/// such as `impasto build`. The envelope checks the pin, the mode tag, and that
/// the payload is non-empty. It does not prove the payload is valid bytecode.
/// Untrusted bytes may cause undefined behavior in `Module::load` and `JS_ReadObject`.
pub unsafe fn open_trusted_bytecode_with_io(
    bytes: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<Box<dyn Fn(crate::session::IoJob) + Send>>,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    // SAFETY: the caller guarantees these bytes came from this toolchain's `compile`.
    let program = unsafe { TrustedBytecode::from_bytes(bytes) };
    finish_session(
        JsSession::open_trusted(program, name, shaper, log, None, config)?,
        io,
    )
}

/// Opens UTF-8 source with generation-aware external I/O after the entry.
pub fn open_program_with_routed_io(
    source: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<RoutedIoSubmit>,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    let session = JsSession::open_source(source, name, shaper, log, None, config)?;
    Ok(finish_routed_session(session, io))
}

/// Opens a trusted IQJS envelope with generation-aware external I/O after the entry.
///
/// # Safety
/// `bytes` must be an unmodified IQJS envelope written by this toolchain's compiler.
pub unsafe fn open_trusted_bytecode_with_routed_io(
    bytes: &[u8],
    name: &str,
    shaper: impl Shaper + 'static,
    log: ConsoleLog,
    io: Option<RoutedIoSubmit>,
    config: crate::session::ModuleConfig,
) -> Result<Box<dyn crate::session::Session>, Error> {
    // SAFETY: the caller attests that the compiler produced this envelope.
    let program = unsafe { TrustedBytecode::from_bytes(bytes) };
    let session = JsSession::open_trusted(program, name, shaper, log, None, config)?;
    Ok(finish_routed_session(session, io))
}

fn finish_routed_session(
    mut session: JsSession,
    io: Option<RoutedIoSubmit>,
) -> Box<dyn crate::session::Session> {
    if let Some(io) = io {
        session.install_routed_io(io);
    }
    Box::new(session)
}

fn finish_session(
    mut session: JsSession,
    io: Option<Box<dyn Fn(crate::session::IoJob) + Send>>,
) -> Result<Box<dyn crate::session::Session>, Error> {
    if let Some(io) = io {
        session.install_io(io);
    }
    Ok(Box::new(session))
}

#[cfg(test)]
mod component_tests;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "engine behavior assertions")]
mod tests;
