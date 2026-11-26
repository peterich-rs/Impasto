//! Evaluation of IQJS bytecode from the pinned compiler.

use rquickjs::qjs::{self, size_t};
use rquickjs::{Ctx, Module, Persistent, Promise};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

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
                let module = TrustedModule(envelope.payload)
                    .declare(ctx)
                    .map_err(|error| caught(ctx, error))?;
                let (_, promise) = module.eval().map_err(|error| caught(ctx, error))?;
                Ok(Some(Persistent::save(ctx, promise)))
            }
            Kind::Archive => Err(BytecodeError::Script(
                "an archive needs the in-memory module loader".to_owned(),
            )),
        }
    }

    pub(crate) fn archive(self) -> Result<Option<TrustedArchive>, BytecodeError> {
        let envelope = bytecode::open(self.0)?;
        if envelope.kind == Kind::Archive {
            TrustedArchive::read(envelope.payload).map(Some)
        } else {
            Ok(None)
        }
    }
}

pub(crate) struct TrustedModule<'a>(&'a [u8]);

impl TrustedModule<'_> {
    pub(crate) fn declare<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Module<'js>> {
        // SAFETY: this wrapper is only created from the trusted module or archive envelope.
        unsafe { Module::load(ctx.clone(), self.0) }
    }
}

pub(crate) struct TrustedArchive {
    pub(crate) entry: String,
    modules: BTreeMap<String, Range<usize>>,
    bytes: Arc<[u8]>,
}

impl TrustedArchive {
    fn read(bytes: &[u8]) -> Result<Self, BytecodeError> {
        let invalid = || BytecodeError::Script("invalid bytecode archive directory".to_owned());
        let (entry, mut rest) = take_name(bytes).ok_or_else(invalid)?;
        let (count, tail) = rest.split_at_checked(4).ok_or_else(invalid)?;
        rest = tail;
        let count = u32::from_le_bytes(count.try_into().map_err(|_| invalid())?);
        if usize::try_from(count).map_err(|_| invalid())? > rest.len() / 18 {
            return Err(invalid());
        }
        let mut modules = BTreeMap::new();
        let mut previous_end = 0usize;
        for _ in 0..count {
            let (name, tail) = take_name(rest).ok_or_else(invalid)?;
            let (pair, tail) = tail.split_at_checked(16).ok_or_else(invalid)?;
            rest = tail;
            let read = |data: &[u8]| {
                usize::try_from(u64::from_le_bytes(data.try_into().map_err(|_| invalid())?))
                    .map_err(|_| invalid())
            };
            let offset = read(&pair[..8])?;
            let size = read(&pair[8..])?;
            let end = offset.checked_add(size).ok_or_else(invalid)?;
            if size == 0
                || Some(offset) != previous_end.checked_next_multiple_of(8)
                || modules
                    .last_key_value()
                    .is_some_and(|(previous, _): (&String, _)| previous.as_str() >= name)
            {
                return Err(invalid());
            }
            modules.insert(name.to_owned(), offset..end);
            previous_end = end;
        }
        let data = (bytes.len() - rest.len()).next_multiple_of(8);
        let payload = bytes.get(data..).ok_or_else(invalid)?;
        if payload.len() != previous_end || !modules.contains_key(entry) {
            return Err(invalid());
        }
        Ok(Self {
            entry: entry.to_owned(),
            modules,
            bytes: Arc::from(payload),
        })
    }

    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    pub(crate) fn module(&self, name: &str) -> Option<TrustedModule<'_>> {
        self.bytes
            .get(self.modules.get(name)?.clone())
            .map(TrustedModule)
    }
}

fn take_name(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let (length, bytes) = bytes.split_at_checked(2)?;
    let length = usize::from(u16::from_le_bytes(length.try_into().ok()?));
    let (name, bytes) = bytes.split_at_checked(length)?;
    let name = std::str::from_utf8(name).ok()?;
    crate::host::valid_specifier(name).then_some((name, bytes))
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
