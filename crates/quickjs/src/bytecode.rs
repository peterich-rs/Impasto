//! UI entry to the shared IQJS envelope.
//!
//! [`compile`] keeps the historical signature and installs the UI module
//! declarations before compilation. The envelope bytes and the compiler live
//! in `js`. Loading the payload remains unsafe: the envelope check does not
//! prove that the bytes are valid bytecode.

use rquickjs::qjs::{self, size_t};
use rquickjs::{Coerced, Ctx, FromJs, Module, Runtime, String as JsString};

use crate::Error;

pub use js::bytecode::CompiledBytecode;

struct UiCompile;

impl js::CompileEnvironment for UiCompile {
    fn prepare(&self, runtime: &Runtime) {
        crate::linked::prepare_compile(runtime);
    }
}

/// Compiles a script or module into the pinned engine's IQJS envelope.
pub fn compile(source: &str, name: &str, module: bool) -> Result<CompiledBytecode, Error> {
    js::bytecode::compile(source, name, module, &UiCompile).map_err(map_bytecode)
}

fn map_bytecode(error: js::bytecode::BytecodeError) -> Error {
    match error {
        js::bytecode::BytecodeError::Runtime => Error::Runtime,
        js::bytecode::BytecodeError::Script(message) => Error::Script(message),
    }
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

/// Detects ESM syntax with the pinned engine's lexer.
pub(crate) fn is_module(source: &[u8]) -> bool {
    js::bytecode::is_module(source)
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

/// Checks the envelope, then reads bytecode.
///
/// # Safety
/// `bytes` must be an unmodified envelope from this toolchain's `compile`.
/// The envelope check does not prove the payload is valid bytecode.
/// Other bytes are undefined behavior in `JS_ReadObject` or [`Module::load`].
unsafe fn load_envelope(
    ctx: &Ctx<'_>,
    bytes: &[u8],
) -> Result<Option<rquickjs::Persistent<rquickjs::Promise<'static>>>, Error> {
    let envelope = js::bytecode::open(bytes).map_err(map_bytecode)?;
    match envelope.kind {
        js::bytecode::Kind::Script => {
            // SAFETY: the caller of this unsafe function promised compile output.
            unsafe { eval_script(ctx, envelope.payload) }?;
            Ok(None)
        }
        js::bytecode::Kind::Module => {
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

fn c_int(bits: u32) -> Result<i32, Error> {
    i32::try_from(bits).map_err(|_| Error::Runtime)
}

fn byte_len(len: usize) -> Result<size_t, Error> {
    size_t::try_from(len).map_err(|_| Error::Script("bytecode is too large".to_owned()))
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
fn with_ctx<T>(body: impl FnOnce(&Ctx<'_>) -> Result<T, Error>) -> Result<T, Error> {
    use js::CompileEnvironment;
    use rquickjs::Context;
    let runtime = Runtime::new().map_err(|_| Error::Runtime)?;
    runtime.set_memory_limit(16 << 20);
    runtime.set_max_stack_size(8 << 20);
    UiCompile.prepare(&runtime);
    let context = Context::full(&runtime).map_err(|_| Error::Runtime)?;
    context.with(|ctx| body(&ctx))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "bytecode round-trip assertions")]
mod tests {
    use super::{eval_module_i32, eval_script, with_ctx};
    use crate::Error;

    fn must<T>(result: Result<T, Error>) -> T {
        result.expect("bytecode operation")
    }

    #[test]
    fn script_roundtrip_sets_global() {
        let bytes = must(crate::compile(
            "globalThis.answer = 40 + 2",
            "app.js",
            false,
        ));
        let payload = js::bytecode::open_kind(bytes.as_bytes(), js::bytecode::Kind::Script)
            .expect("script envelope");
        let answer = must(with_ctx(|ctx| {
            // SAFETY: `payload` is the script envelope `compile` just wrote.
            unsafe { eval_script(ctx, payload) }?;
            ctx.eval::<i32, _>("globalThis.answer")
                .map_err(|err| super::caught(ctx, err))
        }));
        assert_eq!(answer, 42);
    }

    #[test]
    fn module_roundtrip_exports_const() {
        let bytes = must(crate::compile("export const answer = 42", "mod.js", true));
        let envelope = js::bytecode::open(bytes.as_bytes()).expect("module envelope");
        assert_eq!(envelope.kind, js::bytecode::Kind::Module);
        let answer = must(with_ctx(|ctx| {
            // SAFETY: the payload is the module envelope `compile` just wrote.
            unsafe { eval_module_i32(ctx, envelope.payload, "answer") }
        }));
        assert_eq!(answer, 42);
    }
}
