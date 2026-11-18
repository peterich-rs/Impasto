//! The single render-thread VM and the references it owns.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::ptr;

use rquickjs::function::Args;
use rquickjs::{Context, Ctx, Function, IntoJs, JsLifetime, Object, Persistent, Runtime, Value};

use super::{Inner, bootstrap, reactive};
use crate::{Error, bytecode};

mod bindings;
mod canvas;
mod drivers;
mod modules;
mod renderer;
mod turns;
pub(super) use canvas::{
    CanvasSession, CanvasStats, begin_shutdown, deliver_errors, finish_shutdown, paint,
};
pub(crate) use modules::prepare_compile;
pub(super) use turns::*;

type Saved = Persistent<Value<'static>>;

#[derive(JsLifetime)]
struct HostPtr(*mut Host);

#[derive(Default)]
struct References {
    scene: Option<Saved>,
    reactive: Option<Saved>,
    callbacks: HashMap<(u64, i32), Saved>,
    hooks: HashMap<&'static str, Saved>,
    backs: Vec<(u64, Saved)>,
    shortcuts: Vec<(String, Saved)>,
    settings: Vec<(u64, Saved)>,
    motions: Vec<(u64, u64, String, Saved)>,
    listeners: Vec<(String, String, Saved)>,
    promises: HashMap<u64, (Saved, Saved)>,
    values: Vec<Saved>,
    rejections: Vec<(Saved, Saved)>,
}

struct Timer {
    id: u64,
    due: i64,
    interval: i64,
    min_frame: u64,
    kind: i32,
    function: Saved,
    extra: Vec<Saved>,
    active: bool,
}

pub(in crate::linked) struct Host {
    context: Option<Context>,
    runtime: Option<Runtime>,
    inner: *mut Inner,
    /// A borrowed Ctx only while its runtime lock is held by a synchronous entry.
    active: Cell<*const ()>,
    refs: RefCell<References>,
    timers: RefCell<Vec<Timer>>,
    next_id: Cell<u64>,
    token: Cell<u64>,
    socket: Cell<u64>,
    mono: Cell<i64>,
    origin: Cell<Option<i64>>,
    frame: Cell<u64>,
    paused: Cell<bool>,
    wants_frame: Cell<bool>,
    freeze: Cell<bool>,
    reactive_failed: Cell<bool>,
    deadline: Cell<i64>,
    entry: Cell<(&'static str, u64)>,
    error: RefCell<(String, String)>,
}

struct Active<'a> {
    host: &'a Host,
    previous: *const (),
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.host.active.set(self.previous);
    }
}
struct Budget<'a> {
    host: &'a Host,
    previous_entry: (&'static str, u64),
    previous_deadline: i64,
}
impl Drop for Budget<'_> {
    fn drop(&mut self) {
        self.host.entry.set(self.previous_entry);
        self.host.deadline.set(self.previous_deadline);
    }
}

impl Host {
    pub(super) fn discard_if_unattached(&self, id: uicore::NodeId) {
        self.inner().tree.borrow_mut().discard_if_unattached(id);
    }

    fn inner(&self) -> &Inner {
        // SAFETY: Engine keeps Inner allocated until Context and Runtime have been dropped.
        unsafe { &*self.inner }
    }
    fn session(&self) -> *mut c_void {
        self.inner.cast()
    }
    fn enter<'js>(&self, ctx: &Ctx<'js>) -> Active<'_> {
        let previous = self.active.replace(ptr::from_ref(ctx).cast());
        Active {
            host: self,
            previous,
        }
    }
    pub(super) fn with<R>(&self, body: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        if !self.active.get().is_null() {
            // SAFETY: enter installs a borrowed Ctx only for the duration of its
            // synchronous runtime lock. The HRTB prevents values escaping that scope.
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
    fn budget(&self, kind: &'static str, id: u64) -> Budget<'_> {
        let previous_entry = self.entry.replace((kind, id));
        let previous_deadline = self
            .deadline
            .replace(monotonic_ms().saturating_add(if kind == "eval" { 1000 } else { 128 }));
        Budget {
            host: self,
            previous_entry,
            previous_deadline,
        }
    }
    /// Replace the interrupt label without restarting the turn's deadline.
    fn retarget(&self, kind: &'static str, id: u64) {
        self.entry.set((kind, id));
    }
    fn past_deadline(&self) -> bool {
        monotonic_ms() > self.deadline.get()
    }
    fn alloc_id(&self) -> u64 {
        let id = self.next_id.get().wrapping_add(1).max(1);
        self.next_id.set(id);
        id
    }
    fn annotate(&self, message: String) -> String {
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
    fn stash_value<'js>(&self, ctx: &Ctx<'js>, value: Value<'js>) {
        let (message, stack) = exception_text(ctx, value);
        *self.error.borrow_mut() = (self.annotate(message), stack);
    }
    fn stash(&self, ctx: &Ctx<'_>, error: rquickjs::Error) {
        if error.is_exception() {
            self.stash_value(ctx, ctx.catch());
        } else {
            *self.error.borrow_mut() = (self.annotate(error.to_string()), String::new());
        }
    }
    pub(super) fn take_error(&self) -> (String, String) {
        std::mem::take(&mut *self.error.borrow_mut())
    }
    fn report(&self, source: &'static str) {
        let (message, stack) = self.error.borrow().clone();
        self.inner().faults.borrow_mut().push(session::ScriptFault {
            source,
            message: message.clone(),
            stack: stack.clone(),
        });
        (self.inner().log)(3, &super::display_fault(&message, &stack));
    }
    fn finish(
        &self,
        result: rquickjs::Result<()>,
        ctx: &Ctx<'_>,
        out: *mut c_char,
        len: usize,
    ) -> i32 {
        match result {
            Ok(()) => 0,
            Err(error) => {
                self.stash(ctx, error);
                self.write_error(out, len);
                -1
            }
        }
    }
    fn write_error(&self, out: *mut c_char, len: usize) {
        if !out.is_null() && len > 0 {
            let error = self.error.borrow();
            let bytes = error.0.as_bytes();
            let count = bytes.len().min(len - 1);
            // SAFETY: session passes its writable error buffer with this length.
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), count);
                *out.add(count) = 0;
            }
        }
    }
    fn call<'js>(
        &self,
        ctx: &Ctx<'js>,
        saved: &Saved,
        values: Vec<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let func = Function::from_value(saved.clone().restore(ctx)?)?;
        let mut args = Args::new(ctx.clone(), values.len());
        args.push_args(values)?;
        func.call_arg(args)
    }
    fn api<'js>(
        &self,
        ctx: &Ctx<'js>,
        reactive: bool,
        name: &str,
        args: Vec<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let saved = {
            let refs = self.refs.borrow();
            if reactive {
                refs.reactive.clone()
            } else {
                refs.scene.clone()
            }
        };
        let api = Object::from_value(
            saved
                .ok_or_else(|| type_error(ctx, "API is not installed"))?
                .restore(ctx)?,
        )?;
        let func: Function = api.get(name)?;
        let mut values = Args::new(ctx.clone(), args.len());
        values.push_args(args)?;
        func.call_arg(values)
    }
    pub(super) fn same_saved(&self, a: &Saved, b: &Saved) -> bool {
        self.with(|ctx| {
            let (Ok(a), Ok(b)) = (a.clone().restore(&ctx), b.clone().restore(&ctx)) else {
                return false;
            };
            strict_equal(&ctx, &a, &b)
        })
    }
    pub(super) fn reactive_error(&self, ctx: &Ctx<'_>, owner: u64) {
        let error = ctx.catch();
        let handled = self
            .api(
                ctx,
                true,
                "handleError",
                vec![error.clone(), Value::new_float(ctx.clone(), owner as f64)],
            )
            .and_then(|value| rquickjs::Coerced::<bool>::from_js(ctx, value))
            .map(|v| v.0)
            .unwrap_or(false);
        if !handled {
            // Preserve the computation error when its error handler throws.
            if ctx.has_exception() {
                drop(ctx.catch());
            }
            self.stash_value(ctx, error);
            self.report("reactive");
            self.reactive_failed.set(true);
        }
    }
    fn listener_result(
        &self,
        ctx: &Ctx<'_>,
        result: rquickjs::Result<Value<'_>>,
        out: *mut c_char,
        len: usize,
    ) -> i32 {
        match result {
            Ok(_) => 0,
            Err(error) => {
                self.stash(ctx, error);
                if self.freeze.get() {
                    self.write_error(out, len);
                    -1
                } else {
                    self.report("listener");
                    self.take_error();
                    0
                }
            }
        }
    }
    fn eval(&self, source: &[u8], name: &str, module: bool) -> Result<(), Error> {
        let _budget = self.budget("eval", 0);
        let promise = self.with(|ctx| {
            let result = if module {
                rquickjs::Module::evaluate(ctx.clone(), name, source)
                    .map(|p| Some(Persistent::save(&ctx, p)))
            } else {
                let mut options = rquickjs::context::EvalOptions::default();
                options.filename = Some(name.to_owned());
                ctx.eval_with_options::<(), _>(source, options)
                    .map(|()| None)
            };
            result.map_err(|error| {
                self.stash(&ctx, error);
                self.script_error()
            })
        })?;
        self.drain()?;
        if let Some(promise) = promise {
            self.with(|ctx| {
                let promise = promise.restore(&ctx).map_err(|_| Error::Runtime)?;
                if let Some(Err(error)) = promise.result::<()>() {
                    self.stash(&ctx, error);
                    return Err(self.script_error());
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    fn script_error(&self) -> Error {
        let (message, stack) = self.error.borrow().clone();
        Error::Script(super::display_fault(&message, &stack))
    }
    fn drain(&self) -> Result<(), Error> {
        let _budget = self.budget("jobs", 0);
        let Some(runtime) = &self.runtime else {
            return Err(Error::Runtime);
        };
        let mut rounds = 0;
        while runtime.is_job_pending() {
            if rounds >= 32 {
                *self.error.borrow_mut() = (
                    self.annotate("quickjs job round limit".to_owned()),
                    String::new(),
                );
                return Err(self.script_error());
            }
            match runtime.execute_pending_job() {
                Ok(false) => break,
                Ok(true) => rounds += 1,
                Err(error) => {
                    error
                        .0
                        .with(|ctx| self.stash(&ctx, rquickjs::Error::Exception));
                    return Err(self.script_error());
                }
            }
        }
        self.with(|ctx| {
            let rejections = std::mem::take(&mut self.refs.borrow_mut().rejections);
            for (_, reason) in rejections {
                if let Ok(reason) = reason.restore(&ctx) {
                    let (message, stack) = exception_text(&ctx, reason);
                    let message = format!(
                        "unhandled rejection: {}",
                        super::display_fault(&message, &stack)
                    );
                    self.inner().faults.borrow_mut().push(session::ScriptFault {
                        source: "rejection",
                        message: message.clone(),
                        stack,
                    });
                    if let Ok(hook) = ctx.globals().get::<_, Function>("__reportUnhandled") {
                        let _ = hook.call::<_, ()>((message.clone(),));
                        if ctx.has_exception() {
                            drop(ctx.catch());
                        }
                    }
                    (self.inner().log)(3, &message);
                }
            }
        });
        Ok(())
    }
}

fn strict_equal<'js>(ctx: &Ctx<'js>, a: &Value<'js>, b: &Value<'js>) -> bool {
    // SAFETY: both values belong to the live context and are borrowed for this call.
    // The native === does not run JavaScript, so an expired budget cannot turn
    // an unchanged value into a change during the reactive flush.
    unsafe { rquickjs::qjs::JS_IsStrictEqual(ctx.as_raw().as_ptr(), a.as_raw(), b.as_raw()) }
}

use rquickjs::FromJs;
fn exception_text<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> (String, String) {
    let stack = value
        .as_object()
        .and_then(|v| v.get::<_, rquickjs::Coerced<String>>("stack").ok())
        .map(|v| v.0)
        .unwrap_or_default();
    let message = rquickjs::Coerced::<String>::from_js(ctx, value)
        .map(|s| s.0)
        .unwrap_or_else(|_| "quickjs error".to_owned());
    (message, stack)
}
fn type_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    rquickjs::Exception::throw_type(ctx, message)
}
fn monotonic_ms() -> i64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime initializes the writable timespec and retains no pointer.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return 0;
    }
    time.tv_sec
        .saturating_mul(1000)
        .saturating_add(time.tv_nsec / 1_000_000)
}

/// Source text, or bytecode the caller has already promised came from `compile`.
pub(in crate::linked) enum ProgramKind<'a> {
    Source(&'a [u8]),
    Trusted(bytecode::TrustedBytecode<'a>),
}

pub(in crate::linked) struct Engine {
    host: Box<Host>,
    /// Kept alive through driver finalizers, including those run by Runtime::drop.
    _inner: Box<Inner>,
}
impl Engine {
    pub(super) fn new(mut inner: Box<Inner>, app_dir: Option<&str>) -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(|_| Error::Runtime)?;
        runtime.set_memory_limit(16 << 20);
        runtime.set_max_stack_size(8 << 20);
        let context = Context::full(&runtime).map_err(|_| Error::Runtime)?;
        let mut host = Box::new(Host {
            context: Some(context),
            runtime: Some(runtime),
            inner: &mut *inner,
            active: Cell::new(ptr::null()),
            refs: RefCell::new(References::default()),
            timers: RefCell::new(Vec::new()),
            next_id: Cell::new(0),
            token: Cell::new(0),
            socket: Cell::new(0),
            mono: Cell::new(0),
            origin: Cell::new(None),
            frame: Cell::new(0),
            paused: Cell::new(false),
            wants_frame: Cell::new(false),
            freeze: Cell::new(false),
            reactive_failed: Cell::new(false),
            deadline: Cell::new(i64::MAX),
            entry: Cell::new(("", 0)),
            error: RefCell::new((String::new(), String::new())),
        });
        let host_ptr: *mut Host = &mut *host;
        inner.host = host_ptr;
        let engine = Self {
            host,
            _inner: inner,
        };
        if let Some(runtime) = &engine.host.runtime {
            runtime.set_loader(
                modules::AppResolver::new(app_dir),
                modules::AppLoader::new(host_ptr, app_dir),
            );
            runtime.set_interrupt_handler(Some(Box::new(move || {
                // SAFETY: Host outlives its Runtime and this callback runs on its render thread.
                monotonic_ms() > unsafe { &*host_ptr }.deadline.get()
            })));
            runtime.set_host_promise_rejection_tracker(Some(Box::new(
                move |ctx, promise, reason, handled| {
                    // SAFETY: Host outlives the Runtime's rejection tracker.
                    let host = unsafe { &*host_ptr };
                    let mut refs = host.refs.borrow_mut();
                    let saved = Persistent::save(&ctx, promise);
                    if handled {
                        refs.rejections.retain(|(p, _)| *p != saved);
                    } else if refs.rejections.len() < 64
                        && !refs.rejections.iter().any(|(p, _)| *p == saved)
                    {
                        refs.rejections
                            .push((saved, Persistent::save(&ctx, reason)));
                    }
                },
            )));
        }
        Ok(engine)
    }
    pub(super) fn host_ptr(&mut self) -> *mut Host {
        &mut *self.host
    }
    pub(super) fn start(&self, program: ProgramKind<'_>, name: &str) -> Result<(), Error> {
        self.host.with(|ctx| {
            ctx.store_userdata(HostPtr(ptr::from_ref(&*self.host).cast_mut()))
                .map_err(|_| Error::Runtime)?;
            bindings::install(&self.host, &ctx).map_err(|error| {
                self.host.stash(&ctx, error);
                self.host.script_error()
            })
        })?;
        self.host
            .eval(bootstrap::ABI_JS.as_bytes(), "abi.js", false)?;
        self.host
            .eval(bootstrap::SCENE_JS.as_bytes(), "scene.js", false)?;
        self.host
            .eval(bootstrap::PRELUDE.as_bytes(), "prelude.js", false)?;
        self.host.eval(
            bootstrap::REACTIVE_BOOT.as_bytes(),
            "reactive-boot.js",
            true,
        )?;
        self.host
            .with(|ctx| -> rquickjs::Result<()> {
                let reactive: Value = ctx.globals().get("__reactive")?;
                self.host.refs.borrow_mut().reactive = Some(Persistent::save(&ctx, reactive));
                ctx.globals().remove("__reactive")?;
                Ok(())
            })
            .map_err(|_| Error::Runtime)?;
        match program {
            ProgramKind::Trusted(bytecode) => {
                let _budget = self.host.budget("eval", 0);
                let promise = self.host.with(|ctx| bytecode.load(&ctx))?;
                self.host.drain()?;
                if let Some(promise) = promise {
                    self.host.with(|ctx| {
                        let promise = promise.restore(&ctx).map_err(|_| Error::Runtime)?;
                        if let Some(Err(error)) = promise.result::<()>() {
                            self.host.stash(&ctx, error);
                            return Err(self.host.script_error());
                        }
                        Ok(())
                    })?;
                }
            }
            ProgramKind::Source(source) => {
                // Module detection uses the engine parser; bootstrap scripts always use global mode.
                let module = bytecode::is_module(source);
                self.host.eval(source, name, module)?;
            }
        }
        self.host.drain()
    }
    pub(super) fn shutdown(&mut self) {
        if self.host.context.is_none() {
            return;
        }
        // Close leases before owner cleanup. Recorders refund while the budget field still exists.
        begin_shutdown(self.host.inner());
        // All saved values, including cleanups that create more timers, die while the VM is alive.
        self.host.with(|ctx| {
            let _budget = self.host.budget("reactive", 0);
            reactive::release_all(self.host.inner());
            finish_shutdown(self.host.inner());
            self.host.timers.borrow_mut().clear();
            *self.host.refs.borrow_mut() = References::default();
            let _ = ctx.remove_userdata::<HostPtr>();
        });
        if let Some(runtime) = &self.host.runtime {
            runtime.set_host_promise_rejection_tracker(None);
            runtime.set_interrupt_handler(None);
        }
        drop(self.host.context.take());
        drop(self.host.runtime.take());
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn host_from_ctx<'a>(ctx: &'a Ctx<'_>) -> rquickjs::Result<&'a Host> {
    let pointer = ctx
        .userdata::<HostPtr>()
        .ok_or_else(|| type_error(ctx, "session is closed"))?
        .0;
    // SAFETY: userdata is removed before Context is dropped, and Engine owns Host throughout.
    Ok(unsafe { &*pointer })
}
