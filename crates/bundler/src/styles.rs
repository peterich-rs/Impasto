//! Evaluate only literals whose values are known without executing app code.

use engine::style::codec::Value;
use oxc::ast::ast::{
    ArrayExpressionElement, Expression, ObjectPropertyKind, PropertyKind, UnaryOperator,
};
use serde_json::{Map, Number, Value as Json};

pub(super) fn literal(expression: &Expression<'_>) -> Option<Json> {
    let expression = expression.without_parentheses();
    Some(match expression {
        Expression::StringLiteral(literal) => Json::String(literal.value.to_string()),
        Expression::NumericLiteral(literal) => Json::Number(Number::from_f64(literal.value)?),
        Expression::BooleanLiteral(literal) => Json::Bool(literal.value),
        Expression::NullLiteral(_) => Json::Null,
        Expression::TemplateLiteral(literal) if literal.expressions.is_empty() => {
            Json::String(literal.quasis.first()?.value.cooked?.to_string())
        }
        Expression::UnaryExpression(literal)
            if literal.operator == UnaryOperator::UnaryNegation =>
        {
            Json::Number(Number::from_f64(-literal_number(&literal.argument)?)?)
        }
        Expression::ArrayExpression(array) => Json::Array(
            array
                .elements
                .iter()
                .map(|element| match element {
                    ArrayExpressionElement::SpreadElement(_)
                    | ArrayExpressionElement::Elision(_) => None,
                    element => literal(element.to_expression()),
                })
                .collect::<Option<Vec<_>>>()?,
        ),
        Expression::ObjectExpression(object) => Json::Object(
            object
                .properties
                .iter()
                .map(|property| {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return None;
                    };
                    if property.computed || property.method || property.kind != PropertyKind::Init {
                        return None;
                    }
                    let name = property.key.static_name()?.to_string();
                    if name == "__proto__" {
                        return None;
                    }
                    let value = literal(&property.value)?;
                    Some((name, value))
                })
                .collect::<Option<Map<_, _>>>()?,
        ),
        _ => return None,
    })
}

fn literal_number(expression: &Expression<'_>) -> Option<f64> {
    match expression.without_parentheses() {
        Expression::NumericLiteral(literal) => Some(literal.value),
        _ => None,
    }
}

pub(super) fn style_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(value) => Value::Number(value.as_f64().unwrap_or_default()),
        Json::String(value) => Value::String(value.clone()),
        Json::Array(value) => Value::Array(value.iter().map(style_value).collect()),
        Json::Object(value) => Value::Object(
            value
                .iter()
                .map(|(name, value)| (name.clone(), style_value(value)))
                .collect(),
        ),
    }
}

pub(super) fn validate_surface(
    value: &Json,
    surface: engine::style::StyleSurface,
) -> Result<(), String> {
    let style =
        engine::style::codec::from_value(&style_value(value)).map_err(|error| error.to_string())?;
    for declaration in style.fragments().iter().flatten() {
        if !engine::style::author_key_allowed(surface, declaration) {
            return Err(format!(
                "style property \"{}\" is managed by {}",
                declaration.key().name(),
                surface.name()
            ));
        }
    }
    Ok(())
}
