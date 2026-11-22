//! The single render-thread VM and the references it owns.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use rquickjs::function::Args;
use rquickjs::{Ctx, Function, IntoJs, JsLifetime, Object, Persistent, Value};

use javascript::runtime::IoBindings;
use javascript::runtime::extension::exception_text;

use super::{Inner, bootstrap, reactive};
use crate::js::{Error, compiler};

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

type Saved = Persistent<Value<'static>>;

#[derive(JsLifetime)]
struct HostRef(Weak<Host>);

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

pub(super) use javascript::runtime::extension::Timer;

pub(in crate::js::player) struct Host {
    /// The shared VM. Animation frames and UI references stay on this host.
    pub(in crate::js::player) runtime: javascript::runtime::Runtime,
    inner: Rc<Inner>,
    weak: Weak<Host>,
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
    pub(super) fn discard_if_unattached(&self, id: crate::tree::NodeId) {
        self.inner().tree.borrow_mut().discard_if_unattached(id);
    }

    pub(super) fn inner(&self) -> &Inner {
        &self.inner
    }

    pub(super) fn downgrade(&self) -> Weak<Self> {
        Weak::clone(&self.weak)
    }

    fn with_current<R>(&self, ctx: &Ctx<'_>, body: impl FnOnce() -> R) -> R {
        self.runtime.extension().with_current(ctx, body)
    }
    pub(super) fn with<R>(&self, body: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        self.runtime.extension().with(body)
    }
    fn budget(&self, kind: &'static str, id: u64) -> javascript::runtime::extension::Budget<'_> {
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
    fn record_fault(&self, source: &'static str) -> String {
        let diagnostic = self.runtime.extension().diagnostic();
        let display = super::display_fault(&diagnostic.message, &diagnostic.stack);
        self.inner
            .faults
            .borrow_mut()
            .push(crate::session::ScriptFault {
                source,
                message: diagnostic.message,
                stack: diagnostic.stack,
            });
        display
    }

    fn report(&self, source: &'static str) {
        (self.inner.log)(3, &self.record_fault(source));
    }

    fn finish<T>(
        &self,
        ctx: &Ctx<'_>,
        source: &'static str,
        result: rquickjs::Result<T>,
    ) -> Result<T, String> {
        result.map_err(|error| self.stash_fault(ctx, source, error))
    }

    fn stash_fault(&self, ctx: &Ctx<'_>, source: &'static str, error: rquickjs::Error) -> String {
        self.stash(ctx, error);
        self.record_fault(source)
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
        source: &'static str,
    ) -> Result<(), String> {
        match result {
            Ok(_) => Ok(()),
            Err(error) if self.freeze.get() => Err(self.stash_fault(ctx, source, error)),
            Err(error) => {
                self.stash(ctx, error);
                self.report("listener");
                Ok(())
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
            if stop == javascript::runtime::extension::JobStop::Limit {
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
pub(in crate::js::player) enum ProgramKind<'a> {
    Source(&'a [u8]),
    Trusted(javascript::runtime::TrustedBytecode<'a>),
}

pub(in crate::js::player) struct JsVm {
    pub(super) host: Rc<Host>,
}
impl JsVm {
    pub(super) fn new(
        inner: Rc<Inner>,
        app_dir: Option<&str>,
        http: Option<Arc<dyn crate::session::HttpClient>>,
    ) -> Result<Self, Error> {
        let log = Rc::clone(&inner.log);
        let mut builder =
            javascript::runtime::Runtime::builder().log(move |level, text| log(level, text));
        if let Some(clock) = &inner.config.transport_clock {
            builder = builder.transport_clock(Arc::clone(clock));
        }
        let runtime = builder.build_unstarted().map_err(|_| Error::Runtime)?;
        runtime.configure_io(http.map_or_else(IoBindings::disconnected, IoBindings::inline));
        let host = Rc::new_cyclic(|weak| Host {
            runtime,
            inner,
            weak: Weak::clone(weak),
            refs: RefCell::new(References::default()),
            raf: RefCell::new(Vec::new()),
            paused: Cell::new(false),
            wants_frame: Cell::new(false),
            back_enabled: Cell::new(None),
            freeze: Cell::new(false),
            reactive_failed: Cell::new(false),
            closing: Cell::new(false),
        });
        host.runtime
            .extension()
            .install_loader(
                modules::AppResolver::new(app_dir),
                modules::AppLoader::new(app_dir),
            )
            .map_err(|_| Error::Runtime)?;
        Ok(Self { host })
    }

    pub(super) fn install(&self, scene: &str) -> Result<(), Error> {
        self.host.with(|ctx| {
            ctx.store_userdata(HostRef(self.host.downgrade()))
                .map_err(|_| Error::Runtime)?;
            bindings::install(&self.host, &ctx).map_err(|error| {
                self.host.stash(&ctx, error);
                self.host.script_error()
            })
        })?;
        self.host.eval(scene.as_bytes(), "scene.js", false)?;
        let profile = javascript::standard::StandardProfile::legacy();
        {
            let _budget = self.host.budget("eval", 0);
            self.host.with(|ctx| {
                javascript::runtime::extension::install_web(&ctx, profile).map_err(|error| {
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
                let module = javascript::runtime::extension::is_module(source);
                self.host.eval(source, name, module)?;
            }
        }
        self.host.drain()
    }
    pub(super) fn install_io(&mut self, submit: Box<dyn Fn(crate::session::IoJob) + Send>) {
        self.host.runtime.configure_io(IoBindings::external(submit));
    }

    pub(super) fn install_routed_io(&mut self, submit: crate::js::RoutedIoSubmit) {
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

    pub(super) fn emit_host_event(&self, event: javascript::host::HostEvent) {
        let _ = self.host.runtime.emit_host_event(event, |ctx, _error| {
            if ctx.has_exception() {
                drop(ctx.catch());
            }
            true
        });
    }

    fn prepare_shutdown(&self) {
        if self.host.closing.get() {
            return;
        }
        self.host.runtime.begin_shutdown();
        self.host.closing.set(true);
        // Close leases before owner cleanup so recorders can refund against a live budget.
        begin_shutdown(self.host.inner());
        self.host.with(|ctx| {
            let _budget = self.host.budget("reactive", 0);
            reactive::release_all(self.host.inner());
            finish_shutdown(&self.host);
            self.host.raf.borrow_mut().clear();
            *self.host.refs.borrow_mut() = References::default();
            let _ = ctx.remove_userdata::<HostRef>();
        });
    }
}

impl Drop for JsVm {
    fn drop(&mut self) {
        self.prepare_shutdown();
        // Runtime drops before Inner, so native class finalizers can still release resources.
    }
}

fn host_from_ctx(ctx: &Ctx<'_>) -> rquickjs::Result<Rc<Host>> {
    ctx.userdata::<HostRef>()
        .and_then(|host| host.0.upgrade())
        .ok_or_else(|| type_error(ctx, "session is closed"))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "UI bootstrap failure assertions")]
mod tests {
    use super::*;

    #[test]
    fn failed_scene_bootstrap_releases_native_and_timer_roots() {
        let vm = JsVm::new(
            Rc::new(Inner::new(
                crate::text::MonoShaper::default(),
                Box::new(|_, _| {}),
                crate::session::ModuleConfig::disconnected(),
            )),
            None,
            None,
        )
        .expect("allocate extension owner");
        let weak = vm.host.runtime.downgrade();
        let result =
            vm.install("setTimeout(() => {}, 60000); throw new Error('scene bootstrap failed');");
        assert!(
            matches!(result, Err(Error::Script(message)) if message.contains("scene bootstrap failed"))
        );
        assert!(weak.is_open());
        assert_eq!(vm.host.runtime.counts().timers, 1);
        assert!(vm.host.refs.borrow().scene.is_some());

        vm.prepare_shutdown();
        vm.prepare_shutdown();
        assert!(vm.host.refs.borrow().scene.is_none());
        drop(vm);
        assert!(!weak.is_open());
    }
}
