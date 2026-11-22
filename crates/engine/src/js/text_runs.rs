//! Decode structured JavaScript text spans before updating the tree.

use crate::scene::{ColorStop, Gradient, TextSpan};
use rquickjs::{Array, Coerced, Ctx, Exception, FromJs, Object, Value};

pub(super) fn decode<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Vec<TextSpan>> {
    if value.is_null() || value.is_undefined() {
        return Ok(Vec::new());
    }
    let runs = Array::from_value(value)?;
    let mut spans = Vec::new();
    for run in runs.iter::<Object>() {
        spans.push(decode_run(ctx, run?)?);
    }
    Ok(spans)
}

fn decode_run<'js>(ctx: &Ctx<'js>, run: Object<'js>) -> rquickjs::Result<TextSpan> {
    let text: Option<Coerced<String>> = run.get("text")?;
    let family: Option<Coerced<String>> = run.get("family")?;
    let weight = run
        .get::<_, Option<Coerced<i32>>>("weight")?
        .map_or(400, |weight| weight.0);
    let weight = u16::try_from(weight).map_err(|_| invalid(ctx))?;
    if !(100..=900).contains(&weight) || !weight.is_multiple_of(100) {
        return Err(invalid(ctx));
    }
    let gradient = run
        .get::<_, Option<Object>>("gradient")?
        .map(|value| decode_gradient(ctx, value))
        .transpose()?;
    Ok(TextSpan {
        utf8: text.map_or_else(String::new, |text| text.0),
        family: family.map(|name| name.0).filter(|name| !name.is_empty()),
        weight,
        color: decode_color(ctx, run.get("color")?)?,
        underline: run.get::<_, Coerced<bool>>("underline")?.0,
        line_through: run.get::<_, Coerced<bool>>("lineThrough")?.0,
        gradient,
    })
}

fn decode_color<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<[u8; 4]> {
    if value.is_null() || value.is_undefined() {
        return Ok([0, 0, 0, 255]);
    }
    let mut rgba = [0, 0, 0, 255];
    if let Some(array) = value.as_array() {
        if array.len() != rgba.len() {
            return Err(invalid(ctx));
        }
        for (index, channel) in rgba.iter_mut().enumerate() {
            let raw: Coerced<i32> = array.get(index)?;
            *channel = u8::try_from(raw.0).map_err(|_| invalid(ctx))?;
        }
        return Ok(rgba);
    }
    let text = String::from_js(ctx, value)?;
    let hex = text.strip_prefix('#').unwrap_or(&text);
    if !hex.is_ascii() || !matches!(hex.len(), 6 | 8) {
        return Err(invalid(ctx));
    }
    for (channel, pair) in rgba.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        let digits = std::str::from_utf8(pair).map_err(|_| invalid(ctx))?;
        *channel = u8::from_str_radix(digits, 16).map_err(|_| invalid(ctx))?;
    }
    Ok(rgba)
}

fn decode_gradient<'js>(ctx: &Ctx<'js>, value: Object<'js>) -> rquickjs::Result<Gradient> {
    let stops: Array = value.get("stops")?;
    if stops.len() < 2 {
        return Err(invalid(ctx));
    }
    let mut decoded_stops = Vec::new();
    for item in stops.iter::<Object>() {
        let item = item?;
        let offset = finite(ctx, item.get("offset")?)?;
        let color: Value = item.get("color")?;
        if !(0.0..=1.0).contains(&offset) || color.is_null() || color.is_undefined() {
            return Err(invalid(ctx));
        }
        decoded_stops.push(ColorStop::srgb(offset, decode_color(ctx, color)?));
    }

    let point = |key, default| -> rquickjs::Result<[f32; 2]> {
        let Some(point) = value.get::<_, Option<Array>>(key)? else {
            return Ok(default);
        };
        Ok([finite(ctx, point.get(0)?)?, finite(ctx, point.get(1)?)?])
    };
    let number = |key, default| -> rquickjs::Result<f32> {
        value
            .get::<_, Option<f64>>(key)?
            .map(|n| finite(ctx, n))
            .transpose()
            .map(|n| n.unwrap_or(default))
    };

    Ok(match value.get::<_, Option<String>>("kind")?.as_deref() {
        Some("radial") => Gradient::Radial {
            center: point("center", [0.5, 0.5])?,
            radius: number("radius", 0.5)?,
            stops: decoded_stops,
        },
        Some("sweep") => Gradient::Sweep {
            center: point("center", [0.5, 0.5])?,
            start_angle_rad: number("startAngle", 0.0)?,
            end_angle_rad: number("endAngle", std::f32::consts::TAU)?,
            stops: decoded_stops,
        },
        _ => Gradient::Linear {
            start: point("from", [0.0, 0.0])?,
            end: point("to", [1.0, 0.0])?,
            stops: decoded_stops,
        },
    })
}

#[expect(clippy::cast_possible_truncation, reason = "scene coordinates use f32")]
fn finite(ctx: &Ctx<'_>, number: f64) -> rquickjs::Result<f32> {
    let number = number as f32;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(invalid(ctx))
    }
}

fn invalid(ctx: &Ctx<'_>) -> rquickjs::Error {
    Exception::throw_type(ctx, "invalid text run")
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "isolated JavaScript decoder fixtures")]
mod tests {
    use super::{ColorStop, Gradient, TextSpan, Value, decode};

    fn spans(source: &str) -> rquickjs::Result<Vec<TextSpan>> {
        let runtime = rquickjs::Runtime::new().expect("test runtime");
        let context = rquickjs::Context::full(&runtime).expect("test context");
        context.with(|ctx| decode(&ctx, ctx.eval::<Value, _>(source)?))
    }

    #[test]
    fn text_flags_colors_and_gradient_are_decoded_from_values() {
        let runs = spans(
            r##"[{
            text: "中 文\na",
            color: "#01020304",
            family: "Sans",
            weight: 700,
            underline: true,
            lineThrough: true,
            gradient: { kind: "linear", stops: [
                { offset: 0, color: [3, 4, 5, 6] },
                { offset: 1, color: "#112233" }
            ] }
        }]"##,
        )
        .expect("valid text run");
        assert_eq!(runs[0].utf8, "中 文\na");
        assert_eq!(runs[0].family.as_deref(), Some("Sans"));
        assert_eq!(runs[0].weight, 700);
        assert!(runs[0].underline && runs[0].line_through);
        assert_eq!(runs[0].color, [1, 2, 3, 4]);
        assert_eq!(
            runs[0].gradient,
            Some(Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 0.0],
                stops: vec![
                    ColorStop::srgb(0.0, [3, 4, 5, 6]),
                    ColorStop::srgb(1.0, [17, 34, 51, 255])
                ],
            })
        );
    }

    #[test]
    fn empty_runs_clear_and_invalid_runs_fail() {
        assert!(spans("[]").expect("empty spans").is_empty());
        assert!(spans("[{ text: 'a', weight: 450 }]").is_err());
        assert!(
            spans("[{ text: 'a', gradient: { stops: [{ offset: 0, color: '#ffffff' }] } }]")
                .is_err()
        );
    }
}
