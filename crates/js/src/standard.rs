//! Console, clock, and timeout/interval bindings shared by every runtime.
//!
//! Animation frames stay in the UI adapter. This module does not know about a
//! session or a canvas.

use std::cell::{Cell, RefCell};

use rquickjs::function::Rest;
use rquickjs::{Ctx, FromJs, Function, Object, Value};

use crate::quickjs::{Saved, Timer};

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
/// `timers`, `next_id`, `turn`, and `mono` must stay allocated and unchanged
/// for as long as the functions registered here can run.
pub unsafe fn install_timers<'js>(
    ctx: &Ctx<'js>,
    timers: *const RefCell<Vec<Timer>>,
    next_id: *const Cell<u64>,
    turn: *const Cell<u64>,
    mono: *const Cell<i64>,
    on_zero_delay: impl Fn() + Copy + 'static,
) -> rquickjs::Result<()> {
    for (name, kind) in [("setTimeout", 0i32), ("setInterval", 1)] {
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
                    .cloned()
                    .map(|value| rquickjs::Persistent::save(&ctx, value))
                    .collect();
                // SAFETY: the cells belong to the runtime allocation that installed these functions.
                let id = unsafe {
                    let id = (*next_id).get().wrapping_add(1).max(1);
                    (*next_id).set(id);
                    (*timers).borrow_mut().push(schedule_timer(
                        id,
                        kind,
                        delay,
                        (*mono).get(),
                        (*turn).get(),
                        rquickjs::Persistent::save(&ctx, callback.clone()),
                        extra,
                    ));
                    id
                };
                if delay == 0 {
                    on_zero_delay();
                }
                Ok(id as f64)
            },
        )?;
        ctx.globals().set(name, function)?;
    }
    for (name, kind) in [("clearTimeout", 0i32), ("clearInterval", 1)] {
        let function = Function::new(ctx.clone(), move |args: Rest<Value<'js>>| {
            let id = u64::try_from(number_arg(args.0.first())).unwrap_or(0);
            // SAFETY: the timer table belongs to the runtime that installed this function.
            unsafe { clear_timer(&mut (*timers).borrow_mut(), id, kind) };
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
        eligible_turn: turn + 1,
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
