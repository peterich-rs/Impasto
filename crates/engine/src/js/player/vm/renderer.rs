//! Native renderer primitives for the Solid scene.
use super::super::mutation::{Create, Mutation, TemplateCreate, TemplateNode, mount_template};
use super::bindings::{
    apply, class_snapshot, decode_style, handle, node_id, string, style_snapshot, value,
};
use super::*;
use crate::motion::ScrollAxis;
use crate::scene::template::{self, TemplateImage, TemplateKind};
use crate::style::{Declaration, DisplayMode, FlexDir, Property, Style, StyleOrigin, codec};
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
        "defineStyle" => define_style(host, ctx, value(ctx, args, 0))?.into_js(ctx),
        "applyStaticStyle" => {
            let id = node_id(ctx, args, 0)?;
            let index = style_handle(host, ctx, value(ctx, args, 1))?;
            let mut style = host
                .inner()
                .static_styles
                .borrow()
                .get(usize::from(index))
                .cloned()
                .ok_or_else(|| type_error(ctx, "unknown static style"))?;
            if let Some(opacity) = args.get(2) {
                let opacity = decode_style(ctx, opacity.clone(), 0, &mut 0)?;
                let declarations = codec::declarations("opacity", &opacity)
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
        "patchInlineStyle" => {
            let id = node_id(ctx, args, 0)?;
            let selector = value(ctx, args, 1);
            let style = decode_style(ctx, value(ctx, args, 2), 0, &mut 0)?;
            let error = |error: crate::style::StyleError| type_error(ctx, &error.to_string());
            let (keys, declarations): (Vec<_>, _) = if selector.is_array() {
                let properties = Vec::<String>::from_js(ctx, selector)?;
                let mut keys = Vec::new();
                for key in properties {
                    keys.extend(
                        codec::declarations(&key, &codec::Value::Null)
                            .map_err(error)?
                            .iter()
                            .map(Declaration::key),
                    );
                }
                let declarations =
                    crate::style::flatten(&codec::from_value(&style).map_err(error)?);
                if declarations.keys().any(|key| !keys.contains(key)) {
                    return Err(type_error(
                        ctx,
                        "style patch defines a key outside its selector",
                    ));
                }
                (keys, declarations.into_values().collect())
            } else {
                let name = String::from_js(ctx, selector)?;
                let keys = codec::declarations(&name, &codec::Value::Null)
                    .map_err(error)?
                    .iter()
                    .map(Declaration::key)
                    .collect();
                let declarations = codec::declarations(&name, &style).map_err(error)?;
                (keys, declarations)
            };
            apply(
                host,
                ctx,
                Mutation::InlinePatch {
                    id,
                    keys: keys.into(),
                    declarations,
                },
            )?;
            Ok(Value::new_undefined(ctx.clone()))
        }
        "patchScalarStyle" => {
            let id = node_id(ctx, args, 0)?;
            let selector = value(ctx, args, 1);
            let op = selector
                .as_int()
                .and_then(|code| u8::try_from(code).ok())
                .and_then(codec::ScalarStyleOp::from_code)
                .ok_or_else(|| type_error(ctx, "invalid scalar style opcode"))?;
            let style = value(ctx, args, 2);
            let declarations = if let Some(number) = style.as_number() {
                op.number(number)
            } else {
                codec::declarations(op.name(), &decode_style(ctx, style, 0, &mut 0)?)
            }
            .map_err(|error| type_error(ctx, &error.to_string()))?;
            apply(
                host,
                ctx,
                Mutation::InlinePatch {
                    id,
                    keys: op.keys().into(),
                    declarations,
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
        "applyStaticClassList" => {
            let id = node_id(ctx, args, 0)?;
            let indices = Vec::<Option<u16>>::from_js(ctx, value(ctx, args, 1))?;
            let pool = host.inner().static_classes.borrow();
            let mut bytes = 0usize;
            let mut classes = Vec::new();
            for index in indices.into_iter().flatten() {
                let (text, parsed) = pool
                    .get(usize::from(index))
                    .cloned()
                    .ok_or_else(|| type_error(ctx, "unknown static class"))?
                    .into_parts();
                if !text.is_empty() {
                    bytes = bytes
                        .saturating_add(text.len())
                        .saturating_add(usize::from(bytes != 0));
                    if bytes > crate::style::CLASS_BYTE_LIMIT {
                        return Err(type_error(ctx, "static class list is too long"));
                    }
                }
                classes.push(parsed);
            }
            drop(pool);
            apply(host, ctx, Mutation::StaticClassList { id, classes, bytes })?;
            Ok(Value::new_undefined(ctx.clone()))
        }
        "defineTemplate" => {
            let source = value(ctx, args, 0);
            let template = if let Some(encoded) = source.as_string() {
                decode_template(
                    host,
                    ctx,
                    &encoded.to_string()?,
                    value(ctx, args, 1),
                    value(ctx, args, 2),
                )?
            } else {
                legacy_template(ctx, source)?
            };
            store_template(host, ctx, template)?.into_js(ctx)
        }
        "instantiateTemplate" => {
            let source = value(ctx, args, 0);
            let index = if let Some(descriptor) = source.as_array() {
                if let Some(index) = descriptor.get::<Option<u16>>(3)? {
                    index
                } else {
                    let encoded: String = descriptor.get(0)?;
                    let template = decode_template(
                        host,
                        ctx,
                        &encoded,
                        descriptor.get(1)?,
                        descriptor.get(2)?,
                    )?;
                    let index = store_template(host, ctx, template)?;
                    descriptor.set(3, index)?;
                    index
                }
            } else {
                u16::from_js(ctx, source)?
            };
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
        "compileHotFactory" => {
            let filename = string(ctx, args, 0)?;
            let body = string(ctx, args, 1)?;
            let mut options = rquickjs::context::EvalOptions::default();
            options.filename = Some(filename);
            options.strict = false;
            ctx.eval_with_options(
                format!("(function(module, exports, require) {{\n{body}\n}})"),
                options,
            )
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
fn define_style<'js>(host: &Host, ctx: &Ctx<'js>, source: Value<'js>) -> rquickjs::Result<u16> {
    let style = if source.is_string() {
        crate::style::static_pool::decode_style(&String::from_js(ctx, source)?)
    } else {
        let value = decode_style(ctx, source, 0, &mut 0)?;
        codec::from_value(&value).map_err(|error| error.to_string())
    }
    .map_err(|error| type_error(ctx, &error))?;
    let mut styles = host.inner().static_styles.borrow_mut();
    let index =
        u16::try_from(styles.len()).map_err(|_| type_error(ctx, "static style pool is full"))?;
    styles.push(style);
    Ok(index)
}

fn style_handle<'js>(host: &Host, ctx: &Ctx<'js>, source: Value<'js>) -> rquickjs::Result<u16> {
    let Some(descriptor) = source.as_array() else {
        return u16::from_js(ctx, source);
    };
    if let Some(index) = descriptor.get::<Option<u16>>(1)? {
        return Ok(index);
    }
    let index = define_style(host, ctx, descriptor.get(0)?)?;
    descriptor.set(1, index)?;
    Ok(index)
}

fn store_template(
    host: &Host,
    ctx: &Ctx<'_>,
    template: Vec<TemplateNode>,
) -> rquickjs::Result<u16> {
    let mut templates = host.inner().static_templates.borrow_mut();
    let index = u16::try_from(templates.len())
        .map_err(|_| type_error(ctx, "static template pool is full"))?;
    templates.push(template.into_boxed_slice());
    Ok(index)
}

fn template_create(
    ctx: &Ctx<'_>,
    kind: TemplateKind,
    text: Option<String>,
    image: Option<TemplateImage>,
) -> rquickjs::Result<(TemplateCreate, Option<Style>)> {
    let preset = match kind {
        TemplateKind::Row => Some(Property::FlexDirection(FlexDir::Row)),
        TemplateKind::Column => Some(Property::FlexDirection(FlexDir::Column)),
        TemplateKind::Grid => Some(Property::Display(DisplayMode::Grid)),
        _ => None,
    };
    let defaults = preset
        .map(|property| Style::fragment(vec![Declaration::Set(property)]))
        .transpose()
        .map_err(|error| type_error(ctx, &error.to_string()))?;
    let create = match kind {
        TemplateKind::View | TemplateKind::Row | TemplateKind::Column | TemplateKind::Grid => {
            TemplateCreate::View
        }
        TemplateKind::RepaintBoundary => TemplateCreate::RepaintBoundary,
        TemplateKind::Stack => TemplateCreate::Stack,
        TemplateKind::Scroll => TemplateCreate::Scroll,
        TemplateKind::Image => TemplateCreate::Image(image),
        TemplateKind::Text => {
            TemplateCreate::Text(text.ok_or_else(|| type_error(ctx, "missing template text"))?)
        }
    };
    Ok((create, defaults))
}

fn legacy_template<'js>(ctx: &Ctx<'js>, source: Value<'js>) -> rquickjs::Result<Vec<TemplateNode>> {
    let rows = Vec::<rquickjs::Array>::from_js(ctx, source)?;
    if rows.is_empty() || rows.len() > 256 {
        return Err(type_error(ctx, "invalid template size"));
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let tag: String = row.get(0)?;
            let parent: usize = row.get(1)?;
            if (index == 0 && parent != 0) || (index != 0 && parent >= index) {
                return Err(type_error(ctx, "invalid template parent"));
            }
            let kind = TemplateKind::parse(&tag)
                .ok_or_else(|| type_error(ctx, "unknown template element"))?;
            let text = if kind == TemplateKind::Text {
                Some(row.get(2)?)
            } else {
                None
            };
            let image = row
                .get::<Option<Object>>(5)?
                .map(|record| {
                    let size = record
                        .get::<_, Option<rquickjs::Array>>("intrinsic_size_bits")?
                        .map(|bits| Ok::<_, rquickjs::Error>((bits.get(0)?, bits.get(1)?)))
                        .transpose()?;
                    if size.is_some_and(|(w, h)| {
                        [w, h].into_iter().any(|bits| {
                            let value = f32::from_bits(bits);
                            !value.is_finite() || value < 0.
                        })
                    }) {
                        return Err(type_error(ctx, "invalid template image dimensions"));
                    }
                    Ok(TemplateImage {
                        uri: record.get("uri")?,
                        intrinsic_size_bits: size,
                        required: record.get("required")?,
                        alt: record.get("alt")?,
                    })
                })
                .transpose()?;
            if image.is_some() && kind != TemplateKind::Image {
                return Err(type_error(ctx, "invalid template image slot"));
            }
            let (create, defaults) = template_create(ctx, kind, text, image)?;
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
            Ok(TemplateNode {
                create,
                parent,
                style,
                defaults,
                class,
            })
        })
        .collect()
}

fn decode_template<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    encoded: &str,
    style_handles: Value<'js>,
    class_handles: Value<'js>,
) -> rquickjs::Result<Vec<TemplateNode>> {
    let template::Template { rows, images } =
        template::decode(encoded).map_err(|error| type_error(ctx, &error))?;
    let style_handles =
        template_handles(ctx, style_handles, |value| style_handle(host, ctx, value))?;
    let class_handles = template_handles(ctx, class_handles, |value| {
        if let Some(get) = value.as_function() {
            get.call(())
        } else {
            u16::from_js(ctx, value)
        }
    })?;
    let inner = host.inner();
    let styles = inner.static_styles.borrow();
    let classes = inner.static_classes.borrow();
    rows.into_iter()
        .map(|row| {
            let image = row
                .image
                .and_then(|slot| images.get(usize::from(slot)))
                .cloned();
            let (create, defaults) = template_create(ctx, row.kind, row.text, image)?;
            let style = template_dependency(ctx, &style_handles, &styles, row.style)?;
            let class = template_dependency(ctx, &class_handles, &classes, row.class)?;
            Ok(TemplateNode {
                create,
                parent: usize::from(row.parent),
                style,
                defaults,
                class,
            })
        })
        .collect()
}

fn template_handles<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    get: impl Fn(Value<'js>) -> rquickjs::Result<u16>,
) -> rquickjs::Result<Vec<u16>> {
    let array = rquickjs::Array::from_js(ctx, value)?;
    if array.len() > 256 {
        return Err(type_error(ctx, "too many template dependencies"));
    }
    array.iter::<Value>().map(|value| get(value?)).collect()
}

fn template_dependency<'js, T: Clone>(
    ctx: &Ctx<'js>,
    handles: &[u16],
    pool: &[T],
    slot: Option<u8>,
) -> rquickjs::Result<Option<T>> {
    slot.map(|slot| {
        let handle = handles
            .get(usize::from(slot))
            .ok_or_else(|| type_error(ctx, "missing template dependency"))?;
        pool.get(usize::from(*handle))
            .cloned()
            .ok_or_else(|| type_error(ctx, "unknown template dependency"))
    })
    .transpose()
}

pub(super) fn reconcile_range<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    canvas::side_effect(ctx)?;
    let parent = node_id(ctx, args, 0)?;
    let list = |index| {
        rquickjs::Array::from_js(ctx, value(ctx, args, index))?
            .iter::<Value>()
            .map(|value| value.and_then(|value| node_id(ctx, &[value], 0)))
            .collect::<rquickjs::Result<Vec<_>>>()
    };
    let previous = list(1)?;
    let next = list(2)?;
    let anchor = handle(ctx, args, 3)?;
    let before = (anchor != 0).then(|| NodeId::from_raw(anchor));
    apply(
        host,
        ctx,
        Mutation::ReconcileRange {
            parent,
            previous,
            next,
            before,
        },
    )?;
    super::super::frame::bump(&host.inner().marks.ffi_batches);
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
