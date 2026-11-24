//! Cooperative module-stack samples from QuickJS interrupt checkpoints.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::ptr::{self, NonNull};
use std::time::{Duration, Instant};

use rquickjs::qjs::{self, JSAtom, JSContext};

const INTERVAL: Duration = Duration::from_millis(1);
const MAX_DEPTH: usize = 8;
const MAX_SAMPLES: usize = 32;
const MAX_MODULE_BYTES: usize = 72;

/// An owner-thread stack sample. The public QuickJS API provides module names
/// for bytecode frames. Native frames and stripped bytecode can be unnamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackSample {
    pub at: Instant,
    pub modules: Vec<Option<String>>,
}

struct RawSample {
    at: Instant,
    modules: [JSAtom; MAX_DEPTH],
}

pub(super) struct Sampler {
    enabled: Box<dyn Fn() -> bool>,
    last: Cell<Option<Instant>>,
    pending: RefCell<Vec<RawSample>>,
}

impl Sampler {
    pub fn new(enabled: Box<dyn Fn() -> bool>) -> Self {
        Self {
            enabled,
            last: Cell::new(None),
            pending: RefCell::default(),
        }
    }

    pub fn poll(&self, ctx: NonNull<JSContext>) {
        if !(self.enabled)() {
            return;
        }
        let now = Instant::now();
        if self
            .last
            .get()
            .is_some_and(|last| now.duration_since(last) < INTERVAL)
        {
            return;
        }
        self.last.set(Some(now));
        let mut pending = self.pending.borrow_mut();
        if pending.len() >= MAX_SAMPLES {
            return;
        }
        let modules = std::array::from_fn(|depth| module_at(ctx, depth));
        if modules.iter().any(|atom| *atom != qjs::JS_ATOM_NULL) {
            pending.push(RawSample { at: now, modules });
        }
    }

    pub fn take(&self, ctx: NonNull<JSContext>) -> Vec<StackSample> {
        self.pending
            .borrow_mut()
            .drain(..)
            .map(|sample| {
                let modules = sample.modules.map(|atom| {
                    let module = module_name(ctx, atom);
                    release(ctx, atom);
                    module
                });
                let len = modules
                    .iter()
                    .rposition(Option::is_some)
                    .map_or(0, |last| last + 1);
                StackSample {
                    at: sample.at,
                    modules: modules.into_iter().take(len).collect(),
                }
            })
            .collect()
    }

    pub fn clear(&self, ctx: NonNull<JSContext>) {
        for sample in self.pending.borrow_mut().drain(..) {
            for atom in sample.modules {
                release(ctx, atom);
            }
        }
    }
}

#[allow(
    unsafe_code,
    reason = "QuickJS module stack inspection is exposed through its C API"
)]
fn module_at(ctx: NonNull<JSContext>, depth: usize) -> JSAtom {
    // SAFETY: the armed owner-thread interrupt callback runs with this VM alive
    // under its runtime lock. QuickJS returns a retained atom for bytecode frames.
    unsafe { qjs::JS_GetScriptOrModuleName(ctx.as_ptr(), i32::try_from(depth).unwrap_or(0)) }
}

#[allow(
    unsafe_code,
    reason = "Release the atom retained by QuickJS module stack inspection"
)]
fn release(ctx: NonNull<JSContext>, atom: JSAtom) {
    if atom != qjs::JS_ATOM_NULL {
        // SAFETY: the owner-thread context is live and owns this retained atom.
        unsafe { qjs::JS_FreeAtom(ctx.as_ptr(), atom) };
    }
}

#[allow(
    unsafe_code,
    reason = "Copy a retained QuickJS module name on the owner thread"
)]
fn module_name(ctx: NonNull<JSContext>, atom: JSAtom) -> Option<String> {
    if atom == qjs::JS_ATOM_NULL {
        return None;
    }
    // SAFETY: this atom is retained by the sample, and the owner holds the
    // runtime lock. Resolve outside the interrupt handler, after the JS turn.
    let name = unsafe { qjs::JS_AtomToCStringLen(ctx.as_ptr(), ptr::null_mut(), atom) };
    if name.is_null() {
        return None;
    }
    // SAFETY: a successful conversion is NUL terminated and live until freed.
    let name_str = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let end = name_str.floor_char_boundary(name_str.len().min(MAX_MODULE_BYTES));
    let limited = name_str[..end].to_owned();
    // SAFETY: the matching QuickJS context frees this buffer exactly once.
    unsafe { qjs::JS_FreeCString(ctx.as_ptr(), name) };
    Some(limited)
}
