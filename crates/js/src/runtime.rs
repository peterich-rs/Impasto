//! Headless runtime. Standard bindings are installed before the prelude, and
//! the prelude is installed before any user entry. `eval_script` does not drain
//! jobs; [`Runtime::drive`] does.
//!
//! The UI adapter is not this type. It keeps the historical entry-before-I/O
//! order and calls the same [`crate::Vm`] and timer rule.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use js_host::{HttpRequest, HttpResponse, IoEvent, IoJob};
use rquickjs::promise::PromiseState;
use rquickjs::{ArrayBuffer, Ctx, Function, IntoJs, Object, Persistent};

use crate::LEGACY_PRELUDE;
use crate::host::{HttpCall, IoPort};
use crate::modules::{MemoryLoader, MemoryResolver, ModuleTable, NativeFactory};
use crate::quickjs::{JobStop, Saved, Timer, Vm, poll_timer, run_jobs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    pub message: String,
}

/// A handle for one [`Runtime::start_module`] call.
///
/// The handle is only meaningful on the runtime that created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleTicket {
    runtime: u64,
    id: u64,
}

/// Evaluation state of a [`ModuleTicket`].
///
/// `Pending` means top-level await has not finished. [`Runtime::drive`] runs
/// the jobs that can move it to [`ModuleStatus::Fulfilled`] or
/// [`ModuleStatus::Rejected`]. This call does not wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleStatus {
    Pending,
    Fulfilled,
    Rejected,
}

enum TrackedModule {
    Pending(Saved),
    Fulfilled,
    Rejected,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveReport {
    pub timers_ran: u32,
}

struct State {
    vm: Vm,
    timers: RefCell<Vec<Timer>>,
    next_id: Cell<u64>,
    turn: Cell<u64>,
    mono: Cell<i64>,
    origin: Cell<Option<i64>>,
    identity: u64,
    modules: Rc<ModuleTable>,
    tickets: RefCell<HashMap<u64, TrackedModule>>,
    next_ticket: Cell<u64>,
    log: Log,
    jobs: RefCell<Vec<IoJob>>,
    inbox: RefCell<Vec<IoEvent>>,
    cancels: RefCell<HashMap<u64, Arc<AtomicBool>>>,
    next_socket: Cell<u64>,
    closed: Cell<bool>,
}

type Log = Box<dyn Fn(u8, &str)>;

pub struct Runtime {
    state: Box<State>,
}

pub struct RuntimeBuilder {
    modules: HashMap<String, String>,
    natives: HashMap<String, Box<dyn NativeFactory>>,
    duplicate: Option<String>,
    log: Log,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            natives: HashMap::new(),
            duplicate: None,
            log: Box::new(|_, _| {}),
        }
    }

    pub fn log(mut self, log: impl Fn(u8, &str) + 'static) -> Self {
        self.log = Box::new(log);
        self
    }

    pub fn module(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        let name = name.into();
        if self.modules.contains_key(&name) || self.natives.contains_key(&name) {
            self.duplicate = Some(name);
        } else {
            self.modules.insert(name, source.into());
        }
        self
    }

    /// Register a native module under `name`.
    ///
    /// The factory runs when that specifier is loaded. A second registration of
    /// the same name fails in [`RuntimeBuilder::build`].
    pub fn native(mut self, name: impl Into<String>, factory: Box<dyn NativeFactory>) -> Self {
        let name = name.into();
        if self.modules.contains_key(&name) || self.natives.contains_key(&name) {
            self.duplicate = Some(name);
        } else {
            self.natives.insert(name, factory);
        }
        self
    }

    pub fn build(self) -> Result<Runtime, JsError> {
        if let Some(name) = self.duplicate {
            return Err(JsError {
                message: format!("module \"{name}\" is already registered"),
            });
        }
        let vm = Vm::new().map_err(|message| JsError {
            message: message.into(),
        })?;
        let modules = Rc::new(ModuleTable::new(self.modules, self.natives));
        let mut runtime = Runtime {
            state: Box::new(State {
                vm,
                timers: RefCell::new(Vec::new()),
                next_id: Cell::new(0),
                turn: Cell::new(0),
                mono: Cell::new(0),
                origin: Cell::new(None),
                identity: next_runtime_id(),
                modules,
                tickets: RefCell::new(HashMap::new()),
                next_ticket: Cell::new(0),
                log: self.log,
                jobs: RefCell::new(Vec::new()),
                inbox: RefCell::new(Vec::new()),
                cancels: RefCell::new(HashMap::new()),
                next_socket: Cell::new(0),
                closed: Cell::new(false),
            }),
        };
        if let Some(quickjs) = runtime.state.vm.runtime() {
            quickjs.set_loader(
                MemoryResolver(Rc::clone(&runtime.state.modules)),
                MemoryLoader(Rc::clone(&runtime.state.modules)),
            );
        }
        // SAFETY: `state` is a Box. Moving `Runtime` does not move the `Vm`.
        unsafe { Vm::arm(&mut runtime.state.vm) };
        runtime.install_standard()?;
        runtime.eval_script(LEGACY_PRELUDE, "prelude.js")?;
        Ok(runtime)
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn eval_script(&mut self, source: &str, name: &str) -> Result<(), JsError> {
        self.ensure_open()?;
        self.state.vm.with(|ctx| {
            let mut options = rquickjs::context::EvalOptions::default();
            options.filename = Some(name.to_owned());
            ctx.eval_with_options::<(), _>(source, options)
                .map_err(|error| script_error(&ctx, error))
        })
    }

    /// Evaluate one registered module.
    ///
    /// A synchronous failure is returned here. Top-level await stays
    /// [`ModuleStatus::Pending`] until [`Runtime::drive`] runs its jobs.
    pub fn start_module(&mut self, name: &str) -> Result<ModuleTicket, JsError> {
        self.ensure_open()?;
        if !self.state.modules.contains(name) {
            return Err(JsError {
                message: format!("module \"{name}\" is not registered"),
            });
        }
        let source = self.state.modules.source(name).map(str::to_owned);
        let native = self.state.modules.is_native(name);
        let table = Rc::clone(&self.state.modules);
        let saved = self.state.vm.with(|ctx| {
            let promise = if let Some(source) = source {
                rquickjs::Module::evaluate(ctx.clone(), name, source)
                    .map_err(|error| script_error(&ctx, error))?
            } else if native {
                let module = crate::modules::declare_native(&table, &ctx, name)
                    .map_err(|error| script_error(&ctx, error))?;
                module.eval().map_err(|error| script_error(&ctx, error))?.1
            } else {
                return Err(JsError {
                    message: format!("module \"{name}\" is not registered"),
                });
            };
            Ok(Persistent::save(&ctx, promise.into_value()))
        })?;
        let id = self.state.next_ticket.get().wrapping_add(1).max(1);
        self.state.next_ticket.set(id);
        self.state
            .tickets
            .borrow_mut()
            .insert(id, TrackedModule::Pending(saved));
        let ticket = ModuleTicket {
            runtime: self.state.identity,
            id,
        };
        self.refresh_ticket(id);
        Ok(ticket)
    }

    pub fn module_status(&mut self, ticket: ModuleTicket) -> Result<ModuleStatus, JsError> {
        self.ensure_open()?;
        if ticket.runtime != self.state.identity {
            return Err(JsError {
                message: "module ticket belongs to another runtime".into(),
            });
        }
        self.refresh_ticket(ticket.id);
        match self.state.tickets.borrow().get(&ticket.id) {
            Some(TrackedModule::Pending(_)) => Ok(ModuleStatus::Pending),
            Some(TrackedModule::Fulfilled) => Ok(ModuleStatus::Fulfilled),
            Some(TrackedModule::Rejected) => Ok(ModuleStatus::Rejected),
            None => Err(JsError {
                message: "module ticket is not open".into(),
            }),
        }
    }

    pub fn take_jobs(&mut self) -> Vec<IoJob> {
        std::mem::take(&mut *self.state.jobs.borrow_mut())
    }

    /// Queue an owned completion. JavaScript runs on the next [`Runtime::drive`].
    pub fn enqueue(&mut self, event: IoEvent) -> Result<(), JsError> {
        self.ensure_open()?;
        self.state.inbox.borrow_mut().push(event);
        Ok(())
    }

    pub fn drive(&mut self, now_ms: i64) -> Result<DriveReport, JsError> {
        self.ensure_open()?;
        self.deliver_inbox()?;
        self.state.turn.set(self.state.turn.get() + 1);
        self.state.mono.set(now_ms);
        if self.state.origin.get().is_none() {
            self.state.origin.set(Some(now_ms));
        }
        let mut timers_ran = 0;
        for _ in 0..64 {
            let callback = {
                let mut timers = self.state.timers.borrow_mut();
                let Some(callback) = poll_timer(
                    &mut timers,
                    false,
                    self.state.turn.get(),
                    self.state.mono.get(),
                ) else {
                    break;
                };
                callback
            };
            timers_ran += 1;
            let failed = self.state.vm.with(|ctx| {
                let value = callback
                    .0
                    .restore(&ctx)
                    .map_err(|error| script_error(&ctx, error))?;
                let function =
                    Function::from_value(value).map_err(|error| script_error(&ctx, error))?;
                function
                    .call::<_, ()>(())
                    .map_err(|error| script_error(&ctx, error))?;
                Ok(())
            });
            failed?;
        }
        self.state.timers.borrow_mut().retain(|timer| timer.active);
        self.drain()?;
        self.refresh_modules();
        Ok(DriveReport { timers_ran })
    }

    pub fn shutdown(&mut self) {
        if self.state.closed.replace(true) {
            return;
        }
        self.state.timers.borrow_mut().clear();
        self.state.tickets.borrow_mut().clear();
        self.state.jobs.borrow_mut().clear();
        self.state.inbox.borrow_mut().clear();
        for flag in self.state.cancels.borrow().values() {
            flag.store(true, Ordering::Relaxed);
        }
        self.state.cancels.borrow_mut().clear();
        self.state.vm.rejections.borrow_mut().clear();
        self.state.vm.close();
    }

    fn deliver_inbox(&mut self) -> Result<(), JsError> {
        let events = std::mem::take(&mut *self.state.inbox.borrow_mut());
        if events.is_empty() {
            return Ok(());
        }
        self.state.vm.with(|ctx| {
            for event in events {
                deliver(&self.state.vm, &ctx, event).map_err(|error| script_error(&ctx, error))?;
            }
            Ok(())
        })
    }

    fn refresh_modules(&mut self) {
        let ids: Vec<u64> = self.state.tickets.borrow().keys().copied().collect();
        for id in ids {
            self.refresh_ticket(id);
        }
    }

    fn refresh_ticket(&mut self, id: u64) {
        let Some(saved) = (match self.state.tickets.borrow().get(&id) {
            Some(TrackedModule::Pending(saved)) => Some(saved.clone()),
            _ => None,
        }) else {
            return;
        };
        let status = self.state.vm.with(|ctx| {
            let value = saved.clone().restore(&ctx).ok()?;
            let promise = value.as_promise()?;
            Some(match promise.state() {
                PromiseState::Pending => ModuleStatus::Pending,
                PromiseState::Resolved => ModuleStatus::Fulfilled,
                PromiseState::Rejected => ModuleStatus::Rejected,
            })
        });
        if let Some(status) = status {
            let mut tickets = self.state.tickets.borrow_mut();
            if let Some(tracked) = tickets.get_mut(&id) {
                *tracked = match status {
                    ModuleStatus::Pending => TrackedModule::Pending(saved),
                    ModuleStatus::Fulfilled => TrackedModule::Fulfilled,
                    ModuleStatus::Rejected => TrackedModule::Rejected,
                };
            }
        }
    }

    fn ensure_open(&self) -> Result<(), JsError> {
        if self.state.closed.get() || !self.state.vm.is_open() {
            Err(JsError {
                message: "runtime is closed".into(),
            })
        } else {
            Ok(())
        }
    }

    fn drain(&mut self) -> Result<(), JsError> {
        let Some(runtime) = self.state.vm.runtime() else {
            return Err(JsError {
                message: "runtime is closed".into(),
            });
        };
        match run_jobs(runtime, |ctx| {
            let _ = ctx.catch();
        }) {
            Ok(()) => Ok(()),
            Err(JobStop::Limit) => Err(JsError {
                message: self.state.vm.annotate("quickjs job round limit".into()),
            }),
            Err(JobStop::Failed) => Err(JsError {
                message: "job failed".into(),
            }),
        }
    }

    fn install_standard(&mut self) -> Result<(), JsError> {
        let timers = &self.state.timers as *const RefCell<Vec<Timer>>;
        let next_id = &self.state.next_id as *const Cell<u64>;
        let turn = &self.state.turn as *const Cell<u64>;
        let mono = &self.state.mono as *const Cell<i64>;
        let origin = &self.state.origin as *const Cell<Option<i64>>;
        let log = &self.state.log as *const Log;
        self.state.vm.with(|ctx| {
            let vm = &self.state.vm as *const Vm as *mut Vm;
            let state: &State = &self.state;
            let port: *const dyn IoPort = std::ptr::from_ref(state as &dyn IoPort);
            // SAFETY: `vm` and `port` address this runtime's boxed state.
            unsafe { crate::host::install(&ctx, vm, port) }
                .map_err(|error| script_error(&ctx, error))?;
            crate::standard::install_console(&ctx, move |level, text| {
                // SAFETY: `log` points at the callback stored in this runtime.
                unsafe { (*log)(level, text) };
            })
            .map_err(|error| script_error(&ctx, error))?;
            // SAFETY: the timer cells address this runtime's boxed state.
            unsafe { crate::standard::install_timers(&ctx, timers, next_id, turn, mono, || {}) }
                .map_err(|error| script_error(&ctx, error))?;
            crate::standard::install_performance(&ctx, move || {
                // SAFETY: the clock cells live in this runtime allocation.
                let (mono, origin) = unsafe { ((*mono).get(), (*origin).get().unwrap_or(0)) };
                (mono - origin) as f64
            })
            .map_err(|error| script_error(&ctx, error))?;
            Ok(())
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl IoPort for State {
    fn http(&self, call: HttpCall) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels
            .borrow_mut()
            .insert(call.token, Arc::clone(&cancel));
        let request = HttpRequest {
            token: call.token,
            method: call.method,
            url: call.url,
            headers: header_pairs(&call.headers),
            body: call.body,
            cancel,
            timeout_ms: None,
        };
        self.jobs.borrow_mut().push(if call.streaming {
            IoJob::HttpStream(request)
        } else {
            IoJob::Http(request)
        });
    }

    fn abort(&self, token: u64) {
        if let Some(flag) = self.cancels.borrow().get(&token) {
            flag.store(true, Ordering::Relaxed);
        }
        self.jobs.borrow_mut().push(IoJob::Abort { token });
    }

    fn has_module(&self, name: &str) -> bool {
        self.modules.contains(name)
    }

    fn open_socket(&self, url: String, protocols: String) -> u64 {
        let id = self.next_socket.get().wrapping_add(1).max(1);
        self.next_socket.set(id);
        let protocols = protocols
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        self.jobs
            .borrow_mut()
            .push(IoJob::SocketOpen { id, url, protocols });
        id
    }

    fn send_socket(&self, id: u64, text: bool, data: Vec<u8>) {
        self.jobs
            .borrow_mut()
            .push(IoJob::SocketSend { id, text, data });
    }

    fn close_socket(&self, id: u64) {
        self.jobs.borrow_mut().push(IoJob::SocketClose { id });
    }
}

fn header_block(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(
    clippy::cast_precision_loss,
    reason = "host tokens are finite ids and fit in a JavaScript number"
)]
fn js_number(id: u64) -> f64 {
    id as f64
}

fn header_pairs(block: &str) -> Vec<(String, String)> {
    block
        .split('\n')
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            let name = name.trim();
            if name.is_empty() {
                None
            } else {
                Some((name.to_owned(), value.trim().to_owned()))
            }
        })
        .collect()
}

fn deliver(vm: &Vm, ctx: &Ctx<'_>, event: IoEvent) -> rquickjs::Result<()> {
    match event {
        IoEvent::Http { token, result } => match result {
            Ok(response) => settle_http(vm, ctx, token, &response),
            Err(error) => vm.settle(ctx, token, false, error.dom_name().into_js(ctx)?),
        },
        IoEvent::Plugin { token, result } => match result {
            js_host::PluginResult::Done(value) => {
                vm.settle(ctx, token, true, plugin_value(ctx, &value)?)
            }
            js_host::PluginResult::Failed { message } => {
                vm.settle(ctx, token, false, message.into_js(ctx)?)
            }
        },
        IoEvent::Socket {
            id,
            event,
            text,
            data,
        } => {
            let payload = if text {
                String::from_utf8_lossy(&data).into_owned().into_js(ctx)?
            } else {
                ArrayBuffer::new(ctx.clone(), data)?.into_js(ctx)?
            };
            call_hook(
                ctx,
                "_socket",
                vec![js_number(id).into_js(ctx)?, event.into_js(ctx)?, payload],
            )
        }
        IoEvent::HttpHead {
            token,
            status,
            headers,
        } => {
            let head = Object::new(ctx.clone())?;
            head.set("status", i32::from(status))?;
            head.set("headers", header_block(&headers))?;
            call_hook(
                ctx,
                "_httpStream",
                vec![
                    js_number(token).into_js(ctx)?,
                    "head".into_js(ctx)?,
                    head.into_value(),
                ],
            )
        }
        IoEvent::HttpChunk {
            token,
            bytes,
            done,
            error,
        } => {
            let payload = match error {
                Some(error) => error.dom_name().into_js(ctx)?,
                None => ArrayBuffer::new(ctx.clone(), bytes)?.into_value(),
            };
            call_hook(
                ctx,
                "_httpStream",
                vec![
                    js_number(token).into_js(ctx)?,
                    (if done { "done" } else { "chunk" }).into_js(ctx)?,
                    payload,
                ],
            )?;
            if done {
                vm.promises.borrow_mut().remove(&token);
            }
            Ok(())
        }
    }
}

fn settle_http(
    vm: &Vm,
    ctx: &Ctx<'_>,
    token: u64,
    response: &HttpResponse,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    object.set("status", i32::from(response.status))?;
    object.set("headers", header_block(&response.headers))?;
    object.set(
        "body",
        ArrayBuffer::new(ctx.clone(), response.body.clone())?,
    )?;
    vm.settle(ctx, token, true, object.into_value())
}

fn plugin_value<'js>(
    ctx: &Ctx<'js>,
    value: &js_host::PluginValue,
) -> rquickjs::Result<rquickjs::Value<'js>> {
    use js_host::PluginValue;
    Ok(match value {
        PluginValue::Null => rquickjs::Value::new_null(ctx.clone()),
        PluginValue::Bool(value) => (*value).into_js(ctx)?,
        PluginValue::F64(value) => (*value).into_js(ctx)?,
        PluginValue::Str(value) => value.into_js(ctx)?,
        PluginValue::Bytes(value) => ArrayBuffer::new(ctx.clone(), value.clone())?.into_js(ctx)?,
        PluginValue::List(_) | PluginValue::Map(_) => rquickjs::Value::new_null(ctx.clone()),
    })
}

fn call_hook<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    args: Vec<rquickjs::Value<'js>>,
) -> rquickjs::Result<()> {
    let bridge: Object = ctx.globals().get("__host")?;
    let hook: rquickjs::Value = bridge.get(name)?;
    if let Some(hook) = hook.into_function() {
        let mut values = rquickjs::function::Args::new(ctx.clone(), args.len());
        values.push_args(args)?;
        hook.call_arg::<()>(values)?;
    }
    Ok(())
}

fn next_runtime_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn script_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> JsError {
    let message = if error.is_exception() {
        let value = ctx.catch();
        value
            .as_object()
            .and_then(|object| object.get::<_, rquickjs::Coerced<String>>("message").ok())
            .map(|text| text.0)
            .unwrap_or_else(|| "quickjs error".into())
    } else {
        error.to_string()
    };
    JsError { message }
}
