//! Bytecode envelope for the pinned `quickjs-jit` 0.14.2 engine.
//!
//! The CLI writes the envelope in one process and the host loads that file in
//! another (`impasto build`). QuickJS-NG 0.17.0 and this pin both use bytecode
//! format 28, so a raw blob is not rejected by the engine. The envelope names
//! this pin and the compile mode, and is checked before any load.
//!
//! [`compile`] produces trusted build artifacts. Disk loads go through
//! [`crate::open_trusted_bytecode_with_io`]. The loader relies on the provenance
//! of the payload in addition to checking the pin, mode tag, and non-empty payload.

use std::ffi::{CStr, CString};

use rquickjs::qjs::{self, size_t};
use rquickjs::{
    Coerced, Context, Ctx, FromJs, Module, Runtime, String as JsString,
    module::{WriteOptions, WriteOptionsEndianness},
};

use crate::Error;

const BINDING_VERSION: &str = "quickjs-jit-0.14.2";
const NG_REVISION: &str = "0fdea21";
const BC_VERSION: u8 = 28;
const MAGIC: &[u8; 4] = b"IQJS";

// `Module::load` passes this flag. Sessions may outlive their input buffers.
const _: () = assert!(qjs::JS_READ_OBJ_ROM_DATA == 0);

/// Envelope bytes produced by [`compile`] in this process.
#[derive(Debug)]
pub struct CompiledBytecode(Vec<u8>);

impl CompiledBytecode {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for CompiledBytecode {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Compiles a script or module into the pinned engine's IQJS envelope.
pub fn compile(source: &str, name: &str, module: bool) -> Result<CompiledBytecode, Error> {
    compile_envelope(source, name, module).map(CompiledBytecode)
}

pub(crate) struct TrustedBytecode<'a>(&'a [u8]);

impl<'a> TrustedBytecode<'a> {
    /// # Safety
    /// `bytes` must be an unmodified IQJS envelope from this toolchain's [`compile`].
    pub(crate) unsafe fn from_bytes(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub(crate) fn load(
        self,
        ctx: &Ctx<'_>,
    ) -> Result<Option<rquickjs::Persistent<rquickjs::Promise<'static>>>, Error> {
        // SAFETY: creating a TrustedBytecode requires an unmodified compiler envelope.
        unsafe { load_envelope(ctx, self.0) }
    }
}

/// Script or module bytecode. The tag is stored in the envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BytecodeKind {
    Script,
    Module,
}

impl BytecodeKind {
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

/// Engine id written into every envelope and checked before load.
pub(crate) fn bytecode_version() -> Result<String, Error> {
    let engine = engine_release()?;
    Ok(format!(
        "{BINDING_VERSION}/quickjs-ng-{engine}+{NG_REVISION}/bc{BC_VERSION}/le"
    ))
}

/// Compiles `source` and wraps the payload in the envelope.
///
/// `module` selects ESM bytecode. `false` compiles a global script. The two
/// payloads are not interchangeable: a script is loaded with `JS_ReadObject`
/// and `JS_EvalFunction`, and a module is loaded with [`Module::load`].
pub(crate) fn compile_envelope(source: &str, name: &str, module: bool) -> Result<Vec<u8>, Error> {
    let kind = if module {
        BytecodeKind::Module
    } else {
        BytecodeKind::Script
    };
    let payload = with_ctx(&UiCompile, |ctx| match kind {
        BytecodeKind::Module => compile_module(ctx, source, name),
        BytecodeKind::Script => compile_script(ctx, source, name),
    })?;
    let version = bytecode_version()?;
    encode(&version, kind, &payload)
}

/// Parsed envelope whose version matches this process.
#[derive(Debug)]
pub(crate) struct Envelope<'a> {
    pub(crate) kind: BytecodeKind,
    pub(crate) payload: &'a [u8],
}

/// Rejects a missing magic, a different engine id, an unknown mode, or an empty payload.
pub(crate) fn open_envelope(bytes: &[u8]) -> Result<Envelope<'_>, Error> {
    let rest = bytes
        .strip_prefix(MAGIC)
        .ok_or_else(|| Error::Script("bytecode envelope is missing".to_owned()))?;
    if rest.len() < 2 {
        return Err(Error::Script("bytecode envelope is truncated".to_owned()));
    }
    let version_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
    let rest = &rest[2..];
    if rest.len() < version_len.saturating_add(1) {
        return Err(Error::Script("bytecode envelope is truncated".to_owned()));
    }
    let (version_bytes, rest) = rest.split_at(version_len);
    let version = std::str::from_utf8(version_bytes)
        .map_err(|_| Error::Script("bytecode version is not utf-8".to_owned()))?;
    let expected = bytecode_version()?;
    if version != expected {
        return Err(Error::Script(format!(
            "bytecode version {version} does not match {expected}"
        )));
    }
    let (mode, payload) = rest
        .split_first()
        .ok_or_else(|| Error::Script("bytecode envelope is truncated".to_owned()))?;
    let kind = BytecodeKind::from_tag(*mode)
        .ok_or_else(|| Error::Script(format!("bytecode mode {mode} is unknown")))?;
    if payload.is_empty() {
        return Err(Error::Script("bytecode payload is empty".to_owned()));
    }
    Ok(Envelope { kind, payload })
}

/// Opens an envelope and requires `kind`.
#[cfg(test)]
pub(crate) fn open_envelope_kind(bytes: &[u8], kind: BytecodeKind) -> Result<&[u8], Error> {
    let envelope = open_envelope(bytes)?;
    if envelope.kind != kind {
        return Err(Error::Script(
            "bytecode mode does not match the loader".to_owned(),
        ));
    }
    Ok(envelope.payload)
}

fn encode(version: &str, kind: BytecodeKind, payload: &[u8]) -> Result<Vec<u8>, Error> {
    let version_len = u16::try_from(version.len()).map_err(|_| Error::Runtime)?;
    let mut out = Vec::with_capacity(4 + 2 + version.len() + 1 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version_len.to_le_bytes());
    out.extend_from_slice(version.as_bytes());
    out.push(kind.tag());
    out.extend_from_slice(payload);
    Ok(out)
}

fn engine_release() -> Result<&'static str, Error> {
    // SAFETY: `JS_GetVersion` returns the engine's static version, or null.
    let ptr = unsafe { qjs::JS_GetVersion() };
    if ptr.is_null() {
        return Err(Error::Runtime);
    }
    // SAFETY: `ptr` is non-null and points at that static NUL-terminated string.
    let text = unsafe { CStr::from_ptr(ptr) };
    text.to_str().map_err(|_| Error::Runtime)
}

struct UiCompile;

impl js::CompileEnvironment for UiCompile {
    fn prepare(&self, runtime: &Runtime) {
        crate::linked::prepare_compile(runtime);
    }
}

fn with_ctx<T>(
    prepare: &dyn js::CompileEnvironment,
    body: impl FnOnce(&Ctx<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    let runtime = Runtime::new().map_err(|_| Error::Runtime)?;
    runtime.set_memory_limit(16 << 20);
    runtime.set_max_stack_size(8 << 20);
    prepare.prepare(&runtime);
    let context = Context::full(&runtime).map_err(|_| Error::Runtime)?;
    context.with(|ctx| body(&ctx))
}

fn compile_module(ctx: &Ctx<'_>, source: &str, name: &str) -> Result<Vec<u8>, Error> {
    let module = Module::declare(ctx.clone(), name, source).map_err(|err| caught(ctx, err))?;
    module
        .write(WriteOptions {
            endianness: WriteOptionsEndianness::Little,
            strip_source: true,
            ..WriteOptions::default()
        })
        .map_err(|err| caught(ctx, err))
}

fn compile_script(ctx: &Ctx<'_>, source: &str, name: &str) -> Result<Vec<u8>, Error> {
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

fn write_script(ctx: &Ctx<'_>, value: qjs::JSValue) -> Result<Vec<u8>, Error> {
    let ctx_ptr = ctx.as_raw().as_ptr();
    let mut len: size_t = 0;
    let flags = write_flags()?;
    // SAFETY: `value` is a live object in this context. On success the buffer is
    // owned until `js_free` below.
    let buf = unsafe { qjs::JS_WriteObject(ctx_ptr, &mut len, value, flags) };
    if buf.is_null() {
        return Err(pending_exception(ctx));
    }
    let length = usize::try_from(len).map_err(|_| Error::Runtime)?;
    // SAFETY: `buf` points at `length` bytes just allocated by `JS_WriteObject`.
    let bytes = unsafe { std::slice::from_raw_parts(buf, length) }.to_vec();
    // SAFETY: the copy above owns the bytes. `buf` is the QuickJS allocation.
    unsafe { qjs::js_free(ctx_ptr, buf.cast()) };
    Ok(bytes)
}

/// Evaluates a script payload in `ctx`.
///
/// # Safety
/// `payload` must be script bytecode produced by this toolchain's `compile`.
/// The envelope check does not prove that. Other bytes are undefined behavior
/// in `JS_ReadObject`.
pub(crate) unsafe fn eval_script(ctx: &Ctx<'_>, payload: &[u8]) -> Result<(), Error> {
    let ctx_ptr = ctx.as_raw().as_ptr();
    let len = byte_len(payload.len())?;
    let flags = c_int(qjs::JS_READ_OBJ_BYTECODE)?;
    // SAFETY: `payload` is script bytecode accepted by the envelope for this pin.
    // The read flag omits `JS_READ_OBJ_ROM_DATA`, so QuickJS copies the buffer.
    // The returned function is owned until `JS_EvalFunction` consumes it.
    let function = unsafe { qjs::JS_ReadObject(ctx_ptr, payload.as_ptr(), len, flags) };
    if is_exception(function) {
        return Err(pending_exception(ctx));
    }
    // SAFETY: `function` is the owned object from `JS_ReadObject`. `JS_EvalFunction`
    // frees it. The completion value is owned by this call.
    let result = unsafe { qjs::JS_EvalFunction(ctx_ptr, function) };
    if is_exception(result) {
        return Err(pending_exception(ctx));
    }
    // SAFETY: `result` is the owned completion value and is not an exception tag.
    unsafe { qjs::JS_FreeValue(ctx_ptr, result) };
    Ok(())
}

/// Detects ESM syntax with the pinned engine's lexer.
pub(crate) fn is_module(source: &[u8]) -> bool {
    let mut terminated = source.to_vec();
    terminated.push(0);
    // SAFETY: QuickJS borrows these bytes and requires a trailing NUL.
    unsafe { qjs::JS_DetectModule(terminated.as_ptr().cast(), source.len() as size_t) }
}

/// Checks the envelope, then reads bytecode.
///
/// # Safety
/// `bytes` must be an unmodified envelope from this toolchain's `compile`.
/// [`open_envelope`] checks the magic, version, mode tag, and a non-empty
/// payload. It does not prove the payload is valid bytecode of that kind.
/// Other bytes are undefined behavior in `JS_ReadObject` or [`Module::load`].
unsafe fn load_envelope(
    ctx: &Ctx<'_>,
    bytes: &[u8],
) -> Result<Option<rquickjs::Persistent<rquickjs::Promise<'static>>>, Error> {
    let envelope = open_envelope(bytes)?;
    match envelope.kind {
        BytecodeKind::Script => {
            // SAFETY: the caller of this unsafe function promised compile output.
            unsafe { eval_script(ctx, envelope.payload) }?;
            Ok(None)
        }
        BytecodeKind::Module => {
            // SAFETY: the caller promised an unmodified module envelope from
            // this toolchain. The envelope does not validate the payload.
            let module = unsafe { Module::load(ctx.clone(), envelope.payload) }
                .map_err(|err| caught(ctx, err))?;
            let (_, promise) = module.eval().map_err(|err| caught(ctx, err))?;
            Ok(Some(rquickjs::Persistent::save(ctx, promise)))
        }
    }
}

#[cfg(test)]
/// Evaluates a module payload and reads the named numeric export.
///
/// # Safety
/// `payload` must be module bytecode from this toolchain's `compile`.
pub(crate) unsafe fn eval_module_i32(
    ctx: &Ctx<'_>,
    payload: &[u8],
    export: &str,
) -> Result<i32, Error> {
    // SAFETY: the caller promised compile output. Bundled bindings define
    // `JS_READ_OBJ_ROM_DATA` as 0, so this load copies `payload`.
    let module = unsafe { Module::load(ctx.clone(), payload) }.map_err(|err| caught(ctx, err))?;
    let (module, promise) = module.eval().map_err(|err| caught(ctx, err))?;
    promise.finish::<()>().map_err(|err| caught(ctx, err))?;
    module.get(export).map_err(|err| caught(ctx, err))
}

fn write_flags() -> Result<i32, Error> {
    let mut bits = qjs::JS_WRITE_OBJ_BYTECODE | qjs::JS_WRITE_OBJ_STRIP_SOURCE;
    if cfg!(target_endian = "big") {
        bits |= qjs::JS_WRITE_OBJ_BSWAP;
    }
    c_int(bits)
}

fn c_int(bits: u32) -> Result<i32, Error> {
    i32::try_from(bits).map_err(|_| Error::Runtime)
}

fn byte_len(len: usize) -> Result<size_t, Error> {
    size_t::try_from(len).map_err(|_| Error::Script("bytecode is too large".to_owned()))
}

fn c_string(label: &str, text: &str) -> Result<CString, Error> {
    CString::new(text).map_err(|_| Error::Script(format!("{label} contains a nul byte")))
}

fn is_exception(value: qjs::JSValue) -> bool {
    // SAFETY: a `JSValue` is a by-value tag plus payload. Reading the tag does
    // not dereference the payload or change its reference count.
    unsafe { qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_EXCEPTION }
}

fn pending_exception(ctx: &Ctx<'_>) -> Error {
    let value = ctx.catch();
    match Coerced::<JsString<'_>>::from_js(ctx, value) {
        Ok(text) => match text.to_string() {
            Ok(message) => Error::Script(message),
            Err(err) => Error::Script(err.to_string()),
        },
        Err(err) => Error::Script(err.to_string()),
    }
}

fn caught(ctx: &Ctx<'_>, err: rquickjs::Error) -> Error {
    Error::Script(rquickjs::CaughtError::from_error(ctx, err).to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "bytecode round-trip assertions")]
mod tests {
    use super::{
        BC_VERSION, BytecodeKind, NG_REVISION, bytecode_version, compile_envelope, encode,
        engine_release, eval_module_i32, eval_script, open_envelope, open_envelope_kind, with_ctx,
    };
    use crate::Error;

    fn must<T>(result: Result<T, Error>) -> T {
        result.expect("bytecode operation")
    }

    #[test]
    fn script_roundtrip_sets_global() {
        let bytes = must(compile_envelope(
            "globalThis.answer = 40 + 2",
            "app.js",
            false,
        ));
        let payload = must(open_envelope_kind(&bytes, BytecodeKind::Script));
        let answer = must(with_ctx(&super::UiCompile, |ctx| {
            // SAFETY: `payload` is the script envelope `compile_envelope` just wrote.
            unsafe { eval_script(ctx, payload) }?;
            ctx.eval::<i32, _>("globalThis.answer")
                .map_err(|err| super::caught(ctx, err))
        }));
        assert_eq!(answer, 42);
    }

    #[test]
    fn module_roundtrip_exports_const() {
        let bytes = must(compile_envelope("export const answer = 42", "mod.js", true));
        let envelope = must(open_envelope(&bytes));
        assert_eq!(envelope.kind, BytecodeKind::Module);
        assert_eq!(envelope.payload.first().copied(), Some(BC_VERSION));
        let answer = must(with_ctx(&super::UiCompile, |ctx| {
            // SAFETY: the payload is the module envelope `compile_envelope` just wrote.
            unsafe { eval_module_i32(ctx, envelope.payload, "answer") }
        }));
        assert_eq!(answer, 42);
    }

    #[test]
    fn raw_bc28_is_rejected() {
        let error = open_envelope(&[BC_VERSION, 0, 1, 2]).expect_err("raw bytecode");
        assert!(error.to_string().contains("envelope is missing"), "{error}");
    }

    #[test]
    fn mismatched_version_is_rejected() {
        let bytes = must(encode("other", BytecodeKind::Module, &[BC_VERSION]));
        let error = open_envelope(&bytes).expect_err("foreign version");
        assert!(error.to_string().contains("does not match"), "{error}");
    }

    #[test]
    fn script_payload_is_not_a_module() {
        let bytes = must(compile_envelope("globalThis.answer = 1", "app.js", false));
        let error = open_envelope_kind(&bytes, BytecodeKind::Module).expect_err("mode");
        assert!(error.to_string().contains("mode"), "{error}");
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let version = must(bytecode_version());
        let mut bytes = must(encode(&version, BytecodeKind::Script, &[BC_VERSION]));
        let mode_at = 4 + 2 + version.len();
        bytes[mode_at] = 9;
        let error = open_envelope(&bytes).expect_err("mode");
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
        let bytes = must(compile_envelope("export const x = 1", "mod.js", true));
        let envelope = must(open_envelope(&bytes));
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
