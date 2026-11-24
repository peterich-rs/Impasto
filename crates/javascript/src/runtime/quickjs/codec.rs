//! The owned-value boundary shared by host methods, events and transports.

use crate::host::PluginValue;
use rquickjs::object::Property;
use rquickjs::{Array, ArrayBuffer, Ctx, Exception, IntoJs, Object, TypedArray, Value};

use super::buffer;

const MAX_DEPTH: usize = 256;
const MAX_ITEMS: usize = 65_536;
const MAX_BYTES: usize = 8 * 1024 * 1024;

fn bytes<'js>(ctx: &Ctx<'js>, value: &Value<'js>, max: usize) -> rquickjs::Result<Option<Vec<u8>>> {
    let bounded = |bytes: Option<&[u8]>| {
        let bytes = bytes.ok_or_else(|| Exception::throw_type(ctx, "detached buffer"))?;
        if bytes.len() > max {
            return Err(Exception::throw_range(ctx, "byte limit exceeded"));
        }
        Ok(Some(bytes.to_vec()))
    };
    if let Some(buffer) = buffer::array_buffer(value)? {
        // SAFETY: no JavaScript executes before the borrowed bytes are copied.
        return bounded(unsafe { buffer.as_bytes() });
    }
    if buffer::is_typed_array(value) {
        let view = buffer::view(ctx, value.clone())?;
        // SAFETY: the view came from fixed geometry over a non-shared buffer.
        // No JavaScript runs while the bytes are copied.
        return bounded(unsafe { view.as_bytes() });
    }
    Ok(None)
}

struct Budget {
    items: usize,
    bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            items: MAX_ITEMS,
            bytes: MAX_BYTES,
        }
    }
}

impl Budget {
    fn visit(&mut self, ctx: &Ctx<'_>, depth: usize) -> rquickjs::Result<()> {
        if depth > MAX_DEPTH || self.items == 0 {
            return Err(Exception::throw_range(
                ctx,
                "module value exceeds traversal limits",
            ));
        }
        self.items -= 1;
        Ok(())
    }

    fn charge(&mut self, ctx: &Ctx<'_>, bytes: usize) -> rquickjs::Result<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| Exception::throw_range(ctx, "module value exceeds byte limit"))?;
        Ok(())
    }
}

pub(super) fn decode_args_with<'js>(
    ctx: &Ctx<'js>,
    args: Vec<Value<'js>>,
    strict: bool,
) -> rquickjs::Result<Vec<PluginValue>> {
    let mut budget = Budget::default();
    let mut path = Vec::new();
    args.into_iter()
        .map(|value| decode_value(ctx, value, &mut budget, &mut path, strict))
        .collect()
}

fn decode_value<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    budget: &mut Budget,
    path: &mut Vec<Value<'js>>,
    strict: bool,
) -> rquickjs::Result<PluginValue> {
    budget.visit(ctx, path.len())?;
    if let Some(value) = value.as_bool() {
        return Ok(PluginValue::Bool(value));
    }
    if let Some(value) = value.as_number() {
        return Ok(PluginValue::F64(value));
    }
    if let Some(value) = value.as_string() {
        let value = bounded_string(ctx, value, budget)?;
        return Ok(PluginValue::Str(value));
    }
    if let Some(value) = bytes(ctx, &value, budget.bytes)? {
        budget.charge(ctx, value.len())?;
        return Ok(PluginValue::Bytes(value));
    }
    if strict && value.is_function() {
        return Err(Exception::throw_type(
            ctx,
            "native method argument cannot be a function",
        ));
    }
    let Some(object) = value.as_object() else {
        return if strict && !value.is_null() && !value.is_undefined() {
            Err(Exception::throw_type(
                ctx,
                "unsupported native method argument",
            ))
        } else {
            Ok(PluginValue::Null)
        };
    };
    if path.contains(&value) {
        return Err(Exception::throw_type(ctx, "cyclic module value"));
    }
    path.push(value.clone());
    let result = if let Some(array) = value.as_array() {
        if array.len() > budget.items {
            return Err(Exception::throw_range(
                ctx,
                "module value exceeds item limit",
            ));
        }
        array
            .iter::<Value>()
            .map(|item| decode_value(ctx, item?, budget, path, strict))
            .collect::<rquickjs::Result<_>>()
            .map(PluginValue::List)
    } else {
        object
            .keys::<rquickjs::Atom>()
            .map(|key| {
                let key = key?;
                let name = key.to_js_string()?;
                let name = bounded_string(ctx, &name, budget)?;
                let item = decode_value(ctx, object.get(key)?, budget, path, strict)?;
                Ok((name, item))
            })
            .collect::<rquickjs::Result<_>>()
            .map(PluginValue::Map)
    };
    path.pop();
    result
}

fn bounded_string(
    ctx: &Ctx<'_>,
    value: &rquickjs::String<'_>,
    budget: &mut Budget,
) -> rquickjs::Result<String> {
    let text = value.clone().to_cstring()?;
    budget.charge(ctx, text.len())?;
    drop(text);
    // Keep the existing UTF-8 validation (including ill-formed JS strings).
    value.to_string()
}

pub fn encode<'js>(ctx: &Ctx<'js>, value: &PluginValue) -> rquickjs::Result<Value<'js>> {
    encode_value(ctx, value, 0, &mut Budget::default())
}

fn encode_value<'js>(
    ctx: &Ctx<'js>,
    value: &PluginValue,
    depth: usize,
    budget: &mut Budget,
) -> rquickjs::Result<Value<'js>> {
    budget.visit(ctx, depth)?;
    match value {
        PluginValue::Null => Ok(Value::new_null(ctx.clone())),
        PluginValue::Bool(value) => value.into_js(ctx),
        PluginValue::F64(value) => value.into_js(ctx),
        PluginValue::Str(value) => {
            budget.charge(ctx, value.len())?;
            value.into_js(ctx)
        }
        PluginValue::Bytes(value) => {
            budget.charge(ctx, value.len())?;
            if value.len() >= 4096 {
                Ok(ArrayBuffer::new_copy(ctx.clone(), value)?.into_value())
            } else {
                Ok(TypedArray::<u8>::new_copy(ctx.clone(), value)?.into_value())
            }
        }
        PluginValue::List(items) => {
            let array = Array::new(ctx.clone())?;
            for (index, item) in items.iter().enumerate() {
                array.set(index, encode_value(ctx, item, depth + 1, budget)?)?;
            }
            Ok(array.into_value())
        }
        PluginValue::Map(items) => {
            let object = Object::new(ctx.clone())?;
            for (name, item) in items {
                budget.charge(ctx, name.len())?;
                let property = Property::from(encode_value(ctx, item, depth + 1, budget)?)
                    .enumerable()
                    .configurable()
                    .writable();
                object.prop(name.as_str(), property)?;
            }
            Ok(object.into_value())
        }
    }
}
