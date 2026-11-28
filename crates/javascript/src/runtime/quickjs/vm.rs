use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use rquickjs::object::Property;
use rquickjs::{Context, Ctx, Function, Object, Persistent, Runtime, Value};

use crate::runtime::{JsGcResult, JsMemoryDetail, SamplerConfig, StackSample, monotonic_ms};

use super::sampling::Sampler;

pub type Saved = Persistent<Value<'static>>;

/// A script failure the UI maps onto its own fault record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsDiagnostic {
    pub message: String,
    pub stack: String,
}

/// One timer callback. `kind` 0 is timeout, 1 is interval, 2 is an animation frame.
///
/// `eligible_turn` is the timer-turn that may run this callback. The UI adapter
/// advances that counter once at its existing clock point. A headless runtime
/// advances it once per `drive`.
#[derive(Clone)]
pub struct Timer {
    pub id: u64,
    pub due: i64,
    pub interval: i64,
    pub eligible_turn: u64,
    pub kind: i32,
    pub function: Saved,
    pub extra: Vec<Saved>,
    pub active: bool,
}

/// Why a job checkpoint stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStop {
    /// The legacy checkpoint: more than 32 jobs in one drain.
    Limit,
    /// A job threw and the failure callback was invoked.
    Failed,
}

/// The QuickJS context and the generic bookkeeping that belongs to it.
pub struct Vm {
    context: Option<Context>,
    runtime: Option<Runtime>,
    active: Cell<*const ()>,
    pub deadline: Cell<i64>,
    pub entry: Cell<(&'static str, u64)>,
    pub error: RefCell<(String, String)>,
    pub rejections: RefCell<Vec<(Saved, Saved)>>,
    pub promises: RefCell<HashMap<u64, (Saved, Saved)>>,
    pub rejection_overflows: Cell<usize>,
    pub promise_peak: Cell<usize>,
    pub rejection_peak: Cell<usize>,
    pub(crate) job_limit_hits: Cell<u64>,
    pub(crate) budget_overruns: Cell<u64>,
    interrupted: Cell<bool>,
    eval_budget_ms: Cell<i64>,
    callback_budget_ms: Cell<i64>,
    armed: Cell<bool>,
    sampler: Option<Sampler>,
    #[cfg(test)]
    probes: [Option<AcquireProbe>; 2],
}

impl Vm {
    pub(crate) fn heap_bytes(&self) -> Option<usize> {
        usize::try_from(self.runtime.as_ref()?.memory_usage().memory_used_size).ok()
    }

    pub(crate) fn memory_detail(&self) -> Option<JsMemoryDetail> {
        if self.is_active() {
            return None;
        }
        let usage = self.runtime.as_ref()?.memory_usage();
        let unsigned = |value| u64::try_from(value).unwrap_or_default();
        Some(JsMemoryDetail {
            malloc_size: unsigned(usage.malloc_size),
            malloc_limit: unsigned(usage.malloc_limit),
            memory_used_size: unsigned(usage.memory_used_size),
            malloc_count: unsigned(usage.malloc_count),
            memory_used_count: unsigned(usage.memory_used_count),
            atom_count: unsigned(usage.atom_count),
            atom_size: unsigned(usage.atom_size),
            str_count: unsigned(usage.str_count),
            str_size: unsigned(usage.str_size),
            obj_count: unsigned(usage.obj_count),
            obj_size: unsigned(usage.obj_size),
            prop_count: unsigned(usage.prop_count),
            prop_size: unsigned(usage.prop_size),
            shape_count: unsigned(usage.shape_count),
            shape_size: unsigned(usage.shape_size),
            js_func_count: unsigned(usage.js_func_count),
            js_func_size: unsigned(usage.js_func_size),
            js_func_code_size: unsigned(usage.js_func_code_size),
            js_func_pc2line_count: unsigned(usage.js_func_pc2line_count),
            js_func_pc2line_size: unsigned(usage.js_func_pc2line_size),
            c_func_count: unsigned(usage.c_func_count),
            array_count: unsigned(usage.array_count),
            fast_array_count: unsigned(usage.fast_array_count),
            fast_array_elements: unsigned(usage.fast_array_elements),
            binary_object_count: unsigned(usage.binary_object_count),
            binary_object_size: unsigned(usage.binary_object_size),
        })
    }

    pub(crate) fn run_gc(&self) -> Option<JsGcResult> {
        let before = self.memory_detail()?;
        let start = Instant::now();
        self.runtime.as_ref()?.run_gc();
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        let after = self.memory_detail()?;
        Some(JsGcResult {
            before,
            after,
            duration_ms,
            freed_bytes: before
                .memory_used_size
                .saturating_sub(after.memory_used_size),
        })
    }

    pub(crate) fn set_sampler(&self, config: SamplerConfig) -> bool {
        if let Some(sampler) = &self.sampler {
            sampler.set_config(config);
            true
        } else {
            false
        }
    }

    pub(crate) fn note_job_limit(&self) {
        self.job_limit_hits
            .set(self.job_limit_hits.get().saturating_add(1));
    }

    pub(crate) fn configured(
        limits: &crate::runtime::RuntimeLimits,
        stop: Option<AcquireStop>,
        sampling: Option<Box<dyn Fn() -> bool>>,
    ) -> Result<Self, &'static str> {
        if stop == Some(AcquireStop::Runtime) {
            return Err("runtime did not start");
        }
        let runtime = Runtime::new().map_err(|_| "runtime did not start")?;
        #[cfg(test)]
        let runtime_probe = AcquireProbe::new(0);
        runtime.set_memory_limit(limits.heap_bytes);
        runtime.set_max_stack_size(limits.stack_bytes);
        if stop == Some(AcquireStop::Context) {
            return Err("context did not start");
        }
        let context = Context::full(&runtime).map_err(|_| "context did not start")?;
        #[cfg(test)]
        let context_probe = AcquireProbe::new(1);
        let sampler = sampling
            .map(|enabled| Sampler::new(&runtime, enabled))
            .transpose()
            .map_err(|_| "sampling context did not start")?;
        Ok(Self {
            context: Some(context),
            runtime: Some(runtime),
            active: Cell::new(std::ptr::null()),
            deadline: Cell::new(i64::MAX),
            entry: Cell::new(("", 0)),
            error: RefCell::new((String::new(), String::new())),
            rejections: RefCell::new(Vec::new()),
            promises: RefCell::new(HashMap::new()),
            rejection_overflows: Cell::new(0),
            promise_peak: Cell::new(0),
            rejection_peak: Cell::new(0),
            job_limit_hits: Cell::new(0),
            budget_overruns: Cell::new(0),
            interrupted: Cell::new(false),
            eval_budget_ms: Cell::new(limits.eval_budget_ms),
            callback_budget_ms: Cell::new(limits.callback_budget_ms),
            armed: Cell::new(false),
            sampler,
            #[cfg(test)]
            probes: [Some(runtime_probe), Some(context_probe)],
        })
    }

    /// Allocate one promise and remember its resolve and reject functions.
    pub fn make_promise<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<(u64, Value<'js>)> {
        let token = crate::runtime::resources::next_id()
            .ok_or_else(|| rquickjs::Exception::throw_range(ctx, "request IDs exhausted"))?;
        let (promise, resolve, reject) = ctx.promise()?;
        Object::from_value(promise.as_value().clone())?.prop(
            "token",
            Property::from(crate::runtime::resources::number(token)).enumerable(),
        )?;
        let mut promises = self.promises.borrow_mut();
        promises.insert(
            token,
            (
                Persistent::save(ctx, resolve.into_value()),
                Persistent::save(ctx, reject.into_value()),
            ),
        );
        self.promise_peak
            .set(self.promise_peak.get().max(promises.len()));
        Ok((token, promise.into_value()))
    }

    /// Settle a token once. An unknown or already settled token is ignored.
    pub fn settle<'js>(
        &self,
        ctx: &Ctx<'js>,
        token: u64,
        ok: bool,
        value: Value<'js>,
    ) -> rquickjs::Result<()> {
        let Some((resolve, reject)) = self.promises.borrow_mut().remove(&token) else {
            return Ok(());
        };
        let saved = if ok { resolve } else { reject };
        let function = Function::from_value(saved.restore(ctx)?)?;
        function.call((value,))
    }

    pub fn owns(&self, ctx: &Ctx<'_>) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| context.as_raw() == ctx.as_raw())
    }

    pub fn is_active(&self) -> bool {
        !self.active.get().is_null()
    }

    pub fn jobs_pending(&self) -> bool {
        if self.is_active() {
            self.with(|ctx| {
                // SAFETY: the active context owns this live runtime under its existing lock.
                unsafe {
                    let runtime = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                    rquickjs::qjs::JS_IsJobPending(runtime)
                }
            })
        } else {
            self.runtime.as_ref().is_some_and(Runtime::is_job_pending)
        }
    }

    pub fn runtime(&self) -> Option<&Runtime> {
        self.runtime.as_ref()
    }

    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }

    /// Install the interrupt and rejection tracker.
    ///
    /// # Safety
    /// `this` must be the address of this `Vm` and must not move until [`Vm::disarm`].
    /// The hooks run on the thread that owns the `Vm`.
    pub unsafe fn arm(this: *mut Vm) {
        // SAFETY: the caller pins `this` for the life of the hooks.
        let vm = unsafe { &*this };
        if vm.armed.replace(true) {
            return;
        }
        if let Some(runtime) = &vm.runtime {
            runtime.set_interrupt_handler(Some(Box::new(move || {
                // SAFETY: `this` still addresses the armed `Vm`.
                let vm = unsafe { &*this };
                let expired = monotonic_ms() > vm.deadline.get();
                if expired {
                    vm.interrupted.set(true);
                } else if let Some(sampler) = &vm.sampler {
                    sampler.poll(vm.entry.get().0);
                }
                expired
            })));
            runtime.set_host_promise_rejection_tracker(Some(Box::new(
                move |ctx, promise, reason, handled| {
                    // SAFETY: `this` still addresses the armed `Vm`.
                    let vm = unsafe { &*this };
                    let mut rejections = vm.rejections.borrow_mut();
                    let saved = Persistent::save(&ctx, promise);
                    if handled {
                        rejections.retain(|(pending, _)| *pending != saved);
                    } else if !rejections.iter().any(|(pending, _)| *pending == saved) {
                        if rejections.len() < 64 {
                            rejections.push((saved, Persistent::save(&ctx, reason)));
                            vm.rejection_peak
                                .set(vm.rejection_peak.get().max(rejections.len()));
                        } else {
                            vm.rejection_overflows
                                .set(vm.rejection_overflows.get().saturating_add(1));
                        }
                    }
                },
            )));
        }
    }

    pub fn disarm(&self) {
        if let Some(runtime) = &self.runtime {
            runtime.set_host_promise_rejection_tracker(None);
            runtime.set_interrupt_handler(None);
        }
        self.armed.set(false);
    }

    pub(crate) fn prepare_migration(&self) {
        assert!(!self.is_active(), "cannot move a running QuickJS VM");
        self.disarm();
        if let Some(sampler) = &self.sampler {
            sampler.reset_interval();
        }
    }

    #[expect(
        unsafe_code,
        reason = "QuickJS must refresh its native stack limit after a thread handoff"
    )]
    pub(crate) fn finish_migration(&self, eval_ms: i64, callback_ms: i64) {
        assert!(
            !self.is_active() && !self.armed.get(),
            "QuickJS must be disarmed before resuming"
        );
        if let Some(context) = &self.context {
            // SAFETY: the owning thread has exclusive access to the idle runtime
            // and the boxed VM stays at this address until it is disarmed.
            unsafe {
                let runtime = rquickjs::qjs::JS_GetRuntime(context.as_raw().as_ptr());
                rquickjs::qjs::JS_UpdateStackTop(runtime);
                Self::arm(std::ptr::from_ref(self).cast_mut());
            }
        }
        self.eval_budget_ms.set(eval_ms);
        self.callback_budget_ms.set(callback_ms);
    }

    pub fn close(&mut self) {
        // Drop promise roots while the QuickJS runtime is still alive.
        self.promises.borrow_mut().clear();
        self.rejections.borrow_mut().clear();
        self.disarm();
        debug_assert!(self.active.get().is_null());
        drop(self.sampler.take());
        drop(self.context.take());
        #[cfg(test)]
        drop(self.probes[1].take());
        drop(self.runtime.take());
        #[cfg(test)]
        drop(self.probes[0].take());
    }

    pub(crate) fn enter<'a, 'js>(&'a self, ctx: &Ctx<'js>) -> Active<'a> {
        let previous = self.active.replace(std::ptr::from_ref(ctx).cast());
        Active { vm: self, previous }
    }

    pub fn with<R>(&self, body: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        if !self.active.get().is_null() {
            // SAFETY: `enter` stores the borrowed `Ctx` only while its runtime lock is held.
            let ctx = unsafe { &*self.active.get().cast::<Ctx<'_>>() };
            return body(ctx.clone());
        }
        let Some(context) = &self.context else {
            panic!("VM entry after shutdown")
        };
        context.with(|ctx| {
            let _active = self.enter(&ctx);
            body(ctx.clone())
        })
    }

    pub fn take_stack_samples(&self) -> Vec<StackSample> {
        if let Some(sampler) = &self.sampler
            && self.active.get().is_null()
        {
            return sampler.take();
        }
        Vec::new()
    }

    pub fn budget(&self, kind: &'static str) -> Budget<'_> {
        let previous_entry = self.entry.replace((kind, 0));
        let limit = if kind == "eval" {
            self.eval_budget_ms.get()
        } else {
            self.callback_budget_ms.get()
        };
        let deadline = monotonic_ms()
            .saturating_add(limit)
            .min(self.deadline.get());
        let previous_deadline = self.deadline.replace(deadline);
        Budget {
            vm: self,
            previous_entry,
            previous_deadline,
            previous_interrupt: self.interrupted.replace(false),
        }
    }

    pub fn diagnostic(&self) -> JsDiagnostic {
        let error = self.error.borrow();
        JsDiagnostic {
            message: error.0.clone(),
            stack: error.1.clone(),
        }
    }

    pub fn annotate(&self, message: String) -> String {
        let (kind, id) = self.entry.get();
        if !kind.is_empty()
            && (message.contains("interrupted") || message == "quickjs job round limit")
        {
            if id == 0 {
                format!("{message} ({kind})")
            } else {
                format!("{message} ({kind} node {id})")
            }
        } else {
            message
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcquireStop {
    Runtime,
    Context,
}

#[cfg(test)]
thread_local! {
    static ACQUIRES: Cell<[(usize, usize); 2]> = const { Cell::new([(0, 0); 2]) };
}

#[cfg(test)]
struct AcquireProbe(usize);

#[cfg(test)]
impl AcquireProbe {
    fn new(index: usize) -> Self {
        ACQUIRES.with(|counts| {
            let mut next = counts.get();
            next[index].0 += 1;
            counts.set(next);
        });
        Self(index)
    }
}

#[cfg(test)]
impl Drop for AcquireProbe {
    fn drop(&mut self) {
        ACQUIRES.with(|counts| {
            let mut next = counts.get();
            next[self.0].1 += 1;
            counts.set(next);
        });
    }
}

#[cfg(test)]
pub(crate) fn acquires() -> [(usize, usize); 2] {
    ACQUIRES.with(Cell::get)
}

impl Drop for Vm {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct Active<'a> {
    vm: &'a Vm,
    previous: *const (),
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.vm.active.set(self.previous);
    }
}

pub struct Budget<'a> {
    vm: &'a Vm,
    previous_entry: (&'static str, u64),
    previous_deadline: i64,
    previous_interrupt: bool,
}

impl Drop for Budget<'_> {
    fn drop(&mut self) {
        if self.vm.interrupted.replace(self.previous_interrupt) {
            let count = self.vm.budget_overruns.get();
            self.vm.budget_overruns.set(count.saturating_add(1));
        }
        self.vm.entry.set(self.previous_entry);
        self.vm.deadline.set(self.previous_deadline);
    }
}

pub(crate) fn run_job_batch(
    runtime: &Runtime,
    limit: u32,
    mut failed: impl FnMut(&Ctx<'_>),
) -> Result<JobReport, JobStop> {
    let mut rounds = 0;
    while rounds < limit && runtime.is_job_pending() {
        match runtime.execute_pending_job() {
            Ok(false) => break,
            Ok(true) => rounds += 1,
            Err(error) => {
                error.0.with(|ctx| failed(&ctx));
                return Err(JobStop::Failed);
            }
        }
    }
    Ok(JobReport {
        ran: rounds,
        pending: runtime.is_job_pending(),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JobReport {
    pub ran: u32,
    pub pending: bool,
}

/// Take the next ready timer callback and apply the legacy reschedule rule.
///
/// Interval callbacks (`kind == 1`) stay active and move to the next turn.
/// Other kinds run once. The caller invokes the callback outside the timer borrow.
pub fn poll_timer(
    timers: &mut [Timer],
    animation_frame: bool,
    turn: u64,
    mono: i64,
) -> Option<(Saved, Vec<Saved>)> {
    let timer = timers.iter_mut().find(|timer| {
        timer.active
            && (timer.kind == 2) == animation_frame
            && timer.eligible_turn <= turn
            && (animation_frame || timer.due <= mono)
    })?;
    let callback = (timer.function.clone(), timer.extra.clone());
    if timer.kind == 1 {
        timer.due = mono.saturating_add(timer.interval.max(0));
        timer.eligible_turn = turn.saturating_add(1);
    } else {
        timer.active = false;
    }
    Some(callback)
}
