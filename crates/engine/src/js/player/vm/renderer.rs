//! Native renderer primitives for the Solid scene.
use super::super::mutation::{Create, Mutation, TemplateCreate, TemplateNode, mount_template};
use super::bindings::{
    apply, class_snapshot, decode_style, handle, node_id, string, style_snapshot, value,
};
use super::*;
use crate::motion::ScrollAxis;
use crate::style::StyleOrigin;
use crate::tree::NodeId;
use rquickjs::{Coerced, FromJs};
use std::collections::HashSet;

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
        "defineStyle" => {
            let source = value(ctx, args, 0);
            let style = if source.is_string() {
                crate::style::static_pool::decode_style(&String::from_js(ctx, source)?)
            } else {
                let value = decode_style(ctx, source, 0, &mut 0)?;
                crate::style::codec::from_value(&value).map_err(|error| error.to_string())
            }
            .map_err(|error| type_error(ctx, &error))?;
            let mut styles = host.inner().static_styles.borrow_mut();
            let index = u16::try_from(styles.len())
                .map_err(|_| type_error(ctx, "static style pool is full"))?;
            styles.push(style);
            index.into_js(ctx)
        }
        "applyStaticStyle" => {
            let id = node_id(ctx, args, 0)?;
            let index = u16::from_js(ctx, value(ctx, args, 1))?;
            let mut style = host
                .inner()
                .static_styles
                .borrow()
                .get(usize::from(index))
                .cloned()
                .ok_or_else(|| type_error(ctx, "unknown static style"))?;
            if let Some(opacity) = args.get(2) {
                let opacity = decode_style(ctx, opacity.clone(), 0, &mut 0)?;
                let declarations = crate::style::codec::declarations("opacity", &opacity)
                    .map_err(|error| type_error(ctx, &error.to_string()))?;
                style
                    .push_fragment(declarations)
                    .map_err(|error| type_error(ctx, &error.to_string()))?;
            }
            apply(
                host,
                ctx,
                Mutation::StyleSnapshot {
                    id,
                    origin: StyleOrigin::Inline,
                    replace: true,
                    style,
                },
            )?;
            Ok(Value::new_undefined(ctx.clone()))
        }
        "defineClass" => {
            let source = String::from_js(ctx, value(ctx, args, 0))?;
            let class = crate::style::decode_static_class(&source)
                .map_err(|error| type_error(ctx, &error))?;
            let mut classes = host.inner().static_classes.borrow_mut();
            let index = u16::try_from(classes.len())
                .map_err(|_| type_error(ctx, "static class pool is full"))?;
            classes.push(class);
            index.into_js(ctx)
        }
        "applyStaticClass" => {
            let id = node_id(ctx, args, 0)?;
            let index = usize::from(u16::from_js(ctx, value(ctx, args, 1))?);
            let inner = host.inner();
            let class = inner
                .static_classes
                .borrow()
                .get(index)
                .cloned()
                .ok_or_else(|| type_error(ctx, "unknown static class"))?;
            apply(host, ctx, Mutation::StaticClass(id, class))?;
            Ok(Value::new_undefined(ctx.clone()))
        }
        "defineTemplate" => {
            let rows = Vec::<rquickjs::Array>::from_js(ctx, value(ctx, args, 0))?;
            if rows.is_empty() || rows.len() > 256 {
                return Err(type_error(ctx, "invalid template size"));
            }
            let mut template = Vec::with_capacity(rows.len());
            for (index, row) in rows.into_iter().enumerate() {
                let tag: String = row.get(0)?;
                let parent: usize = row.get(1)?;
                if (index == 0 && parent != 0) || (index != 0 && parent >= index) {
                    return Err(type_error(ctx, "invalid template parent"));
                }
                let create = match tag.as_str() {
                    "view" => TemplateCreate::View,
                    "stack" => TemplateCreate::Stack,
                    "scroll" => TemplateCreate::Scroll,
                    "text" => TemplateCreate::Text(row.get(2)?),
                    _ => return Err(type_error(ctx, "unknown template element")),
                };
                let style = row
                    .get::<Option<String>>(3)?
                    .as_deref()
                    .map(crate::style::static_pool::decode_style)
                    .transpose()
                    .map_err(|error| type_error(ctx, &error))?;
                let class = row
                    .get::<Option<String>>(4)?
                    .as_deref()
                    .map(crate::style::decode_static_class)
                    .transpose()
                    .map_err(|error| type_error(ctx, &error))?;
                template.push(TemplateNode {
                    create,
                    parent,
                    style,
                    class,
                });
            }
            let mut templates = host.inner().static_templates.borrow_mut();
            let index = u16::try_from(templates.len())
                .map_err(|_| type_error(ctx, "static template pool is full"))?;
            templates.push(template.into_boxed_slice());
            index.into_js(ctx)
        }
        "instantiateTemplate" => {
            let index = u16::from_js(ctx, value(ctx, args, 0))?;
            let with_bindings =
                Option::<bool>::from_js(ctx, value(ctx, args, 1))?.unwrap_or_default();
            let inner = host.inner();
            let templates = inner.static_templates.borrow();
            let template = templates
                .get(usize::from(index))
                .ok_or_else(|| type_error(ctx, "unknown template"))?;
            let nodes = mount_template(&mut inner.tree.borrow_mut(), template)
                .map_err(|error| type_error(ctx, &error.to_string()))?;
            let root = nodes[0];
            inner
                .graph
                .on_cleanup(reactive::Cleanup::UnattachedNode(host.downgrade(), root));
            let marks = &inner.marks;
            super::super::frame::bump(&marks.ffi_batches);
            let created = u32::try_from(template.len()).unwrap_or(u32::MAX);
            marks
                .nodes_created
                .set(marks.nodes_created.get().saturating_add(created));
            if with_bindings {
                return nodes
                    .into_iter()
                    .map(|node| node.to_raw() as f64)
                    .collect::<Vec<_>>()
                    .into_js(ctx);
            }
            (root.to_raw() as f64).into_js(ctx)
        }
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
pub(super) fn reconcile_range<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    let parent = node_id(ctx, args, 0)?;
    let list = |index| {
        Vec::<Value>::from_js(ctx, value(ctx, args, index))?
            .iter()
            .map(|value| handle(ctx, std::slice::from_ref(value), 0))
            .collect::<rquickjs::Result<Vec<_>>>()
    };
    let previous = list(1)?;
    let next = list(2)?;
    let anchor = handle(ctx, args, 3)?;
    let mut before = (anchor != 0).then(|| NodeId::from_raw(anchor));
    let inner = host.inner();
    let tree = &inner.tree;
    let invalid = |_| type_error(ctx, "invalid scene range");

    let retained = next.iter().copied().collect::<HashSet<_>>();
    for raw in previous {
        if !retained.contains(&raw) {
            let child = NodeId::from_raw(raw);
            let attached = tree.borrow().parent(child).map_err(invalid)? == Some(parent);
            if attached {
                apply(host, ctx, Mutation::Remove { parent, child })?;
            }
        }
    }
    for raw in next.into_iter().rev() {
        let child = NodeId::from_raw(raw);
        let already_placed = {
            let tree = tree.borrow();
            tree.parent(child).map_err(invalid)? == Some(parent)
                && tree.next_sibling(child).map_err(invalid)? == before
        };
        if !already_placed {
            apply(
                host,
                ctx,
                Mutation::Insert {
                    parent,
                    child,
                    before,
                },
            )?;
        }
        before = Some(child);
    }
    super::super::frame::bump(&inner.marks.ffi_batches);
    Ok(Value::new_undefined(ctx.clone()))
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
