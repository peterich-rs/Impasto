//! JavaScript argument decoding. Tree writes continue through player::mutation.
use super::super::{bridge, call, mutation};
use super::*;
use crate::ui::abi::MutationOp;
use rquickjs::{Array, Coerced, FromJs, function::Rest};
use uicore::style::codec::Value as StyleValue;

pub(super) const GESTURES: &[(&str, i32)] = &[
    ("onClick", 0),
    ("onPress", 1),
    ("onRelease", 2),
    ("onDragBegin", 3),
    ("onDragBy", 4),
    ("onDragEnd", 5),
    ("onFocus", 6),
    ("onBlur", 7),
    ("onKey", 8),
    ("onText", 9),
    ("onComposing", 10),
    ("onHover", 11),
    ("onLeave", 12),
    ("onWheel", 13),
    ("onSecondaryClick", 14),
    ("onScroll", 15),
    ("onTextReplaced", 16),
    ("onLink", 17),
    ("onLongPress", 18),
    ("onZoom", 20),
];
pub(super) const CORE_NAMES: &[&str] = &[
    "_mutate",
    "_query",
    "_effect",
    "_reactiveCall",
    "setStyleEnvironment",
    "createStyleSource",
    "setStyleSource",
    "removeStyleSource",
    "setClass",
    "setStyle",
    "patchStyle",
    "clearStyle",
    "setComponentStyle",
    "onScrollState",
    "applyMutation",
    "requestFrame",
    "onFrame",
    "onLayout",
    "onBack",
    "offBack",
    "onMotionEnd",
    "offMotionEnd",
    "onShortcut",
    "setListenerErrorPolicy",
    "log",
    "debugLayoutIssues",
    "debugSnapshot",
    "debugProbe",
    "debugLastPointer",
    "debugSetScan",
    "debugSetPointerTrace",
    "debugSetOwned",
    "debugMemory",
    "settings",
    "onSettings",
    "offSettings",
    "listDriver",
    "scrollController",
    "onDriver",
];
pub(super) fn value<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> Value<'js> {
    args.get(index)
        .cloned()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()))
}
pub(super) fn num<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<f64> {
    Coerced::<f64>::from_js(ctx, value(ctx, args, index)).map(|v| v.0)
}
fn optional_num<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<f64> {
    if args.get(index).is_none_or(Value::is_undefined) {
        Ok(0.0)
    } else {
        num(ctx, args, index)
    }
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(super) fn handle<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<u64> {
    let val = value(ctx, args, index);
    if val.is_null() || val.is_undefined() {
        return Ok(0);
    }
    let n = num(ctx, args, index)?;
    if !(0.0..9007199254740992.0).contains(&n) || n.fract() != 0.0 {
        return Err(type_error(ctx, "bad handle"));
    }
    Ok(n as u64)
}
pub(super) fn string<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<String> {
    Coerced::<String>::from_js(ctx, value(ctx, args, index)).map(|v| v.0)
}
pub(super) fn boolean<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<bool> {
    Coerced::<bool>::from_js(ctx, value(ctx, args, index)).map(|v| v.0)
}
fn color<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> rquickjs::Result<[u8; 4]> {
    let Some(array) = value.as_array() else {
        return Ok([0; 4]);
    };
    let mut color = [0; 4];
    for (i, c) in color.iter_mut().enumerate() {
        let v: Coerced<i32> = array.get(i)?;
        *c = u8::try_from(v.0).map_err(|_| type_error(ctx, "bad color"))?;
    }
    Ok(color)
}
pub(super) fn empty_record(op: u32) -> mutation::OwnedRec {
    mutation::OwnedRec::from_queued(op, 0, 0, 0, 0, 0.0, 0.0, [0; 4], String::new())
}
pub(super) fn apply(
    host: &Host,
    ctx: &Ctx<'_>,
    record: &mutation::OwnedRec,
) -> rquickjs::Result<u64> {
    mutation::mutate(host.inner(), record).map_err(|_| type_error(ctx, "invalid scene mutation"))
}
fn result_object<'js>(ctx: &Ctx<'js>, out: &call::CallOut) -> rquickjs::Result<Value<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("ok", out.status == 0)?;
    object.set("id", out.handle as f64)?;
    let array = Array::new(ctx.clone())?;
    for (i, n) in out
        .nums
        .iter()
        .take(out.nnums.clamp(0, 128) as usize)
        .enumerate()
    {
        array.set(i, *n)?;
    }
    object.set("n", array)?;
    // SAFETY: player::call keeps the returned text in session storage until the next query.
    object.set(
        "t",
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        unsafe { super::super::utf8_of(out.text, out.text_len) }.unwrap_or(""),
    )?;
    Ok(object.into_value())
}
fn output() -> call::CallOut {
    call::CallOut {
        status: 0,
        nnums: 0,
        handle: 0,
        nums: [0.0; 128],
        text: ptr::null(),
        text_len: 0,
    }
}

pub(super) fn bind<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &'static str,
) -> rquickjs::Result<()> {
    let pointer = ptr::from_ref(host).cast_mut();
    let function = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        // SAFETY: VM functions are destroyed before Engine releases Host.
        let host = unsafe { &*pointer };
        host.with_current(&ctx, || native(host, &ctx, name, &args.0))
    })?;
    function.set_name(name)?;
    object.set(name, function)
}
pub(super) fn install<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let core = Object::new(ctx.clone())?;
    for name in CORE_NAMES {
        bind(host, ctx, &core, name)?;
    }
    for (name, _) in GESTURES {
        bind(host, ctx, &core, name)?;
    }
    canvas::install_exports(host, ctx, &core)?;
    host.refs.borrow_mut().scene = Some(Persistent::save(ctx, core.clone().into_value()));
    ctx.globals().set("__scene", core)?;
    let pointer = ptr::from_ref(host).cast_mut();
    host.runtime.install_standard_bindings(ctx, move || {
        // SAFETY: the installed callbacks die before the Engine drops Host.
        unsafe { (*pointer).wants_frame.set(true) };
    })?;
    for name in ["requestAnimationFrame", "cancelAnimationFrame"] {
        bind(host, ctx, &ctx.globals(), name)?;
    }
    host.runtime
        .install_host(ctx, canvas::side_effect, move |name| {
            // SAFETY: the installed callbacks die before the Engine drops Host.
            unsafe { &*pointer }
                .inner()
                .config
                .registry
                .module(name)
                .is_some()
        })?;
    let config = &host.inner().config;
    host.runtime.install_modules(
        ctx,
        crate::runtime::quickjs::binder::ModuleBindings::new(
            config.registry.as_host_registry().clone(),
            Arc::clone(&config.exec),
        ),
        canvas::side_effect,
        Some(Arc::clone(&config.settle)),
        config.post_delivery.clone(),
    )?;
    rquickjs::Module::declare_def::<modules::GraphModule, _>(ctx.clone(), "#reactive")?;
    Ok(())
}

fn saved_function<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], i: usize) -> rquickjs::Result<Saved> {
    let value = value(ctx, args, i);
    if !value.is_function() {
        return Err(type_error(ctx, "callback must be a function"));
    }
    Ok(Persistent::save(ctx, value))
}
pub(super) fn style_snapshot<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    op: MutationOp,
    id: u64,
    origin: u64,
    replace: bool,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let decoded = decode_style(ctx, value, 0, &mut 0)?;
    let mut record = empty_record(op as u32);
    record.id = id;
    record.parent = origin;
    record.number = f64::from(replace);
    record.style_value = Some(decoded);
    apply(host, ctx, &record)?;
    Ok(())
}
fn decode_style<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    depth: usize,
    count: &mut usize,
) -> rquickjs::Result<StyleValue> {
    if depth > 64 || *count >= 65536 {
        return Err(rquickjs::Exception::throw_range(
            ctx,
            "style value is too large",
        ));
    }
    *count += 1;
    Ok(if value.is_undefined() {
        StyleValue::Undefined
    } else if value.is_null() {
        StyleValue::Null
    } else if let Some(n) = value.as_number() {
        StyleValue::Number(n)
    } else if let Some(b) = value.as_bool() {
        StyleValue::Bool(b)
    } else if let Some(s) = value.as_string() {
        StyleValue::String(s.to_string()?)
    } else if let Some(array) = value.as_array() {
        if array.len() > 65536 {
            return Err(type_error(ctx, "style value is too large"));
        }
        let mut values = Vec::new();
        for item in array.iter::<Value>() {
            values.push(decode_style(ctx, item?, depth + 1, count)?);
        }
        StyleValue::Array(values)
    } else if value.is_object() && !value.is_function() {
        let mut values = Vec::new();
        let object = Object::from_value(value)?;
        for key in object.keys::<String>() {
            let key = key?;
            let item = object.get(key.as_str())?;
            values.push((key, decode_style(ctx, item, depth + 1, count)?));
        }
        StyleValue::Object(values)
    } else {
        return Err(type_error(ctx, "unsupported style value"));
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(super) fn native<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    canvas::side_effect(ctx)?;
    let undefined = || Value::new_undefined(ctx.clone());
    let minimum = match name {
        "setStyle" | "patchStyle" | "setComponentStyle" | "setClass" | "setStyleSource"
        | "createStyleSource" | "http" | "httpStream" => 2,
        "clearStyle"
        | "removeStyleSource"
        | "setStyleEnvironment"
        | "socketOpen"
        | "offBack"
        | "offSettings" => 1,
        _ if GESTURES.iter().any(|(n, _)| *n == name) => 2,
        _ => 0,
    };
    if args.len() < minimum {
        return Err(type_error(ctx, name));
    }
    if let Some((_, kind)) = GESTURES.iter().find(|(n, _)| *n == name) {
        let id = handle(ctx, args, 0)?;
        let value = value(ctx, args, 1);
        if value.is_null() || value.is_undefined() {
            host.refs.borrow_mut().callbacks.remove(&(id, *kind));
            return Ok(undefined());
        }
        let callback = saved_function(ctx, args, 1)?;
        let mut record = empty_record(MutationOp::PointerTarget as u32);
        record.id = id;
        apply(host, ctx, &record)?;
        host.refs
            .borrow_mut()
            .callbacks
            .insert((id, *kind), callback);
        return Ok(undefined());
    }
    match name {
        "_mutate" => {
            let mut record = empty_record(num(ctx, args, 0)? as u32);
            record.id = num(ctx, args, 1)? as u64;
            record.parent = num(ctx, args, 2)? as u64;
            record.before = num(ctx, args, 3)? as u64;
            record.field = num(ctx, args, 4)? as u32;
            record.number = num(ctx, args, 5)?;
            record.number2 = num(ctx, args, 6)?;
            record.number3 = num(ctx, args, 7)?;
            record.number4 = num(ctx, args, 8)?;
            record.color = color(ctx, &value(ctx, args, 9))?;
            if value(ctx, args, 10).is_string() {
                record.text = string(ctx, args, 10)?;
            }
            let result = mutation::mutate(host.inner(), &record);
            let mut out = output();
            match result {
                Ok(id) => out.handle = id,
                Err(_) => out.status = -1,
            };
            return result_object(ctx, &out);
        }
        "_query" => {
            let kind = num(ctx, args, 0)? as i32;
            let node = optional_num(ctx, args, 1)? as u64;
            let a = optional_num(ctx, args, 2)?;
            let b = optional_num(ctx, args, 3)?;
            let text = if value(ctx, args, 4).is_string() {
                string(ctx, args, 4)?
            } else {
                String::new()
            };
            let color = color(ctx, &value(ctx, args, 5))?;
            let mut out = output();
            // SAFETY: the session and all string/color/output buffers remain alive for this call.
            unsafe {
                call::query(
                    host.session(),
                    kind,
                    node,
                    a,
                    b,
                    text.as_ptr().cast(),
                    text.len(),
                    color.as_ptr(),
                    &mut out,
                );
            }
            return result_object(ctx, &out);
        }
        "_effect" => {
            let kind = num(ctx, args, 0)? as i32;
            let node = optional_num(ctx, args, 1)? as u64;
            let mut nums = Vec::new();
            if let Some(array) = value(ctx, args, 2).as_array() {
                for n in array.iter::<Coerced<f64>>().take(16) {
                    nums.push(n?.0);
                }
            }
            let text = if value(ctx, args, 3).is_string() {
                string(ctx, args, 3)?
            } else {
                String::new()
            };
            let color_value = value(ctx, args, 4);
            let color = color(ctx, &color_value)?;
            let data = read_bytes(ctx, &value(ctx, args, 5))?.unwrap_or_default();
            let mut out = output();
            // SAFETY: all inputs are owned buffers; player::call borrows them only through this call.
            unsafe {
                call::effect(
                    host.session(),
                    kind,
                    node,
                    nums.as_ptr(),
                    nums.len() as i32,
                    if color_value.is_array() {
                        color.as_ptr()
                    } else {
                        ptr::null()
                    },
                    text.as_ptr().cast(),
                    text.len(),
                    data.as_ptr(),
                    data.len(),
                    &mut out,
                );
            }
            return result_object(ctx, &out);
        }
        "applyMutation" => {
            let op = num(ctx, args, 0)? as u32;
            let id = handle(ctx, args, 1)?;
            let parent = handle(ctx, args, 2)?;
            let before = handle(ctx, args, 3)?;
            let field = num(ctx, args, 4)? as u32;
            let n = num(ctx, args, 5)?;
            let n2 = num(ctx, args, 6)?;
            let color = color(ctx, &value(ctx, args, 7))?;
            let text = if value(ctx, args, 8).is_string() {
                string(ctx, args, 8)?
            } else {
                String::new()
            };
            // SAFETY: the callback copies text and color before returning.
            let status = unsafe {
                mutation::push_mutation(
                    host.session(),
                    op,
                    id,
                    parent,
                    before,
                    field,
                    n,
                    n2,
                    color.as_ptr(),
                    text.as_ptr().cast(),
                    text.len(),
                )
            };
            if status != 0 {
                return Err(type_error(ctx, "applyMutation failed"));
            }
        }
        "_reactiveCall" => {
            let name = string(ctx, args, 0)?;
            return host.api(ctx, true, &name, args[1..].to_vec());
        }
        "setStyle" | "patchStyle" | "setComponentStyle" | "setClass" | "setStyleSource"
        | "clearStyle" => {
            let id = handle(ctx, args, 0)?;
            let (op, origin, replace) = match name {
                "setClass" => (MutationOp::ClassSnapshot, 0, true),
                "setStyleSource" => (MutationOp::StyleSourceSnapshot, 0, true),
                "clearStyle" => (MutationOp::StyleClear, 0, false),
                "setComponentStyle" => (MutationOp::StyleSnapshot, 0, true),
                _ => (MutationOp::StyleSnapshot, 3, name != "patchStyle"),
            };
            let val = if name == "clearStyle" && args.len() < 2 {
                Value::new_null(ctx.clone())
            } else {
                value(ctx, args, 1)
            };
            style_snapshot(host, ctx, op, id, origin, replace, val)?;
        }
        "setStyleEnvironment" => {
            style_snapshot(
                host,
                ctx,
                MutationOp::StyleEnvironment,
                0,
                0,
                true,
                value(ctx, args, 0),
            )?;
        }
        "createStyleSource" => {
            let origin = string(ctx, args, 1)?;
            let origin = ["component", "classes", "inline", "imperative", "animation"]
                .iter()
                .position(|s| *s == origin)
                .ok_or_else(|| type_error(ctx, "unknown style origin"))?;
            let mut record = empty_record(MutationOp::StyleSourceCreate as u32);
            record.id = handle(ctx, args, 0)?;
            record.parent = origin as u64;
            return (apply(host, ctx, &record)? as f64).into_js(ctx);
        }
        "removeStyleSource" => {
            let mut record = empty_record(MutationOp::StyleSourceRemove as u32);
            record.id = handle(ctx, args, 0)?;
            apply(host, ctx, &record)?;
        }
        "onFrame" | "onLayout" | "onScrollState" | "onDriver" => {
            let callback = saved_function(ctx, args, 0)?;
            let key = match name {
                "onFrame" => "frame",
                "onLayout" => "layout",
                "onDriver" => "driver",
                _ => "scroll",
            };
            host.refs.borrow_mut().hooks.insert(key, callback);
        }
        "requestFrame" => {
            host.wants_frame.set(true);
        }
        "onBack" | "onSettings" => {
            let callback = saved_function(ctx, args, 0)?;
            let id = host
                .runtime
                .allocate_timer_id()
                .ok_or_else(|| rquickjs::Exception::throw_range(ctx, "callback IDs exhausted"))?;
            let mut refs = host.refs.borrow_mut();
            if name == "onBack" {
                refs.backs.push((id, callback));
            } else {
                refs.settings.push((id, callback));
            }
            return (id as f64).into_js(ctx);
        }
        "offBack" | "offSettings" => {
            let id = num(ctx, args, 0)? as u64;
            let mut refs = host.refs.borrow_mut();
            if name == "offBack" {
                refs.backs.retain(|(i, _)| *i != id);
            } else {
                refs.settings.retain(|(i, _)| *i != id);
            }
        }
        "onMotionEnd" => {
            if args.len() < 3 || !args[1].is_string() {
                return Err(type_error(ctx, "onMotionEnd(id, prop, function)"));
            }
            let node = handle(ctx, args, 0)?;
            let prop = string(ctx, args, 1)?;
            let callback = saved_function(ctx, args, 2)?;
            let id = host
                .runtime
                .allocate_timer_id()
                .ok_or_else(|| rquickjs::Exception::throw_range(ctx, "callback IDs exhausted"))?;
            host.refs
                .borrow_mut()
                .motions
                .push((id, node, prop, callback));
            return (id as f64).into_js(ctx);
        }
        "offMotionEnd" => {
            let id = optional_num(ctx, args, 0)? as u64;
            host.refs
                .borrow_mut()
                .motions
                .retain(|(i, _, _, _)| *i != id);
        }
        "onShortcut" => {
            let spec = string(ctx, args, 0)?.to_ascii_lowercase();
            if spec.is_empty() {
                return Err(type_error(ctx, "onShortcut(spec, function)"));
            }
            let callback = saved_function(ctx, args, 1)?;
            let mut refs = host.refs.borrow_mut();
            if let Some((_, f)) = refs.shortcuts.iter_mut().find(|(s, _)| *s == spec) {
                *f = callback;
            } else {
                refs.shortcuts.push((spec, callback));
            }
        }
        "setListenerErrorPolicy" => match string(ctx, args, 0)?.as_str() {
            "freeze" => host.freeze.set(true),
            "report" => host.freeze.set(false),
            _ => return Err(type_error(ctx, "setListenerErrorPolicy(policy)")),
        },
        "log" => {
            if args.len() < 2 {
                return Ok(undefined());
            }
            let level = string(ctx, args, 0)?;
            let message = string(ctx, args, 1)?;
            (host.inner().log)(
                match level.as_str() {
                    "error" => 3,
                    "warn" => 2,
                    "debug" => 4,
                    _ => 1,
                },
                &message,
            );
        }
        "settings" => return settings(host, ctx),
        "listDriver" => return drivers::list(host, ctx, args),
        "scrollController" => return drivers::scroll(host, ctx),
        "debugMemory" => {
            use rquickjs::qjs;
            let mut usage = std::mem::MaybeUninit::<qjs::JSMemoryUsage>::uninit();
            // SAFETY: the current context holds the runtime lock. QuickJS initializes
            // the usage struct and retains neither pointer.
            let usage = unsafe {
                qjs::JS_ComputeMemoryUsage(
                    qjs::JS_GetRuntime(ctx.as_raw().as_ptr()),
                    usage.as_mut_ptr(),
                );
                usage.assume_init()
            };
            let (malloc, objects, atoms, strings) = (
                usage.malloc_size,
                usage.memory_used_count,
                usage.atom_count,
                usage.str_count,
            );
            return format!("{{\"mallocBytes\":{malloc},\"objects\":{objects},\"atoms\":{atoms},\"strings\":{strings}}}").into_js(ctx);
        }
        "debugLayoutIssues" | "debugSnapshot" | "debugProbe" | "debugLastPointer" => {
            let what = match name {
                "debugLayoutIssues" => 0,
                "debugSnapshot" => 1,
                "debugProbe" => 2,
                _ => 3,
            };
            let node = handle(ctx, args, 0)?;
            let x = optional_num(ctx, args, 1)? as f32;
            let y = optional_num(ctx, args, 2)? as f32;
            // SAFETY: session storage remains valid while copying its JSON string.
            unsafe {
                if bridge::debug_json(host.session(), what, node, x, y) != 0 {
                    return Ok(Value::new_null(ctx.clone()));
                }
            }
            let text = host
                .inner()
                .debug_json
                .borrow()
                .as_ref()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            return text.into_js(ctx);
        }
        "debugSetScan" | "debugSetPointerTrace" | "debugSetOwned" => {
            let a = boolean(ctx, args, 0)?;
            let b = boolean(ctx, args, 1)?;
            let id = if name == "debugSetOwned" {
                handle(ctx, args, 0)?
            } else {
                0
            };
            // SAFETY: the callback receives a live session and scalar flags.
            unsafe {
                match name {
                    "debugSetScan" => bridge::debug_set_scan(host.session(), i32::from(a)),
                    "debugSetPointerTrace" => {
                        bridge::debug_set_pointer_trace(host.session(), i32::from(a), i32::from(b))
                    }
                    _ => bridge::debug_set_owned(host.session(), id, i32::from(b)),
                }
            }
        }
        "requestAnimationFrame" => {
            let function = saved_function(ctx, args, 0)?;
            let _ = optional_num(ctx, args, 1)?;
            let id = host
                .runtime
                .allocate_timer_id()
                .ok_or_else(|| rquickjs::Exception::throw_range(ctx, "callback IDs exhausted"))?;
            let extra = args
                .iter()
                .skip(2)
                .take(8)
                .cloned()
                .map(|v| Persistent::save(ctx, v))
                .collect();
            host.raf.borrow_mut().push(Timer {
                id,
                due: 0,
                interval: 0,
                eligible_turn: host.runtime.timer_turn().saturating_add(1),
                kind: 2,
                function,
                extra,
                active: true,
            });
            host.wants_frame.set(true);
            return (id as f64).into_js(ctx);
        }
        "cancelAnimationFrame" => {
            let id = optional_num(ctx, args, 0).unwrap_or(0.0) as u64;
            let kind = 2;
            host.raf
                .borrow_mut()
                .retain(|timer| timer.id != id || timer.kind != kind);
            if ctx.has_exception() {
                drop(ctx.catch());
            }
        }
        _ => return Err(type_error(ctx, "unknown native operation")),
    }
    Ok(undefined())
}

pub(super) fn settings<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let settings = host.inner().platform.borrow();
    let object = Object::new(ctx.clone())?;
    object.set("locale", settings.locale.as_str())?;
    object.set("textScale", settings.text_scale)?;
    object.set(
        "brightness",
        if settings.brightness == crate::session::Brightness::Dark {
            "dark"
        } else {
            "light"
        },
    )?;
    object.set("reduceMotion", settings.reduce_motion)?;
    object.set("boldText", settings.bold_text)?;
    object.set("highContrast", settings.high_contrast)?;
    Ok(object.into_value())
}
fn bytes_of<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> rquickjs::Result<Option<Vec<u8>>> {
    if let Some(buffer) = value.as_object().and_then(|o| o.as_array_buffer()) {
        // SAFETY: copy the borrowed buffer before any JavaScript can detach or resize it.
        return unsafe { buffer.as_bytes() }
            .map(|b| Some(b.to_vec()))
            .ok_or_else(|| type_error(ctx, "detached buffer"));
    }
    let view = host_from_ctx(ctx)?.api(ctx, false, "_bytesView", vec![value.clone()])?;
    let Some(array) = view.as_object().and_then(|o| o.as_typed_array::<u8>()) else {
        return Ok(None);
    };
    // SAFETY: the intrinsic returned a Uint8Array view; no JS runs while its bytes are copied.
    unsafe { array.as_bytes() }
        .map(|b| Some(b.to_vec()))
        .ok_or_else(|| type_error(ctx, "detached buffer"))
}
pub(super) fn read_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> rquickjs::Result<Option<Vec<u8>>> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    bytes_of(ctx, value)?
        .map(Some)
        .ok_or_else(|| type_error(ctx, "expected bytes"))
}
