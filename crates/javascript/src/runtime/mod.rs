//! Owner-thread execution and the runtime's owned host boundary.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::host::{
    CreditPool, CreditUsage, Delivery, HostEvent, IoEvent, NativeResource, PluginValue, RequestKey,
    Route, RuntimeKey, Settle, StreamSink,
};
use rquickjs::promise::PromiseState;
use rquickjs::{Ctx, Exception, Object, Persistent};

use crate::runtime::resources::{Inbox, Kind, Resource, Resources, failure, next_id};
use modules::ModuleTable;
use quickjs::binder::{self, ModuleBindings, ModuleRuntime};
use quickjs::{JobReport, JobStop, Vm, poll_timer, run_job_batch};
use scheduler::Scheduler;
mod builder;
mod catalog;
mod delivery;
pub(crate) mod modules;
pub(crate) mod quickjs;
pub(crate) mod resources;
pub(crate) mod scheduler;

pub use builder::RuntimeBuilder;
pub use catalog::{CatalogError, ModuleCatalog};
pub use modules::NativeFactory;
pub use quickjs::{ExtensionContext, JsDiagnostic, Saved, StackSample, TrustedBytecode};
pub use scheduler::{DrivePolicy, RuntimeLimits};

/// QuickJS capabilities available to trusted native extensions.
pub mod extension {
    pub use super::quickjs::binder::{ModuleBindings, declare_host_module};
    pub use super::quickjs::buffer::array_buffer;
    pub use super::quickjs::{Budget, JobStop, Timer, exception_text, poll_timer};
    pub use crate::compiler::bytecode::is_module;
    pub use crate::standard::install_web;
}

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
    pub queued_events: usize,
    pub queued_bytes: usize,
    pub peak_promises: usize,
    pub peak_timers: usize,
    pub peak_listeners: usize,
    pub peak_tickets: usize,
    pub peak_rejections: usize,
    pub peak_requests: usize,
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
pub type TransportClock = Arc<dyn Fn() -> i64 + Send + Sync>;

struct State {
    vm: Vm,
    scheduler: Scheduler,
    transport_clock: Option<TransportClock>,
    key: RuntimeKey,
    phase: Cell<Phase>,
    life: RefCell<Option<Rc<()>>>,
    limits: RuntimeLimits,
    drive_policy: DrivePolicy,
    modules: Rc<ModuleTable>,
    tickets: RefCell<HashMap<u64, TrackedModule>>,
    listeners: RefCell<Vec<(String, String, Saved)>>,
    ticket_peak: Cell<usize>,
    listener_peak: Cell<usize>,
    log: Log,
    user_timing: Option<UserTimingHook>,
    inbox: Arc<Inbox>,
    local: RefCell<VecDeque<Delivery>>,
    resources: RefCell<Resources>,
    native_resources: Rc<quickjs::native_resource::ResourceTable>,
    ingress: Arc<CreditPool>,
    outbound: Arc<CreditPool>,
}

/// A User Timing entry expressed on the process monotonic clock.
pub struct UserTiming<'a> {
    pub name: &'a str,
    pub kind: UserTimingKind,
    pub start: Instant,
    pub end: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserTimingKind {
    Mark,
    Measure,
}

pub(crate) type UserTimingHook = Rc<dyn Fn(UserTiming<'_>)>;

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

    /// Drains stack samples from completed VM turns on the owner thread.
    pub fn take_stack_samples(&self) -> Vec<StackSample> {
        self.state.vm.take_stack_samples()
    }

    pub fn key(&self) -> RuntimeKey {
        self.state.key
    }

    pub fn extension(&self) -> ExtensionContext<'_> {
        ExtensionContext::new(&self.state.vm)
    }

    pub fn finish_installation(&self) {
        if self.state.phase.get() == Phase::Installing {
            self.state.phase.set(Phase::Ready);
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

    pub fn heap_bytes(&self) -> Option<usize> {
        if self.state.phase.get() == Phase::Closed {
            return None;
        }
        self.state.vm.heap_bytes()
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
            queued_events: state.inbox.len() + state.local.borrow().len(),
            queued_bytes: bytes,
            peak_promises: state.vm.promise_peak.get(),
            peak_timers: state.scheduler.peak.get(),
            peak_tickets: state.ticket_peak.get(),
            peak_listeners: state.listener_peak.get(),
            peak_rejections: state.vm.rejection_peak.get(),
            peak_requests: resources.peak,
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
        compiled: &crate::compiler::bytecode::CompiledBytecode,
    ) -> Result<(), JsError> {
        self.state.ensure_open()?;
        if compiled.kind() != crate::compiler::bytecode::Kind::Script {
            return Err(JsError::new(
                ErrorKind::Configuration,
                "expected compiled script",
            ));
        }
        let _budget = self.state.vm.budget("eval");
        let entry = TrustedBytecode::from_compiled(compiled);
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
                modules::declare_native(&state.modules, &ctx, name)
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
        compiled: &crate::compiler::bytecode::CompiledBytecode,
        name: &str,
    ) -> Result<ModuleTicket, JsError> {
        self.state.ensure_open()?;
        if compiled.kind() != crate::compiler::bytecode::Kind::Module {
            return Err(JsError::new(
                ErrorKind::Configuration,
                "expected compiled module",
            ));
        }
        let id = self.ticket_id()?;
        let _budget = self.state.vm.budget("eval");
        let entry = TrustedBytecode::from_compiled(compiled);
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
        if self.state.scheduler.set_clock(now_ms) && self.state.transport_clock.is_none() {
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
            in_flight: resources.requests(),
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
            if monotonic_ms() > state.vm.deadline.get() {
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
        let reserved_terminal = delivery.event.is_terminal()
            && known.is_some()
            && bytes <= crate::host::TERMINAL_PAYLOAD_LIMIT;
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
            if let Err(error) = delivery::apply(
                &state.vm,
                &ctx,
                delivery.event,
                delivery.credit,
                &state.native_resources,
                state.limits.requests,
            ) {
                let diagnostic = state.script_error(&ctx, error);
                if let Some(id) = id {
                    let _ = delivery::reject_bad_completion(&state.vm, &ctx, id);
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

    /// Frame delivery preserves module completion ordering before stream items.
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
        on_zero_delay: impl Fn() + Clone + 'static,
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
        crate::standard::install_user_timing(
            ctx,
            move || {
                // SAFETY: the context and these callbacks are owned by boxed state.
                unsafe { &*state }.scheduler.now()
            },
            self.state.user_timing.clone(),
        )
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
            self.state.native_resources.close();
            let _budget = self.state.vm.budget("shutdown");
            self.state.vm.with(|ctx| {
                if let Err(error) = crate::standard::native_stream::close(&ctx) {
                    let diagnostic = self.state.script_error(&ctx, error);
                    (self.state.log)(3, &diagnostic.message);
                }
            });
        }
    }

    pub fn shutdown(&mut self) {
        if self.state.phase.get() == Phase::Closed {
            return;
        }
        self.begin_shutdown();
        self.state.local.borrow_mut().clear();
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
    fn enter<'a>(&'a self, ctx: &Ctx<'_>) -> Option<quickjs::Active<'a>> {
        self.state().map(|state| state.vm.enter(ctx))
    }

    fn before(&self, ctx: &Ctx<'_>, removing_listener: bool) -> rquickjs::Result<()> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        if !(state.is_open() || removing_listener && state.phase.get() == Phase::Closing) {
            return Err(Exception::throw_type(ctx, "runtime is closing"));
        }
        (self.policy)(ctx)
    }

    fn reserve_plugin<'js>(
        &self,
        ctx: &Ctx<'js>,
        awaits_user: bool,
    ) -> rquickjs::Result<(rquickjs::Value<'js>, Settle)> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        if state.resources.borrow().requests() >= state.limits.requests {
            return Err(Exception::throw_range(ctx, "request limit exceeded"));
        }
        let (id, promise) = state.vm.make_promise(ctx)?;
        let deadline = (!awaits_user).then(|| state.deadline());
        let route = state.register(id, Kind::Plugin, None, deadline);
        let cancel = route.cancellation();
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
                    event = failure(id, Kind::Plugin, "host completion exceeds queue limit");
                }
                let delivery = route.deliver(event, credit);
                if let Some(post) = post {
                    post(delivery);
                } else {
                    let _ = inbox.push(delivery);
                }
            })
        };
        Ok((promise, settle.with_cancellation(cancel)))
    }

    fn open_resource<'js>(
        &self,
        ctx: &Ctx<'js>,
        create: &mut dyn FnMut() -> Result<Box<dyn NativeResource>, String>,
    ) -> rquickjs::Result<rquickjs::Value<'js>> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        state
            .native_resources
            .open(ctx, state.limits.requests, create)
    }

    fn reserve_stream<'js>(
        &self,
        ctx: &Ctx<'js>,
        signal: Option<Object<'js>>,
    ) -> rquickjs::Result<(rquickjs::Value<'js>, StreamSink)> {
        let state = self
            .state()
            .ok_or_else(|| Exception::throw_type(ctx, "runtime is closed"))?;
        let canceled = state.resources.borrow_mut().reap_canceled_streams();
        crate::standard::native_stream::reap(ctx, canceled);
        if state.resources.borrow().requests() >= state.limits.requests {
            return Err(Exception::throw_range(ctx, "request limit exceeded"));
        }
        let id = next_id().ok_or_else(|| Exception::throw_range(ctx, "stream IDs exhausted"))?;
        let route = state.register(id, Kind::NativeStream, None, None);
        let stream = match crate::standard::native_stream::create(ctx, route.clone(), signal) {
            Ok(stream) => stream,
            Err(error) => {
                route.cancel();
                state.resources.borrow_mut().remove(id);
                return Err(error);
            }
        };
        let inbox = Arc::clone(&state.inbox);
        let post = self.post.clone();
        let sink = StreamSink::new(route, move |delivery| {
            if let Some(post) = &post {
                post(delivery);
            } else {
                let _ = inbox.push(delivery);
            }
        });
        Ok((stream, sink))
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
            quickjs::exception_text(ctx, ctx.catch())
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
                    let (message, _) = quickjs::exception_text(&ctx, reason);
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

    fn transport_now(&self) -> i64 {
        self.transport_clock
            .as_ref()
            .map_or_else(|| self.scheduler.mono.get(), |clock| clock())
    }

    fn deadline(&self) -> i64 {
        self.transport_now()
            .saturating_add(i64::try_from(self.limits.request_timeout_ms).unwrap_or(i64::MAX))
    }

    fn register(
        &self,
        id: u64,
        kind: Kind,
        cancel: Option<Arc<AtomicBool>>,
        deadline: Option<i64>,
    ) -> Route {
        let route = self.route(
            id,
            cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
            matches!(kind, Kind::NativeStream),
        );
        self.resources.borrow_mut().insert(
            id,
            Resource {
                kind,
                route: route.clone(),
                deadline,
            },
        );
        route
    }

    fn expire(&self) -> Vec<Delivery> {
        let expired = self.resources.borrow().expired(self.transport_now());
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
                event: failure(id, resource.0, "transport deadline expired"),
                credit: None,
            });
        }
        events
    }
}

/// Milliseconds from a monotonic clock. Tests must not freeze this with a manual clock.
pub fn monotonic_ms() -> i64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes the local timespec and keeps no pointer.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return 0;
    }
    time.tv_sec
        .saturating_mul(1000)
        .saturating_add(time.tv_nsec / 1_000_000)
}
