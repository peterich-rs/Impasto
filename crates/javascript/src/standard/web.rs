use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use rquickjs::class::JsClass;
use rquickjs::function::{Constructor, IntoJsFunc, Opt, ParamRequirement, Params, This};
use rquickjs::{Coerced, Ctx, Exception, FromJs, Function, JsLifetime, Object, Value};

use super::StandardProfile;

const NATIVE_BYTES: usize = 16 * 1024 * 1024;

#[derive(JsLifetime)]
struct Platform {
    owned_bytes: Rc<Cell<usize>>,
}

pub(super) fn is_nullish(value: &Value<'_>) -> bool {
    value.is_null() || value.is_undefined()
}

pub(super) fn constructor<'js, C, F, P>(
    ctx: &Ctx<'js>,
    function: F,
) -> rquickjs::Result<Constructor<'js>>
where
    C: JsClass<'js>,
    F: IntoJsFunc<'js, P> + 'js,
    P: 'js,
{
    Constructor::new_class::<C, _, _>(ctx.clone(), NewOnly(function, PhantomData))
}

struct NewOnly<F, P>(F, PhantomData<fn() -> P>);

impl<'js, F: IntoJsFunc<'js, P>, P> IntoJsFunc<'js, ()> for NewOnly<F, P> {
    fn param_requirements() -> ParamRequirement {
        F::param_requirements()
    }

    fn call(&self, params: Params<'_, 'js>) -> rquickjs::Result<Value<'js>> {
        if !params.is_constructor() {
            return Err(Exception::throw_type(
                params.ctx(),
                "Constructor requires new",
            ));
        }
        self.0.call(params)
    }
}

/// Accounts for Rust-owned storage that QuickJS's heap limit cannot see.
pub(super) struct Charge {
    total: Rc<Cell<usize>>,
    bytes: usize,
}

impl Charge {
    pub fn new(ctx: &Ctx<'_>) -> rquickjs::Result<Self> {
        let total = ctx
            .userdata::<Platform>()
            .map(|state| Rc::clone(&state.owned_bytes))
            .ok_or_else(|| Exception::throw_internal(ctx, "standard library is unavailable"))?;
        Ok(Self { total, bytes: 0 })
    }

    pub fn size(&self) -> usize {
        self.bytes
    }

    pub fn resize(&mut self, ctx: &Ctx<'_>, bytes: usize) -> rquickjs::Result<()> {
        let total = self
            .total
            .get()
            .saturating_sub(self.bytes)
            .saturating_add(bytes);
        if total > NATIVE_BYTES {
            return Err(Exception::throw_range(
                ctx,
                "standard library native storage limit exceeded",
            ));
        }
        self.total.set(total);
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.total.set(self.total.get().saturating_sub(self.bytes));
    }
}

/// Install the standard Web globals into a QuickJS context.
pub fn install_web(ctx: &Ctx<'_>, profile: StandardProfile) -> rquickjs::Result<()> {
    install_with_entropy(ctx, profile, std::sync::Arc::new(super::SystemEntropy))
}

pub(crate) fn install_with_entropy(
    ctx: &Ctx<'_>,
    _profile: StandardProfile,
    entropy: std::sync::Arc<dyn super::EntropySource>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    ctx.store_userdata(Platform {
        owned_bytes: Rc::default(),
    })
    .map_err(|_| Exception::throw_internal(ctx, "standard installation is already borrowed"))?;
    let error: Object = globals.get("Error")?;
    error.set("stackTraceLimit", 50)?;
    super::primitives::install(ctx, &globals, entropy)?;
    super::events::install(ctx, &globals)?;
    super::encoding::install(ctx, &globals)?;
    super::url::install(ctx, &globals)?;
    super::streams::install(ctx, &globals)?;
    super::native_stream::install(ctx)?;
    install_console_timing(ctx)?;
    Ok(())
}

fn install_console_timing<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    struct Timers {
        labels: HashMap<String, f64>,
        charge: Charge,
    }
    let console: Object = ctx.globals().get("console")?;
    let timers = Rc::new(RefCell::new(Timers {
        labels: HashMap::new(),
        charge: Charge::new(ctx)?,
    }));
    for (method, end) in [("time", false), ("timeLog", false), ("timeEnd", true)] {
        let timers = Rc::clone(&timers);
        console.set(
            method,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, Opt(label): Opt<Value<'js>>| {
                    let label = label
                        .filter(|label| !label.is_undefined())
                        .map(|label| Coerced::<String>::from_js(&ctx, label).map(|label| label.0))
                        .transpose()?
                        .unwrap_or_else(|| "default".into());
                    if method == "time" {
                        let now = performance_now(&ctx)?;
                        let mut timers = timers.borrow_mut();
                        if !timers.labels.contains_key(&label) {
                            let bytes = timers
                                .charge
                                .size()
                                .saturating_add(label.len())
                                .saturating_add(32);
                            timers.charge.resize(&ctx, bytes)?;
                        }
                        timers.labels.insert(label, now);
                        return Ok(());
                    }
                    let started = {
                        let mut timers = timers.borrow_mut();
                        if end {
                            let timer = timers.labels.remove(&label);
                            if timer.is_some() {
                                let bytes = timers
                                    .charge
                                    .size()
                                    .saturating_sub(label.len().saturating_add(32));
                                timers.charge.resize(&ctx, bytes)?;
                            }
                            timer
                        } else {
                            timers.labels.get(&label).copied()
                        }
                    };
                    let (level, message) = match started {
                        Some(started) => (
                            "info",
                            format!("{label}: {:.1} ms", performance_now(&ctx)? - started),
                        ),
                        None => ("warn", format!("Timer \"{label}\" does not exist")),
                    };
                    let console: Object = ctx.globals().get("console")?;
                    let report: Function = console.get(level)?;
                    report.call((This(console), message))
                },
            )?,
        )?;
    }
    Ok(())
}

fn performance_now(ctx: &Ctx<'_>) -> rquickjs::Result<f64> {
    let performance: Object = ctx.globals().get("performance")?;
    let now: Function = performance.get("now")?;
    now.call((This(performance),))
}
