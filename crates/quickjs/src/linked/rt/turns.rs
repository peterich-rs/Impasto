//! Session turns, timers and asynchronous host completions.
use super::*;
use rquickjs::{ArrayBuffer, FromJs};

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
fn bytes_at(pointer: *const u8, len: usize) -> Vec<u8> {
    if pointer.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: session borrows this initialized buffer for the duration of the call.
    unsafe { std::slice::from_raw_parts(pointer, len) }.to_vec()
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

pub(in crate::linked) unsafe fn vm_dispatch(
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
pub(in crate::linked) unsafe fn vm_dispatch_input(
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
pub(in crate::linked) unsafe fn vm_dispatch_text(
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
pub(in crate::linked) unsafe fn vm_frame(
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
pub(in crate::linked) unsafe fn vm_layout(pointer: *mut Host, err: *mut c_char, len: usize) -> i32 {
    hook_turn(host(pointer), "layout", |_| Ok(vec![]), err, len)
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::linked) unsafe fn vm_scroll_state(
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
pub(in crate::linked) unsafe fn vm_driver(
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
pub(in crate::linked) unsafe fn vm_run_updates(
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
pub(in crate::linked) unsafe fn vm_drain_jobs(
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
pub(in crate::linked) unsafe fn vm_jobs_pending(pointer: *mut Host) -> i32 {
    i32::from(
        host(pointer)
            .runtime
            .as_ref()
            .is_some_and(Runtime::is_job_pending),
    )
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(in crate::linked) unsafe fn vm_tap_count(pointer: *mut Host) -> i32 {
    host(pointer)
        .refs
        .borrow()
        .callbacks
        .keys()
        .map(|(id, _)| *id)
        .collect::<std::collections::HashSet<_>>()
        .len() as i32
}
pub(in crate::linked) unsafe fn vm_forget_ids(pointer: *mut Host, ids: *const u64, len: usize) {
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
pub(in crate::linked) unsafe fn vm_take_wants_frame(pointer: *mut Host) -> i32 {
    i32::from(host(pointer).wants_frame.replace(false))
}
pub(in crate::linked) unsafe fn vm_dispatch_back(
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
pub(in crate::linked) unsafe fn vm_try_shortcut(
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
pub(in crate::linked) unsafe fn vm_apply_settings(
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
pub(in crate::linked) unsafe fn vm_motion_end(
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

pub(in crate::linked) unsafe fn vm_set_clock(pointer: *mut Host, mono: i64, paused: i32) {
    let host = host(pointer);
    host.mono.set(mono);
    host.paused.set(paused != 0);
    host.frame.set(host.frame.get() + 1);
    if host.origin.get().is_none() {
        host.origin.set(Some(mono));
    }
}
fn fire(host: &Host, raf: bool, err: *mut c_char, len: usize) -> i32 {
    if host.paused.get() {
        return 0;
    }
    let _budget = host.budget(if raf { "raf" } else { "timer" }, 0);
    let mut failed = 0;
    for _ in 0..if raf { usize::MAX } else { 64 } {
        let callback = {
            let mut timers = host.timers.borrow_mut();
            let Some(timer) = timers.iter_mut().find(|timer| {
                timer.active
                    && (timer.kind == 2) == raf
                    && timer.min_frame <= host.frame.get()
                    && (raf || timer.due <= host.mono.get())
            }) else {
                break;
            };
            let callback = (timer.function.clone(), timer.extra.clone());
            if timer.kind == 1 {
                timer.due = host.mono.get().saturating_add(timer.interval.max(0));
                timer.min_frame = host.frame.get() + 1;
            } else {
                timer.active = false;
            }
            callback
        };
        host.with(|ctx| {
            let result = (|| {
                let values = callback
                    .1
                    .into_iter()
                    .take(8)
                    .map(|v| v.restore(&ctx))
                    .collect::<rquickjs::Result<Vec<_>>>()?;
                host.call(&ctx, &callback.0, values).map(|_| ())
            })();
            if let Err(error) = result {
                if failed == 0 {
                    host.stash(&ctx, error);
                    host.write_error(err, len);
                    failed = -1;
                } else if ctx.has_exception() {
                    drop(ctx.catch());
                }
            }
        });
    }
    host.timers.borrow_mut().retain(|timer| timer.active);
    if !raf
        && host
            .timers
            .borrow()
            .iter()
            .any(|t| t.kind != 2 && t.min_frame <= host.frame.get() && t.due <= host.mono.get())
    {
        host.wants_frame.set(true);
    }
    failed
}
pub(in crate::linked) unsafe fn vm_fire_timers(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    fire(host(pointer), false, err, len)
}
pub(in crate::linked) unsafe fn vm_fire_raf(
    pointer: *mut Host,
    err: *mut c_char,
    len: usize,
) -> i32 {
    fire(host(pointer), true, err, len)
}
pub(in crate::linked) unsafe fn vm_needs_frame(pointer: *mut Host) -> i32 {
    let h = host(pointer);
    i32::from(
        !h.paused.get()
            && h.timers.borrow().iter().any(|t| {
                t.active
                    && (t.kind == 2 || t.min_frame <= h.frame.get() + 1 && t.due <= h.mono.get())
            }),
    )
}
pub(in crate::linked) unsafe fn vm_next_deadline(pointer: *mut Host) -> i64 {
    let h = host(pointer);
    if h.paused.get() {
        return -1;
    }
    h.timers
        .borrow()
        .iter()
        .filter(|t| t.active && t.kind != 2 && t.due > h.mono.get())
        .map(|t| t.due)
        .min()
        .unwrap_or(-1)
}

impl Host {
    pub(super) fn make_promise<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<(u64, Value<'js>)> {
        let (promise, resolve, reject) = ctx.promise()?;
        let token = self.token.get().wrapping_add(1).max(1);
        self.token.set(token);
        self.refs.borrow_mut().promises.insert(
            token,
            (
                Persistent::save(ctx, resolve.into_value()),
                Persistent::save(ctx, reject.into_value()),
            ),
        );
        Object::from_value(promise.as_value().clone())?.set("token", token as f64)?;
        Ok((token, promise.into_value()))
    }
    pub(super) fn settle<'js>(
        &self,
        ctx: &Ctx<'js>,
        token: u64,
        ok: bool,
        value: Value<'js>,
    ) -> rquickjs::Result<()> {
        let Some((resolve, reject)) = self.refs.borrow_mut().promises.remove(&token) else {
            return Ok(());
        };
        self.call(ctx, if ok { &resolve } else { &reject }, vec![value])
            .map(|_| ())
    }
    fn host_hook<'js>(
        &self,
        ctx: &Ctx<'js>,
        name: &str,
        args: Vec<Value<'js>>,
    ) -> rquickjs::Result<()> {
        let bridge: Object = ctx.globals().get("__host")?;
        let hook: Value = bridge.get(name)?;
        if let Some(hook) = hook.into_function() {
            let mut values = Args::new(ctx.clone(), args.len());
            values.push_args(args)?;
            hook.call_arg::<()>(values)?;
        }
        Ok(())
    }
}
pub(in crate::linked) fn push_plugin(pointer: *mut Host, value: &session::PluginValue) {
    let host = host(pointer);
    host.with(|ctx| {
        if let Ok(value) = bindings::plugin_to_js(&ctx, value) {
            host.refs
                .borrow_mut()
                .values
                .push(Persistent::save(&ctx, value));
        }
    });
}
pub(in crate::linked) unsafe fn vm_settle_top(pointer: *mut Host, token: u64, ok: i32) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("http", 0);
    let value = host.refs.borrow_mut().values.pop();
    host.with(|ctx| {
        let result = (|| {
            let value = match value {
                Some(v) => v.restore(&ctx)?,
                None => Value::new_null(ctx.clone()),
            };
            host.settle(&ctx, token, ok != 0, value)
        })();
        if result.is_err() {
            drop(ctx.catch());
            -1
        } else {
            0
        }
    })
}
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(in crate::linked) unsafe fn vm_settle_http(
    pointer: *mut Host,
    token: u64,
    status: i32,
    headers: *const c_char,
    headers_len: usize,
    body: *const u8,
    body_len: usize,
    error: *const c_char,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("http", 0);
    host.with(|ctx| {
        let result = (|| {
            let value = if !error.is_null() {
                text(&ctx, &cstr(error))?
            } else {
                let object = Object::new(ctx.clone())?;
                object.set("status", status)?;
                object.set("headers", string_at(headers, headers_len))?;
                object.set(
                    "body",
                    ArrayBuffer::new(ctx.clone(), bytes_at(body, body_len))?,
                )?;
                object.into_value()
            };
            host.settle(&ctx, token, error.is_null(), value)
        })();
        if result.is_err() {
            drop(ctx.catch());
            -1
        } else {
            0
        }
    })
}
pub(in crate::linked) unsafe fn vm_http_head(
    pointer: *mut Host,
    token: u64,
    status: i32,
    headers: *const c_char,
    headers_len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("http", 0);
    host.with(|ctx| {
        let result = (|| {
            let head = Object::new(ctx.clone())?;
            head.set("status", status)?;
            head.set("headers", string_at(headers, headers_len))?;
            host.host_hook(
                &ctx,
                "_httpStream",
                vec![
                    number(&ctx, token as f64),
                    text(&ctx, "head")?,
                    head.into_value(),
                ],
            )
        })();
        if result.is_err() {
            drop(ctx.catch());
            -1
        } else {
            0
        }
    })
}
pub(in crate::linked) unsafe fn vm_http_chunk(
    pointer: *mut Host,
    token: u64,
    body: *const u8,
    body_len: usize,
    done: i32,
    error: *const c_char,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("http", 0);
    host.with(|ctx| {
        let result = (|| {
            let payload = if !error.is_null() {
                text(&ctx, &cstr(error))?
            } else {
                ArrayBuffer::new(ctx.clone(), bytes_at(body, body_len))?.into_value()
            };
            host.host_hook(
                &ctx,
                "_httpStream",
                vec![
                    number(&ctx, token as f64),
                    text(&ctx, if done != 0 { "done" } else { "chunk" })?,
                    payload,
                ],
            )
        })();
        if done != 0 {
            host.refs.borrow_mut().promises.remove(&token);
        }
        if result.is_err() {
            drop(ctx.catch());
            -1
        } else {
            0
        }
    })
}
pub(in crate::linked) unsafe fn vm_socket_event(
    pointer: *mut Host,
    id: u64,
    event: *const c_char,
    is_text: i32,
    data: *const u8,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("socket", 0);
    host.with(|ctx| {
        let result = (|| {
            let bytes = bytes_at(data, len);
            let payload = if is_text != 0 {
                text(&ctx, &String::from_utf8_lossy(&bytes))?
            } else {
                ArrayBuffer::new(ctx.clone(), bytes)?.into_value()
            };
            host.host_hook(
                &ctx,
                "_socket",
                vec![number(&ctx, id as f64), text(&ctx, &cstr(event))?, payload],
            )
        })();
        if result.is_err() {
            drop(ctx.catch());
            -1
        } else {
            0
        }
    })
}
pub(in crate::linked) unsafe fn vm_module_event(
    pointer: *mut Host,
    module: *const c_char,
    event: *const c_char,
    shape: i32,
    text_ptr: *const c_char,
    len: usize,
) -> i32 {
    let host = host(pointer);
    let _budget = host.budget("module", 0);
    let module = cstr(module);
    let event = cstr(event);
    let text_value = string_at(text_ptr, len);
    let callbacks: Vec<_> = host
        .refs
        .borrow()
        .listeners
        .iter()
        .filter(|(m, e, _)| *m == module && *e == event)
        .map(|(_, _, f)| f.clone())
        .collect();
    host.with(|ctx| {
        for callback in callbacks {
            let result = (|| {
                let value = if shape == 1 {
                    let object = Object::new(ctx.clone())?;
                    object.set("url", text_value.clone())?;
                    object.into_value()
                } else {
                    text(&ctx, &text_value)?
                };
                host.call(&ctx, &callback, vec![value])
            })();
            if result.is_err() {
                drop(ctx.catch());
            }
        }
        0
    })
}
