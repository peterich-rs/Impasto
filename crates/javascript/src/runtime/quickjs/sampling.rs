//! Cooperative stack samples from the owner thread's QuickJS interrupt checkpoints.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime, qjs};

const INTERVAL: Duration = Duration::from_millis(1);
const MAX_DEPTH: usize = 8;
const MAX_SAMPLES: usize = 32;
const MAX_STACK_BYTES: usize = 2048;

/// A sample of the generated JavaScript stack at a VM checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackSample {
    pub at: Instant,
    /// QuickJS frames, in leaf-to-root order, using generated file locations.
    pub stack: String,
}

pub(super) struct Sampler {
    // The capture context shares the VM stack but has its own Error intrinsics.
    // Application Error.prepareStackTrace and stackTraceLimit must never run here.
    context: Context,
    enabled: Box<dyn Fn() -> bool>,
    last: Cell<Option<Instant>>,
    pending: RefCell<Vec<StackSample>>,
}

impl Sampler {
    pub fn new(runtime: &Runtime, enabled: Box<dyn Fn() -> bool>) -> rquickjs::Result<Self> {
        Ok(Self {
            context: Context::full(runtime)?,
            enabled,
            last: Cell::new(None),
            pending: RefCell::default(),
        })
    }

    pub fn poll(&self) {
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
        let Ok(mut pending) = self.pending.try_borrow_mut() else {
            return;
        };
        if pending.len() < MAX_SAMPLES
            && let Some(stack) = self.capture()
        {
            pending.push(StackSample { at: now, stack });
        }
    }

    pub fn take(&self) -> Vec<StackSample> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    #[allow(
        unsafe_code,
        reason = "Stack inspection must use the owner thread's already locked QuickJS runtime"
    )]
    fn capture(&self) -> Option<String> {
        let ctx = self.context.as_raw().as_ptr();
        // SAFETY: the interrupt runs with this live runtime locked by its owner.
        // The private context is never evaluated by the application. Its original
        // Error stack getter reads captured data without running user callbacks.
        // Save and restore the runtime-wide pending exception, even on allocation
        // failure. Every separately owned JS value and C string is released here.
        unsafe {
            let previous = qjs::JS_GetException(ctx);
            let error = qjs::JS_NewError(ctx);
            let result = if qjs::JS_IsException(error) {
                None
            } else {
                let frames = qjs::JS_GetPropertyStr(ctx, error, c"stack".as_ptr());
                let stack = if qjs::JS_IsString(frames) {
                    let raw = qjs::JS_ToCString(ctx, frames);
                    if raw.is_null() {
                        None
                    } else {
                        let stack = CStr::from_ptr(raw).to_string_lossy();
                        let mut bounded = String::new();
                        for line in stack.lines().take(MAX_DEPTH) {
                            if bounded.len() + line.len() + 1 > MAX_STACK_BYTES {
                                break;
                            }
                            bounded.push_str(line);
                            bounded.push('\n');
                        }
                        qjs::JS_FreeCString(ctx, raw);
                        (!bounded.is_empty()).then_some(bounded)
                    }
                } else {
                    None
                };
                qjs::JS_FreeValue(ctx, frames);
                qjs::JS_FreeValue(ctx, error);
                stack
            };
            qjs::JS_Throw(ctx, previous);
            result
        }
    }
}
