//! Cooperative stack samples from the owner thread's QuickJS interrupt checkpoints.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::runtime::{SamplerConfig, StackSample};

use rquickjs::{Context, Runtime, qjs};

const MAX_DEPTH: usize = 64;
const MAX_SAMPLES: usize = 512;
const MAX_STACK_BYTES: usize = 8192;

pub(super) struct Sampler {
    // The capture context shares the VM stack but has its own Error intrinsics.
    // Application Error.prepareStackTrace and stackTraceLimit must never run here.
    context: Context,
    enabled: Box<dyn Fn() -> bool>,
    forced: AtomicBool,
    interval_us: AtomicU64,
    max_depth: AtomicUsize,
    max_samples: AtomicUsize,
    last: Cell<Option<Instant>>,
    pending: RefCell<Vec<StackSample>>,
}

impl Sampler {
    pub fn new(runtime: &Runtime, enabled: Box<dyn Fn() -> bool>) -> rquickjs::Result<Self> {
        let config = SamplerConfig::default();
        Ok(Self {
            context: Context::full(runtime)?,
            enabled,
            forced: AtomicBool::new(config.enabled),
            interval_us: AtomicU64::new(config.interval_us),
            max_depth: AtomicUsize::new(config.max_depth),
            max_samples: AtomicUsize::new(config.max_samples),
            last: Cell::new(None),
            pending: RefCell::default(),
        })
    }

    pub(super) fn set_config(&self, config: SamplerConfig) {
        self.interval_us
            .store(config.interval_us.clamp(100, 1_000_000), Ordering::Relaxed);
        self.max_depth
            .store(config.max_depth.clamp(1, MAX_DEPTH), Ordering::Relaxed);
        self.max_samples
            .store(config.max_samples.clamp(1, MAX_SAMPLES), Ordering::Relaxed);
        self.forced.store(config.enabled, Ordering::Relaxed);
        self.reset_interval();
    }

    pub fn poll(&self, entry: &'static str) {
        if !self.forced.load(Ordering::Relaxed) && !(self.enabled)() {
            return;
        }
        let now = Instant::now();
        if self.last.get().is_some_and(|last| {
            now.duration_since(last)
                < Duration::from_micros(self.interval_us.load(Ordering::Relaxed))
        }) {
            return;
        }
        self.last.set(Some(now));
        let Ok(mut pending) = self.pending.try_borrow_mut() else {
            return;
        };
        if pending.len() < self.max_samples.load(Ordering::Relaxed)
            && let Some(stack) = self.capture(self.max_depth.load(Ordering::Relaxed))
        {
            pending.push(StackSample {
                at: now,
                entry,
                stack,
            });
        }
    }

    pub(super) fn reset_interval(&self) {
        self.last.set(None);
    }

    pub fn take(&self) -> Vec<StackSample> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    #[allow(
        unsafe_code,
        reason = "Stack inspection must use the owner thread's already locked QuickJS runtime"
    )]
    fn capture(&self, depth: usize) -> Option<String> {
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
                        for line in stack.lines().take(depth) {
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
