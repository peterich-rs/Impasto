//! Copy one script theme snapshot into the tree's shared native environment.

use crate::style::codec::{Value, declarations};
use crate::style::{DirectionMode, FontSizes, Property, Radii, TypeRole};
use crate::{Error, tree::Tree};

pub(super) fn apply(tree: &mut Tree, value: &Value) -> Result<(), Error> {
    let mut env = tree.style_environment().clone();
    env.direction = match field(value, "direction")? {
        Value::String(value) if value == "ltr" => DirectionMode::Ltr,
        Value::String(value) if value == "rtl" => DirectionMode::Rtl,
        _ => return Err(Error::BadNumber),
    };
    env.dark = match field(value, "dark")? {
        Value::Bool(value) => *value,
        _ => return Err(Error::BadNumber),
    };
    env.spacing = number(field(value, "spacing")?)?;
    env.root_rem = number(field(value, "rootRem")?)?;
    let Value::Object(colors) = field(value, "colors")? else {
        return Err(Error::BadNumber);
    };
    let palette = colors
        .iter()
        .map(|(name, value)| {
            let declarations = declarations("color", value)?;
            let Some(crate::style::Declaration::Set(Property::Color(color))) = declarations.first()
            else {
                return Err(Error::BadNumber);
            };
            Ok((name.as_str(), env.resolve_color(color)?))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    env.install_palette(&palette);
    if let Some(lengths) = value.field("lengths") {
        let Value::Object(lengths) = lengths else {
            return Err(Error::BadNumber);
        };
        let entries = lengths
            .iter()
            .map(|(name, value)| Ok((name.as_str(), number(value)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        env.install_lengths(entries);
    }
    let [none, sm, md, lg, xl, xl2, full] = numbers(field(value, "radii")?)?;
    env.radii = Radii {
        none,
        sm,
        md,
        lg,
        xl,
        xl2,
        full,
    };
    let [xs, sm, base, lg, xl, xl2, xl3, xl4] = numbers(field(value, "fontSizes")?)?;
    env.font_sizes = FontSizes {
        xs,
        sm,
        base,
        lg,
        xl,
        xl2,
        xl3,
        xl4,
    };
    let roles = field(value, "roles")?;
    for name in ["display", "headline", "title", "body", "label", "caption"] {
        let role = field(roles, name)?;
        let weight = number(field(role, "weight")?)?;
        if weight.fract() != 0.0 || !(1.0..=1000.0).contains(&weight) {
            return Err(Error::BadNumber);
        }
        // The integer range was checked above before converting the wire number.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the finite integer weight was checked in the u16 range"
        )]
        let weight = weight as u16;
        env.set_role(
            name,
            TypeRole {
                size: number(field(role, "size")?)?,
                weight,
                line_height: number(field(role, "lineHeight")?)?,
            },
        );
    }
    tree.set_style_environment(env)
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, Error> {
    value.field(name).ok_or(Error::BadNumber)
}

fn number(value: &Value) -> Result<f32, Error> {
    let Value::Number(value) = value else {
        return Err(Error::BadNumber);
    };
    if !value.is_finite() || *value < 0.0 || *value > f64::from(f32::MAX) {
        return Err(Error::BadNumber);
    }
    // f32 is the tree's scene-point unit; range and finiteness were checked.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "scene units use f32 after finite range validation"
    )]
    Ok(*value as f32)
}

fn numbers<const N: usize>(value: &Value) -> Result<[f32; N], Error> {
    let Value::Array(values) = value else {
        return Err(Error::BadNumber);
    };
    let mut result = [0.0; N];
    if values.len() != N {
        return Err(Error::BadNumber);
    }
    for (out, value) in result.iter_mut().zip(values) {
        *out = number(value)?;
    }
    Ok(result)
}
