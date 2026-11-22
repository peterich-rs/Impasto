//! Console, clock, and timeout/interval bindings shared by every runtime.
//!
//! Animation frames stay in the UI adapter. This module does not know about a
//! session or a canvas.

use rquickjs::function::Rest;
use rquickjs::{Ctx, FromJs, Function, Object, Value};

use crate::runtime::quickjs::{Saved, Timer};
use crate::runtime::scheduler::Scheduler;

mod encoding;
mod events;
mod headers;
mod http;
mod profile;
mod socket;
mod streams;
mod url;
mod web;

pub use profile::{StandardModule, StandardProfile};
pub use web::install_web;

pub fn install_console<'js>(
    ctx: &Ctx<'js>,
    log: impl Fn(u8, &str) + Copy + 'static,
) -> rquickjs::Result<()> {
    let console = Object::new(ctx.clone())?;
    for (name, level) in [
        ("log", 0u8),
        ("info", 1),
        ("warn", 2),
        ("error", 3),
        ("debug", 4),
    ] {
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<()> {
                let mut parts = Vec::new();
                for arg in args.0 {
                    let text = rquickjs::Coerced::<String>::from_js(&ctx, arg)?;
                    parts.push(text.0);
                }
                log(level, &parts.join(" "));
                Ok(())
            },
        )?;
        console.set(name, function)?;
    }
    ctx.globals().set("console", console)
}

pub fn install_performance<'js>(
    ctx: &Ctx<'js>,
    now: impl Fn() -> f64 + 'static,
) -> rquickjs::Result<()> {
    let performance = Object::new(ctx.clone())?;
    let function = Function::new(ctx.clone(), now)?;
    performance.set("now", function)?;
    ctx.globals().set("performance", performance)
}

/// Register timeout and interval on the global object.
///
/// `kind` is 0 for timeout and 1 for interval. Clearing one kind does not
/// cancel the other. A negative delay is treated as zero. The callback becomes
/// eligible on the next timer turn.
/// # Safety
///
/// `scheduler` stays at a stable address until the registered functions
/// and their context are dropped.
pub(crate) unsafe fn install_timers<'js>(
    ctx: &Ctx<'js>,
    scheduler: *const Scheduler,
    limit: usize,
    on_zero_delay: impl Fn() + Clone + 'static,
) -> rquickjs::Result<()> {
    for (name, kind) in [("setTimeout", 0i32), ("setInterval", 1)] {
        let on_zero_delay = on_zero_delay.clone();
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<f64> {
                let Some(callback) = args.0.first() else {
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
                let delay = number_arg(args.0.get(1)).max(0);
                let extra = args
                    .0
                    .iter()
                    .skip(2)
                    .take(8)
                    .cloned()
                    .map(|value| rquickjs::Persistent::save(&ctx, value))
                    .collect();
                // SAFETY: the scheduler belongs to the context that installed these functions.
                let scheduler = unsafe { &*scheduler };
                let mut timers = scheduler.timers.borrow_mut();
                if timers.len() >= limit {
                    return Err(rquickjs::Exception::throw_range(
                        &ctx,
                        "timer limit exceeded",
                    ));
                }
                let id = scheduler
                    .allocate_id()
                    .ok_or_else(|| rquickjs::Exception::throw_range(&ctx, "timer IDs exhausted"))?;
                timers.push(schedule_timer(
                    id,
                    kind,
                    delay,
                    scheduler.mono.get(),
                    scheduler.turn.get(),
                    rquickjs::Persistent::save(&ctx, callback.clone()),
                    extra,
                ));
                scheduler.peak.set(scheduler.peak.get().max(timers.len()));
                drop(timers);
                if delay == 0 {
                    on_zero_delay();
                }
                Ok(crate::runtime::resources::number(id))
            },
        )?;
        ctx.globals().set(name, function)?;
    }
    for (name, kind) in [("clearTimeout", 0i32), ("clearInterval", 1)] {
        let function = Function::new(ctx.clone(), move |args: Rest<Value<'js>>| {
            let id = u64::try_from(number_arg(args.0.first())).unwrap_or(0);
            // SAFETY: the timer table belongs to the runtime that installed this function.
            unsafe { clear_timer(&mut (*scheduler).timers.borrow_mut(), id, kind) };
        })?;
        ctx.globals().set(name, function)?;
    }
    Ok(())
}

pub fn schedule_timer(
    id: u64,
    kind: i32,
    delay: i64,
    mono: i64,
    turn: u64,
    function: Saved,
    extra: Vec<Saved>,
) -> Timer {
    let delay = delay.max(0);
    Timer {
        id,
        due: if kind == 2 {
            mono
        } else {
            mono.saturating_add(delay)
        },
        interval: if kind == 1 { delay } else { 0 },
        eligible_turn: turn.saturating_add(1),
        kind,
        function,
        extra,
        active: true,
    }
}

pub fn clear_timer(timers: &mut Vec<Timer>, id: u64, kind: i32) {
    timers.retain(|timer| timer.id != id || timer.kind != kind);
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "timer delays arrive as JavaScript numbers, matching the existing host conversion"
)]
fn number_arg(value: Option<&Value<'_>>) -> i64 {
    value.and_then(|value| value.as_number()).unwrap_or(0.0) as i64
}

pub(super) const CONSOLE_DECLARATIONS: &str = r#"
declare const console: {
  log(...values: unknown[]): void;
  info(...values: unknown[]): void;
  warn(...values: unknown[]): void;
  error(...values: unknown[]): void;
  debug(...values: unknown[]): void;
  time(label?: string): void;
  timeLog(label?: string): void;
  timeEnd(label?: string): void;
};
"#;

pub(super) const TIMER_DECLARATIONS: &str = r#"
declare function setTimeout<Args extends unknown[]>(callback: (...args: Args) => void, delay?: number, ...args: Args): number;
declare function clearTimeout(id?: number): void;
declare function setInterval<Args extends unknown[]>(callback: (...args: Args) => void, delay?: number, ...args: Args): number;
declare function clearInterval(id?: number): void;
"#;

pub(super) const PERFORMANCE_DECLARATIONS: &str = r#"
declare const performance: { now(): number };
"#;
