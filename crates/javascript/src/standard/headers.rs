//! The header list and Fetch's header normalization rules.

use std::collections::BTreeMap;
use std::fmt::Write;

use rquickjs::atom::PredefinedAtom;
use rquickjs::class::Trace;
use rquickjs::function::{Opt, This};
use rquickjs::{
    Array, Class, Coerced, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Value,
};

use super::web::{Charge, array_from, constructor, is_nullish, platform};

#[derive(JsLifetime, Trace)]
#[rquickjs::class]
pub(super) struct Headers {
    #[qjs(skip_trace)]
    values: BTreeMap<String, Vec<String>>,
    #[qjs(skip_trace)]
    charge: Charge,
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, bridge: &Object<'js>) -> rquickjs::Result<()> {
    bridge.set(
        "Headers",
        constructor::<Headers, _, _>(ctx, |ctx: Ctx<'js>, init: Opt<Value<'js>>| {
            let init = init.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            let headers = Headers::from_value(&ctx, init, false)?;
            Class::instance(ctx, headers)
        })?,
    )?;
    let prototype = Class::<Headers>::prototype(ctx)?
        .ok_or_else(|| Exception::throw_internal(ctx, "Headers prototype is unavailable"))?;
    let entries: Function = prototype.get("entries")?;
    prototype.prop(
        PredefinedAtom::SymbolIterator,
        rquickjs::object::Property::from(entries)
            .configurable()
            .writable(),
    )
}

pub(super) fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))
}

fn name(ctx: &Ctx<'_>, value: String) -> rquickjs::Result<String> {
    if !is_token(&value) {
        return Err(Exception::throw_type(ctx, "Invalid header name"));
    }
    Ok(value.to_ascii_lowercase())
}

fn text(ctx: &Ctx<'_>, value: &str) -> rquickjs::Result<String> {
    let value = value.trim_matches(['\t', '\n', '\r', ' ']);
    if value.contains(['\0', '\n', '\r'])
        || platform(ctx)?.is_portable() && value.chars().any(|ch| u32::from(ch) > 255)
    {
        return Err(Exception::throw_type(ctx, "Invalid header value"));
    }
    Ok(value.to_owned())
}

fn units(value: &str) -> usize {
    // Account for the owned string and its list or map slot.
    value.len().saturating_add(32)
}

impl Headers {
    pub(super) fn from_value<'js>(
        ctx: &Ctx<'js>,
        input: Value<'js>,
        wire: bool,
    ) -> rquickjs::Result<Self> {
        let mut headers = Self {
            values: BTreeMap::new(),
            charge: Charge::new(ctx)?,
        };
        if is_nullish(&input) {
            return Ok(headers);
        }
        if let Ok(source) = Class::<Self>::from_value(&input) {
            let source = source.try_borrow()?;
            headers.charge.resize(ctx, source.charge.size())?;
            headers.values = source.values.clone();
        } else if let Some(value) = input.as_string() {
            if platform(ctx)?.is_portable() && !wire {
                return Err(Exception::throw_type(ctx, "Invalid headers initializer"));
            }
            for line in value.to_string()?.split('\n') {
                if let Some((key, value)) = line.split_once(':')
                    && !key.is_empty()
                {
                    let key = name(ctx, key.to_owned())?;
                    let value = if wire {
                        value.trim().to_owned()
                    } else {
                        text(ctx, value)?
                    };
                    headers.insert(ctx, key, value, false)?;
                }
            }
        } else if let Some(object) = input.as_object() {
            if !is_nullish(&object.get::<_, Value>(PredefinedAtom::SymbolIterator)?) {
                for pair in array_from(ctx, input.clone())?.iter::<Value>() {
                    let pair = pair?;
                    let object = pair
                        .as_object()
                        .ok_or_else(|| Exception::throw_type(ctx, "Expected a header pair"))?;
                    if is_nullish(&object.get::<_, Value>(PredefinedAtom::SymbolIterator)?) {
                        return Err(Exception::throw_type(ctx, "Expected a header pair"));
                    }
                    let pair = array_from(ctx, pair)?;
                    if pair.len() != 2 {
                        return Err(Exception::throw_type(
                            ctx,
                            "Expected a header name and value",
                        ));
                    }
                    let key = name(ctx, pair.get::<Coerced<String>>(0)?.0)?;
                    let value = text(ctx, &pair.get::<Coerced<String>>(1)?.0)?;
                    headers.insert(ctx, key, value, false)?;
                }
            } else {
                for pair in object.props::<String, Coerced<String>>() {
                    let (key, value) = pair?;
                    let key = name(ctx, key)?;
                    let value = text(ctx, &value.0)?;
                    headers.insert(ctx, key, value, false)?;
                }
            }
        } else {
            return Err(Exception::throw_type(ctx, "Invalid headers initializer"));
        }
        Ok(headers)
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub(super) fn set_valid(
        &mut self,
        ctx: &Ctx<'_>,
        name: String,
        value: String,
    ) -> rquickjs::Result<()> {
        self.insert(ctx, name, value, true)
    }

    pub(super) fn merged(&self) -> impl Iterator<Item = (&str, String)> {
        self.values
            .iter()
            .map(|(name, values)| (name.as_str(), values.join(", ")))
    }

    pub(super) fn wire(&self) -> String {
        let mut output = String::new();
        for (name, values) in &self.values {
            if name == "set-cookie" {
                for value in values {
                    let _ = writeln!(output, "{name}: {value}");
                }
            } else {
                let _ = writeln!(output, "{name}: {}", values.join(", "));
            }
        }
        output
    }

    fn insert(
        &mut self,
        ctx: &Ctx<'_>,
        name: String,
        value: String,
        replace: bool,
    ) -> rquickjs::Result<()> {
        let size = match self.values.get(&name) {
            None => self.charge.size().saturating_add(units(&name)),
            Some(values) if replace => values.iter().fold(self.charge.size(), |sum, value| {
                sum.saturating_sub(units(value))
            }),
            Some(_) => self.charge.size(),
        };
        self.charge
            .resize(ctx, size.saturating_add(units(&value)))?;
        let values = self.values.entry(name).or_default();
        if replace {
            values.clear();
        }
        values.push(value);
        Ok(())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Headers {
    fn append<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Coerced<String>,
        value: Coerced<String>,
    ) -> rquickjs::Result<()> {
        let key = name(&ctx, key.0)?;
        let value = text(&ctx, &value.0)?;
        headers.try_borrow_mut()?.insert(&ctx, key, value, false)
    }

    fn set<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Coerced<String>,
        value: Coerced<String>,
    ) -> rquickjs::Result<()> {
        let key = name(&ctx, key.0)?;
        let value = text(&ctx, &value.0)?;
        headers.try_borrow_mut()?.insert(&ctx, key, value, true)
    }

    fn delete<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Coerced<String>,
    ) -> rquickjs::Result<()> {
        let key = name(&ctx, key.0)?;
        let mut headers = headers.try_borrow_mut()?;
        if let Some(values) = headers.values.remove(&key) {
            let size = values.iter().fold(
                headers.charge.size().saturating_sub(units(&key)),
                |sum, value| sum.saturating_sub(units(value)),
            );
            headers.charge.resize(&ctx, size)?;
        }
        Ok(())
    }

    fn get<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Coerced<String>,
    ) -> rquickjs::Result<Value<'js>> {
        let key = name(&ctx, key.0)?;
        let value = headers
            .try_borrow()?
            .values
            .get(&key)
            .map(|values| values.join(", "));
        value.map_or_else(
            || Ok(Value::new_null(ctx.clone())),
            |value| value.into_js(&ctx),
        )
    }

    fn has<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Coerced<String>,
    ) -> rquickjs::Result<bool> {
        let key = name(&ctx, key.0)?;
        Ok(headers.try_borrow()?.values.contains_key(&key))
    }

    fn get_set_cookie(&self) -> Vec<String> {
        self.values.get("set-cookie").cloned().unwrap_or_default()
    }

    fn entries<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<Class<'js, HeaderIterator<'js>>> {
        Class::instance(ctx.clone(), HeaderIterator::new(&ctx, headers, Part::Pair)?)
    }

    fn keys<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<Class<'js, HeaderIterator<'js>>> {
        Class::instance(ctx.clone(), HeaderIterator::new(&ctx, headers, Part::Name)?)
    }

    fn values<'js>(
        This(headers): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<Class<'js, HeaderIterator<'js>>> {
        Class::instance(
            ctx.clone(),
            HeaderIterator::new(&ctx, headers, Part::Value)?,
        )
    }

    fn for_each<'js>(
        This(headers): This<Class<'js, Self>>,
        callback: Function<'js>,
        this_arg: Opt<Value<'js>>,
    ) -> rquickjs::Result<()> {
        let ctx = headers.ctx();
        let receiver = this_arg
            .0
            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let mut iter = HeaderIterator::new(ctx, headers.clone(), Part::Pair)?;
        while let Some((key, value)) = iter.advance(ctx)? {
            callback.call::<_, ()>((This(receiver.clone()), value, key, headers.clone()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Part {
    Pair,
    Name,
    Value,
}

#[derive(JsLifetime, Trace)]
#[rquickjs::class]
struct HeaderIterator<'js> {
    headers: Class<'js, Headers>,
    names: Option<Vec<String>>,
    index: usize,
    cookie: usize,
    #[qjs(skip_trace)]
    part: Part,
    #[qjs(skip_trace)]
    charge: Charge,
}

impl<'js> HeaderIterator<'js> {
    fn new(ctx: &Ctx<'js>, headers: Class<'js, Headers>, part: Part) -> rquickjs::Result<Self> {
        Ok(Self {
            headers,
            names: None,
            index: 0,
            cookie: 0,
            part,
            charge: Charge::new(ctx)?,
        })
    }

    fn advance(&mut self, ctx: &Ctx<'js>) -> rquickjs::Result<Option<(String, String)>> {
        let headers = self.headers.try_borrow()?;
        if self.names.is_none() {
            let size = headers
                .values
                .keys()
                .fold(0usize, |size, key| size.saturating_add(units(key)));
            self.charge.resize(ctx, size)?;
            self.names = Some(headers.values.keys().cloned().collect());
        }
        let Some(names) = &self.names else {
            return Ok(None);
        };
        while let Some(key) = names.get(self.index) {
            if let Some(values) = headers.values.get(key) {
                if key != "set-cookie" {
                    self.index += 1;
                    return Ok(Some((key.clone(), values.join(", "))));
                }
                if let Some(value) = values.get(self.cookie) {
                    self.cookie += 1;
                    return Ok(Some((key.clone(), value.clone())));
                }
            }
            self.index += 1;
            self.cookie = 0;
        }
        Ok(None)
    }
}

#[rquickjs::methods]
impl<'js> HeaderIterator<'js> {
    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    fn identity(This(iter): This<Class<'js, Self>>) -> Class<'js, Self> {
        iter
    }

    fn next(This(iter): This<Class<'js, Self>>, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let (pair, part) = {
            let mut iter = iter.try_borrow_mut()?;
            (iter.advance(&ctx)?, iter.part)
        };
        let output = Object::new(ctx.clone())?;
        output.set("done", pair.is_none())?;
        let value = if let Some((key, value)) = pair {
            match part {
                Part::Name => key.into_js(&ctx)?,
                Part::Value => value.into_js(&ctx)?,
                Part::Pair => {
                    let array = Array::new(ctx)?;
                    array.set(0, key)?;
                    array.set(1, value)?;
                    array.into_value()
                }
            }
        } else {
            Value::new_undefined(ctx)
        };
        output.set("value", value)?;
        Ok(output)
    }
}

pub(super) const DECLARATIONS: &str = r#"
type HeaderPair = readonly [string, string];
type HeadersInit = readonly HeaderPair[] | Iterable<HeaderPair> | Record<string, string>;

declare class Headers implements Iterable<[string, string]> {
  constructor(init?: HeadersInit);
  append(name: string, value: string): void;
  delete(name: string): void;
  get(name: string): string | null;
  getSetCookie(): string[];
  has(name: string): boolean;
  set(name: string, value: string): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  forEach(callback: (value: string, name: string, headers: Headers) => void, thisArg?: unknown): void;
  [Symbol.iterator](): IterableIterator<[string, string]>;
}
"#;
