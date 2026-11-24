//! Named scene reads and operations. Argument coercion runs before borrowing the tree.

use super::{boolean, decode_handle, num, read_bytes, value};
use crate::input::{DRAG_SLOP, DoubleTapConfig, DragAxis, RecognizerSet};
use crate::js::player::{bridge, mutation, nodes};
use crate::motion::{Band, Decay, Easing, OverscrollMode, ScrollAccept, ScrollPhysics};
use crate::scene::{
    MAX_SHADER_TEXTURES, MAX_SHADER_UNIFORM_BYTES, ShaderClock, ShaderEffect, ShaderProgramId,
    TextStyle,
};
use crate::semantics::{AutofillField, LiveRegion, SemanticsHint};
use crate::text::{TextEditingValue, grapheme_count, grapheme_index, grapheme_unit};
use crate::tree::{FocusDirection, PanelAlign, ScrollEdge, SubtreeSpace};
use crate::{NodeId, Rect};
use rquickjs::{
    Array, Coerced, Ctx, Exception, FromJs, Function, IntoJs, Object, TypedArray, Value,
};
use std::sync::Arc;

use super::super::{Host, canvas, host_from_ctx, type_error};

pub(super) const NAMES: &[&str] = &[
    "parent",
    "firstChild",
    "nextSibling",
    "isText",
    "rect",
    "scrollOffset",
    "overscroll",
    "metrics",
    "measureText",
    "caretAt",
    "caretAtUnit",
    "wordRange",
    "focused",
    "isFocusable",
    "pointerPosition",
    "textScale",
    "reduceMotion",
    "layerTreeStats",
    "flingTarget",
    "graphemeIndex",
    "graphemeCount",
    "graphemeUnit",
    "selectionRects",
    "focus",
    "blur",
    "focusStep",
    "pushFocusScope",
    "popFocusScope",
    "scrollTo",
    "fling",
    "stopScroll",
    "animate",
    "scrollIntoView",
    "placePanel",
    "textEdit",
    "textCommand",
    "setImeEnabled",
    "setImeText",
    "setImeConfig",
    "setImeState",
    "setSemantics",
    "setPlatformView",
    "setRecognizers",
    "setScrollPhysics",
    "registerFont",
    "registerShaderProgram",
    "setShaderEffect",
    "animateShaderUniform",
    "reload",
    "measurePanel",
    "setTextScale",
];

#[expect(
    clippy::cast_possible_truncation,
    reason = "live node handles are exact numbers in the validated range"
)]
pub(super) fn install_node_check<'js>(ctx: &Ctx<'js>, scene: &Object<'js>) -> rquickjs::Result<()> {
    let check = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, value: Value<'js>| -> rquickjs::Result<bool> {
            let Some(n) = value
                .as_number()
                .filter(|n| (4294967296.0..9007199254740992.0).contains(n) && n.fract() == 0.0)
            else {
                return Ok(false);
            };
            let host = host_from_ctx(&ctx)?;
            host.with_current(&ctx, || {
                canvas::check_side_effect(&host, &ctx)?;
                Ok(host
                    .inner()
                    .tree
                    .borrow()
                    .has_node(NodeId::from_raw(n as u64)))
            })
        },
    )?;
    check.set_name("_isNode")?;
    scene.set("_isNode", check)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "scene geometry crosses from JavaScript f64 to engine f32"
)]
pub(super) fn call<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    let inner = host.inner();
    let tree = &inner.tree;
    let node = || scene_node(ctx, args, 0);
    let error = || Exception::throw_message(ctx, &format!("{name} failed"));
    let check = |ok: bool| if ok { Ok(()) } else { Err(error()) };
    let numeric = |index| num(ctx, args, index).map(|n| n as f32);

    match name {
        "parent" | "firstChild" | "nextSibling" => {
            if args.is_empty() {
                return Err(Exception::throw_message(ctx, &format!("{name}(node)")));
            }
            let node = node()?;
            let relative = {
                let tree = tree.borrow();
                match name {
                    "parent" => tree.parent(node),
                    "firstChild" => tree.first_child(node),
                    _ => tree.next_sibling(node),
                }
            }
            .map_err(|_| error())?;
            return node_value(ctx, relative);
        }
        "isText" | "isFocusable" => {
            if name == "isText" {
                require_args(ctx, args, 1, "isText(node)")?;
            }
            let node = node()?;
            let result = {
                let tree = tree.borrow();
                if name == "isText" {
                    tree.is_text(node)
                } else {
                    tree.is_focusable(node)
                }
            };
            return result.map_err(|_| error())?.into_js(ctx);
        }
        "rect" => {
            let node = node()?;
            check(mutation::flush_for_read(host))?;
            let rect = tree.borrow().rect(node).map_err(|_| error())?;
            return Ok(rect_object(ctx, rect)?.into_value());
        }
        "scrollOffset" | "overscroll" => {
            let node = node()?;
            let (x, y) = {
                let tree = tree.borrow();
                if name == "scrollOffset" {
                    tree.scroll_offset(node)
                } else {
                    tree.overscroll(node)
                }
            }
            .map_err(|_| error())?;
            return pair(ctx, ("x", x), ("y", y));
        }
        "metrics" => {
            let (width, height) = tree.borrow().viewport_size();
            let metrics = Object::new(ctx.clone())?;
            metrics.set("width", width)?;
            metrics.set("height", height)?;
            let insets = Object::new(ctx.clone())?;
            for side in ["top", "right", "bottom", "left"] {
                insets.set(side, 0)?;
            }
            metrics.set("viewInsets", insets)?;
            return Ok(metrics.into_value());
        }
        "measureText" => {
            let font_size = query_number(numeric(1)?);
            let max_width = query_number(numeric(2)?);
            let text = text_arg(ctx, args, 0)?;
            let color = color(ctx, value(ctx, args, 3))?;
            let size = tree.borrow_mut().measure_text(
                &text,
                font_size,
                max_width,
                color,
                &TextStyle::default(),
            );
            return pair(ctx, ("w", size.width), ("h", size.height));
        }
        "caretAt" | "caretAtUnit" => {
            let point = name == "caretAt";
            if point {
                require_args(ctx, args, 3, "caretAt(node, x, y)")?;
            }
            let node = node()?;
            let caret = if point {
                let x = query_number(numeric(1)?);
                let y = query_number(numeric(2)?);
                tree.borrow_mut().caret_at(node, x, y)
            } else {
                let unit = unit(ctx, args, 1)?;
                tree.borrow_mut().caret_at_unit(node, unit)
            }
            .map_err(|_| error())?;
            let out = Object::new(ctx.clone())?;
            if point {
                out.set("unit", caret.unit)?;
            }
            set_rect(
                &out,
                Rect {
                    x: caret.x,
                    y: caret.y,
                    w: caret.w,
                    h: caret.h,
                },
            )?;
            return Ok(out.into_value());
        }
        "wordRange" => {
            let node = node()?;
            let unit = unit(ctx, args, 1)?;
            let (start, end) = tree
                .borrow_mut()
                .word_range(node, unit)
                .map_err(|_| error())?;
            return pair(ctx, ("start", start), ("end", end));
        }
        "focused" => return node_value(ctx, tree.borrow().focused()),
        "pointerPosition" => {
            let (x, y) = tree.borrow().pointer_position();
            return pair(ctx, ("x", x), ("y", y));
        }
        "textScale" => return inner.text_scale.get().into_js(ctx),
        "reduceMotion" => return inner.reduce_motion.get().into_js(ctx),
        "layerTreeStats" => {
            let (epoch, layers, pictures) = tree.borrow().layer_tree_stats();
            let out = Object::new(ctx.clone())?;
            out.set("epoch", epoch as f64)?;
            out.set("layers", u32::try_from(layers).unwrap_or(u32::MAX))?;
            out.set("pictures", u32::try_from(pictures).unwrap_or(u32::MAX))?;
            return Ok(out.into_value());
        }
        "flingTarget" => {
            let node = node()?;
            let target = tree.borrow().fling_target(node);
            return match target {
                Some((x, y)) => pair(ctx, ("x", x), ("y", y)),
                None => Ok(Value::new_null(ctx.clone())),
            };
        }
        "graphemeIndex" | "graphemeCount" | "graphemeUnit" => {
            let unit = if name == "graphemeCount" {
                0
            } else {
                unit(ctx, args, 1)?
            };
            let text = text_arg(ctx, args, 0)?;
            let result = match name {
                "graphemeIndex" => grapheme_index(&text, unit),
                "graphemeUnit" => grapheme_unit(&text, unit),
                _ => grapheme_count(&text),
            };
            return result.into_js(ctx);
        }
        "selectionRects" => {
            let node = node()?;
            let start = unit(ctx, args, 1)?;
            let end = unit(ctx, args, 2)?;
            let rects = tree
                .borrow_mut()
                .selection_rects(node, start, end)
                .map_err(|_| error())?;
            let result = Array::new(ctx.clone())?;
            for (index, rect) in rects.into_iter().enumerate() {
                result.set(index, rect_object(ctx, rect)?)?;
            }
            return Ok(result.into_value());
        }
        "focus" => check(bridge::focus_node(host, node()?))?,
        "blur" => check(bridge::blur_node(host))?,
        "focusStep" => {
            let direction = match text_arg(ctx, args, 0)?.as_str() {
                "next" => FocusDirection::Next,
                "previous" => FocusDirection::Previous,
                "left" => FocusDirection::Left,
                "right" => FocusDirection::Right,
                "up" => FocusDirection::Up,
                "down" => FocusDirection::Down,
                _ => return Err(Exception::throw_message(ctx, "focus direction")),
            };
            check(bridge::focus_step(host, direction))?;
        }
        "pushFocusScope" => check(bridge::push_focus_scope(host, node()?))?,
        "popFocusScope" => {
            let raw = if args.first().is_none_or(Value::is_undefined) {
                0
            } else {
                node()?.to_raw()
            };
            let active = {
                let mut tree = tree.borrow_mut();
                if raw == 0 {
                    tree.pop_focus_scope()
                } else {
                    tree.remove_focus_scope(NodeId::from_raw(raw))
                }
            };
            return active.into_js(ctx);
        }
        "scrollTo" => {
            let node = node()?;
            let (x, y) = (numeric(1)?, numeric(2)?);
            let (stiffness, damping) = (numeric(3)?, numeric(4)?);
            tree.borrow_mut()
                .scroll_to(
                    node,
                    x,
                    y,
                    (stiffness > 0.0).then_some(stiffness),
                    (damping > 0.0).then_some(damping),
                )
                .map_err(|_| error())?;
        }
        "fling" => {
            let node = node()?;
            let (vx, vy) = (numeric(1)?, numeric(2)?);
            tree.borrow_mut()
                .arm_fling(node, vx, vy)
                .map_err(|_| error())?;
        }
        "stopScroll" => {
            let node = node()?;
            tree.borrow_mut().stop_scroll(node).map_err(|_| error())?;
        }
        "animate" => {
            let node = node()?;
            let values = [
                num(ctx, args, 2)?,
                num(ctx, args, 3)?,
                num(ctx, args, 4)?,
                num(ctx, args, 5)?,
            ];
            let retarget = boolean(ctx, args, 6)?;
            let prop = text_arg(ctx, args, 1)?;
            check(bridge::animate(host, node, &prop, values, retarget))?;
        }
        "scrollIntoView" => {
            let node = node()?;
            let edge = match int(ctx, args, 1)? {
                1 => ScrollEdge::End,
                2 => ScrollEdge::Nearest,
                _ => ScrollEdge::Start,
            };
            tree.borrow_mut()
                .scroll_into_view(node, edge)
                .map_err(|_| Exception::throw_message(ctx, "scrollIntoView"))?;
        }
        "placePanel" => {
            let panel = node()?;
            let anchor = scene_node(ctx, args, 1)?;
            let align = match int(ctx, args, 2)? {
                1 => PanelAlign::BelowEnd,
                2 => PanelAlign::BelowCenter,
                3 => PanelAlign::AboveStart,
                _ => PanelAlign::BelowStart,
            };
            tree.borrow_mut()
                .place_panel(panel, anchor, align)
                .map_err(|_| error())?;
        }
        "textEdit" => {
            require_args(ctx, args, 2, "textEdit(node, op, text)")?;
            let node = node()?;
            let op = int(ctx, args, 1)?;
            let text = optional_text(ctx, args, 2)?.unwrap_or_default();
            let edited = nodes::text_edit(inner, node, op, text).ok_or_else(error)?;
            return editing_value(ctx, edited);
        }
        "textCommand" => {
            require_args(ctx, args, 2, "textCommand(node, name, extra)")?;
            let extra = optional_text(ctx, args, 2)?.unwrap_or_default();
            let node = node()?;
            let command = text_arg(ctx, args, 1)?;
            let record_undo = boolean(ctx, args, 3)?;
            let edited =
                nodes::text_command(inner, node, &command, extra, record_undo).ok_or_else(error)?;
            return editing_value(ctx, edited);
        }
        "setImeEnabled" => {
            let shown = boolean(ctx, args, 0)?;
            let mut ime = inner.ime.borrow_mut();
            ime.shown = shown;
            ime.dirty = true;
        }
        "setImeText" => bridge::set_ime_text(inner, &text_arg(ctx, args, 0)?),
        "setImeConfig" => nodes::set_ime_config(inner, int(ctx, args, 0)?, int(ctx, args, 1)?),
        "setImeState" => {
            if let Some(text) = optional_text(ctx, args, 0)? {
                let selection = (unit(ctx, args, 1)?, unit(ctx, args, 2)?);
                let composing = (int(ctx, args, 3)?, int(ctx, args, 4)?);
                let caret = Rect {
                    x: numeric(5)?,
                    y: numeric(6)?,
                    w: numeric(7)?,
                    h: numeric(8)?,
                };
                check(bridge::set_ime_state(
                    inner, &text, selection, composing, caret,
                ))?;
            } else {
                bridge::hide_ime(inner);
            }
        }
        "setSemantics" => {
            require_args(ctx, args, 2, "setSemantics(node, role)")?;
            let node = node()?;
            let hint = semantics_hint(ctx, args)?;
            tree.borrow_mut()
                .set_semantics(node, hint)
                .map_err(|_| error())?;
        }
        "setPlatformView" => {
            let kind = optional_text(ctx, args, 1)?.unwrap_or_default();
            if kind.contains('\0') {
                return Err(type_error(ctx, "Invalid platform view kind"));
            }
            let path = optional_text(ctx, args, 2)?;
            let node = node()?;
            tree.borrow_mut()
                .set_platform_view_with_path(node, &kind, path.as_deref())
                .map_err(|_| error())?;
        }
        "setRecognizers" => {
            if !boolean(ctx, args, 1)? {
                return Err(Exception::throw_message(ctx, "setRecognizers(node, spec)"));
            }
            let node = node()?;
            let set = recognizers(ctx, &value(ctx, args, 1))?;
            tree.borrow_mut()
                .set_recognizers(node, set)
                .map_err(|_| Exception::throw_message(ctx, "setRecognizers"))?;
        }
        "setScrollPhysics" => {
            require_args(
                ctx,
                args,
                10,
                "setScrollPhysics(node, tau, mode, stiffness, damping, factor, snapX, alignX, snapY, alignY)",
            )?;
            let node = node()?;
            let physics = scroll_physics(ctx, args)?;
            tree.borrow_mut()
                .set_scroll_physics(node, physics)
                .map_err(|_| error())?;
        }
        "registerFont" => {
            require_args(ctx, args, 2, "registerFont(family, bytes)")?;
            let family = text_arg(ctx, args, 0)?;
            let bytes = read_bytes(ctx, &value(ctx, args, 1))?.unwrap_or_default();
            inner
                .fonts
                .borrow_mut()
                .enqueue(family, bytes)
                .map_err(|()| error())?;
        }
        "registerShaderProgram" => {
            let source = String::from_js(ctx, value(ctx, args, 0))?;
            let clock = match Option::<bool>::from_js(ctx, value(ctx, args, 1))? {
                Some(true) => ShaderClock::Frame,
                _ => ShaderClock::Static,
            };
            let id = tree
                .borrow_mut()
                .register_shader_program(&source, clock)
                .map_err(|error| type_error(ctx, &error.to_string()))?;
            return (id.0 as f64).into_js(ctx);
        }
        "setShaderEffect" => {
            let node = node()?;
            let effect = shader_effect(ctx, args)?;
            tree.borrow_mut()
                .set_shader_effect(node, effect)
                .map_err(|error| type_error(ctx, &error.to_string()))?;
        }
        "animateShaderUniform" => {
            let node = node()?;
            let index = u8::try_from(decode_handle(ctx, num(ctx, args, 1)?)?)
                .map_err(|_| type_error(ctx, "invalid shader uniform index"))?;
            let target = numeric(2)?;
            let duration = num(ctx, args, 3)?;
            let easing = match Option::<String>::from_js(ctx, value(ctx, args, 4))?.as_deref() {
                None | Some("ease-in-out") => Easing::EaseInOut,
                Some("linear") => Easing::Linear,
                _ => return Err(type_error(ctx, "unknown shader uniform easing")),
            };
            tree.borrow_mut()
                .animate_shader_uniform(node, index, target, duration, easing)
                .map_err(|error| type_error(ctx, &error.to_string()))?;
            host.wants_frame.set(true);
        }
        "reload" => {
            if !boolean(ctx, args, 0)? {
                return Err(Exception::throw_message(ctx, "reload(source)"));
            }
            let source = text_arg(ctx, args, 0)?;
            check(!source.is_empty())?;
            inner.reloads.borrow_mut().push(source);
        }
        "measurePanel" => {
            let node = node()?;
            let mode = int(ctx, args, 1)?;
            let (width, height) = (numeric(2)?, numeric(3)?);
            let space = match mode {
                0 => SubtreeSpace::MaxContent,
                1 => SubtreeSpace::CappedMax(width),
                2 => SubtreeSpace::Definite {
                    width,
                    height: height.is_finite().then_some(height),
                },
                _ => return Err(error()),
            };
            let (width, height) = tree
                .borrow_mut()
                .measure_panel(node, space)
                .map_err(|_| error())?;
            return pair(ctx, ("w", width), ("h", height));
        }
        "setTextScale" => {
            let scale = numeric(0)?;
            inner.text_scale.set(scale);
            tree.borrow_mut()
                .set_text_scale(scale)
                .map_err(|_| error())?;
        }
        _ => return Err(type_error(ctx, "unknown scene operation")),
    }
    Ok(Value::new_undefined(ctx.clone()))
}

fn shader_effect<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Option<ShaderEffect>> {
    let id = value(ctx, args, 1);
    if id.is_null() || id.is_undefined() {
        return Ok(None);
    }
    let id = id
        .as_number()
        .ok_or_else(|| type_error(ctx, "expected a shader program handle"))?;
    let shader = ShaderProgramId(decode_handle(ctx, id)?);
    let uniforms = match Option::<TypedArray<u8>>::from_js(ctx, value(ctx, args, 2))? {
        None => Arc::from([]),
        Some(array) if array.len() <= MAX_SHADER_UNIFORM_BYTES => {
            Arc::from(read_bytes(ctx, array.as_value())?.unwrap_or_default())
        }
        Some(_) => {
            return Err(type_error(
                ctx,
                "shader uniform data exceeds the byte limit",
            ));
        }
    };
    let textures = match Option::<Array>::from_js(ctx, value(ctx, args, 3))? {
        None => Arc::from([]),
        Some(array) if array.len() <= MAX_SHADER_TEXTURES => Arc::from(
            array
                .iter::<String>()
                .map(|source| source.map(Arc::<str>::from))
                .collect::<rquickjs::Result<Vec<_>>>()?,
        ),
        Some(_) => return Err(type_error(ctx, "shader texture count exceeds the limit")),
    };
    Ok(Some(ShaderEffect {
        shader,
        uniforms,
        textures,
    }))
}

fn require_args(
    ctx: &Ctx<'_>,
    args: &[Value<'_>],
    count: usize,
    message: &str,
) -> rquickjs::Result<()> {
    if args.len() < count {
        Err(Exception::throw_message(ctx, message))
    } else {
        Ok(())
    }
}

fn scene_node<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<NodeId> {
    decode_handle(ctx, num(ctx, args, index)?).map(NodeId::from_raw)
}

fn node_value<'js>(ctx: &Ctx<'js>, node: Option<NodeId>) -> rquickjs::Result<Value<'js>> {
    (node.map_or(0, NodeId::to_raw) as f64).into_js(ctx)
}

fn int<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<i32> {
    Coerced::<i32>::from_js(ctx, value(ctx, args, index)).map(|number| number.0)
}

fn unit<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<u32> {
    Ok(u32::try_from(int(ctx, args, index)?).unwrap_or_default())
}

fn query_number(number: f32) -> f32 {
    if number.is_nan() || number == 0.0 {
        0.0
    } else {
        number
    }
}

fn text<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    if value.is_symbol() {
        // Explicit String(symbol) is valid even though implicit coercion throws.
        return ctx.globals().get::<_, Function>("String")?.call((value,));
    }
    Coerced::<String>::from_js(ctx, value).map(|text| text.0)
}

fn text_arg<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> rquickjs::Result<String> {
    text(ctx, value(ctx, args, index))
}

fn optional_text<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<Option<String>> {
    let value = value(ctx, args, index);
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        text(ctx, value).map(Some)
    }
}

fn pair<'js, T: IntoJs<'js>>(
    ctx: &Ctx<'js>,
    (first, a): (&str, T),
    (second, b): (&str, T),
) -> rquickjs::Result<Value<'js>> {
    let out = Object::new(ctx.clone())?;
    out.set(first, a)?;
    out.set(second, b)?;
    Ok(out.into_value())
}

fn set_rect(out: &Object<'_>, rect: Rect) -> rquickjs::Result<()> {
    out.set("x", rect.x)?;
    out.set("y", rect.y)?;
    out.set("w", rect.w)?;
    out.set("h", rect.h)
}

fn rect_object<'js>(ctx: &Ctx<'js>, rect: Rect) -> rquickjs::Result<Object<'js>> {
    let out = Object::new(ctx.clone())?;
    set_rect(&out, rect)?;
    Ok(out)
}

fn editing_value<'js>(ctx: &Ctx<'js>, edited: TextEditingValue) -> rquickjs::Result<Value<'js>> {
    let out = Object::new(ctx.clone())?;
    out.set("text", edited.text)?;
    out.set("start", edited.selection.base)?;
    out.set("end", edited.selection.extent)?;
    Ok(out.into_value())
}

fn color<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<[u8; 4]> {
    let invalid = || type_error(ctx, "bad color");
    let mut rgba = [0, 0, 0, 255];
    if let Some(text) = value.as_string() {
        let text = text.to_string()?;
        let hex = text.strip_prefix('#').unwrap_or(&text);
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let (channels, _) = hex.as_bytes().as_chunks::<2>();
        for (channel, digits) in rgba.iter_mut().zip(channels) {
            let digits = std::str::from_utf8(digits).map_err(|_| invalid())?;
            *channel = u8::from_str_radix(digits, 16).map_err(|_| invalid())?;
        }
    } else {
        let invalid_length = || Exception::throw_message(ctx, "color must be [r,g,b,a]");
        let object = value.as_object().ok_or_else(invalid_length)?;
        if object.get::<_, Value>("length")?.as_number() != Some(4.0) {
            return Err(invalid_length());
        }
        for (index, channel) in (0u32..4).zip(&mut rgba) {
            let coerced: Coerced<i32> = object.get(index)?;
            *channel = u8::try_from(coerced.0).map_err(|_| invalid())?;
        }
    }
    Ok(rgba)
}

fn property<'js>(ctx: &Ctx<'js>, value: &Value<'js>, key: &str) -> rquickjs::Result<Value<'js>> {
    value.as_object().map_or_else(
        || Ok(Value::new_undefined(ctx.clone())),
        |object| object.get(key),
    )
}

fn string_property<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    key: &str,
) -> rquickjs::Result<Option<String>> {
    property(ctx, value, key)?
        .as_string()
        .map(|text| text.to_string())
        .transpose()
}

fn semantics_hint<'js>(ctx: &Ctx<'js>, args: &[Value<'js>]) -> rquickjs::Result<SemanticsHint> {
    let hint = value(ctx, args, 11);
    let autofill = property(ctx, &hint, "autofill")?;
    let autofill_kind = match string_property(ctx, &autofill, "hint")?.as_deref() {
        Some("username") => Some("username"),
        Some("password") => Some("password"),
        Some("newUsername") => Some("newUsername"),
        Some("newPassword") => Some("newPassword"),
        Some("emailAddress") => Some("emailAddress"),
        _ => None,
    };
    let autofill = if let Some(hint) = autofill_kind {
        let value = property(ctx, &autofill, "value")?;
        let value = if value.is_null() || value.is_undefined() {
            String::new()
        } else {
            text(ctx, value)?
        };
        Some(AutofillField { hint, value })
    } else {
        None
    };
    let label = |index| -> rquickjs::Result<Option<String>> {
        if !boolean(ctx, args, index)? {
            return Ok(None);
        }
        let text = text_arg(ctx, args, index)?;
        Ok((!text.is_empty()).then_some(text))
    };
    let optional_int = |index| -> rquickjs::Result<i32> {
        if args.get(index).is_none_or(Value::is_undefined) {
            Ok(-1)
        } else {
            int(ctx, args, index)
        }
    };
    let password = property(ctx, &hint, "password")?.as_bool() == Some(true);
    let selection = u32::try_from(optional_int(5)?)
        .ok()
        .zip(u32::try_from(optional_int(6)?).ok())
        .filter(|(start, end)| start <= end);
    let (order, live) = if hint.is_object() && !hint.is_function() {
        let order = property(ctx, &hint, "order")?;
        let order = if order.is_null() || order.is_undefined() {
            None
        } else {
            Some(Coerced::<i32>::from_js(ctx, order)?.0)
        };
        let live = match string_property(ctx, &hint, "live")?.as_deref() {
            Some("off") => Some(LiveRegion::Off),
            Some("polite") => Some(LiveRegion::Polite),
            Some("assertive") => Some(LiveRegion::Assertive),
            _ => None,
        };
        (order, live)
    } else {
        (None, None)
    };
    Ok(SemanticsHint {
        role: bridge::role_of(int(ctx, args, 1)?),
        label: label(2)?,
        value: if password { None } else { label(3)? },
        checked: bridge::tri_state(optional_int(4)?),
        selection,
        disabled: bridge::tri_state(optional_int(7)?),
        editable: bridge::tri_state(optional_int(8)?),
        read_only: bridge::tri_state(optional_int(9)?),
        password,
        description: label(10)?,
        autofill,
        order,
        live,
    })
}

fn recognizers<'js>(ctx: &Ctx<'js>, spec: &Value<'js>) -> rquickjs::Result<RecognizerSet> {
    let drag = match string_property(ctx, spec, "drag")?.as_deref() {
        Some("free") => Some(DragAxis::Free),
        Some("horizontal") => Some(DragAxis::Horizontal),
        Some("vertical") => Some(DragAxis::Vertical),
        _ => None,
    };
    let tap = Coerced::<bool>::from_js(ctx, property(ctx, spec, "tap")?)?.0;
    let long_press = Coerced::<bool>::from_js(ctx, property(ctx, spec, "longPress")?)?.0;
    let window_ms = Coerced::<f64>::from_js(ctx, property(ctx, spec, "doubleTapMs")?)?.0;
    let double_tap = (window_ms > 0.0).then_some(DoubleTapConfig {
        slop: DRAG_SLOP,
        window_ms,
    });
    Ok(RecognizerSet {
        tap,
        long_press,
        drag,
        double_tap,
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript numbers decode to scroll physics fields"
)]
fn scroll_physics<'js>(ctx: &Ctx<'js>, args: &[Value<'js>]) -> rquickjs::Result<ScrollPhysics> {
    let numeric = |index| num(ctx, args, index).map(|n| n as f32);
    let optional = |index| -> rquickjs::Result<f64> {
        if boolean(ctx, args, index)? {
            num(ctx, args, index)
        } else {
            Ok(0.0)
        }
    };
    let tau = f64::from(numeric(1)?);
    let mode = int(ctx, args, 2)?;
    let stiffness = numeric(3)?;
    let damping = numeric(4)?;
    let factor = numeric(5)?;
    let snap_x = nodes::snap_grid(numeric(6)?, int(ctx, args, 7)?);
    let snap_y = nodes::snap_grid(numeric(8)?, int(ctx, args, 9)?);
    let accept = ScrollAccept::from_code(optional(10)? as i32);
    let decay = if optional(11)? as i32 == 1 {
        Decay::Rate(tau)
    } else {
        Decay::Tau(tau)
    };
    let snap_stiffness = optional(12)? as f32;
    let snap_damping = optional(13)? as f32;
    let band = if optional(14)? as i32 == 1 {
        Band::Curve { c: factor }
    } else {
        Band::Linear { factor }
    };
    Ok(ScrollPhysics {
        decay,
        band,
        overscroll: if mode == 1 {
            OverscrollMode::Clamp
        } else {
            OverscrollMode::RubberBand
        },
        overscroll_stiffness: stiffness,
        overscroll_damping: damping,
        overscroll_factor: factor,
        snap_stiffness,
        snap_damping,
        snap_x,
        snap_y,
        accept,
    })
}
