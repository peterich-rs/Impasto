use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use rquickjs::object::Property;
use rquickjs::{Context, Ctx, Function, Object, Persistent, Runtime, Value};

use crate::runtime::monotonic_ms;

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
    eval_budget_ms: i64,
    callback_budget_ms: i64,
    armed: Cell<bool>,
    #[cfg(test)]
    probes: [Option<AcquireProbe>; 2],
}

impl Vm {
    pub(crate) fn configured(
        limits: &crate::runtime::RuntimeLimits,
        stop: Option<AcquireStop>,
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
            eval_budget_ms: limits.eval_budget_ms,
            callback_budget_ms: limits.callback_budget_ms,
            armed: Cell::new(false),
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
                monotonic_ms() > unsafe { &*this }.deadline.get()
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

    pub fn close(&mut self) {
        // Drop promise roots while the QuickJS runtime is still alive.
        self.promises.borrow_mut().clear();
        self.rejections.borrow_mut().clear();
        self.disarm();
        debug_assert!(self.active.get().is_null());
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

    pub fn budget(&self, kind: &'static str) -> Budget<'_> {
        let previous_entry = self.entry.replace((kind, 0));
        let limit = if kind == "eval" {
            self.eval_budget_ms
        } else {
            self.callback_budget_ms
        };
        let deadline = monotonic_ms()
            .saturating_add(limit)
            .min(self.deadline.get());
        let previous_deadline = self.deadline.replace(deadline);
        Budget {
            vm: self,
            previous_entry,
            previous_deadline,
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
}

impl Drop for Budget<'_> {
    fn drop(&mut self) {
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
