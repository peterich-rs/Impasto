//! Renderer primitives use the same mutation path as the named scene API.
use super::super::scene::scene_field;
use super::bindings::{apply, empty_record, handle, string, style_snapshot, value};
use super::*;
use crate::abi::{CreateKind, MutationOp};
use rquickjs::{Array, Coerced, FromJs};

pub(super) fn call<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    js::host::OperationPolicy::before(host, ctx)?;
    match name {
        "createElement" => create_element(host, ctx, args),
        "createTextNode" => text_node(
            host,
            ctx,
            if args.is_empty() {
                "".into_js(ctx)?
            } else {
                args[0].clone()
            },
        ),
        "setProp" => set_prop(host, ctx, args),
        "insertNode" => {
            if args.len() < 2 {
                return Ok(Value::new_undefined(ctx.clone()));
            }
            let mut record = empty_record(MutationOp::Insert as u32);
            record.id = handle(ctx, args, 1)?;
            record.parent = handle(ctx, args, 0)?;
            record.before = handle(ctx, args, 2)?;
            apply(host, ctx, &record)?;
            Ok(Value::new_undefined(ctx.clone()))
        }
        "effect" | "memo" => host.api(
            ctx,
            true,
            if name == "memo" {
                "createMemo"
            } else {
                "createRenderEffect"
            },
            args.to_vec(),
        ),
        "createComponent" => {
            let component = Function::from_value(value(ctx, args, 0))?;
            let props = if args.len() > 1 {
                args[1].clone()
            } else {
                Object::new(ctx.clone())?.into_value()
            };
            let reader = reactive::reactive_entry(host.inner(), 12, &[]) & !(1 << 63);
            let result = component.call((props,));
            reactive::reactive_entry(host.inner(), 13, &[reader]);
            result
        }
        _ => Err(type_error(ctx, "unknown renderer primitive")),
    }
}
fn text_node<'js>(host: &Host, ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let text = Coerced::<String>::from_js(ctx, value)?.0;
    let mut record = empty_record(MutationOp::Create as u32);
    record.field = CreateKind::Text as u32;
    record.number = 16.0;
    record.color = [24, 24, 27, 255];
    record.text = text;
    (apply(host, ctx, &record)? as f64).into_js(ctx)
}
fn create_element<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    let tag = string(ctx, args, 0)?;
    let props = match value(ctx, args, 1).into_object() {
        Some(props) => props,
        None => Object::new(ctx.clone())?,
    };
    let keys: &[&str] = match tag.as_str() {
        "text" => &["value"],
        "column" | "row" => &["gap"],
        "padding" => &["edges"],
        "align" => &["align"],
        "grid" => &["columns", "rows", "gap"],
        "clip" => &["radius"],
        "opacity" => &["alpha"],
        "decorated" => &["color", "radius"],
        "scroll" => &["axis"],
        "image" => &["source", "w", "h", "mode", "required"],
        _ => &[],
    };
    let mut creation = Vec::new();
    for index in 0..5 {
        let mut val = if let Some(key) = keys.get(index) {
            props.get::<_, Value>(*key)?
        } else {
            Value::new_undefined(ctx.clone())
        };
        if val.is_undefined() {
            val = match (tag.as_str(), index) {
                ("grid", 0 | 1) => Array::new(ctx.clone())?.into_value(),
                ("align", 0) => "start".into_js(ctx)?,
                ("image", 0) => "".into_js(ctx)?,
                ("opacity", 0) => 255.into_js(ctx)?,
                ("decorated", 0) => "#e4e4e7".into_js(ctx)?,
                _ => 0.into_js(ctx)?,
            };
        }
        if tag == "decorated"
            && index == 0
            && let Some(text) = val.as_string()
        {
            let channels = parse_hex(&text.to_string()?).unwrap_or([0, 0, 0, 255]);
            let array = Array::new(ctx.clone())?;
            for (i, channel) in channels.iter().enumerate() {
                array.set(i, *channel)?;
            }
            val = array.into_value();
        }
        if tag == "scroll" && index == 0 && val.is_string() {
            let text = Coerced::<String>::from_js(ctx, val)?.0;
            val = match text.as_str() {
                "vertical" => 1,
                "horizontal" => 2,
                _ => 0,
            }
            .into_js(ctx)?;
        }
        creation.push(val);
    }
    let id = if tag == "text" {
        text_node(host, ctx, creation[0].clone())?
    } else {
        host.api(
            ctx,
            false,
            if tag == "pointer" {
                "pointerRegion"
            } else {
                &tag
            },
            creation,
        )?
    };
    // JSX may compute several branches before the insertion effect commits a range.
    // By the end of the update, any discarded branch which never attached has
    // no tree parent to retire it.
    let node = uicore::NodeId::from_raw(handle(ctx, std::slice::from_ref(&id), 0)?);
    host.inner()
        .graph
        .arena
        .on_cleanup(reactive::Cleanup::UnattachedNode(
            ptr::from_ref(host).cast_mut(),
            node,
        ));
    for key in props.keys::<String>() {
        let key = key?;
        if keys.contains(&key.as_str()) {
            continue;
        }
        let val = props.get(key.as_str())?;
        set_prop(host, ctx, &[id.clone(), key.into_js(ctx)?, val])?;
    }
    Ok(id)
}
fn scene_num(value: &Value<'_>) -> f64 {
    value.as_number().unwrap_or(0.0)
}
fn object_num(value: &Value<'_>, key: &str) -> rquickjs::Result<f64> {
    match value.as_object() {
        Some(object) => Ok(scene_num(&object.get(key)?)),
        None => Ok(0.0),
    }
}
fn named(name: &str, table: &[&str]) -> f64 {
    table.iter().position(|s| *s == name).unwrap_or(0) as f64
}
fn set_prop<'js>(host: &Host, ctx: &Ctx<'js>, args: &[Value<'js>]) -> rquickjs::Result<Value<'js>> {
    if args.len() < 3 || !args[1].is_string() {
        return Ok(Value::new_undefined(ctx.clone()));
    }
    let name = string(ctx, args, 1)?;
    let id = handle(ctx, args, 0)?;
    let value = args[2].clone();
    if matches!(name.as_str(), "style" | "class" | "classList") {
        let (op, origin) = match name.as_str() {
            "style" => (MutationOp::StyleSnapshot, 2),
            "classList" => (MutationOp::ClassSnapshot, 1),
            _ => (MutationOp::ClassSnapshot, 0),
        };
        style_snapshot(host, ctx, op, id, origin, true, value.clone())?;
        return Ok(value);
    }
    let field = scene_field(&name);
    if let Some(previous) = args.get(3)
        && strict_equal(ctx, &value, previous)
    {
        return Ok(value);
    }
    if field == 0 {
        let method = match name.as_str() {
            "children" | "ref" => return Ok(Value::new_undefined(ctx.clone())),
            "value" => "setText",
            "runs" => "setTextRuns",
            "recognizers" => "setRecognizers",
            name if name.starts_with("on") => name,
            _ => return Err(type_error(ctx, "unknown JSX property")),
        };
        host.api(ctx, false, method, vec![args[0].clone(), value.clone()])?;
        return Ok(value);
    }
    let mut record = empty_record(MutationOp::SetStyle as u32);
    record.id = id;
    record.field = field;
    match field {
        9 | 10 => {
            if value.is_number() {
                record.number = scene_num(&value);
                record.number2 = record.number;
            } else {
                record.number = object_num(&value, "w")?;
                record.number2 = object_num(&value, "h")?;
            }
        }
        4 => {
            let n = scene_num(&value);
            record.number = n;
            record.number2 = n;
            record.number3 = n;
            record.number4 = n;
        }
        48 => {
            record.number = match name.as_str() {
                "right" => 1.0,
                "bottom" => 2.0,
                "left" => 3.0,
                _ => 0.0,
            };
            record.number2 = scene_num(&value);
        }
        49 => {
            record.number = match name.as_str() {
                "insetRight" => 1.0,
                "insetBottom" => 2.0,
                "insetLeft" => 3.0,
                _ => 0.0,
            };
            if value.is_undefined()
                || value.is_null()
                || value
                    .as_string()
                    .is_some_and(|s| s.to_string().is_ok_and(|s| s == "auto"))
            {
                record.number3 = 1.0;
            } else if value.is_number() {
                record.number2 = scene_num(&value);
            } else {
                return Err(type_error(ctx, "bad inset"));
            }
        }
        50 => {
            record.number = match name.as_str() {
                "columnEnd" => 1.0,
                "rowStart" => 2.0,
                "rowEnd" => 3.0,
                _ => 0.0,
            };
            if !value.is_string() {
                let object = Object::from_value(value.clone())?;
                let line: Value = object.get("line")?;
                let span: Value = object.get("span")?;
                if line.is_number() {
                    record.number2 = 1.0;
                    record.number3 = scene_num(&line);
                } else if span.is_number() {
                    record.number2 = 2.0;
                    record.number3 = scene_num(&span);
                }
            }
        }
        44 => {
            record.number = f64::from(name == "h");
            if value.is_undefined() {
                record.number2 = 0.0;
            } else if let Some(number) = value.as_number() {
                if !number.is_finite() || number < 0.0 {
                    return Err(type_error(ctx, "bad axis"));
                }
                record.number2 = 1.0;
                record.number3 = number;
            } else if let Some(text) = value.as_string() {
                let text = text.to_string()?;
                if text == "auto" {
                    record.number2 = 0.0;
                } else if let Some(percent) = text.strip_suffix('%') {
                    let number = percent
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| type_error(ctx, "bad axis"))?;
                    if !number.is_finite() || number < 0.0 {
                        return Err(type_error(ctx, "bad axis"));
                    }
                    record.number2 = 2.0;
                    record.number3 = number / 100.0;
                } else {
                    return Err(type_error(ctx, "bad axis"));
                }
            } else {
                return Err(type_error(ctx, "bad axis"));
            }
        }
        43 => {
            record.number = match name.as_str() {
                "y" => 1.0,
                "rotate" => 2.0,
                "scale" => 3.0,
                _ => 0.0,
            };
            record.number2 = scene_num(&value);
        }
        34 => {
            if !value.is_null() && !value.is_undefined() {
                record.number = 1.0;
                record.number2 = scene_num(&value);
            }
        }
        26 => {
            if let Some(array) = value.as_array() {
                for (i, c) in record.color.iter_mut().enumerate() {
                    let v: Coerced<i32> = array.get(i)?;
                    *c = u8::try_from(v.0).map_err(|_| type_error(ctx, "bad color"))?;
                }
            } else if let Some(text) = value.as_string() {
                record.color = parse_hex(&text.to_string()?).unwrap_or([0; 4]);
            }
        }
        _ => {
            if let Some(text) = value.as_string() {
                let word = text.to_string()?;
                record.number = match field {
                    17 => named(&word, &["left", "center", "right"]),
                    11 | 54 => named(&word, &["start", "center", "end"]),
                    12 => named(
                        &word,
                        &[
                            "start",
                            "center",
                            "end",
                            "space-between",
                            "space-around",
                            "space-evenly",
                        ],
                    ),
                    35 => named(&word, &["start", "center", "end", "stretch", "baseline"]),
                    38 => match word.as_str() {
                        "vertical" => 1.0,
                        "horizontal" => 2.0,
                        _ => 0.0,
                    },
                    30 => match word.as_str() {
                        "ellipsis" => 1.0,
                        "visible" => 2.0,
                        _ => 0.0,
                    },
                    31 => f64::from(word == "rtl"),
                    _ => {
                        record.text = word;
                        0.0
                    }
                };
            } else if let Some(flag) = value.as_bool() {
                record.number = f64::from(flag);
            } else {
                record.number = scene_num(&value);
            }
        }
    }
    if field == 36 && !Coerced::<bool>::from_js(ctx, value.clone())?.0 {
        return Ok(Value::new_undefined(ctx.clone()));
    }
    apply(host, ctx, &record)?;
    Ok(value)
}

fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return None;
    }
    let mut channels = [0, 0, 0, 255];
    for (i, channel) in channels.iter_mut().enumerate().take(hex.len() / 2) {
        *channel = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(channels)
}
