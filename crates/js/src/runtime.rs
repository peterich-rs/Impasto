//! Owner-thread execution and the runtime's owned host boundary.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use js_host::{
    CreditPool, CreditUsage, Delivery, HostEvent, HttpClient, HttpFailure, HttpPiece, HttpRequest,
    IoEvent, IoJob, PluginValue, RequestKey, Route, RuntimeKey, Settle,
};
use rquickjs::promise::PromiseState;
use rquickjs::{Ctx, Exception, Persistent};

use crate::host::{External, HttpCall, HttpMode, IoBindings, IoPort, OperationPolicy};
use crate::modules::ModuleTable;
use crate::quickjs::binder::{self, ModuleBindings, ModuleRuntime};
use crate::quickjs::{ExtensionContext, JobReport, JobStop, Saved, Vm, poll_timer, run_job_batch};
use crate::resources::{Inbox, Kind, Resource, Resources, SocketState, failure, next_id};
use crate::scheduler::{DrivePolicy, RuntimeLimits, Scheduler};
#[path = "builder.rs"]
mod builder;
pub use builder::RuntimeBuilder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Configuration,
    Script,
    Closed,
    Capacity,
    ForeignTicket,
    UnknownTicket,
    JobLimit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    pub kind: ErrorKind,
    pub message: String,
    pub stack: String,
    pub phase: &'static str,
    pub module: Option<String>,
}

impl JsError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            stack: String::new(),
            phase: "",
            module: None,
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JsError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleTicket {
    runtime: RuntimeKey,
    id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleStatus {
    Pending,
    Fulfilled,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleOutcome {
    Fulfilled,
    Rejected(JsError),
}

enum TrackedModule {
    Pending { promise: Saved, module: String },
    Finished(ModuleOutcome),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScheduleState {
    pub now_ms: i64,
    pub timer_turn: u64,
    pub jobs_pending: bool,
    pub due_next_turn: bool,
    pub queued_events: usize,
    pub in_flight: usize,
    pub next_deadline_ms: Option<i64>,
}

impl ScheduleState {
    pub fn runnable(self) -> bool {
        self.jobs_pending || self.due_next_turn || self.queued_events > 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveReport {
    pub timers_ran: u32,
    pub jobs_ran: u32,
    pub events_delivered: u32,
    pub schedule: ScheduleState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeCounts {
    pub promises: usize,
    pub timers: usize,
    pub tickets: usize,
    pub rejections: usize,
    pub listeners: usize,
    pub requests: usize,
    pub sockets: usize,
    pub queued_events: usize,
    pub queued_bytes: usize,
    pub peak_promises: usize,
    pub peak_timers: usize,
    pub peak_listeners: usize,
    pub peak_tickets: usize,
    pub peak_rejections: usize,
    pub peak_requests: usize,
    pub peak_sockets: usize,
    pub peak_queued_events: usize,
    pub peak_queued_bytes: usize,
    pub discarded_events: usize,
    pub rejection_overflows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Installing,
    Ready,
    Closing,
    Closed,
}

type Log = Box<dyn Fn(u8, &str)>;
type HostPolicy = dyn for<'js> Fn(&Ctx<'js>) -> rquickjs::Result<()>;
pub type CompletionFactory = Arc<dyn Fn(u64) -> Settle + Send + Sync>;
pub type PostDelivery = Arc<dyn Fn(Delivery) + Send + Sync>;

struct State {
    vm: Vm,
    scheduler: Scheduler,
    key: RuntimeKey,
    phase: Cell<Phase>,
    life: RefCell<Option<Rc<()>>>,
    limits: RuntimeLimits,
    drive_policy: DrivePolicy,
    modules: Rc<ModuleTable>,
    tickets: RefCell<HashMap<u64, TrackedModule>>,
    listeners: RefCell<Vec<(String, String, Saved)>>,
    web_cleanup: RefCell<Option<Saved>>,
    ticket_peak: Cell<usize>,
    listener_peak: Cell<usize>,
    log: Log,
    host_policy: RefCell<Rc<HostPolicy>>,
    io: RefCell<IoBindings>,
    jobs: RefCell<Vec<IoJob>>,
    jobs_bytes: Cell<usize>,
    inbox: Arc<Inbox>,
    local: RefCell<VecDeque<Delivery>>,
    resources: RefCell<Resources>,
    ingress: Arc<CreditPool>,
    outbound: Arc<CreditPool>,
}

pub struct Runtime {
    state: Box<State>,
}

#[derive(Clone)]
pub struct WeakRuntime {
    key: RuntimeKey,
    life: Weak<()>,
}

impl WeakRuntime {
    pub fn is_open(&self) -> bool {
        self.life.strong_count() > 0
    }

    pub fn same_runtime(&self, runtime: &Runtime) -> bool {
        self.is_open() && self.key == runtime.key()
    }
}

/// Failed admission returns the original owned event so the host can retry it.
pub enum EnqueueError {
    Closed(IoEvent),
    QueueFull(IoEvent),
    Foreign(Delivery),
}

impl fmt::Debug for EnqueueError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::Closed(_) => "Closed",
            Self::QueueFull(_) => "QueueFull",
            Self::Foreign(_) => "Foreign",
        })
    }
}

impl Runtime {
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    pub fn key(&self) -> RuntimeKey {
        self.state.key
    }

    pub fn extension(&self) -> ExtensionContext<'_> {
        ExtensionContext::new(&self.state.vm)
    }

    pub fn finish_installation(&self) {
        if self.state.phase.get() == Phase::Installing {
            let _budget = self.state.vm.budget("eval");
            let cleanup = self.state.vm.with(|ctx| {
                let cleanup: rquickjs::Result<rquickjs::Value> = ctx
                    .globals()
                    .get::<_, rquickjs::Object>("__host")
                    .and_then(|bridge| bridge.get("_closeWeb"));
                if ctx.has_exception() {
                    drop(ctx.catch());
                }
                cleanup
                    .ok()
                    .filter(rquickjs::Value::is_function)
                    .map(|cleanup| Persistent::save(&ctx, cleanup))
            });
            *self.state.web_cleanup.borrow_mut() = cleanup;
            self.state.phase.set(Phase::Ready);
        }
    }

    pub fn configure_io(&self, io: IoBindings) {
        if self.state.is_open() {
            *self.state.io.borrow_mut() = io;
        }
    }

    pub fn downgrade(&self) -> WeakRuntime {
        WeakRuntime {
            key: self.key(),
            life: self
                .state
                .life
                .borrow()
                .as_ref()
                .map(Rc::downgrade)
                .unwrap_or_default(),
        }
    }

    pub fn counts(&self) -> RuntimeCounts {
        let state = &self.state;
        if state.phase.get() == Phase::Closed {
            return RuntimeCounts::default();
        }
        let resources = state.resources.borrow();
        let CreditUsage {
            bytes,
            peak_events,
            peak_bytes,
            ..
        } = state.ingress.usage();
        RuntimeCounts {
            promises: state.vm.promises.borrow().len(),
            timers: state
                .scheduler
                .timers
                .borrow()
                .iter()
                .filter(|timer| timer.active)
                .count(),
            tickets: state.tickets.borrow().len(),
            rejections: state.vm.rejections.borrow().len(),
            listeners: state.listeners.borrow().len(),
            requests: resources.requests(),
            sockets: resources.sockets(),
            queued_events: state.inbox.len() + state.local.borrow().len(),
            queued_bytes: bytes,
            peak_promises: state.vm.promise_peak.get(),
            peak_timers: state.scheduler.peak.get(),
            peak_tickets: state.ticket_peak.get(),
            peak_listeners: state.listener_peak.get(),
            peak_rejections: state.vm.rejection_peak.get(),
            peak_requests: resources.peak,
            peak_sockets: resources.socket_peak,
            peak_queued_events: peak_events,
            peak_queued_bytes: peak_bytes,
            discarded_events: resources.discarded,
            rejection_overflows: state.vm.rejection_overflows.get(),
        }
    }

    pub fn eval_script(&mut self, source: &str, name: &str) -> Result<(), JsError> {
        let state = &self.state;
        state.ensure_open()?;
        let _budget = state.vm.budget("eval");
        state.vm.with(|ctx| {
            let mut options = rquickjs::context::EvalOptions::default();
            options.filename = Some(name.to_owned());
            ctx.eval_with_options::<(), _>(source, options)
                .map_err(|error| state.script_error(&ctx, error))
        })
    }

    pub fn eval_compiled(
        &mut self,
        compiled: &crate::bytecode::CompiledBytecode,
    ) -> Result<(), JsError> {
        self.state.ensure_open()?;
        if compiled.kind() != crate::bytecode::Kind::Script {
            return Err(JsError::new(
                ErrorKind::Configuration,
                "expected compiled script",
            ));
        }
        let _budget = self.state.vm.budget("eval");
        let entry = crate::quickjs::TrustedBytecode::from_compiled(compiled);
        self.state.vm.with(|ctx| {
            entry
                .load(&ctx)
                .map(|_| ())
                .map_err(|error| JsError::new(ErrorKind::Script, error.to_string()))
        })
    }

    pub fn start_module(&mut self, name: &str) -> Result<ModuleTicket, JsError> {
        let state = &self.state;
        state.ensure_open()?;
        if !state.modules.contains(name) {
            return Err(JsError::new(
                ErrorKind::Configuration,
                format!("module {name:?} is not registered"),
            ));
        }
        let id = self.ticket_id()?;
        let _budget = state.vm.budget("eval");
        let saved = state.vm.with(|ctx| {
            let promise = if let Some(source) = state.modules.source(name) {
                rquickjs::Module::evaluate(ctx.clone(), name, source)
            } else {
                crate::modules::declare_native(&state.modules, &ctx, name)
                    .and_then(|module| module.eval().map(|(_, promise)| promise))
            }
            .map_err(|error| {
                let mut diagnostic = state.script_error(&ctx, error);
                diagnostic.module = Some(name.to_owned());
                diagnostic
            })?;
            Ok(Persistent::save(&ctx, promise.into_value()))
        })?;
        Ok(self.track_module(id, saved, name))
    }

    pub fn start_compiled_module(
        &mut self,
        compiled: &crate::bytecode::CompiledBytecode,
        name: &str,
    ) -> Result<ModuleTicket, JsError> {
        self.state.ensure_open()?;
        if compiled.kind() != crate::bytecode::Kind::Module {
            return Err(JsError::new(
                ErrorKind::Configuration,
                "expected compiled module",
            ));
        }
        let id = self.ticket_id()?;
        let _budget = self.state.vm.budget("eval");
        let entry = crate::quickjs::TrustedBytecode::from_compiled(compiled);
        let saved = self.state.vm.with(|ctx| {
            let promise = entry
                .load(&ctx)
                .map_err(|error| JsError::new(ErrorKind::Script, error.to_string()))?
                .ok_or_else(|| {
                    JsError::new(ErrorKind::Script, "compiled module returned no promise")
                })?;
            let value = promise
                .restore(&ctx)
                .map_err(|error| self.state.script_error(&ctx, error))?;
            Ok(Persistent::save(&ctx, value.into_value()))
        })?;
        Ok(self.track_module(id, saved, name))
    }

    fn ticket_id(&self) -> Result<u64, JsError> {
        if self.state.tickets.borrow().len() >= self.state.limits.tickets {
            return Err(JsError::new(
                ErrorKind::Capacity,
                "module ticket limit exceeded",
            ));
        }
        next_id().ok_or_else(|| JsError::new(ErrorKind::Capacity, "module IDs exhausted"))
    }

    fn track_module(&self, id: u64, promise: Saved, name: &str) -> ModuleTicket {
        self.state.tickets.borrow_mut().insert(
            id,
            TrackedModule::Pending {
                promise,
                module: name.into(),
            },
        );
        let count = self.state.tickets.borrow().len();
        self.state
            .ticket_peak
            .set(self.state.ticket_peak.get().max(count));
        self.state.refresh_ticket(id);
        ModuleTicket {
            runtime: self.key(),
            id,
        }
    }

    fn validate_ticket(&self, ticket: ModuleTicket) -> Result<(), JsError> {
        self.state.ensure_open()?;
        if ticket.runtime != self.key() {
            return Err(JsError::new(
                ErrorKind::ForeignTicket,
                "module ticket belongs to another runtime",
            ));
        }
        if !self.state.tickets.borrow().contains_key(&ticket.id) {
            return Err(JsError::new(
                ErrorKind::UnknownTicket,
                "module ticket was consumed or abandoned",
            ));
        }
        Ok(())
    }

    pub fn module_status(&self, ticket: ModuleTicket) -> Result<ModuleStatus, JsError> {
        self.validate_ticket(ticket)?;
        let tickets = self.state.tickets.borrow();
        Ok(match tickets.get(&ticket.id) {
            Some(TrackedModule::Finished(ModuleOutcome::Fulfilled)) => ModuleStatus::Fulfilled,
            Some(TrackedModule::Finished(ModuleOutcome::Rejected(_))) => ModuleStatus::Rejected,
            _ => ModuleStatus::Pending,
        })
    }

    pub fn take_module_result(
        &mut self,
        ticket: ModuleTicket,
    ) -> Result<Option<ModuleOutcome>, JsError> {
        if self.module_status(ticket)? == ModuleStatus::Pending {
            return Ok(None);
        }
        let result = match self.state.tickets.borrow_mut().remove(&ticket.id) {
            Some(TrackedModule::Finished(result)) => Some(result),
            _ => None,
        };
        Ok(result)
    }

    pub fn abandon_module(&mut self, ticket: ModuleTicket) -> Result<(), JsError> {
        self.validate_ticket(ticket)?;
        self.state.tickets.borrow_mut().remove(&ticket.id);
        Ok(())
    }

    pub fn set_clock(&self, now_ms: i64) {
        if self.state.scheduler.set_clock(now_ms) {
            self.state.resources.borrow_mut().shift_deadlines(now_ms);
        }
    }

    pub fn clock_ms(&self) -> i64 {
        self.state.scheduler.mono.get()
    }

    pub fn jobs_pending(&self) -> bool {
        self.state.vm.jobs_pending()
    }

    pub fn due_timer_next_turn(&self) -> bool {
        self.state
            .scheduler
            .next_timer_ms()
            .is_some_and(|due| due <= self.clock_ms())
    }

    pub fn begin_timer_turn(&self) {
        self.state.scheduler.begin_turn();
    }

    pub fn performance_now(&self) -> f64 {
        self.state.scheduler.now()
    }

    /// Reserve an ID in the same sequence as timeouts and intervals.
    pub fn allocate_timer_id(&self) -> Option<u64> {
        if !self.state.vm.is_open() {
            return None;
        }
        self.state.scheduler.allocate_id()
    }

    pub fn timer_turn(&self) -> u64 {
        self.state.scheduler.turn.get()
    }

    pub fn next_deadline_ms(&self) -> Option<i64> {
        self.state
            .scheduler
            .next_timer_ms()
            .into_iter()
            .chain(self.state.resources.borrow().deadline())
            .min()
    }

    pub fn schedule_state(&self) -> ScheduleState {
        let state = &self.state;
        let resources = state.resources.borrow();
        let now_ms = state.scheduler.mono.get();
        ScheduleState {
            now_ms,
            timer_turn: self.timer_turn(),
            jobs_pending: self.jobs_pending(),
            due_next_turn: self.due_timer_next_turn(),
            queued_events: state.inbox.len() + state.local.borrow().len(),
            in_flight: resources.requests() + resources.sockets(),
            next_deadline_ms: state
                .scheduler
                .next_timer_ms()
                .into_iter()
                .chain(resources.deadline())
                .min(),
        }
    }

    pub fn run_due_timers(
        &self,
        mut on_error: impl FnMut(&Ctx<'_>, rquickjs::Error) -> bool,
    ) -> Result<u32, JsError> {
        let state = &self.state;
        state.ensure_open()?;
        let _budget = state.vm.budget("timer");
        let mut ran = 0;
        loop {
            if ran == state.limits.timer_batch {
                break;
            }
            let timer = poll_timer(
                &mut state.scheduler.timers.borrow_mut(),
                false,
                self.timer_turn(),
                state.scheduler.mono.get(),
            );
            let Some(timer) = timer else { break };
            ran += 1;
            let result = state.vm.with(|ctx| {
                if let Err(error) = Scheduler::call_timer(&ctx, timer) {
                    let is_exception = error.is_exception();
                    let message = error.to_string();
                    if !on_error(&ctx, error) {
                        return Err(if is_exception {
                            state.script_error(&ctx, rquickjs::Error::Exception)
                        } else {
                            JsError::new(ErrorKind::Script, message)
                        });
                    }
                }
                Ok(())
            });
            state
                .scheduler
                .timers
                .borrow_mut()
                .retain(|timer| timer.active);
            result?;
            if crate::monotonic_ms() > state.vm.deadline.get() {
                break;
            }
        }
        Ok(ran)
    }

    pub fn checkpoint(&self, failed: impl FnMut(&Ctx<'_>)) -> Result<JobReport, JobStop> {
        if self.state.vm.is_active() {
            return Err(JobStop::Failed);
        }
        let _budget = self.state.vm.budget("jobs");
        let Some(runtime) = self.state.vm.runtime() else {
            return Err(JobStop::Failed);
        };
        let report = run_job_batch(runtime, self.state.limits.job_batch, failed)?;
        if report.pending && self.state.drive_policy == DrivePolicy::LegacyStrict {
            return Err(JobStop::Limit);
        }
        Ok(report)
    }

    pub fn drive(&mut self, now_ms: i64) -> Result<DriveReport, JsError> {
        let state = &self.state;
        state.ensure_open()?;
        let mut events_delivered = 0;
        for _ in 0..state.limits.event_batch {
            let event = state.inbox.pop().map(|event| (event, false)).or_else(|| {
                state
                    .local
                    .borrow_mut()
                    .pop_front()
                    .map(|event| (event, true))
            });
            let Some((event, local)) = event else { break };
            self.deliver_routed_inner(event, local)?;
            events_delivered += 1;
        }
        self.begin_timer_turn();
        self.set_clock(now_ms);
        let mut expiry_error = None;
        for expired in state.expire() {
            match self.deliver_routed_inner(expired, true) {
                Ok(()) => events_delivered += 1,
                Err(error) => {
                    expiry_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = expiry_error {
            return Err(error);
        }
        let timers_ran = self.run_due_timers(|_, _| false)?;
        let mut failure = None;
        let jobs = self.checkpoint(|ctx| {
            failure = Some(state.script_error(ctx, rquickjs::Error::Exception));
        });
        let tickets: Vec<_> = state.tickets.borrow().keys().copied().collect();
        for id in tickets {
            state.refresh_ticket(id);
        }
        let jobs_ran = match jobs {
            Ok(report) => report.ran,
            Err(JobStop::Limit) => {
                return Err(JsError::new(ErrorKind::JobLimit, "quickjs job round limit"));
            }
            Err(JobStop::Failed) => {
                return Err(
                    failure.unwrap_or_else(|| JsError::new(ErrorKind::Script, "job failed"))
                );
            }
        };
        state.report_rejections();
        Ok(DriveReport {
            timers_ran,
            jobs_ran,
            events_delivered,
            schedule: self.schedule_state(),
        })
    }

    /// Work queued for an external owner. Inline work has already run.
    pub fn take_jobs(&mut self) -> Vec<IoJob> {
        self.state.jobs_bytes.set(0);
        std::mem::take(&mut *self.state.jobs.borrow_mut())
    }

    pub fn enqueue(&self, event: IoEvent) -> Result<(), EnqueueError> {
        self.enqueue_delivery(Delivery {
            runtime: self.key(),
            event,
            credit: None,
        })
    }

    pub fn enqueue_delivery(&self, mut delivery: Delivery) -> Result<(), EnqueueError> {
        let state = &self.state;
        if delivery.runtime != self.key() {
            return Err(EnqueueError::Foreign(delivery));
        }
        if !state.is_open() {
            return Err(EnqueueError::Closed(delivery.event));
        }
        let id = delivery.event.id().unwrap_or_default();
        let bytes = delivery.event.payload_bytes();
        let known = state
            .resources
            .borrow()
            .get(id)
            .map(|resource| resource.route.clone());
        let carries_chunk =
            matches!(&delivery.event, IoEvent::HttpChunk { bytes, .. } if !bytes.is_empty());
        let reserved_terminal = delivery.event.is_terminal()
            && !carries_chunk
            && known.is_some()
            && bytes <= js_host::TERMINAL_PAYLOAD_LIMIT;
        let credit_verified = delivery
            .credit
            .as_ref()
            .is_some_and(|credit| credit.covers(&state.ingress, id, bytes));
        if !credit_verified {
            drop(delivery.credit.take());
        }
        if delivery.credit.is_none() && !reserved_terminal {
            let route = known
                .as_ref()
                .filter(|route| route.is_active())
                .cloned()
                .unwrap_or_else(|| state.route(id, Arc::new(AtomicBool::new(false)), false));
            delivery.credit = route.reserve(bytes, Duration::ZERO);
            if delivery.credit.is_none() {
                return Err(EnqueueError::QueueFull(delivery.event));
            }
        }
        if !credit_verified
            && delivery.event.is_terminal()
            && let Some(route) = known
            && !route.claim_terminal()
        {
            let mut resources = state.resources.borrow_mut();
            resources.discarded = resources.discarded.saturating_add(1);
            return Ok(());
        }
        state.inbox.push(delivery).map_err(EnqueueError::Foreign)
    }

    pub fn deliver_event(&self, event: IoEvent) -> Result<(), JsError> {
        self.deliver_routed(Delivery {
            runtime: self.key(),
            event,
            credit: None,
        })
    }

    pub fn deliver_routed(&self, delivery: Delivery) -> Result<(), JsError> {
        self.deliver_routed_inner(delivery, false)
    }

    fn deliver_routed_inner(&self, delivery: Delivery, local: bool) -> Result<(), JsError> {
        let state = &self.state;
        state.ensure_open()?;
        if delivery.runtime != self.key() {
            let mut resources = state.resources.borrow_mut();
            resources.discarded = resources.discarded.saturating_add(1);
            return Ok(());
        }
        if !state.resources.borrow_mut().accepts(&delivery.event, local) {
            return Ok(());
        }
        if let IoEvent::Host(event) = delivery.event {
            let mut failure = None;
            self.emit_host_event(event, |ctx, error| {
                failure = Some(state.script_error(ctx, error));
                false
            })?;
            return failure.map_or(Ok(()), Err);
        }
        let _budget = state.vm.budget("delivery");
        let id = delivery.event.id();
        state.vm.with(|ctx| {
            if let Err(error) =
                crate::deliver::apply(&state.vm, &ctx, delivery.event, delivery.credit)
            {
                let diagnostic = state.script_error(&ctx, error);
                if let Some(id) = id {
                    let _ = crate::deliver::reject_bad_completion(&state.vm, &ctx, id);
                    if ctx.has_exception() {
                        drop(ctx.catch());
                    }
                    if let Some(resource) = state.resources.borrow_mut().remove(id) {
                        resource.route.cancel();
                    }
                }
                return Err(diagnostic);
            }
            Ok(())
        })
    }

    /// Frame delivery retains the UI's HTTP, module completion, socket ordering.
    pub fn drain_local_io(&self, mut on_error: impl FnMut(JsError)) {
        let expired = self.state.expire();
        let mut events: Vec<_> = self
            .state
            .local
            .borrow_mut()
            .drain(..)
            .map(|event| (event, true))
            .collect();
        for _ in 0..self.state.limits.event_batch {
            let Some(event) = self.state.inbox.pop() else {
                break;
            };
            events.push((event, false));
        }
        events.extend(expired.into_iter().map(|event| (event, true)));
        events.sort_by_key(|(delivery, _)| match delivery.event {
            IoEvent::Plugin { .. } | IoEvent::Host(_) => 1,
            IoEvent::Socket { .. } => 2,
            _ => 0,
        });
        for (event, local) in events {
            if let Err(error) = self.deliver_routed_inner(event, local) {
                on_error(error);
            }
        }
    }

    pub fn listen(&mut self, module: &str, event: &str, source: &str) -> Result<(), JsError> {
        let state = &self.state;
        state.ensure_open()?;
        if state.listeners.borrow().len() >= state.limits.listeners {
            return Err(JsError::new(ErrorKind::Capacity, "listener limit exceeded"));
        }
        let _budget = state.vm.budget("eval");
        let saved = state.vm.with(|ctx| {
            let value: rquickjs::Value = ctx
                .eval(source)
                .map_err(|error| state.script_error(&ctx, error))?;
            if !value.is_function() {
                return Err(JsError::new(
                    ErrorKind::Script,
                    "listener must be a function",
                ));
            }
            Ok(Persistent::save(&ctx, value))
        })?;
        state.add_listener(module, event, saved);
        Ok(())
    }

    pub fn emit_listener(&self, module: &str, event: &str, text: &str) -> Result<(), JsError> {
        self.deliver_event(IoEvent::Host(HostEvent {
            module: module.into(),
            event: event.into(),
            payload: PluginValue::Str(text.into()),
        }))
    }

    pub fn emit_host_event(
        &self,
        event: HostEvent,
        on_error: impl FnMut(&Ctx<'_>, rquickjs::Error) -> bool,
    ) -> Result<usize, JsError> {
        let state = &self.state;
        state.ensure_open()?;
        let callbacks = state
            .listeners
            .borrow()
            .iter()
            .filter(|(module, name, _)| *module == event.module && *name == event.event)
            .map(|(_, _, callback)| callback.clone())
            .collect();
        let _budget = state.vm.budget("listener");
        Ok(state
            .vm
            .with(|ctx| binder::deliver_event(&ctx, callbacks, &event, on_error)))
    }

    pub fn install_standard_bindings(
        &self,
        ctx: &Ctx<'_>,
        on_zero_delay: impl Fn() + Copy + 'static,
    ) -> rquickjs::Result<()> {
        self.check_context(ctx)?;
        let state = std::ptr::from_ref(&*self.state);
        crate::standard::install_console(ctx, move |level, text| {
            // SAFETY: the context and these callbacks are owned by boxed state.
            (unsafe { &*state }.log)(level, text);
        })?;
        let scheduler = &self.state.scheduler;
        // SAFETY: the scheduler lives at a stable address in the context owner.
        unsafe {
            crate::standard::install_timers(ctx, scheduler, self.state.limits.timers, on_zero_delay)
        }?;
        crate::standard::install_performance(ctx, move || {
            // SAFETY: the context and these callbacks are owned by boxed state.
            unsafe { &*state }.scheduler.now()
        })
    }

    pub fn install_host(
        &self,
        ctx: &Ctx<'_>,
        policy: impl for<'js> Fn(&Ctx<'js>) -> rquickjs::Result<()> + 'static,
        has_module: impl Fn(&str) -> bool + 'static,
    ) -> rquickjs::Result<()> {
        self.install_host_inner(ctx, policy, has_module, false)
    }

    fn install_host_inner(
        &self,
        ctx: &Ctx<'_>,
        policy: impl for<'js> Fn(&Ctx<'js>) -> rquickjs::Result<()> + 'static,
        has_module: impl Fn(&str) -> bool + 'static,
        stop: bool,
    ) -> rquickjs::Result<()> {
        self.check_context(ctx)?;
        let state = &*self.state;
        *state.host_policy.borrow_mut() = Rc::new(policy);
        // SAFETY: the stable boxed state owns the VM and both providers until its context closes.
        unsafe { crate::host::install(ctx, &state.vm, state, state, has_module, stop) }
    }

    pub fn install_modules(
        &self,
        ctx: &Ctx<'_>,
        bindings: ModuleBindings,
        policy: impl for<'js> Fn(&Ctx<'js>) -> rquickjs::Result<()> + 'static,
        completion: Option<CompletionFactory>,
        post: Option<PostDelivery>,
    ) -> rquickjs::Result<()> {
        self.check_context(ctx)?;
        let provider = Rc::new(BoundRuntime {
            state: std::ptr::from_ref(&*self.state),
            life: self.downgrade(),
            policy: Rc::new(policy),
            completion,
            post,
        });
        binder::install(ctx, bindings, provider)
    }

    fn check_context(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.state.vm.owns(ctx) && self.state.is_open() {
            Ok(())
        } else {
            Err(Exception::throw_type(
                ctx,
                "extension context is closed or foreign",
            ))
        }
    }

    pub fn begin_shutdown(&self) {
        if self.state.is_open() {
            self.state.phase.set(Phase::Closing);
            self.state.inbox.close();
            self.state.ingress.close();
            self.state.outbound.close();
            self.state.resources.borrow_mut().close();
            let cleanup = self.state.web_cleanup.borrow_mut().take();
            if let Some(cleanup) = cleanup {
                let _budget = self.state.vm.budget("shutdown");
                self.state.vm.with(|ctx| {
                    let result = cleanup
                        .restore(&ctx)
                        .and_then(rquickjs::Function::from_value)
                        .and_then(|close| close.call::<_, ()>(()));
                    if let Err(error) = result {
                        let diagnostic = self.state.script_error(&ctx, error);
                        (self.state.log)(3, &diagnostic.message);
                    }
                });
            }
        }
    }

    pub fn shutdown(&mut self) {
        if self.state.phase.get() == Phase::Closed {
            return;
        }
        self.begin_shutdown();
        self.state.local.borrow_mut().clear();
        self.state.jobs.borrow_mut().clear();
        self.state.scheduler.timers.borrow_mut().clear();
        self.state.listeners.borrow_mut().clear();
        self.state.tickets.borrow_mut().clear();
        self.state.life.borrow_mut().take();
        self.state.vm.close();
        self.state.phase.set(Phase::Closed);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct BoundRuntime {
    state: *const State,
    life: WeakRuntime,
    policy: Rc<HostPolicy>,
    completion: Option<CompletionFactory>,
    post: Option<PostDelivery>,
}

impl BoundRuntime {
    fn state(&self) -> Option<&State> {
        // SAFETY: the lifetime token is invalidated before boxed state is released.
        self.life.is_open().then(|| unsafe { &*self.state })
    }
}

impl ModuleRuntime for BoundRuntime {
    fn enter<'a>(&'a self, ctx: &Ctx<'_>) -> Option<crate::quickjs::Active<'a>> {
        self.state().map(|state| state.vm.enter(ctx))
    }

    fn before(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        if !state.is_open() {
            return Err(Exception::throw_type(ctx, "runtime is closing"));
        }
        (self.policy)(ctx)
    }

    fn reserve_plugin<'js>(
        &self,
        ctx: &Ctx<'js>,
    ) -> rquickjs::Result<(rquickjs::Value<'js>, Settle)> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        if state.resources.borrow().requests() >= state.limits.requests {
            return Err(Exception::throw_range(ctx, "request limit exceeded"));
        }
        let (id, promise) = state.vm.make_promise(ctx)?;
        let route = state.register(id, Kind::Plugin, None);
        let settle = if let Some(factory) = &self.completion
            && self.post.is_none()
        {
            factory(id)
        } else {
            let inbox = Arc::clone(&state.inbox);
            let post = self.post.clone();
            Settle::new(move |result| {
                if !route.claim_terminal() {
                    return;
                }
                let mut event = IoEvent::Plugin { token: id, result };
                let credit = route.reserve(event.payload_bytes(), Duration::ZERO);
                if credit.is_none() {
                    event = failure(
                        id,
                        Kind::Plugin,
                        HttpFailure::TooLarge,
                        "host completion exceeds queue limit",
                    );
                }
                let delivery = route.deliver(event, credit);
                if let Some(post) = post {
                    post(delivery);
                } else {
                    let _ = inbox.push(delivery);
                }
            })
        };
        Ok((promise, settle))
    }

    fn add_listener(&self, module: &str, event: &str, callback: Saved) {
        if let Some(state) = self.state() {
            state.add_listener(module, event, callback);
        }
    }

    fn remove_listener(&self, module: &str, event: &str, callback: &Saved) {
        if let Some(state) = self.state() {
            state
                .listeners
                .borrow_mut()
                .retain(|(m, e, saved)| m != module || e != event || saved != callback);
        }
    }
}

impl State {
    fn is_open(&self) -> bool {
        matches!(self.phase.get(), Phase::Installing | Phase::Ready)
    }

    fn ensure_open(&self) -> Result<(), JsError> {
        if self.is_open() {
            Ok(())
        } else {
            Err(JsError::new(ErrorKind::Closed, "runtime is closed"))
        }
    }

    fn script_error(&self, ctx: &Ctx<'_>, error: rquickjs::Error) -> JsError {
        let (message, stack) = if error.is_exception() {
            crate::quickjs::exception_text(ctx, ctx.catch())
        } else {
            (error.to_string(), String::new())
        };
        JsError {
            kind: ErrorKind::Script,
            message: self.vm.annotate(message),
            stack,
            phase: self.vm.entry.get().0,
            module: None,
        }
    }

    fn refresh_ticket(&self, id: u64) {
        let (saved, module) = match self.tickets.borrow().get(&id) {
            Some(TrackedModule::Pending { promise, module }) => (promise.clone(), module.clone()),
            _ => return,
        };
        let _budget = self.vm.budget("module");
        let outcome = self.vm.with(|ctx| {
            let value = saved.restore(&ctx).ok()?;
            let promise = value.as_promise()?;
            match promise.state() {
                PromiseState::Pending => None,
                PromiseState::Resolved => Some(ModuleOutcome::Fulfilled),
                PromiseState::Rejected => {
                    let result = promise.result::<()>()?;
                    let error = result.err()?;
                    let mut diagnostic = self.script_error(&ctx, error);
                    diagnostic.module = Some(module);
                    Some(ModuleOutcome::Rejected(diagnostic))
                }
            }
        });
        if let Some(outcome) = outcome {
            self.tickets
                .borrow_mut()
                .insert(id, TrackedModule::Finished(outcome));
        }
    }

    fn report_rejections(&self) {
        let _budget = self.vm.budget("rejection");
        // Report the snapshot after a checkpoint, before releasing its roots.
        let reasons = std::mem::take(&mut *self.vm.rejections.borrow_mut());
        if reasons.is_empty() {
            return;
        }
        self.vm.with(|ctx| {
            for (_, reason) in reasons {
                if let Ok(reason) = reason.restore(&ctx) {
                    let (message, _) = crate::quickjs::exception_text(&ctx, reason);
                    (self.log)(3, &message);
                }
            }
        });
    }

    fn add_listener(&self, module: &str, event: &str, callback: Saved) {
        let mut listeners = self.listeners.borrow_mut();
        if listeners.len() < self.limits.listeners {
            listeners.push((module.into(), event.into(), callback));
            self.listener_peak
                .set(self.listener_peak.get().max(listeners.len()));
        }
    }

    fn route(&self, id: u64, cancel: Arc<AtomicBool>, stream: bool) -> Route {
        Route::new(
            RequestKey {
                runtime: self.key,
                sequence: id,
            },
            Arc::clone(&self.ingress),
            Arc::clone(&self.outbound),
            cancel,
            stream,
        )
    }

    fn deadline(&self) -> i64 {
        self.scheduler
            .mono
            .get()
            .saturating_add(i64::try_from(self.limits.request_timeout_ms).unwrap_or(i64::MAX))
    }

    fn register(&self, id: u64, kind: Kind, cancel: Option<Arc<AtomicBool>>) -> Route {
        let route = self.route(
            id,
            cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
            matches!(kind, Kind::Stream { .. } | Kind::Socket(_)),
        );
        self.resources.borrow_mut().insert(
            id,
            Resource {
                kind,
                route: route.clone(),
                deadline: Some(self.deadline()),
            },
        );
        route
    }

    fn queue_terminal(&self, id: u64, kind: Kind, error: HttpFailure, reason: &str) {
        self.local.borrow_mut().push_back(Delivery {
            runtime: self.key,
            event: failure(id, kind, error, reason),
            credit: None,
        });
    }

    fn expire(&self) -> Vec<Delivery> {
        let expired = self.resources.borrow().expired(self.scheduler.mono.get());
        let mut events = Vec::with_capacity(expired.len());
        for id in expired {
            let resource = {
                let mut resources = self.resources.borrow_mut();
                let Some(resource) = resources.get(id) else {
                    continue;
                };
                let kind = resource.kind;
                let route = resource.route.clone();
                resources.set_kind(id, kind, None);
                (kind, route)
            };
            resource.1.cancel();
            self.inbox.discard(id);
            self.local
                .borrow_mut()
                .retain(|event| event.event.id() != Some(id));
            events.push(Delivery {
                runtime: self.key,
                event: failure(
                    id,
                    resource.0,
                    HttpFailure::Timeout,
                    "transport deadline expired",
                ),
                credit: None,
            });
        }
        events
    }

    fn submit(&self, transport: &External, route: &Route, job: IoJob) -> bool {
        match transport {
            External::Jobs => {
                let mut jobs = self.jobs.borrow_mut();
                let pending = self.jobs_bytes.get().saturating_add(job.payload_bytes());
                if jobs.len() >= self.limits.queue_events || pending > self.limits.queue_bytes {
                    return false;
                }
                self.jobs_bytes.set(pending);
                jobs.push(job);
                true
            }
            External::Legacy(submit) => submit(job).is_ok(),
            External::Routed(submit) => {
                let outbound = match &job {
                    IoJob::SocketSend { .. } => {
                        let Some(credit) = route.reserve_send() else {
                            return false;
                        };
                        Some(credit)
                    }
                    IoJob::Http(_) | IoJob::HttpStream(_) | IoJob::SocketOpen { .. } => {
                        let Some(credit) = route.reserve_upload(job.payload_bytes()) else {
                            return false;
                        };
                        Some(credit)
                    }
                    _ => None,
                };
                submit(IoJob::Routed {
                    route: route.clone(),
                    job: Box::new(job),
                    outbound,
                })
                .is_ok()
            }
        }
    }

    fn inline_http(
        &self,
        client: &dyn HttpClient,
        request: &HttpRequest,
        kind: Kind,
        route: &Route,
    ) {
        let token = request.token;
        if matches!(kind, Kind::Http) {
            let result = client.perform(request);
            let event = IoEvent::Http { token, result };
            if let Some(credit) = route.reserve(event.payload_bytes(), Duration::ZERO) {
                self.local
                    .borrow_mut()
                    .push_back(route.deliver(event, Some(credit)));
            } else {
                self.queue_terminal(
                    token,
                    kind,
                    HttpFailure::TooLarge,
                    "inline response exceeds queue limit",
                );
            }
            return;
        }
        // Inline cannot wait for a reader while it is on the JavaScript stack.
        // Stage the bounded whole response, then replay it at the local checkpoint.
        let mut events = Vec::new();
        let mut ended = false;
        let mut overflow = false;
        let mut total: usize = 0;
        let result = client.stream(request, &mut |piece| {
            if ended || overflow {
                return false;
            }
            let event = match piece {
                HttpPiece::Head(status, headers) => IoEvent::HttpHead {
                    token,
                    status,
                    headers: headers.to_vec(),
                },
                HttpPiece::Chunk(bytes, done) => {
                    if bytes.len() > js_host::HTTP_BODY_LIMIT.saturating_sub(total) {
                        overflow = true;
                        return false;
                    }
                    ended = done;
                    IoEvent::HttpChunk {
                        token,
                        bytes: bytes.to_vec(),
                        done,
                        error: None,
                    }
                }
            };
            total = total.saturating_add(event.payload_bytes());
            let whole_response = self.route(token, Arc::clone(&request.cancel), false);
            if let Some(credit) = whole_response.reserve(event.payload_bytes(), Duration::ZERO) {
                events.push(route.deliver(event, Some(credit)));
                true
            } else {
                overflow = true;
                false
            }
        });
        if overflow {
            self.queue_terminal(
                token,
                kind,
                HttpFailure::TooLarge,
                "inline stream exceeds queue limit",
            );
            return;
        }
        self.local.borrow_mut().extend(events);
        if !ended {
            let error = result.err();
            self.local.borrow_mut().push_back(route.deliver(
                IoEvent::HttpChunk {
                    token,
                    bytes: Vec::new(),
                    done: true,
                    error,
                },
                None,
            ));
        }
    }
}

impl OperationPolicy for State {
    fn before(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if !self.is_open() {
            return Err(Exception::throw_type(ctx, "runtime is closing"));
        }
        let policy = Rc::clone(&self.host_policy.borrow());
        policy(ctx)
    }
}

impl IoPort for State {
    fn http(&self, call: HttpCall) -> bool {
        if self.resources.borrow().requests() >= self.limits.requests {
            return false;
        }
        let kind = if call.streaming {
            Kind::Stream { head: false }
        } else {
            Kind::Http
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let route = self.register(call.token, kind, Some(Arc::clone(&cancel)));
        let request = HttpRequest {
            token: call.token,
            method: call.method,
            url: call.url,
            headers: header_pairs(&call.headers),
            body: call.body,
            cancel,
            timeout_ms: Some(self.limits.request_timeout_ms),
        };
        let mode = self.io.borrow().http.clone();
        match mode {
            HttpMode::External(external) => {
                let token = request.token;
                let job = if call.streaming {
                    IoJob::HttpStream(request)
                } else {
                    IoJob::Http(request)
                };
                if !self.submit(&external, &route, job) {
                    self.queue_terminal(
                        token,
                        kind,
                        HttpFailure::Network,
                        "transport is busy or disconnected",
                    );
                }
            }
            HttpMode::Inline(client) if !call.external_only => {
                self.inline_http(&*client, &request, kind, &route)
            }
            _ => self.queue_terminal(
                request.token,
                kind,
                HttpFailure::Network,
                "HTTP transport is not linked",
            ),
        }
        true
    }

    fn abort(&self, ctx: &Ctx<'_>, token: u64) -> rquickjs::Result<()> {
        let resource = self.resources.borrow_mut().remove(token);
        let Some(resource) = resource else {
            return Ok(());
        };
        let kind = resource.kind;
        if !matches!(kind, Kind::Http | Kind::Stream { .. }) {
            self.resources.borrow_mut().insert(token, resource);
            return Ok(());
        }
        resource.route.cancel();
        let mode = self.io.borrow().http.clone();
        if let HttpMode::External(external) = mode {
            let _ = self.submit(&external, &resource.route, IoJob::Abort { token });
        }
        self.inbox.discard(token);
        self.local
            .borrow_mut()
            .retain(|event| event.event.id() != Some(token));
        crate::deliver::apply(
            &self.vm,
            ctx,
            failure(token, kind, HttpFailure::Canceled, "request canceled"),
            None,
        )
    }

    fn open_socket(&self, url: String, protocols: String) -> Option<u64> {
        if self.resources.borrow().sockets() >= self.limits.sockets {
            return None;
        }
        let id = next_id()?;
        let kind = Kind::Socket(SocketState::Connecting);
        let route = self.register(id, kind, None);
        let protocols = protocols
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        let socket = self.io.borrow().socket.clone();
        if !socket.is_some_and(|transport| {
            self.submit(&transport, &route, IoJob::SocketOpen { id, url, protocols })
        }) {
            self.queue_terminal(
                id,
                kind,
                HttpFailure::Network,
                "socket transport is not linked",
            );
        }
        Some(id)
    }

    fn send_socket(&self, id: u64, text: bool, data: Vec<u8>) -> bool {
        let route = {
            let resources = self.resources.borrow();
            let Some(resource) = resources.get(id) else {
                return false;
            };
            if !matches!(resource.kind, Kind::Socket(SocketState::Open)) {
                return false;
            }
            resource.route.clone().with_send_bytes(data.len())
        };
        if data.len()
            > self
                .limits
                .socket_send_bytes
                .saturating_sub(self.buffered_amount(id))
        {
            return false;
        }
        let socket = self.io.borrow().socket.clone();
        socket.is_some_and(|transport| {
            self.submit(&transport, &route, IoJob::SocketSend { id, text, data })
        })
    }

    fn close_socket(&self, id: u64) {
        let route = {
            let mut resources = self.resources.borrow_mut();
            let Some(resource) = resources.get(id) else {
                return;
            };
            if matches!(resource.kind, Kind::Socket(SocketState::Closing)) {
                return;
            }
            let route = resource.route.clone();
            resources.set_kind(
                id,
                Kind::Socket(SocketState::Closing),
                Some(self.deadline()),
            );
            route
        };
        let socket = self.io.borrow().socket.clone();
        if !socket
            .is_some_and(|transport| self.submit(&transport, &route, IoJob::SocketClose { id }))
        {
            self.local.borrow_mut().push_back(route.deliver(
                IoEvent::Socket {
                    id,
                    event: "close".into(),
                    text: true,
                    data: Vec::new(),
                },
                None,
            ));
        }
    }

    fn buffered_amount(&self, id: u64) -> usize {
        let routed = self
            .resources
            .borrow()
            .get(id)
            .map_or(0, |resource| resource.route.buffered_amount());
        self.jobs
            .borrow()
            .iter()
            .filter_map(|job| match job {
                IoJob::SocketSend {
                    id: socket, data, ..
                } if *socket == id => Some(data.len()),
                _ => None,
            })
            .fold(routed, usize::saturating_add)
    }
}

fn header_pairs(block: &str) -> Vec<(String, String)> {
    block
        .split('\n')
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.into(), value.trim().into()))
        })
        .collect()
}
