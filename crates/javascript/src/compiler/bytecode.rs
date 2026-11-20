//! IQJS envelope for the pinned `quickjs-jit` 0.14.2 engine.
//!
//! [`compile`] writes the envelope. Loading those bytes in a session stays
//! unsafe and stays in the UI adapter: this module checks the magic, the pin,
//! the mode tag, and that the payload is non-empty. It does not turn an
//! arbitrary `&[u8]` into a safe bytecode API.

use std::ffi::{CStr, CString};
use std::fmt;

use rquickjs::qjs::{self, size_t};
use rquickjs::{
    Coerced, Context, Ctx, FromJs, Module, Runtime, String as JsString,
    module::{WriteOptions, WriteOptionsEndianness},
};

use crate::compiler::CompileEnvironment;

const BINDING_VERSION: &str = "quickjs-jit-0.14.2";
const NG_REVISION: &str = "0fdea21";
pub(crate) const BC_VERSION: u8 = 28;
const MAGIC: &[u8; 4] = b"IQJS";

const _: () = assert!(qjs::JS_READ_OBJ_ROM_DATA == 0);

#[derive(Debug)]
pub enum BytecodeError {
    Runtime,
    Script(String),
}

impl fmt::Display for BytecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime => formatter.write_str("quickjs runtime was not created"),
            Self::Script(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BytecodeError {}

/// Envelope bytes produced by [`compile`] in this process.
#[derive(Debug)]
pub struct CompiledBytecode {
    bytes: Vec<u8>,
    kind: Kind,
}

impl CompiledBytecode {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }
}

impl AsRef<[u8]> for CompiledBytecode {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Script or module bytecode. The tag is stored in the envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Script,
    Module,
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Self::Script => 0,
            Self::Module => 1,
        }
    }

    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Script),
            1 => Some(Self::Module),
            _ => None,
        }
    }
}

/// Parsed envelope whose version matches this process.
#[derive(Debug)]
pub struct Envelope<'a> {
    pub kind: Kind,
    pub payload: &'a [u8],
}

/// Compile `source` after `environment` has installed its module declarations.
///
/// The UI adapter passes the environment that declares scene and reactive.
/// [`crate::compiler::EmptyCompile`] declares nothing.
pub fn compile(
    source: &str,
    name: &str,
    module: bool,
    environment: &dyn CompileEnvironment,
) -> Result<CompiledBytecode, BytecodeError> {
    let kind = if module { Kind::Module } else { Kind::Script };
    let payload = with_ctx(environment, |ctx| match kind {
        Kind::Module => {
            let payload = compile_module(ctx, source, name)?;
            environment.validate_module(ctx, name, &payload)?;
            Ok(payload)
        }
        Kind::Script => compile_script(ctx, source, name),
    })?;
    let version = bytecode_version()?;
    Ok(CompiledBytecode {
        bytes: encode(&version, kind, &payload)?,
        kind,
    })
}

/// Rejects a missing magic, a different engine id, an unknown mode, or an empty payload.
pub fn open(bytes: &[u8]) -> Result<Envelope<'_>, BytecodeError> {
    let rest = bytes
        .strip_prefix(MAGIC)
        .ok_or_else(|| BytecodeError::Script("bytecode envelope is missing".to_owned()))?;
    if rest.len() < 2 {
        return Err(BytecodeError::Script(
            "bytecode envelope is truncated".to_owned(),
        ));
    }
    let version_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
    let rest = &rest[2..];
    if rest.len() < version_len.saturating_add(1) {
        return Err(BytecodeError::Script(
            "bytecode envelope is truncated".to_owned(),
        ));
    }
    let (version_bytes, rest) = rest.split_at(version_len);
    let version = std::str::from_utf8(version_bytes)
        .map_err(|_| BytecodeError::Script("bytecode version is not utf-8".to_owned()))?;
    let expected = bytecode_version()?;
    if version != expected {
        return Err(BytecodeError::Script(format!(
            "bytecode version {version} does not match {expected}"
        )));
    }
    let (mode, payload) = rest
        .split_first()
        .ok_or_else(|| BytecodeError::Script("bytecode envelope is truncated".to_owned()))?;
    let kind = Kind::from_tag(*mode)
        .ok_or_else(|| BytecodeError::Script(format!("bytecode mode {mode} is unknown")))?;
    if payload.is_empty() {
        return Err(BytecodeError::Script(
            "bytecode payload is empty".to_owned(),
        ));
    }
    Ok(Envelope { kind, payload })
}

/// Opens an envelope and requires `kind`.
pub fn open_kind(bytes: &[u8], kind: Kind) -> Result<&[u8], BytecodeError> {
    let envelope = open(bytes)?;
    if envelope.kind != kind {
        return Err(BytecodeError::Script(
            "bytecode mode does not match the loader".to_owned(),
        ));
    }
    Ok(envelope.payload)
}

/// Detects ESM syntax with the pinned engine's lexer.
pub fn is_module(source: &[u8]) -> bool {
    let mut terminated = source.to_vec();
    terminated.push(0);
    // SAFETY: QuickJS borrows these bytes and requires a trailing NUL.
    unsafe { qjs::JS_DetectModule(terminated.as_ptr().cast(), source.len() as size_t) }
}

pub(crate) fn bytecode_version() -> Result<String, BytecodeError> {
    let engine = engine_release()?;
    Ok(format!(
        "{BINDING_VERSION}/quickjs-ng-{engine}+{NG_REVISION}/bc{BC_VERSION}/le"
    ))
}

fn encode(version: &str, kind: Kind, payload: &[u8]) -> Result<Vec<u8>, BytecodeError> {
    let version_len = u16::try_from(version.len()).map_err(|_| BytecodeError::Runtime)?;
    let mut out = Vec::with_capacity(4 + 2 + version.len() + 1 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version_len.to_le_bytes());
    out.extend_from_slice(version.as_bytes());
    out.push(kind.tag());
    out.extend_from_slice(payload);
    Ok(out)
}

fn engine_release() -> Result<&'static str, BytecodeError> {
    // SAFETY: `JS_GetVersion` returns the engine's static version, or null.
    let ptr = unsafe { qjs::JS_GetVersion() };
    if ptr.is_null() {
        return Err(BytecodeError::Runtime);
    }
    // SAFETY: `ptr` is non-null and points at that static NUL-terminated string.
    let text = unsafe { CStr::from_ptr(ptr) };
    text.to_str().map_err(|_| BytecodeError::Runtime)
}

fn with_ctx<T>(
    environment: &dyn CompileEnvironment,
    body: impl FnOnce(&Ctx<'_>) -> Result<T, BytecodeError>,
) -> Result<T, BytecodeError> {
    let runtime = Runtime::new().map_err(|_| BytecodeError::Runtime)?;
    runtime.set_memory_limit(16 << 20);
    runtime.set_max_stack_size(8 << 20);
    environment.prepare(&runtime);
    let context = Context::full(&runtime).map_err(|_| BytecodeError::Runtime)?;
    context.with(|ctx| body(&ctx))
}

fn compile_module(ctx: &Ctx<'_>, source: &str, name: &str) -> Result<Vec<u8>, BytecodeError> {
    let module = Module::declare(ctx.clone(), name, source).map_err(|err| caught(ctx, err))?;
    write_module(&module).map_err(|err| caught(ctx, err))
}

pub(crate) fn write_module(module: &Module<'_>) -> rquickjs::Result<Vec<u8>> {
    module.write(WriteOptions {
        endianness: WriteOptionsEndianness::Little,
        strip_source: true,
        ..WriteOptions::default()
    })
}

fn compile_script(ctx: &Ctx<'_>, source: &str, name: &str) -> Result<Vec<u8>, BytecodeError> {
    let source = c_string("source", source)?;
    let name = c_string("name", name)?;
    let ctx_ptr = ctx.as_raw().as_ptr();
    let flags = c_int(qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY)?;
    let len = byte_len(source.as_bytes().len())?;
    // SAFETY: `ctx_ptr` is the live context. Both strings are NUL-terminated and
    // borrowed for the call. `COMPILE_ONLY` returns an owned script function.
    let value = unsafe { qjs::JS_Eval(ctx_ptr, source.as_ptr(), len, name.as_ptr(), flags) };
    if is_exception(value) {
        return Err(pending_exception(ctx));
    }
    let written = write_script(ctx, value);
    // SAFETY: `value` is the owned script function. `JS_WriteObject` does not free it.
    unsafe { qjs::JS_FreeValue(ctx_ptr, value) };
    written
}

fn write_script(ctx: &Ctx<'_>, value: qjs::JSValue) -> Result<Vec<u8>, BytecodeError> {
    let ctx_ptr = ctx.as_raw().as_ptr();
    let mut len: size_t = 0;
    let flags = write_flags()?;
    // SAFETY: `value` is a live object in this context. On success the buffer is
    // owned until `js_free` below.
    let buf = unsafe { qjs::JS_WriteObject(ctx_ptr, &mut len, value, flags) };
    if buf.is_null() {
        return Err(pending_exception(ctx));
    }
    let length = usize::try_from(len).map_err(|_| BytecodeError::Runtime)?;
    // SAFETY: `buf` points at `length` bytes just allocated by `JS_WriteObject`.
    let bytes = unsafe { std::slice::from_raw_parts(buf, length) }.to_vec();
    // SAFETY: the copy above owns the bytes. `buf` is the QuickJS allocation.
    unsafe { qjs::js_free(ctx_ptr, buf.cast()) };
    Ok(bytes)
}

fn write_flags() -> Result<i32, BytecodeError> {
    let mut bits = qjs::JS_WRITE_OBJ_BYTECODE | qjs::JS_WRITE_OBJ_STRIP_SOURCE;
    if cfg!(target_endian = "big") {
        bits |= qjs::JS_WRITE_OBJ_BSWAP;
    }
    c_int(bits)
}

fn c_int(bits: u32) -> Result<i32, BytecodeError> {
    i32::try_from(bits).map_err(|_| BytecodeError::Runtime)
}

fn byte_len(len: usize) -> Result<size_t, BytecodeError> {
    size_t::try_from(len).map_err(|_| BytecodeError::Script("bytecode is too large".to_owned()))
}

fn c_string(label: &str, text: &str) -> Result<CString, BytecodeError> {
    CString::new(text).map_err(|_| BytecodeError::Script(format!("{label} contains a nul byte")))
}

fn is_exception(value: qjs::JSValue) -> bool {
    // SAFETY: a `JSValue` is a by-value tag plus payload. Reading the tag does
    // not dereference the payload or change its reference count.
    unsafe { qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_EXCEPTION }
}

fn pending_exception(ctx: &Ctx<'_>) -> BytecodeError {
    let value = ctx.catch();
    match Coerced::<JsString<'_>>::from_js(ctx, value) {
        Ok(text) => match text.to_string() {
            Ok(message) => BytecodeError::Script(message),
            Err(err) => BytecodeError::Script(err.to_string()),
        },
        Err(err) => BytecodeError::Script(err.to_string()),
    }
}

fn caught(ctx: &Ctx<'_>, err: rquickjs::Error) -> BytecodeError {
    BytecodeError::Script(rquickjs::CaughtError::from_error(ctx, err).to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "bytecode envelope assertions")]
mod tests {
    use super::{
        BC_VERSION, Kind, NG_REVISION, bytecode_version, compile, encode, engine_release, open,
        open_kind,
    };
    use crate::compiler::EmptyCompile;
    use crate::compiler::bytecode::BytecodeError;

    fn must<T>(result: Result<T, BytecodeError>) -> T {
        result.expect("bytecode operation")
    }

    #[test]
    fn raw_bc28_is_rejected() {
        let error = open(&[BC_VERSION, 0, 1, 2]).expect_err("raw bytecode");
        assert!(error.to_string().contains("envelope is missing"), "{error}");
    }

    #[test]
    fn mismatched_version_is_rejected() {
        let bytes = must(encode("other", Kind::Module, &[BC_VERSION]));
        let error = open(&bytes).expect_err("foreign version");
        assert!(error.to_string().contains("does not match"), "{error}");
    }

    #[test]
    fn script_payload_is_not_a_module() {
        let bytes = must(compile(
            "globalThis.answer = 1",
            "app.js",
            false,
            &EmptyCompile,
        ));
        let error = open_kind(bytes.as_bytes(), Kind::Module).expect_err("mode");
        assert!(error.to_string().contains("mode"), "{error}");
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let version = must(bytecode_version());
        let mut bytes = must(encode(&version, Kind::Script, &[BC_VERSION]));
        let mode_at = 4 + 2 + version.len();
        bytes[mode_at] = 9;
        let error = open(&bytes).expect_err("mode");
        assert!(error.to_string().contains("unknown"), "{error}");
    }

    #[test]
    fn bytecode_version_matches_engine_and_lockfile() {
        let engine = must(engine_release());
        let version = must(bytecode_version());
        assert_eq!(
            version,
            format!("quickjs-jit-0.14.2/quickjs-ng-{engine}+{NG_REVISION}/bc{BC_VERSION}/le")
        );
        let bytes = must(compile("export const x = 1", "mod.js", true, &EmptyCompile));
        let envelope = must(open(bytes.as_bytes()));
        assert_eq!(envelope.payload.first().copied(), Some(BC_VERSION));
        let lock = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
        )
        .expect("Cargo.lock");
        for (package, pkg_version) in [("quickjs-jit", "0.14.2"), ("quickjs-jit-sys", "0.14.2")] {
            let entry = format!("name = \"{package}\"\nversion = \"{pkg_version}\"\n");
            assert!(
                lock.contains(&entry),
                "Cargo.lock has no {package} {pkg_version}"
            );
        }
    }
}
