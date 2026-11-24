//! Native renderer primitives for the Solid scene.
use super::super::mutation::{Create, Mutation};
use super::bindings::{apply, class_snapshot, handle, node_id, string, style_snapshot, value};
use super::*;
use crate::motion::ScrollAxis;
use crate::style::StyleOrigin;
use crate::tree::NodeId;
use rquickjs::{Coerced, FromJs};

pub(super) fn call<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    canvas::side_effect(ctx)?;
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
            let parent = node_id(ctx, args, 0)?;
            let child = node_id(ctx, args, 1)?;
            let anchor = handle(ctx, args, 2)?;
            let before = (anchor != 0).then(|| NodeId::from_raw(anchor));
            apply(
                host,
                ctx,
                Mutation::Insert {
                    parent,
                    child,
                    before,
                },
            )?;
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
            #[cfg(debug_assertions)]
            let _trace =
                tracing::trace_span!(target: "impasto::script::solid", "component_mount").entered();
            let component = Function::from_value(value(ctx, args, 0))?;
            let props = if args.len() > 1 {
                args[1].clone()
            } else {
                Object::new(ctx.clone())?.into_value()
            };
            let graph = &host.inner().graph;
            let reader = graph.push_untrack();
            let result = component.call((props,));
            graph.pop_reader(reader);
            result
        }
        _ => Err(type_error(ctx, "unknown renderer primitive")),
    }
}
fn text_node<'js>(host: &Host, ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let text = Coerced::<String>::from_js(ctx, value)?.0;
    (apply(host, ctx, Mutation::Create(Create::Text { text }))? as f64).into_js(ctx)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JS sizes use f64 and tree geometry uses f32"
)]
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
    let (create, construction_props): (_, &[_]) = match tag.as_str() {
        "view" => (Create::View, &[]),
        "stack" => (Create::Stack, &[]),
        "scroll" => {
            let axis: Option<String> = props.get("axis")?;
            let axis = scroll_axis(ctx, axis.as_deref())?;
            (Create::Scroll(axis), &["axis"])
        }
        "text" => {
            let val: Value = props.get("value")?;
            let text = if val.is_undefined() {
                String::new()
            } else {
                Coerced::<String>::from_js(ctx, val)?.0
            };
            (Create::Text { text }, &["value"])
        }
        "image" => {
            let source = props
                .get::<_, Option<String>>("source")?
                .unwrap_or_default();
            let required = props
                .get::<_, Option<bool>>("required")?
                .unwrap_or_default();
            let (width, height) =
                if let Some(size) = props.get::<_, Option<Object>>("intrinsicSize")? {
                    let width: f64 = size.get("width")?;
                    let height: f64 = size.get("height")?;
                    if ![width, height]
                        .into_iter()
                        .all(|size| size.is_finite() && size >= 0.0 && size <= f64::from(f32::MAX))
                    {
                        return Err(type_error(ctx, "invalid intrinsic image size"));
                    }
                    (width as f32, height as f32)
                } else {
                    (0.0, 0.0)
                };
            (
                Create::Image {
                    source,
                    width,
                    height,
                    required,
                },
                &["source", "required", "intrinsicSize"],
            )
        }
        _ => return Err(type_error(ctx, "unknown JSX element")),
    };
    let raw = apply(host, ctx, Mutation::Create(create))?;
    let id = (raw as f64).into_js(ctx)?;
    // Conditional JSX branches which never attach retire with their owner.
    host.inner()
        .graph
        .on_cleanup(reactive::Cleanup::UnattachedNode(
            host.downgrade(),
            NodeId::from_raw(raw),
        ));
    for key in props.keys::<String>() {
        let key = key?;
        if !construction_props.contains(&key.as_str()) {
            let val = props.get(key.as_str())?;
            set_prop(host, ctx, &[id.clone(), key.into_js(ctx)?, val])?;
        }
    }
    Ok(id)
}

fn scroll_axis(ctx: &Ctx<'_>, axis: Option<&str>) -> rquickjs::Result<ScrollAxis> {
    match axis {
        None | Some("both") => Ok(ScrollAxis::Both),
        Some("vertical") => Ok(ScrollAxis::Vertical),
        Some("horizontal") => Ok(ScrollAxis::Horizontal),
        _ => Err(type_error(ctx, "unknown scroll axis")),
    }
}

fn set_prop<'js>(host: &Host, ctx: &Ctx<'js>, args: &[Value<'js>]) -> rquickjs::Result<Value<'js>> {
    if args.len() < 3 || !args[1].is_string() {
        return Ok(Value::new_undefined(ctx.clone()));
    }
    let name = string(ctx, args, 1)?;
    let id = node_id(ctx, args, 0)?;
    let value = args[2].clone();
    match name.as_str() {
        "style" => style_snapshot(host, ctx, id, StyleOrigin::Inline, true, value.clone())?,
        "class" | "classList" => class_snapshot(host, ctx, id, name == "classList", value.clone())?,
        _ => {
            if let Some(previous) = args.get(3)
                && strict_equal(ctx, &value, previous)
            {
                return Ok(value);
            }
            if name == "axis" {
                let axis = Option::<String>::from_js(ctx, value.clone())?;
                apply(
                    host,
                    ctx,
                    Mutation::ScrollAxis {
                        id,
                        axis: scroll_axis(ctx, axis.as_deref())?,
                    },
                )?;
                return Ok(value);
            }
            let method = match name.as_str() {
                "children" | "ref" => return Ok(Value::new_undefined(ctx.clone())),
                "value" => "setText",
                "runs" => "setTextRuns",
                "focusable" => "setFocusable",
                "recognizers" => "setRecognizers",
                name if name.starts_with("on") => name,
                _ => return Err(type_error(ctx, "unknown JSX property")),
            };
            host.api(ctx, false, method, vec![args[0].clone(), value.clone()])?;
        }
    }
    Ok(value)
}
