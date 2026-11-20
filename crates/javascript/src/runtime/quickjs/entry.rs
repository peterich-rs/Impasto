//! Evaluation of IQJS bytecode from the pinned compiler.

use rquickjs::qjs::{self, size_t};
use rquickjs::{Ctx, Module, Persistent, Promise};

use crate::compiler::bytecode::{self, BytecodeError, CompiledBytecode, Kind};

use super::exception_text;

/// An IQJS envelope whose payload came from this toolchain's compiler.
#[derive(Clone, Copy)]
pub struct TrustedBytecode<'a>(&'a [u8]);

impl<'a> TrustedBytecode<'a> {
    pub fn from_compiled(program: &'a CompiledBytecode) -> Self {
        Self(program.as_bytes())
    }

    /// # Safety
    /// These bytes must be an unmodified envelope written by this toolchain.
    /// Its header identifies the format and version. Only the caller can attest
    /// that the bytecode payload is valid.
    pub unsafe fn from_bytes(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    /// Execute a script or start a module and return its evaluation promise.
    pub fn load(
        self,
        ctx: &Ctx<'_>,
    ) -> Result<Option<Persistent<Promise<'static>>>, BytecodeError> {
        let envelope = bytecode::open(self.0)?;
        match envelope.kind {
            Kind::Script => {
                // SAFETY: TrustedBytecode attests that the payload was produced by the compiler.
                unsafe { eval_script(ctx, envelope.payload) }?;
                Ok(None)
            }
            Kind::Module => {
                // SAFETY: the validated mode matches the trusted module payload.
                let module = unsafe { Module::load(ctx.clone(), envelope.payload) }
                    .map_err(|error| caught(ctx, error))?;
                let (_, promise) = module.eval().map_err(|error| caught(ctx, error))?;
                Ok(Some(Persistent::save(ctx, promise)))
            }
        }
    }
}

/// # Safety
/// The payload must be compiler-produced script bytecode for this engine version.
unsafe fn eval_script(ctx: &Ctx<'_>, payload: &[u8]) -> Result<(), BytecodeError> {
    let ctx_ptr = ctx.as_raw().as_ptr();
    let len = size_t::try_from(payload.len())
        .map_err(|_| BytecodeError::Script("bytecode is too large".into()))?;
    let flags = i32::try_from(qjs::JS_READ_OBJ_BYTECODE).map_err(|_| BytecodeError::Runtime)?;
    // SAFETY: the caller verified the payload; QuickJS copies it and returns an owned value.
    let function = unsafe { qjs::JS_ReadObject(ctx_ptr, payload.as_ptr(), len, flags) };
    if is_exception(function) {
        return Err(caught(ctx, rquickjs::Error::Exception));
    }
    // SAFETY: JS_EvalFunction consumes the owned function from JS_ReadObject.
    let result = unsafe { qjs::JS_EvalFunction(ctx_ptr, function) };
    if is_exception(result) {
        return Err(caught(ctx, rquickjs::Error::Exception));
    }
    // SAFETY: the completion value is owned and live in this context.
    unsafe { qjs::JS_FreeValue(ctx_ptr, result) };
    Ok(())
}

fn is_exception(value: qjs::JSValue) -> bool {
    // SAFETY: reading the value tag does not access the payload.
    unsafe { qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_EXCEPTION }
}

fn caught(ctx: &Ctx<'_>, error: rquickjs::Error) -> BytecodeError {
    if error.is_exception() {
        let (message, stack) = exception_text(ctx, ctx.catch());
        if stack.is_empty() {
            BytecodeError::Script(message)
        } else {
            BytecodeError::Script(format!("{message}\n{stack}"))
        }
    } else {
        BytecodeError::Script(error.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "bytecode round-trip assertions")]
mod tests {
    use super::*;

    #[test]
    fn trusted_bytecode_runs_scripts_and_modules() {
        let runtime = rquickjs::Runtime::new().expect("runtime");
        let context = rquickjs::Context::full(&runtime).expect("context");
        for (source, module) in [
            ("globalThis.answer = 40 + 2;", false),
            (
                "export {}; globalThis.answer = await Promise.resolve(42);",
                true,
            ),
        ] {
            let program =
                bytecode::compile(source, "app.js", module, &crate::compiler::EmptyCompile)
                    .expect("compile");
            context.with(|ctx| {
                let promise = TrustedBytecode::from_compiled(&program)
                    .load(&ctx)
                    .expect("load bytecode");
                assert_eq!(promise.is_some(), module);
                if let Some(promise) = promise {
                    promise
                        .restore(&ctx)
                        .expect("restore promise")
                        .finish::<()>()
                        .expect("complete module");
                }
                assert_eq!(ctx.globals().get::<_, i32>("answer").expect("answer"), 42);
            });
        }
    }
}
