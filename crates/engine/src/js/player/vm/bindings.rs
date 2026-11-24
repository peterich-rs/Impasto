//! JavaScript argument decoding. Tree writes continue through player::mutation.
use super::super::mutation::{self, Mutation};
use super::*;

mod scene;

use crate::style::codec::Value as StyleValue;
use crate::style::{StyleOrigin, StyleSourceId, StyleSurface, codec};
use crate::tree::{DragClaim, NodeId};
use rquickjs::{Array, Coerced, FromJs, function::Rest};

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
    ("observeScrollOffset", 22),
];
const CORE_NAMES: &[&str] = &[
    "_insertNode",
    "_createTextNode",
    "remove",
    "detach",
    "discard",
    "reparentChildren",
    "setText",
    "setTextRuns",
    "setImage",
    "setStyleSurface",
    "setFocusable",
    "setRepaintBoundary",
    "setTransitionSnapshot",
    "setColorFilter",
    "setBackdropMaterial",
    "setDragClaim",
    "setTextLimit",
    "setScrollOffset",
    "setScrollChain",
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
    "onScrollThreshold",
    "addScrollThreshold",
    "removeScrollThreshold",
    "bindScrollProp",
    "bindBackProgressOpacity",
    "unbindScrollProp",
    "requestFrame",
    "onFrame",
    "onLayout",
    "onBack",
    "offBack",
    "onBackGesture",
    "setBackEnabled",
    "setSemanticsHidden",
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
    "windowMetrics",
    "penSample",
    "onWindowMetrics",
    "offWindowMetrics",
    "listDriver",
    "scrollController",
    "onDriver",
];
pub(super) fn names() -> impl Iterator<Item = &'static str> {
    CORE_NAMES
        .iter()
        .chain(scene::NAMES)
        .copied()
        .chain(GESTURES.iter().map(|&(name, _)| name))
}

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
pub(super) fn handle<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<u64> {
    let val = value(ctx, args, index);
    if val.is_null() || val.is_undefined() {
        return Ok(0);
    }
    decode_handle(ctx, num(ctx, args, index)?)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "validated exact JavaScript handle"
)]
fn decode_handle(ctx: &Ctx<'_>, number: f64) -> rquickjs::Result<u64> {
    if !(0.0..9007199254740992.0).contains(&number) || number.fract() != 0.0 {
        return Err(type_error(ctx, "bad handle"));
    }
    Ok(number as u64)
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
pub(super) fn node_id<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<NodeId> {
    Ok(NodeId::from_raw(handle(ctx, args, index)?))
}

pub(super) fn apply(host: &Host, ctx: &Ctx<'_>, mutation: Mutation) -> rquickjs::Result<u64> {
    mutation::mutate(host, mutation).map_err(|_| type_error(ctx, "invalid scene mutation"))
}

type NativeCall =
    for<'js> fn(&Host, &Ctx<'js>, &str, &[Value<'js>]) -> rquickjs::Result<Value<'js>>;

fn bind<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &'static str,
    call: NativeCall,
) -> rquickjs::Result<()> {
    let function = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            canvas::check_side_effect(&host, &ctx)?;
            call(&host, &ctx, name, &args.0)
        })
    })?;
    function.set_name(name)?;
    object.set(name, function)
}

pub(super) fn install<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let core = Object::new(ctx.clone())?;
    for (names, call) in [
        (CORE_NAMES, native as NativeCall),
        (scene::NAMES, scene::call),
    ] {
        for &name in names {
            bind(ctx, &core, name, call)?;
        }
    }
    for &(name, _) in GESTURES {
        bind(ctx, &core, name, native)?;
    }
    scene::install_node_check(ctx, &core)?;
    canvas::install_exports(ctx, &core)?;
    host.refs.borrow_mut().scene = Some(Persistent::save(ctx, core.clone().into_value()));
    ctx.globals().set("__scene", core)?;
    let weak = host.downgrade();
    host.runtime.install_standard_bindings(ctx, move || {
        if let Some(host) = weak.upgrade() {
            host.wants_frame.set(true);
        }
    })?;
    for name in ["requestAnimationFrame", "cancelAnimationFrame"] {
        bind(ctx, &ctx.globals(), name, native)?;
    }
    let config = &host.inner().config;
    host.runtime.install_modules(
        ctx,
        javascript::runtime::extension::ModuleBindings::new(
            config.registry.as_ref().clone(),
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
    id: NodeId,
    origin: StyleOrigin,
    replace: bool,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let decoded = decode_style(ctx, value, 0, &mut 0)?;
    let style = codec::from_value(&decoded).map_err(|error| type_error(ctx, &error.to_string()))?;
    apply(
        host,
        ctx,
        Mutation::StyleSnapshot {
            id,
            origin,
            replace,
            style,
        },
    )?;
    Ok(())
}

pub(super) fn class_snapshot<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    id: NodeId,
    list: bool,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let value = decode_style(ctx, value, 0, &mut 0)?;
    let mutation = mutation::class_snapshot(id, list, value)
        .map_err(|_| type_error(ctx, "invalid class declaration"))?;
    apply(host, ctx, mutation)?;
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
fn native<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    let undefined = || Value::new_undefined(ctx.clone());
    let minimum = match name {
        "setStyle" | "patchStyle" | "setComponentStyle" | "setClass" | "setStyleSource"
        | "createStyleSource" => 2,
        "clearStyle"
        | "removeStyleSource"
        | "setStyleEnvironment"
        | "offBack"
        | "offSettings"
        | "offWindowMetrics" => 1,
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
        if name != "observeScrollOffset" {
            apply(host, ctx, Mutation::PointerTarget(NodeId::from_raw(id)))?;
        }
        host.refs
            .borrow_mut()
            .callbacks
            .insert((id, *kind), callback);
        return Ok(undefined());
    }
    match name {
        "_insertNode" => return renderer::call(host, ctx, "insertNode", args),
        "_createTextNode" => return renderer::call(host, ctx, "createTextNode", args),
        "remove" | "detach" => {
            let parent = node_id(ctx, args, 0)?;
            let child = node_id(ctx, args, 1)?;
            let change = if name == "remove" {
                Mutation::Remove { parent, child }
            } else {
                Mutation::Detach { parent, child }
            };
            apply(host, ctx, change)?;
        }
        "discard" => {
            apply(host, ctx, Mutation::Discard(node_id(ctx, args, 0)?))?;
        }
        "reparentChildren" => {
            let from = node_id(ctx, args, 0)?;
            let to = node_id(ctx, args, 1)?;
            apply(host, ctx, Mutation::Reparent { from, to })?;
        }
        "setText" => {
            let id = node_id(ctx, args, 0)?;
            let text = string(ctx, args, 1)?;
            apply(host, ctx, Mutation::SetText { id, text })?;
        }
        "setTextRuns" => {
            let id = node_id(ctx, args, 0)?;
            let spans = crate::js::text_runs::decode(ctx, value(ctx, args, 1))?;
            apply(host, ctx, Mutation::TextRuns { id, spans })?;
        }
        "setImage" => {
            let id = node_id(ctx, args, 0)?;
            let source = value(ctx, args, 1);
            let source = if source.is_null() || source.is_undefined() {
                String::new()
            } else {
                Coerced::<String>::from_js(ctx, source)?.0
            };
            let size = |index| {
                let value = value(ctx, args, index);
                if value.is_null() || value.is_undefined() {
                    Ok(None)
                } else {
                    num(ctx, args, index).map(|value| (value >= 0.0).then_some(value as f32))
                }
            };
            let intrinsic = size(2)?.zip(size(3)?);
            apply(
                host,
                ctx,
                Mutation::SetImage {
                    id,
                    source,
                    intrinsic,
                },
            )?;
        }
        "setStyleSurface" => {
            let id = node_id(ctx, args, 0)?;
            let surface = StyleSurface::parse(&string(ctx, args, 1)?)
                .ok_or_else(|| type_error(ctx, "unknown style surface"))?;
            apply(host, ctx, Mutation::SetStyleSurface { id, surface })?;
        }
        "setRepaintBoundary" | "setTransitionSnapshot" => {
            let id = node_id(ctx, args, 0)?;
            let enabled = boolean(ctx, args, 1)?;
            let change = if name == "setTransitionSnapshot" {
                Mutation::TransitionSnapshot { id, enabled }
            } else {
                Mutation::RepaintBoundary { id, enabled }
            };
            apply(host, ctx, change)?;
        }
        "setColorFilter" => {
            let id = node_id(ctx, args, 0)?;
            let input = value(ctx, args, 1);
            let matrix = if input.is_null() || input.is_undefined() {
                None
            } else {
                let invalid =
                    || type_error(ctx, "expected a color matrix with 20 finite coefficients");
                let entries = input
                    .as_array()
                    .filter(|array| array.len() == 20)
                    .ok_or_else(invalid)?;
                let mut matrix = [[0.0; 5]; 4];
                for (index, coefficient) in matrix.iter_mut().flatten().enumerate() {
                    *coefficient = entries.get::<f32>(index)?;
                    if !coefficient.is_finite() {
                        return Err(invalid());
                    }
                }
                Some(Arc::new(matrix))
            };
            apply(host, ctx, Mutation::ColorFilter { id, matrix })?;
        }
        "setBackdropMaterial" => {
            let id = node_id(ctx, args, 0)?;
            let input = value(ctx, args, 1);
            let material = if input.is_null() || input.is_undefined() {
                None
            } else {
                let entries = input
                    .as_array()
                    .filter(|array| array.len() == 8)
                    .ok_or_else(|| {
                        type_error(ctx, "expected eight backdrop material parameters")
                    })?;
                let mut values = [0.0; 8];
                for (index, value) in values.iter_mut().enumerate() {
                    *value = entries.get::<f32>(index)?;
                }
                let [sigma, saturation, brightness, taper, r, g, b, a] = values;
                Some(Arc::new(crate::scene::BackdropMaterial {
                    sigma,
                    saturation,
                    brightness,
                    tint: [r, g, b, a],
                    taper,
                }))
            };
            apply(host, ctx, Mutation::BackdropMaterial { id, material })?;
        }
        "setFocusable" | "setScrollChain" => {
            let id = node_id(ctx, args, 0)?;
            let enabled = boolean(ctx, args, 1)?;
            let change = if name == "setFocusable" {
                Mutation::Focusable { id, enabled }
            } else {
                Mutation::ScrollChain { id, enabled }
            };
            apply(host, ctx, change)?;
        }
        "setDragClaim" => {
            let id = node_id(ctx, args, 0)?;
            let claim = match string(ctx, args, 1)?.as_str() {
                "none" => DragClaim::None,
                "horizontal" => DragClaim::Horizontal,
                "free" => DragClaim::Free,
                _ => return Err(type_error(ctx, "unknown drag claim")),
            };
            apply(host, ctx, Mutation::DragClaim { id, claim })?;
        }
        "setTextLimit" => {
            let id = node_id(ctx, args, 0)?;
            let limit = Coerced::<i32>::from_js(ctx, value(ctx, args, 1))?.0;
            apply(
                host,
                ctx,
                Mutation::TextLimit {
                    id,
                    limit: u32::try_from(limit).ok(),
                },
            )?;
        }
        "bindScrollProp" | "bindBackProgressOpacity" | "unbindScrollProp" => {
            return scroll_rules::binding(host, ctx, name, args);
        }
        "addScrollThreshold" => {
            let node = node_id(ctx, args, 0)?;
            let source = crate::scene::ScrollSource::parse(&string(ctx, args, 1)?)
                .ok_or_else(|| type_error(ctx, "unknown scroll channel"))?;
            let rule = crate::scene::ScrollThreshold {
                source,
                value: num(ctx, args, 2)? as f32,
                from_end: boolean(ctx, args, 3)?,
                hysteresis: num(ctx, args, 4)? as f32,
            };
            let id = host
                .inner()
                .tree
                .borrow_mut()
                .add_scroll_threshold(node, rule)
                .map_err(|_| type_error(ctx, "invalid scroll threshold"))?;
            return id.into_js(ctx);
        }
        "removeScrollThreshold" => {
            let id = u32::try_from(handle(ctx, args, 0)?)
                .map_err(|_| type_error(ctx, "invalid scroll threshold id"))?;
            host.inner().tree.borrow_mut().remove_scroll_threshold(id);
        }
        "setScrollOffset" => {
            let id = node_id(ctx, args, 0)?;
            let x = num(ctx, args, 1)? as f32;
            let y = num(ctx, args, 2)? as f32;
            apply(host, ctx, Mutation::ScrollOffset { id, x, y })?;
        }
        "_reactiveCall" => {
            let name = string(ctx, args, 0)?;
            return host.api(ctx, true, &name, args[1..].to_vec());
        }
        "setStyle" | "patchStyle" | "setComponentStyle" => {
            let id = node_id(ctx, args, 0)?;
            let origin = if name == "setComponentStyle" {
                StyleOrigin::Component
            } else {
                StyleOrigin::Imperative
            };
            style_snapshot(
                host,
                ctx,
                id,
                origin,
                name != "patchStyle",
                value(ctx, args, 1),
            )?;
        }
        "setClass" => class_snapshot(
            host,
            ctx,
            node_id(ctx, args, 0)?,
            false,
            value(ctx, args, 1),
        )?,
        "clearStyle" => {
            let id = node_id(ctx, args, 0)?;
            let value = decode_style(ctx, value(ctx, args, 1), 0, &mut 0)?;
            let mutation = mutation::clear_style(id, value)
                .map_err(|error| type_error(ctx, &error.to_string()))?;
            apply(host, ctx, mutation)?;
        }
        "setStyleSource" => {
            let source = StyleSourceId::from_raw(handle(ctx, args, 0)?);
            let value = decode_style(ctx, value(ctx, args, 1), 0, &mut 0)?;
            let mutation = if let StyleValue::String(text) = value {
                Mutation::SourceClass { source, text }
            } else {
                let style = codec::from_value(&value)
                    .map_err(|error| type_error(ctx, &error.to_string()))?;
                Mutation::SourceSnapshot { source, style }
            };
            apply(host, ctx, mutation)?;
        }
        "setStyleEnvironment" => {
            let environment = decode_style(ctx, value(ctx, args, 0), 0, &mut 0)?;
            apply(host, ctx, Mutation::StyleEnvironment(environment))?;
        }
        "createStyleSource" => {
            let origin = match string(ctx, args, 1)?.as_str() {
                "component" => StyleOrigin::Component,
                "classes" => StyleOrigin::Classes,
                "inline" => StyleOrigin::Inline,
                "imperative" => StyleOrigin::Imperative,
                "animation" => StyleOrigin::Animation,
                _ => return Err(type_error(ctx, "unknown style origin")),
            };
            let id = node_id(ctx, args, 0)?;
            return (apply(host, ctx, Mutation::SourceCreate { id, origin })? as f64).into_js(ctx);
        }
        "removeStyleSource" => {
            let source = StyleSourceId::from_raw(handle(ctx, args, 0)?);
            apply(host, ctx, Mutation::SourceRemove(source))?;
        }
        "onFrame" | "onLayout" | "onScrollState" | "onScrollThreshold" | "onDriver"
        | "onBackGesture" => {
            if name == "onFrame" && args.first().is_none_or(|value| value.is_undefined()) {
                host.refs.borrow_mut().hooks.remove("frame");
                return Ok(Value::new_undefined(ctx.clone()));
            }
            let callback = saved_function(ctx, args, 0)?;
            let key = match name {
                "onFrame" => "frame",
                "onLayout" => "layout",
                "onDriver" => "driver",
                "onScrollThreshold" => "scrollThreshold",
                "onBackGesture" => "backGesture",
                _ => "scroll",
            };
            host.refs.borrow_mut().hooks.insert(key, callback);
        }
        "requestFrame" => {
            host.wants_frame.set(true);
        }
        "setBackEnabled" => {
            let enabled = args
                .first()
                .and_then(Value::as_bool)
                .ok_or_else(|| type_error(ctx, "setBackEnabled(enabled)"))?;
            host.back_enabled.set(Some(enabled));
        }
        "setSemanticsHidden" => {
            let id = handle(ctx, args, 0)?;
            let hidden = args
                .get(1)
                .and_then(Value::as_bool)
                .ok_or_else(|| type_error(ctx, "setSemanticsHidden(node, hidden)"))?;
            host.inner()
                .tree
                .borrow_mut()
                .set_semantics_hidden(NodeId::from_raw(id), hidden)
                .map_err(|_| type_error(ctx, "setSemanticsHidden requires a live node"))?;
        }
        "onBack" | "onSettings" | "onWindowMetrics" => {
            let callback = saved_function(ctx, args, 0)?;
            let id = host
                .runtime
                .allocate_timer_id()
                .ok_or_else(|| rquickjs::Exception::throw_range(ctx, "callback IDs exhausted"))?;
            let mut refs = host.refs.borrow_mut();
            if name == "onBack" && !refs.hooks.contains_key("backGesture") {
                host.back_enabled.set(Some(true));
            }
            let callbacks = match name {
                "onBack" => &mut refs.backs,
                "onSettings" => &mut refs.settings,
                _ => &mut refs.window_metrics,
            };
            callbacks.push((id, callback));
            return (id as f64).into_js(ctx);
        }
        "offBack" | "offSettings" | "offWindowMetrics" => {
            let id = num(ctx, args, 0)? as u64;
            let mut refs = host.refs.borrow_mut();
            let callbacks = match name {
                "offBack" => &mut refs.backs,
                "offSettings" => &mut refs.settings,
                _ => &mut refs.window_metrics,
            };
            callbacks.retain(|(i, _)| *i != id);
            if name == "offBack" && callbacks.is_empty() && !refs.hooks.contains_key("backGesture")
            {
                host.back_enabled.set(Some(false));
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
        "windowMetrics" => return window_metrics(host, ctx),
        "penSample" => {
            let Some(event) = host.inner().last_pen.get() else {
                return Ok(Value::new_null(ctx.clone()));
            };
            let Some(pen) = event.pen else {
                return Ok(Value::new_null(ctx.clone()));
            };
            let sample = Object::new(ctx.clone())?;
            sample.set("id", event.pointer.0.to_string())?;
            sample.set("x", event.x)?;
            sample.set("y", event.y)?;
            sample.set("time", event.time_ms)?;
            sample.set("force", pen.force)?;
            sample.set("radius", pen.radius)?;
            sample.set("altitude", pen.altitude)?;
            sample.set("azimuth", pen.azimuth)?;
            sample.set(
                "phase",
                match event.phase {
                    crate::input::Phase::Down => "down",
                    crate::input::Phase::Move => "move",
                    crate::input::Phase::Up => "up",
                    crate::input::Phase::Cancel => "cancel",
                    crate::input::Phase::Wheel | crate::input::Phase::Zoom => "move",
                },
            )?;
            return Ok(sample.into_value());
        }
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
            let _ = handle(ctx, args, 0)?;
            let x = optional_num(ctx, args, 1)? as f32;
            let y = optional_num(ctx, args, 2)? as f32;
            let text = super::super::vm_entry!(None, {
                let mut tree = host.inner().tree.borrow_mut();
                Some(match name {
                    "debugLayoutIssues" => tree.inspect_layout_issues(),
                    "debugSnapshot" => tree.debug_snapshot(),
                    "debugProbe" => tree.debug_probe(x, y),
                    _ => match tree.debug_last_pointer() {
                        Some((px, py, phase)) => {
                            format!("{{\"x\":{px:.2},\"y\":{py:.2},\"phase\":\"{phase}\"}}")
                        }
                        None => "null".to_owned(),
                    },
                })
            });
            return match text {
                Some(text) => text.into_js(ctx),
                None => Ok(Value::new_null(ctx.clone())),
            };
        }
        "debugSetScan" | "debugSetPointerTrace" | "debugSetOwned" => {
            let a = boolean(ctx, args, 0)?;
            let b = boolean(ctx, args, 1)?;
            let id = if name == "debugSetOwned" {
                handle(ctx, args, 0)?
            } else {
                0
            };
            super::super::vm_entry!((), {
                let mut tree = host.inner().tree.borrow_mut();
                match name {
                    "debugSetScan" => tree.set_debug_scan(a),
                    "debugSetPointerTrace" => tree.set_pointer_trace(a, b),
                    _ => tree.set_debug_owned(NodeId::from_raw(id), b),
                }
            });
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

pub(super) fn window_metrics<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    use crate::session::{DisplayFeatureKind, FoldPosture};

    let metrics = host.inner().window_metrics.get();
    let display = host.inner().window_display.borrow();
    let snapshot = Object::new(ctx.clone())?;
    snapshot.set("width", metrics.width)?;
    snapshot.set("height", metrics.height)?;
    snapshot.set(
        "pixelWidth",
        (metrics.width * metrics.device_pixel_ratio).round(),
    )?;
    snapshot.set(
        "pixelHeight",
        (metrics.height * metrics.device_pixel_ratio).round(),
    )?;
    snapshot.set("devicePixelRatio", metrics.device_pixel_ratio)?;
    snapshot.set("displayId", display.display_id)?;
    snapshot.set("refreshRate", display.refresh_rate)?;
    let features = Array::new(ctx.clone())?;
    for (index, feature) in display.features.iter().enumerate() {
        let value = Object::new(ctx.clone())?;
        value.set("x", feature.bounds.x)?;
        value.set("y", feature.bounds.y)?;
        value.set("width", feature.bounds.w)?;
        value.set("height", feature.bounds.h)?;
        value.set(
            "type",
            match feature.kind {
                DisplayFeatureKind::Fold => "fold",
                DisplayFeatureKind::Hinge => "hinge",
                DisplayFeatureKind::Cutout => "cutout",
            },
        )?;
        value.set(
            "posture",
            feature.posture.map(|posture| match posture {
                FoldPosture::Flat => "flat",
                FoldPosture::HalfOpened => "halfOpened",
            }),
        )?;
        value.set("separating", feature.separating)?;
        features.set(index, value)?;
    }
    snapshot.set("displayFeatures", features)?;
    if let Some(rect) = metrics.keyboard_occlusion {
        let value = Object::new(ctx.clone())?;
        value.set("x", rect.x)?;
        value.set("y", rect.y)?;
        value.set("width", rect.w)?;
        value.set("height", rect.h)?;
        snapshot.set("keyboardOcclusion", value)?;
    }
    for (name, value) in [
        ("padding", metrics.padding),
        ("viewInsets", metrics.view_insets),
        ("systemGestureInsets", metrics.system_gesture_insets),
    ] {
        let insets = Object::new(ctx.clone())?;
        insets.set("left", value.left)?;
        insets.set("top", value.top)?;
        insets.set("right", value.right)?;
        insets.set("bottom", value.bottom)?;
        snapshot.set(name, insets)?;
    }
    Ok(snapshot.into_value())
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
    if let Some(buffer) = javascript::runtime::extension::array_buffer(value)? {
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
