//! The single render-thread VM and the references it owns.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use rquickjs::function::Args;
use rquickjs::{Ctx, Function, IntoJs, JsLifetime, Object, Persistent, Value};

use crate::runtime::IoBindings;
use crate::runtime::quickjs::exception_text;

use super::{Inner, bootstrap, reactive};
use crate::ui::{Error, compiler};

mod bindings;
mod canvas;
mod drivers;
mod modules;
mod renderer;
mod turns;
pub(super) use canvas::{
    CanvasSession, CanvasStats, begin_shutdown, deliver_errors, finish_shutdown, paint,
};
pub(crate) use modules::{compile_declarations, prepare_compile};
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
    window_metrics: Vec<(u64, Saved)>,
    motions: Vec<(u64, u64, String, Saved)>,
}

pub(super) use crate::runtime::quickjs::Timer;

pub(in crate::ui::player) struct Host {
    /// The shared VM. Animation frames and UI references stay on this host.
    pub(in crate::ui::player) runtime: crate::runtime::Runtime,
    inner: *mut Inner,
    refs: RefCell<References>,
    raf: RefCell<Vec<Timer>>,
    paused: Cell<bool>,
    wants_frame: Cell<bool>,
    back_enabled: Cell<Option<bool>>,
    freeze: Cell<bool>,
    reactive_failed: Cell<bool>,
    closing: Cell<bool>,
}

impl Host {
    pub(super) fn discard_if_unattached(&self, id: uicore::NodeId) {
        self.inner().tree.borrow_mut().discard_if_unattached(id);
    }

    pub(super) fn inner(&self) -> &Inner {
        // SAFETY: Engine keeps Inner allocated until Context and Runtime have been dropped.
        unsafe { &*self.inner }
    }
    fn session(&self) -> *mut c_void {
        self.inner.cast()
    }
    fn with_current<R>(&self, ctx: &Ctx<'_>, body: impl FnOnce() -> R) -> R {
        self.runtime.extension().with_current(ctx, body)
    }
    pub(super) fn with<R>(&self, body: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        self.runtime.extension().with(body)
    }
    fn budget(&self, kind: &'static str, id: u64) -> crate::runtime::quickjs::Budget<'_> {
        self.runtime.extension().budget(kind, id)
    }
    /// Replace the interrupt label without restarting the turn's deadline.
    fn retarget(&self, kind: &'static str, id: u64) {
        self.runtime.extension().retarget(kind, id);
    }
    pub(super) fn is_closing(&self) -> bool {
        self.closing.get()
    }
    pub(super) fn past_deadline(&self) -> bool {
        self.runtime.extension().past_deadline()
    }
    fn stash_value<'js>(&self, ctx: &Ctx<'js>, value: Value<'js>) {
        self.runtime.extension().stash_value(ctx, value);
    }
    fn stash(&self, ctx: &Ctx<'_>, error: rquickjs::Error) {
        self.runtime.extension().stash(ctx, error);
    }
    pub(super) fn take_error(&self) -> (String, String) {
        let diagnostic = self.runtime.extension().diagnostic();
        (diagnostic.message, diagnostic.stack)
    }
    fn report(&self, source: &'static str) {
        let diagnostic = self.runtime.extension().diagnostic();
        let (message, stack) = (diagnostic.message, diagnostic.stack);
        self.inner()
            .faults
            .borrow_mut()
            .push(crate::session::ScriptFault {
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
            let diagnostic = self.runtime.extension().diagnostic();
            let bytes = diagnostic.message.as_bytes();
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
        let diagnostic = self.runtime.extension().diagnostic();
        Error::Script(super::display_fault(&diagnostic.message, &diagnostic.stack))
    }
    pub(super) fn drain(&self) -> Result<(), Error> {
        let _budget = self.budget("jobs", 0);
        if let Err(stop) = self.runtime.checkpoint(|ctx| {
            self.stash(ctx, rquickjs::Error::Exception);
        }) {
            if stop == crate::runtime::quickjs::JobStop::Limit {
                self.runtime
                    .extension()
                    .record_error("quickjs job round limit".to_owned(), String::new());
            }
            return Err(self.script_error());
        }
        self.with(|ctx| {
            for reason in self.runtime.extension().take_rejections() {
                if let Ok(reason) = reason.restore(&ctx) {
                    let (message, stack) = exception_text(&ctx, reason);
                    let message = format!(
                        "unhandled rejection: {}",
                        super::display_fault(&message, &stack)
                    );
                    self.inner()
                        .faults
                        .borrow_mut()
                        .push(crate::session::ScriptFault {
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
fn type_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    rquickjs::Exception::throw_type(ctx, message)
}
/// Source text, or bytecode the caller has already promised came from `compile`.
pub(in crate::ui::player) enum ProgramKind<'a> {
    Source(&'a [u8]),
    Trusted(crate::runtime::TrustedBytecode<'a>),
}

pub(in crate::ui::player) struct Engine {
    host: Box<Host>,
    /// Kept alive through driver finalizers, including those run by Runtime::drop.
    _inner: Box<Inner>,
}
impl Engine {
    pub(super) fn new(
        mut inner: Box<Inner>,
        app_dir: Option<&str>,
        http: Option<Arc<dyn crate::session::HttpClient>>,
    ) -> Result<Self, Error> {
        let log = Rc::clone(&inner.log);
        let mut builder =
            crate::runtime::Runtime::builder().log(move |level, text| log(level, text));
        if let Some(clock) = &inner.config.transport_clock {
            builder = builder.transport_clock(Arc::clone(clock));
        }
        let runtime = builder.build_unstarted().map_err(|_| Error::Runtime)?;
        runtime.configure_io(http.map_or_else(IoBindings::disconnected, IoBindings::inline));
        let mut host = Box::new(Host {
            runtime,
            inner: &mut *inner,
            refs: RefCell::new(References::default()),
            raf: RefCell::new(Vec::new()),
            paused: Cell::new(false),
            wants_frame: Cell::new(false),
            back_enabled: Cell::new(None),
            freeze: Cell::new(false),
            reactive_failed: Cell::new(false),
            closing: Cell::new(false),
        });
        let host_ptr: *mut Host = &mut *host;
        inner.host = host_ptr;
        let engine = Self {
            host,
            _inner: inner,
        };
        engine
            .host
            .runtime
            .extension()
            .install_loader(
                modules::AppResolver::new(app_dir),
                modules::AppLoader::new(app_dir),
            )
            .map_err(|_| Error::Runtime)?;
        Ok(engine)
    }

    pub(super) fn inner_ptr(&self) -> *mut Inner {
        self.host.inner
    }

    pub(super) fn install(&self, scene: &str) -> Result<(), Error> {
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
        self.host.eval(scene.as_bytes(), "scene.js", false)?;
        let profile = crate::standard::StandardProfile::legacy();
        {
            let _budget = self.host.budget("eval", 0);
            self.host.with(|ctx| {
                crate::standard::install_web(&ctx, profile).map_err(|error| {
                    self.host.stash(&ctx, error);
                    self.host.script_error()
                })
            })?;
            self.host.drain()?;
        }
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
        self.host.runtime.finish_installation();
        Ok(())
    }

    pub(super) fn start(&self, program: ProgramKind<'_>, name: &str) -> Result<(), Error> {
        match program {
            ProgramKind::Trusted(bytecode) => {
                let _budget = self.host.budget("eval", 0);
                let promise = self
                    .host
                    .with(|ctx| bytecode.load(&ctx))
                    .map_err(compiler::map_bytecode)?;
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
                let module = crate::compiler::bytecode::is_module(source);
                self.host.eval(source, name, module)?;
            }
        }
        self.host.drain()
    }
    pub(super) fn install_io(&mut self, submit: Box<dyn Fn(crate::session::IoJob) + Send>) {
        self.host.runtime.configure_io(IoBindings::external(submit));
    }

    pub(super) fn install_routed_io(&mut self, submit: crate::ui::RoutedIoSubmit) {
        self.host.runtime.configure_io(IoBindings::routed(submit));
    }

    pub(super) fn drain_local_io(&self) {
        self.host.runtime.drain_local_io(|_error| {});
    }

    pub(super) fn deliver_event(&self, event: crate::session::IoEvent) {
        if let crate::session::IoEvent::Host(event) = event {
            self.emit_host_event(event);
            return;
        }
        let _ = self.host.runtime.deliver_event(event);
    }

    pub(super) fn deliver_routed(&self, delivery: crate::session::Delivery) {
        if delivery.runtime == self.host.runtime.key()
            && let crate::session::IoEvent::Host(event) = delivery.event
        {
            self.emit_host_event(event);
            return;
        }
        let _ = self.host.runtime.deliver_routed(delivery);
    }

    pub(super) fn emit_host_event(&self, event: crate::host::HostEvent) {
        let _ = self.host.runtime.emit_host_event(event, |ctx, _error| {
            if ctx.has_exception() {
                drop(ctx.catch());
            }
            true
        });
    }

    pub(super) fn shutdown(&mut self) {
        if !self.host.runtime.extension().is_open() {
            return;
        }
        self.host.runtime.begin_shutdown();
        self.host.closing.set(true);
        // Close leases before owner cleanup so recorders can refund against a live budget.
        begin_shutdown(self.host.inner());
        self.host.with(|ctx| {
            let _budget = self.host.budget("reactive", 0);
            reactive::release_all(self.host.inner());
            finish_shutdown(self.host.inner());
            self.host.raf.borrow_mut().clear();
            *self.host.refs.borrow_mut() = References::default();
            let _ = ctx.remove_userdata::<HostPtr>();
        });
        // Host and Inner still exist while Context and VM run their finalizers.
        self.host.runtime.shutdown();
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "UI bootstrap failure assertions")]
mod tests {
    use super::*;

    #[test]
    fn failed_scene_bootstrap_releases_native_and_timer_roots() {
        let mut engine = Engine::new(
            Box::new(Inner::new(
                uicore::MonoShaper::default(),
                Box::new(|_, _| {}),
                crate::session::ModuleConfig::disconnected(),
            )),
            None,
            None,
        )
        .expect("allocate extension owner");
        let weak = engine.host.runtime.downgrade();
        let result = engine
            .install("setTimeout(() => {}, 60000); throw new Error('scene bootstrap failed');");
        assert!(
            matches!(result, Err(Error::Script(message)) if message.contains("scene bootstrap failed"))
        );
        assert!(weak.is_open());
        assert_eq!(engine.host.runtime.counts().timers, 1);
        assert!(engine.host.refs.borrow().scene.is_some());

        engine.shutdown();
        engine.shutdown();
        assert!(!weak.is_open());
        assert_eq!(engine.host.runtime.counts().timers, 0);
        assert!(engine.host.refs.borrow().scene.is_none());
        assert!(!engine.host.runtime.extension().is_open());
    }
}
