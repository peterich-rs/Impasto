//! The single render-thread VM and the references it owns.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use rquickjs::function::Args;
use rquickjs::{Ctx, Function, IntoJs, JsLifetime, Object, Persistent, Value};

use javascript::runtime::extension::exception_text;

use super::{Inner, bootstrap, reactive};
use crate::js::{Error, compiler};

mod bindings;
mod canvas;
mod drivers;
mod modules;
mod renderer;
mod scroll_rules;
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

    pub(super) fn resources_snapshot(&self) -> javascript::runtime::JsResourcesSnapshot {
        use javascript::runtime::{TimerKind, TimerSnapshot};
        let now = self.runtime.clock_ms();
        let raf = self.raf.borrow();
        self.runtime
            .resources_snapshot(raf.iter().filter(|timer| timer.active).map(|timer| {
                TimerSnapshot {
                    id: timer.id,
                    kind: TimerKind::AnimationFrame,
                    due_in_ms: timer.due.saturating_sub(now),
                    interval_ms: 0,
                }
            }))
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
    fn record_fault(&self, source: &'static str, ctx: Option<&Ctx<'_>>) -> String {
        let mut diagnostic = self.runtime.extension().diagnostic();
        if let Some(ctx) = ctx
            && let Ok(Some(hot)) = ctx.globals().get::<_, Option<Object>>("__impasto_hot")
            && let Ok(symbolize) = hot.get::<_, Function>("symbolize")
        {
            match symbolize.call::<_, String>((diagnostic.stack.as_str(),)) {
                Ok(stack) => diagnostic.stack = stack,
                Err(error) if error.is_exception() => {
                    let _ = ctx.catch();
                }
                Err(_) => {}
            }
        }
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
        (self.inner.log)(3, &self.record_fault(source, None));
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
        self.record_fault(source, Some(ctx))
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

fn trace_user_timing(entry: javascript::runtime::UserTiming<'_>) {
    let now = std::time::Instant::now();
    let Some(age) = now.checked_duration_since(entry.start) else {
        return;
    };
    let Some(duration) = entry.end.checked_duration_since(entry.start) else {
        return;
    };
    if entry.end > now {
        return;
    }
    let age_ns = u64::try_from(age.as_nanos()).unwrap_or(u64::MAX);
    match entry.kind {
        javascript::runtime::UserTimingKind::Mark => tracing::event!(
            name: "performance_mark", target: "impasto::user_timing", tracing::Level::DEBUG,
            trace_event = true, queue = "ui", label = entry.name, age_ns, kind = "mark"
        ),
        javascript::runtime::UserTimingKind::Measure => {
            let duration_ns = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
            tracing::event!(name: "performance_measure", target: "impasto::user_timing", tracing::Level::DEBUG,
                trace_event = true, queue = "ui", label = entry.name, age_ns, duration_ns, kind = "measure");
        }
    }
}

pub(in crate::js::player) struct JsVm {
    live_sampling: Cell<bool>,
    live_samples: RefCell<Vec<javascript::runtime::StackSample>>,
    pub(super) host: Rc<Host>,
}
impl JsVm {
    pub(super) fn apply_source_patch(&self, source: &str) -> Result<bool, String> {
        let delta = source.strip_prefix("// impasto:hot-patch/1\n");
        if delta.is_none() && !source.starts_with("// impasto:hot/1\n") {
            return Ok(false);
        }
        let state = delta
            .or_else(|| {
                source
                    .split_once("\n// impasto:hot-state\n")
                    .and_then(|(_, state)| state.split_once("\n// impasto:hot-state-end\n"))
                    .map(|(state, _)| state)
            })
            .ok_or_else(|| "missing development module state".to_owned())?;
        let _budget = self.host.budget("source patch", 0);
        let applied = self.host.with(|ctx| {
            let hot = ctx.globals().get::<_, Option<Object>>("__impasto_hot");
            let result = hot.and_then(|hot| {
                let Some(hot) = hot else {
                    return Ok(false);
                };
                let update: Function = hot.get(if delta.is_some() { "patch" } else { "update" })?;
                let state = ctx.json_parse(state)?;
                update.call((rquickjs::function::This(hot), state))
            });
            result.map_err(|error| self.host.stash_fault(&ctx, "source patch", error))
        })?;
        if delta.is_some() && !applied {
            return Err("development patch does not match the running module graph".to_owned());
        }
        Ok(applied)
    }

    pub(super) fn set_sampler(&self, config: javascript::runtime::SamplerConfig) -> bool {
        let applied = self.host.runtime.set_sampler_config(config);
        self.live_sampling.set(applied && config.enabled);
        if !self.live_sampling.get() {
            self.live_samples.borrow_mut().clear();
        }
        applied
    }

    pub(super) fn take_live_samples(&self) -> Vec<javascript::runtime::StackSample> {
        std::mem::take(&mut *self.live_samples.borrow_mut())
    }

    pub(super) fn record_stack_samples(&self) {
        for sample in self.host.runtime.take_stack_samples() {
            let age_ns = u64::try_from(sample.at.elapsed().as_nanos()).unwrap_or(u64::MAX);
            tracing::event!(name: "js_stack_sample", target: "impasto::js_profile",
                tracing::Level::TRACE, trace_event = true, queue = "ui", age_ns,
                js_entry = sample.entry, js_stack = sample.stack.as_str());
            if self.live_sampling.get() {
                let mut live = self.live_samples.borrow_mut();
                if live.len() < 512 {
                    live.push(sample);
                }
            }
        }
    }

    pub(super) fn new(
        inner: Rc<Inner>,
        app_dir: Option<&str>,
        development: bool,
    ) -> Result<Self, Error> {
        let log = Rc::clone(&inner.log);
        let mut builder = javascript::runtime::Runtime::builder()
            .log(move |level, text| log(level, text))
            .user_timing(trace_user_timing)
            .stack_sampling(|| {
                tracing::event_enabled!(
                    target: "impasto::js_profile", tracing::Level::TRACE, trace_event
                )
            });
        if let Some(clock) = &inner.config.transport_clock {
            builder = builder.transport_clock(Arc::clone(clock));
        }
        let mut limits = match inner.config.script_execution {
            crate::session::ScriptExecution::DedicatedUi => Default::default(),
            crate::session::ScriptExecution::PlatformUi => javascript::runtime::RuntimeLimits {
                eval_budget_ms: 500,
                callback_budget_ms: 32,
                ..Default::default()
            },
            crate::session::ScriptExecution::HeadlessTest => javascript::runtime::RuntimeLimits {
                heap_bytes: 128 << 20,
                eval_budget_ms: 5_000,
                callback_budget_ms: 5_000,
                job_batch: 256,
                ..Default::default()
            },
        };
        if development {
            // Source factories initialize their dependency graph on the first
            // turn. The frame callback limit remains the host's normal limit.
            limits.eval_budget_ms = 5_000;
        }
        builder = builder.limits(limits);
        let runtime = builder.build_unstarted().map_err(|_| Error::Runtime)?;
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
                modules::AppResolver::new(app_dir, Arc::clone(&host.inner.config.registry)),
                modules::AppLoader::new(app_dir, Arc::clone(&host.inner.config.registry)),
            )
            .map_err(|_| Error::Runtime)?;
        Ok(Self {
            host,
            live_sampling: Cell::new(false),
            live_samples: RefCell::default(),
        })
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
        let profile = javascript::standard::StandardProfile;
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
                    .runtime
                    .load_trusted(bytecode)
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
            false,
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
