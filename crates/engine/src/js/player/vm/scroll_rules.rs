//! Decodes scroll curves at the JavaScript boundary.

use rquickjs::{Array, Ctx, IntoJs, Value};

use crate::scene::{
    Extrapolate, PiecewiseColorMap, PiecewiseMap, ScrollCurve, ScrollSource, ScrollTargetProp,
};

use super::bindings::{handle, node_id, num, string, value};
use super::{Host, type_error};

fn stops<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Array<'js>> {
    value
        .into_array()
        .filter(|array| (2..=4096).contains(&array.len()))
        .ok_or_else(|| type_error(ctx, "expected between 2 and 4096 interpolation stops"))
}

pub(super) fn binding<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    operation: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    if operation == "unbindScrollProp" {
        let id = u32::try_from(handle(ctx, args, 0)?)
            .map_err(|_| type_error(ctx, "invalid scroll binding id"))?;
        host.inner().tree.borrow_mut().remove_scroll_binding(id);
        return Ok(Value::new_undefined(ctx.clone()));
    }
    if operation == "bindBackProgressOpacity" {
        let target = node_id(ctx, args, 0)?;
        let inputs = stops(ctx, value(ctx, args, 1))?
            .iter()
            .collect::<rquickjs::Result<Box<[f32]>>>()?;
        let outputs = stops(ctx, value(ctx, args, 2))?
            .iter()
            .collect::<rquickjs::Result<Box<[f32]>>>()?;
        let curve = PiecewiseMap::new(inputs, outputs, Extrapolate::Clamp)
            .ok_or_else(|| type_error(ctx, "invalid back gesture interpolation"))?;
        let id = host
            .inner()
            .tree
            .borrow_mut()
            .bind_back_progress_opacity(target, curve)
            .map_err(|error| type_error(ctx, &error.to_string()))?;
        return id.into_js(ctx);
    }
    let source = node_id(ctx, args, 0)?;
    let channel = ScrollSource::parse(&string(ctx, args, 1)?)
        .ok_or_else(|| type_error(ctx, "unknown scroll channel"))?;
    let target = node_id(ctx, args, 2)?;
    let prop = ScrollTargetProp::parse(&string(ctx, args, 3)?)
        .ok_or_else(|| type_error(ctx, "unknown scroll target property"))?;
    let curve = match string(ctx, args, 6)?.as_str() {
        "clamp" => curve(ctx, args, prop, Extrapolate::Clamp)?,
        "extend" => curve(ctx, args, prop, Extrapolate::Extend)?,
        #[expect(
            clippy::cast_possible_truncation,
            reason = "native scroll geometry is f32"
        )]
        "linear" => ScrollCurve::Linear {
            factor: num(ctx, args, 4)? as f32,
            offset: num(ctx, args, 5)? as f32,
            maximum: None,
        },
        _ => return Err(type_error(ctx, "invalid scroll extrapolation")),
    };
    let id = host
        .inner()
        .tree
        .borrow_mut()
        .add_scroll_binding(source, channel, target, prop, curve)
        .map_err(|error| type_error(ctx, &error.to_string()))?;
    id.into_js(ctx)
}

fn curve<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    prop: ScrollTargetProp,
    extrapolate: Extrapolate,
) -> rquickjs::Result<ScrollCurve> {
    let inputs = stops(ctx, value(ctx, args, 4))?
        .iter()
        .collect::<rquickjs::Result<Box<[f32]>>>()?;
    let outputs = stops(ctx, value(ctx, args, 5))?;
    let result = if prop == ScrollTargetProp::BackgroundColor {
        let colors = outputs
            .iter()
            .map(|output: rquickjs::Result<String>| {
                let raw = output?;
                let color = csscolorparser::parse(raw.trim())
                    .map_err(|_| type_error(ctx, "invalid scroll interpolation color"))?;
                let [r, g, b, a] = color.to_linear_rgba();
                Ok([r * a, g * a, b * a, a])
            })
            .collect::<rquickjs::Result<Box<[_]>>>()?;
        PiecewiseColorMap::new(inputs, colors, Extrapolate::Clamp).map(ScrollCurve::Color)
    } else {
        let values = outputs.iter().collect::<rquickjs::Result<Box<[f32]>>>()?;
        PiecewiseMap::new(inputs, values, extrapolate).map(ScrollCurve::Scalar)
    };
    result.ok_or_else(|| {
        type_error(
            ctx,
            "scroll stops must be strictly increasing and have matching outputs",
        )
    })
}
