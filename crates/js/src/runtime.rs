//! Headless runtime. Standard bindings are installed before the prelude, and
//! the prelude is installed before any user entry. `eval_script` does not drain
//! jobs; [`Runtime::drive`] does.
//!
//! The UI adapter is not this type. It keeps the historical entry-before-I/O
//! order and calls the same [`crate::Vm`] and timer rule.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use rquickjs::{Ctx, FromJs, Function, Value};

use crate::LEGACY_PRELUDE;
use crate::quickjs::{JobStop, Timer, Vm, poll_timer, run_jobs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleTicket {
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
    modules: HashMap<String, String>,
    log: Log,
    closed: Cell<bool>,
}

type Log = Box<dyn Fn(u8, &str)>;

pub struct Runtime {
    state: Box<State>,
}

pub struct RuntimeBuilder {
    modules: HashMap<String, String>,
    log: Log,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            log: Box::new(|_, _| {}),
        }
    }

    pub fn log(mut self, log: impl Fn(u8, &str) + 'static) -> Self {
        self.log = Box::new(log);
        self
    }

    pub fn module(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        self.modules.insert(name.into(), source.into());
        self
    }

    pub fn build(self) -> Result<Runtime, JsError> {
        let vm = Vm::new().map_err(|message| JsError {
            message: message.into(),
        })?;
        let mut runtime = Runtime {
            state: Box::new(State {
                vm,
                timers: RefCell::new(Vec::new()),
                next_id: Cell::new(0),
                turn: Cell::new(0),
                mono: Cell::new(0),
                origin: Cell::new(None),
                modules: self.modules,
                log: self.log,
                closed: Cell::new(false),
            }),
        };
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

    /// Load one registered module. `scene` and `reactive` are not built in.
    ///
    /// The returned ticket is the state visible without waiting. This call does
    /// not block on a pending promise.
    pub fn start_module(&mut self, name: &str) -> Result<ModuleTicket, JsError> {
        self.ensure_open()?;
        if name == "scene" || name == "reactive" || name == "#reactive" {
            return Err(JsError {
                message: format!("module \"{name}\" is not installed"),
            });
        }
        let Some(source) = self.state.modules.get(name).cloned() else {
            return Err(JsError {
                message: format!("module \"{name}\" is not registered"),
            });
        };
        self.state.vm.with(|ctx| {
            rquickjs::Module::evaluate(ctx.clone(), name, source)
                .map_err(|error| script_error(&ctx, error))?;
            Ok(ModuleTicket::Fulfilled)
        })
    }

    pub fn drive(&mut self, now_ms: i64) -> Result<DriveReport, JsError> {
        self.ensure_open()?;
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
        Ok(DriveReport { timers_ran })
    }

    pub fn shutdown(&mut self) {
        if self.state.closed.replace(true) {
            return;
        }
        self.state.timers.borrow_mut().clear();
        self.state.vm.rejections.borrow_mut().clear();
        self.state.vm.close();
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
            let host =
                rquickjs::Object::new(ctx.clone()).map_err(|error| script_error(&ctx, error))?;
            ctx.globals()
                .set("__host", host)
                .map_err(|error| script_error(&ctx, error))?;
            install_console(&ctx, log)?;
            install_timers(&ctx, timers, next_id, turn, mono)?;
            install_now(&ctx, mono, origin)?;
            Ok(())
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type CoercedString = rquickjs::Coerced<String>;

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

#[allow(
    clippy::cast_possible_truncation,
    reason = "timer delays and ids arrive as JavaScript numbers and match the existing host conversion"
)]
fn f64_to_i64(value: f64) -> i64 {
    if value.is_finite() { value as i64 } else { 0 }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "timer ids arrive as JavaScript numbers and match the existing host conversion"
)]
fn f64_to_u64(value: f64) -> u64 {
    if value.is_finite() && value > 0.0 {
        value as u64
    } else {
        0
    }
}

fn install_console<'js>(ctx: &Ctx<'js>, log: *const Log) -> Result<(), JsError> {
    let console = rquickjs::Object::new(ctx.clone()).map_err(|error| script_error(ctx, error))?;
    for (name, level) in [("log", 0u8), ("info", 1), ("warn", 2), ("error", 3)] {
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: rquickjs::function::Rest<Value<'js>>| {
                let mut parts = Vec::new();
                for arg in args.0 {
                    if let Ok(text) = CoercedString::from_js(&ctx, arg) {
                        parts.push(text.0);
                    }
                }
                // SAFETY: `log` points at the boxed callback inside the runtime allocation.
                unsafe { (*log)(level, &parts.join(" ")) };
                rquickjs::Result::Ok(())
            },
        )
        .map_err(|error| script_error(ctx, error))?;
        console
            .set(name, function)
            .map_err(|error| script_error(ctx, error))?;
    }
    ctx.globals()
        .set("console", console)
        .map_err(|error| script_error(ctx, error))
}

fn install_now<'js>(
    ctx: &Ctx<'js>,
    mono: *const Cell<i64>,
    origin: *const Cell<Option<i64>>,
) -> Result<(), JsError> {
    let performance =
        rquickjs::Object::new(ctx.clone()).map_err(|error| script_error(ctx, error))?;
    let now = Function::new(ctx.clone(), move || {
        // SAFETY: both cells live in the runtime allocation that owns this function.
        let (mono, origin) = unsafe { ((*mono).get(), (*origin).get().unwrap_or(0)) };
        (mono - origin) as f64
    })
    .map_err(|error| script_error(ctx, error))?;
    performance
        .set("now", now)
        .map_err(|error| script_error(ctx, error))?;
    ctx.globals()
        .set("performance", performance)
        .map_err(|error| script_error(ctx, error))
}

fn install_timers<'js>(
    ctx: &Ctx<'js>,
    timers: *const RefCell<Vec<Timer>>,
    next_id: *const Cell<u64>,
    turn: *const Cell<u64>,
    mono: *const Cell<i64>,
) -> Result<(), JsError> {
    for (name, kind) in [("setTimeout", 0i32), ("setInterval", 1)] {
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  args: rquickjs::function::Rest<Value<'js>>|
                  -> rquickjs::Result<f64> {
                let Some(callback) = args.0.first().cloned() else {
                    return Err(rquickjs::Exception::throw_type(
                        &ctx,
                        "callback must be a function",
                    ));
                };
                if !callback.is_function() {
                    return Err(rquickjs::Exception::throw_type(
                        &ctx,
                        "callback must be a function",
                    ));
                }
                let delay = f64_to_i64(
                    args.0
                        .get(1)
                        .and_then(|value| value.as_number())
                        .unwrap_or(0.0),
                );
                let delay = delay.max(0);
                // SAFETY: timer cells live in the runtime allocation that owns this function.
                let id = unsafe {
                    let id = (*next_id).get().wrapping_add(1).max(1);
                    (*next_id).set(id);
                    (*timers).borrow_mut().push(Timer {
                        id,
                        due: (*mono).get().saturating_add(delay),
                        interval: if kind == 1 { delay } else { 0 },
                        eligible_turn: (*turn).get() + 1,
                        kind,
                        function: rquickjs::Persistent::save(&ctx, callback),
                        extra: Vec::new(),
                        active: true,
                    });
                    id
                };
                Ok(id as f64)
            },
        )
        .map_err(|error| script_error(ctx, error))?;
        ctx.globals()
            .set(name, function)
            .map_err(|error| script_error(ctx, error))?;
    }
    for (name, kind) in [("clearTimeout", 0i32), ("clearInterval", 1)] {
        let function = Function::new(
            ctx.clone(),
            move |args: rquickjs::function::Rest<Value<'_>>| {
                let id = f64_to_u64(
                    args.0
                        .first()
                        .and_then(|value| value.as_number())
                        .unwrap_or(0.0),
                );
                // SAFETY: the timer table lives in the runtime allocation that owns this function.
                unsafe {
                    (*timers)
                        .borrow_mut()
                        .retain(|timer| timer.id != id || timer.kind != kind);
                }
            },
        )
        .map_err(|error| script_error(ctx, error))?;
        ctx.globals()
            .set(name, function)
            .map_err(|error| script_error(ctx, error))?;
    }
    Ok(())
}
