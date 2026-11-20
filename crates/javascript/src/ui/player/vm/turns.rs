//! UI callback turns and animation frames.
use super::*;
use rquickjs::FromJs;

fn number<'js>(ctx: &Ctx<'js>, number: impl Into<f64>) -> Value<'js> {
    Value::new_float(ctx.clone(), number.into())
}
fn text<'js>(ctx: &Ctx<'js>, text: &str) -> rquickjs::Result<Value<'js>> {
    text.into_js(ctx)
}
fn string_at(pointer: *const c_char, len: usize) -> String {
    // SAFETY: all callers pass session-owned strings or buffers borrowed through this call.
    unsafe { super::super::utf8_of(pointer, len) }
        .unwrap_or("")
        .to_owned()
}
fn cstr(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: session uses CString for arguments without an explicit length.
    unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}
fn host<'a>(pointer: *mut Host) -> &'a Host {
    // SAFETY: these private adapters are called only with the session's live Host.
    unsafe { &*pointer }
}

const KINDS: [&str; 22] = [
    "click",
    "press",
    "release",
    "dragBegin",
    "dragBy",
    "dragEnd",
    "focus",
    "blur",
    "key",
    "text",
    "composing",
    "hover",
    "leave",
    "wheel",
    "secondaryClick",
    "scroll",
    "textReplaced",
    "link",
    "longPress",
    "doubleTap",
    "zoom",
    "scrollPhase",
];
fn dispatch_kind(kind: i32) -> &'static str {
    usize::try_from(kind)
        .ok()
        .and_then(|k| KINDS.get(k))
        .copied()
        .unwrap_or("tap")
}

pub(in crate::ui::player) unsafe fn vm_dispatch(
    pointer: *mut Host,
    id: u64,
    kind: i32,
    a: f64,
    b: f64,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget(dispatch_kind(kind), id);
    let callback = host.refs.borrow().callbacks.get(&(id, kind)).cloned();
    let Some(callback) = callback else {
        return 0;
    };
    host.with(|ctx| {
        let args = match kind {
            4 | 5 | 13 | 15 => vec![number(&ctx, a), number(&ctx, b)],
            17 | 20 => vec![number(&ctx, a)],
            _ => vec![],
        };
        host.listener_result(&ctx, host.call(&ctx, &callback, args), err, len)
    })
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::ui::player) unsafe fn vm_dispatch_input(
    pointer: *mut Host,
    id: u64,
    kind: i32,
    utf8: *const c_char,
    text_len: usize,
    with_span: i32,
    start: i32,
    end: i32,
    compose_start: i32,
    compose_end: i32,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget(dispatch_kind(kind), id);
    let callback = host.refs.borrow().callbacks.get(&(id, kind)).cloned();
    let Some(callback) = callback else {
        return 0;
    };
    host.with(|ctx| {
        #[allow(
            clippy::redundant_closure_call,
            reason = "the closure is the `?` boundary for a function that returns a status code"
        )]
        let result = (|| {
            let value = if utf8.is_null() {
                Value::new_null(ctx.clone())
            } else {
                text(&ctx, &string_at(utf8, text_len))?
            };
            let mut args = vec![value];
            if with_span != 0 {
                args.extend([
                    number(&ctx, start),
                    number(&ctx, end),
                    number(&ctx, compose_start),
                    number(&ctx, compose_end),
                ]);
            }
            host.call(&ctx, &callback, args)
        })();
        host.listener_result(&ctx, result, err, len)
    })
}
pub(in crate::ui::player) unsafe fn vm_dispatch_text(
    pointer: *mut Host,
    id: u64,
    utf8: *const c_char,
    text_len: usize,
    err: *mut c_char,
    len: usize,
) -> i32 {
    // SAFETY: forwards the same live Host and borrowed text/error buffers.
    unsafe { vm_dispatch_input(pointer, id, 16, utf8, text_len, 0, -1, -1, -1, -1, err, len) }
}
fn hook_turn(
    host: &Host,
    name: &'static str,
    values: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<Vec<Value<'js>>>,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let _budget = host.budget(name, 0);
    let hook = host.refs.borrow().hooks.get(name).cloned();
    let Some(hook) = hook else {
        return 0;
    };
    host.with(|ctx| {
        let result = values(&ctx)
            .and_then(|args| host.call(&ctx, &hook, args))
            .map(|_| ());
        host.finish(result, &ctx, err, len)
    })
}
pub(in crate::ui::player) unsafe fn vm_frame(
    pointer: *mut Host,
    target: f64,
    interval: f64,
    err: *mut c_char,
    len: usize,
) -> i32 {
    hook_turn(
        host(pointer),
        "frame",
        |ctx| Ok(vec![number(ctx, target), number(ctx, interval)]),
        err,
        len,
    )
}
pub(in crate::ui::player) unsafe fn vm_layout(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    hook_turn(host(pointer), "layout", |_| Ok(vec![]), err, len)
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::ui::player) unsafe fn vm_scroll_state(
    pointer: *mut Host,
    id: u64,
    phase: i32,
    vx: f32,
    vy: f32,
    ox: f32,
    oy: f32,
    err: *mut c_char,
    len: usize,
) -> i32 {
    hook_turn(
        host(pointer),
        "scroll",
        |ctx| {
            Ok(vec![
                number(ctx, id as f64),
                number(ctx, phase),
                number(ctx, vx),
                number(ctx, vy),
                number(ctx, ox),
                number(ctx, oy),
            ])
        },
        err,
        len,
    )
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::ui::player) unsafe fn vm_driver(
    pointer: *mut Host,
    kind: i32,
    index: u32,
    generation: u32,
    tag: i32,
    i0: i32,
    i1: i32,
    f0: f32,
    f1: f32,
    f2: f32,
    f3: f32,
    f4: f32,
    f5: f32,
    s0: *const c_char,
    n0: usize,
    s1: *const c_char,
    n1: usize,
    err: *mut c_char,
    len: usize,
) -> i32 {
    hook_turn(
        host(pointer),
        "driver",
        |ctx| {
            let mut args = vec![
                number(ctx, kind),
                number(ctx, index),
                number(ctx, generation),
                number(ctx, tag),
                number(ctx, i0),
                number(ctx, i1),
            ];
            args.extend([f0, f1, f2, f3, f4, f5].map(|n| number(ctx, n)));
            args.push(text(ctx, &string_at(s0, n0))?);
            args.push(text(ctx, &string_at(s1, n1))?);
            Ok(args)
        },
        err,
        len,
    )
}
pub(in crate::ui::player) unsafe fn vm_run_updates(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("reactive", 0);
    host.reactive_failed.set(false);
    host.with(|_| {
        reactive::reactive_entry(host.inner(), 14, &[]);
    });
    if host.reactive_failed.get() {
        host.write_error(err, len);
        -1
    } else {
        0
    }
}
pub(in crate::ui::player) unsafe fn vm_drain_jobs(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    if host.drain().is_err() {
        host.write_error(err, len);
        -1
    } else {
        0
    }
}
pub(in crate::ui::player) unsafe fn vm_jobs_pending(pointer: *mut Host) -> i32 {
    i32::from(host(pointer).runtime.jobs_pending())
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(in crate::ui::player) unsafe fn vm_tap_count(pointer: *mut Host) -> i32 {
    host(pointer)
        .refs
        .borrow()
        .callbacks
        .keys()
        .map(|(id, _)| *id)
        .collect::<std::collections::HashSet<_>>()
        .len() as i32
}
pub(in crate::ui::player) unsafe fn vm_forget_ids(pointer: *mut Host, ids: *const u64, len: usize) {
    // SAFETY: session borrows len initialized ids for this call.
    let ids = unsafe { std::slice::from_raw_parts(ids, len) };
    let host = host(pointer);
    {
        let mut refs = host.refs.borrow_mut();
        for &id in ids {
            for &(_, kind) in bindings::GESTURES {
                refs.callbacks.remove(&(id, kind));
            }
        }
        refs.motions.retain(|(_, id, _, _)| !ids.contains(id));
    }
    canvas::forget_nodes(host.inner(), ids);
}
pub(in crate::ui::player) unsafe fn vm_take_wants_frame(pointer: *mut Host) -> i32 {
    i32::from(host(pointer).wants_frame.replace(false))
}
pub(in crate::ui::player) unsafe fn vm_take_back_enabled(pointer: *mut Host) -> Option<bool> {
    host(pointer).back_enabled.take()
}

pub(in crate::ui::player) unsafe fn vm_back_gesture(
    pointer: *mut Host,
    event: crate::session::BackGesture,
    err: *mut c_char,
    len: usize,
) -> i32 {
    use crate::session::BackGesture;
    let vm = host(pointer);
    let hook = vm.refs.borrow().hooks.get("backGesture").cloned();
    let Some(hook) = hook else {
        if event == BackGesture::Committed {
            // SAFETY: forwards the same live VM and diagnostic buffer.
            return unsafe { vm_dispatch_back(pointer, err, len) };
        }
        return 1;
    };
    let _budget = vm.budget("backGesture", 0);
    vm.with(|ctx| {
        let (phase, value) = match event {
            BackGesture::Started { from_right } => ("start", f64::from(i32::from(from_right))),
            BackGesture::Progress(value) => ("progress", f64::from(value)),
            BackGesture::Cancelled => ("cancel", 0.0),
            BackGesture::Committed => ("commit", 1.0),
        };
        let result = text(&ctx, phase)
            .and_then(|phase| vm.call(&ctx, &hook, vec![phase, number(&ctx, value)]));
        match result {
            Ok(value) => {
                i32::from(event != BackGesture::Committed || value.as_bool() != Some(false))
            }
            Err(error) => vm.finish(Err(error), &ctx, err, len),
        }
    })
}
pub(in crate::ui::player) unsafe fn vm_dispatch_back(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("back", 0);
    let callbacks = host.refs.borrow().backs.clone();
    host.with(|ctx| {
        for (_, callback) in callbacks.iter().rev() {
            match host
                .call(&ctx, callback, vec![])
                .and_then(|v| rquickjs::Coerced::<bool>::from_js(&ctx, v))
            {
                Ok(value) if value.0 => return 1,
                Ok(_) => {}
                Err(error) => {
                    host.stash(&ctx, error);
                    host.write_error(err, len);
                    return -1;
                }
            }
        }
        0
    })
}
pub(in crate::ui::player) unsafe fn vm_try_shortcut(
    pointer: *mut Host,
    spec: *const c_char,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("shortcut", 0);
    let spec = cstr(spec);
    let callback = host
        .refs
        .borrow()
        .shortcuts
        .iter()
        .find(|(s, _)| *s == spec)
        .map(|(_, f)| f.clone());
    let Some(callback) = callback else {
        return 0;
    };
    host.with(|ctx| {
        if host.listener_result(&ctx, host.call(&ctx, &callback, vec![]), err, len) < 0 {
            -1
        } else {
            1
        }
    })
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::ui::player) unsafe fn vm_apply_settings(
    pointer: *mut Host,
    _locale: *const c_char,
    _locale_len: usize,
    _scale: f64,
    _dark: i32,
    _reduce: i32,
    _bold: i32,
    _contrast: i32,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("settings", 0);
    let callbacks = host.refs.borrow().settings.clone();
    host.with(|ctx| {
        #[allow(
            clippy::redundant_closure_call,
            reason = "the closure is the `?` boundary for a function that returns a status code"
        )]
        let result = (|| {
            let settings = bindings::settings(host, &ctx)?;
            for (_, callback) in callbacks {
                host.call(&ctx, &callback, vec![settings.clone()])?;
            }
            Ok(())
        })();
        host.finish(result, &ctx, err, len)
    })
}
pub(in crate::ui::player) unsafe fn vm_apply_window_metrics(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let callbacks = host.refs.borrow().window_metrics.clone();
    if callbacks.is_empty() {
        return 0;
    }
    let _budget = host.budget("window metrics", 0);
    host.with(|ctx| {
        let result = bindings::window_metrics(host, &ctx).and_then(|metrics| {
            for (_, callback) in callbacks {
                host.call(&ctx, &callback, vec![metrics.clone()])?;
            }
            Ok(())
        });
        host.finish(result, &ctx, err, len)
    })
}

pub(in crate::ui::player) unsafe fn vm_motion_end(
    pointer: *mut Host,
    node: u64,
    prop: *const c_char,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("motion", node);
    let prop = cstr(prop);
    let callbacks: Vec<_> = host
        .refs
        .borrow()
        .motions
        .iter()
        .filter(|(_, n, p, _)| *n == node && *p == prop)
        .map(|(_, _, _, f)| f.clone())
        .collect();
    host.with(|ctx| {
        let mut failed = 0;
        for callback in callbacks {
            if let Err(error) = host.call(&ctx, &callback, vec![]) {
                if failed == 0 {
                    host.stash(&ctx, error);
                    host.write_error(err, len);
                    failed = -1;
                } else if ctx.has_exception() {
                    drop(ctx.catch());
                }
            }
        }
        failed
    })
}

pub(in crate::ui::player) unsafe fn vm_set_clock(pointer: *mut Host, mono: i64, paused: i32) {
    let host = host(pointer);
    host.runtime.set_clock(mono);
    host.runtime.begin_timer_turn();
    host.paused.set(paused != 0);
}

fn stash_first(
    host: &Host,
    ctx: &Ctx<'_>,
    error: rquickjs::Error,
    failed: &mut bool,
    out: *mut c_char,
    len: usize,
) {
    if *failed {
        if ctx.has_exception() {
            drop(ctx.catch());
        }
    } else {
        host.stash(ctx, error);
        host.write_error(out, len);
        *failed = true;
    }
}

pub(in crate::ui::player) unsafe fn vm_fire_timers(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    if host.paused.get() {
        return 0;
    }
    let mut failed = false;
    let result = host.runtime.run_due_timers(|ctx, error| {
        stash_first(host, ctx, error, &mut failed, err, len);
        true
    });
    if let Err(error) = result
        && !failed
    {
        host.runtime
            .extension()
            .record_error(error.message, String::new());
        host.write_error(err, len);
        failed = true;
    }
    if host.runtime.due_timer_next_turn() {
        host.wants_frame.set(true);
    }
    -i32::from(failed)
}

pub(in crate::ui::player) unsafe fn vm_fire_raf(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    if host.paused.get() {
        return 0;
    }
    let _budget = host.budget("raf", 0);
    let mut failed = false;
    let turn = host.runtime.timer_turn();
    loop {
        let callback =
            crate::runtime::quickjs::poll_timer(&mut host.raf.borrow_mut(), true, turn, 0);
        let Some((function, arguments)) = callback else {
            break;
        };
        host.with(|ctx| {
            let result = arguments
                .into_iter()
                .map(|value| value.restore(&ctx))
                .collect::<rquickjs::Result<Vec<_>>>()
                .and_then(|values| host.call(&ctx, &function, values));
            if let Err(error) = result {
                stash_first(host, &ctx, error, &mut failed, err, len);
            }
        });
    }
    host.raf.borrow_mut().retain(|timer| timer.active);
    -i32::from(failed)
}

pub(in crate::ui::player) unsafe fn vm_needs_frame(pointer: *mut Host) -> i32 {
    let host = host(pointer);
    i32::from(
        !host.paused.get()
            && (host.raf.borrow().iter().any(|timer| timer.active)
                || host.runtime.due_timer_next_turn()),
    )
}

pub(in crate::ui::player) unsafe fn vm_next_deadline(pointer: *mut Host) -> i64 {
    let host = host(pointer);
    if host.paused.get() {
        return -1;
    }
    host.runtime
        .next_deadline_ms()
        .filter(|deadline| *deadline > host.runtime.clock_ms())
        .unwrap_or(-1)
}
