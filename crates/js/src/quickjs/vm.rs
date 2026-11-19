use std::cell::{Cell, RefCell};

use rquickjs::{Context, Ctx, Persistent, Runtime, Value};

use crate::monotonic_ms;

pub type Saved = Persistent<Value<'static>>;

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

/// Why [`run_jobs`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStop {
    /// The legacy checkpoint: more than 32 jobs in one drain.
    Limit,
    /// A job threw. `run_jobs` already invoked the failure callback.
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
    armed: Cell<bool>,
}

impl Vm {
    pub fn new() -> Result<Self, &'static str> {
        let runtime = Runtime::new().map_err(|_| "runtime did not start")?;
        runtime.set_memory_limit(16 << 20);
        runtime.set_max_stack_size(8 << 20);
        let context = Context::full(&runtime).map_err(|_| "context did not start")?;
        Ok(Self {
            context: Some(context),
            runtime: Some(runtime),
            active: Cell::new(std::ptr::null()),
            deadline: Cell::new(i64::MAX),
            entry: Cell::new(("", 0)),
            error: RefCell::new((String::new(), String::new())),
            rejections: RefCell::new(Vec::new()),
            armed: Cell::new(false),
        })
    }

    pub fn context(&self) -> Option<&Context> {
        self.context.as_ref()
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
                    } else if rejections.len() < 64
                        && !rejections.iter().any(|(pending, _)| *pending == saved)
                    {
                        rejections.push((saved, Persistent::save(&ctx, reason)));
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
        self.disarm();
        drop(self.context.take());
        drop(self.runtime.take());
    }

    pub fn enter<'a, 'js>(&'a self, ctx: &Ctx<'js>) -> Active<'a> {
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
        let previous_deadline = self
            .deadline
            .replace(monotonic_ms().saturating_add(if kind == "eval" { 1000 } else { 128 }));
        Budget {
            vm: self,
            previous_entry,
            previous_deadline,
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

/// Run pending jobs, stopping after 32.
///
/// `failed` runs on the context of a job that threw. That is not the round limit.
pub fn run_jobs(runtime: &Runtime, mut failed: impl FnMut(&Ctx<'_>)) -> Result<(), JobStop> {
    let mut rounds = 0;
    while runtime.is_job_pending() {
        if rounds >= 32 {
            return Err(JobStop::Limit);
        }
        match runtime.execute_pending_job() {
            Ok(false) => break,
            Ok(true) => rounds += 1,
            Err(error) => {
                error.0.with(|ctx| failed(&ctx));
                return Err(JobStop::Failed);
            }
        }
    }
    Ok(())
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
        timer.eligible_turn = turn + 1;
    } else {
        timer.active = false;
    }
    Some(callback)
}
