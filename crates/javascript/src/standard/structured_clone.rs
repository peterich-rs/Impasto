use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rquickjs::class::Trace;
use rquickjs::function::{Constructor, Opt, This};
use rquickjs::object::Property;
use rquickjs::{
    Array, ArrayBuffer, Atom, Class, Ctx, Exception, Function, JsLifetime, Object, Result, Value,
    qjs,
};

use crate::runtime::quickjs::buffer;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct CloneIntrinsics {}

fn intrinsics<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    Class::<CloneIntrinsics>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)
}

fn class_id(value: &Value<'_>) -> u32 {
    // SAFETY: QuickJS reads the class tag of this live value.
    unsafe { qjs::JS_GetClassID(value.as_raw()) }
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    let holder = intrinsics(ctx)?;
    let object: Object = globals.get("Object")?;
    let descriptor: Function = object.get("getOwnPropertyDescriptor")?;
    let array: Object = globals.get("Array")?;
    holder.set("arrayFrom", array.get::<_, Function>("from")?)?;
    holder.set("array", array)?;
    holder.set("Object", object)?;
    holder.set("descriptor", descriptor.clone())?;

    for (name, sample, methods, getters) in [
        ("Date", "new Date()", &["getTime"][..], &[][..]),
        (
            "RegExp",
            "/x/",
            &[][..],
            &[
                "source",
                "hasIndices",
                "global",
                "ignoreCase",
                "multiline",
                "dotAll",
                "unicode",
                "unicodeSets",
                "sticky",
            ][..],
        ),
        ("Map", "new Map()", &["forEach", "set"][..], &[][..]),
        ("Set", "new Set()", &["forEach", "add"][..], &[][..]),
        ("Boolean", "new Boolean()", &["valueOf"][..], &[][..]),
        ("Number", "new Number()", &["valueOf"][..], &[][..]),
        ("String", "new String()", &["valueOf"][..], &[][..]),
        ("BigInt", "Object(0n)", &["valueOf"][..], &[][..]),
        ("Error", "new Error()", &[][..], &[][..]),
        (
            "DOMException",
            "new DOMException()",
            &[][..],
            &["name", "message"][..],
        ),
    ] {
        let constructor: Object = globals.get(name)?;
        let info = Object::new_proto(ctx.clone(), None)?;
        let prototype: Object = constructor.get("prototype")?;
        info.set("constructor", constructor)?;
        info.set("name", name)?;
        for method in methods {
            info.set(*method, prototype.get::<_, Function>(*method)?)?;
        }
        for getter in getters {
            let property: Option<Object> = descriptor.call((prototype.clone(), *getter))?;
            if let Some(property) = property {
                info.set(*getter, property.get::<_, Function>("get")?)?;
            }
        }
        let sample: Value = ctx.eval(sample)?;
        holder.set(class_id(&sample), info)?;
    }
    for name in [
        "EvalError",
        "RangeError",
        "ReferenceError",
        "SyntaxError",
        "TypeError",
        "URIError",
        "DataView",
        "ArrayBuffer",
    ] {
        holder.set(name, globals.get::<_, Constructor>(name)?)?;
    }
    let buffer: Constructor = holder.get("ArrayBuffer")?;
    let prototype: Object = buffer.get("prototype")?;
    for name in ["resizable", "maxByteLength"] {
        let property: Option<Object> = descriptor.call((prototype.clone(), name))?;
        if let Some(property) = property {
            holder.set(name, property.get::<_, Function>("get")?)?;
        }
    }
    holder.set("arrayClass", class_id(Array::new(ctx.clone())?.as_value()))?;
    holder.set("plain", class_id(Object::new(ctx.clone())?.as_value()))?;

    globals.set(
        "structuredClone",
        Function::new(ctx.clone(), structured_clone)?.with_name("structuredClone")?,
    )
}

fn clone_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    match super::events::named_error(ctx, "DataCloneError", message) {
        Ok(error) => ctx.throw(error),
        Err(error) => error,
    }
}

fn structured_clone<'js>(
    ctx: Ctx<'js>,
    value: Value<'js>,
    Opt(options): Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let native = intrinsics(&ctx)?;
    let mut transfer = HashSet::new();
    if let Some(options) = options.filter(|value| !super::web::is_nullish(value)) {
        let options = options
            .as_object()
            .ok_or_else(|| Exception::throw_type(&ctx, "Expected structured clone options"))?;
        let list: Value = options.get("transfer")?;
        if !list.is_undefined() {
            // Array.from accepts array-likes too. Check WebIDL's iterable requirement first.
            let iterator =
                Atom::from_predefined(ctx.clone(), rquickjs::atom::PredefinedAtom::SymbolIterator);
            if list
                .as_object()
                .and_then(|list| list.get::<_, Function>(iterator).ok())
                .is_none()
            {
                return Err(Exception::throw_type(&ctx, "transfer must be iterable"));
            }
            let array: Object = native.get("array")?;
            let from: Function = native.get("arrayFrom")?;
            let list: Array = from.call((This(array), list))?;
            for item in list.iter::<Value>() {
                let item = item?;
                if buffer::array_buffer(&item).ok().flatten().is_none() || !transfer.insert(item) {
                    return Err(clone_error(
                        &ctx,
                        "Expected distinct, attached ArrayBuffers",
                    ));
                }
            }
        }
    }
    let mut state = CloneState {
        ctx: &ctx,
        native: &native,
        seen: HashMap::new(),
    };
    let cloned = state.value(value, 0)?;
    let attached = transfer
        .into_iter()
        .map(|item| {
            buffer::array_buffer(&item)
                .ok()
                .flatten()
                .ok_or_else(|| clone_error(&ctx, "Transfer buffer was detached"))
        })
        .collect::<Result<Vec<_>>>()?;
    for mut buffer in attached {
        buffer.detach();
    }
    Ok(cloned)
}

struct CloneState<'a, 'js> {
    ctx: &'a Ctx<'js>,
    native: &'a Object<'js>,
    seen: HashMap<Value<'js>, Value<'js>>,
}

impl<'js> CloneState<'_, 'js> {
    fn value(&mut self, input: Value<'js>, depth: usize) -> Result<Value<'js>> {
        if input.is_symbol() {
            return Err(clone_error(self.ctx, "A symbol cannot be cloned"));
        }
        let Some(source) = input.as_object() else {
            return Ok(input);
        };
        if let Some(existing) = self.seen.get(&input) {
            return Ok(existing.clone());
        }
        if depth >= 256 || self.seen.len() >= 100_000 {
            return Err(clone_error(
                self.ctx,
                "Structured clone size limit exceeded",
            ));
        }
        let depth = depth + 1;
        let id = class_id(&input);
        let plain: u32 = self.native.get("plain")?;
        let array: u32 = self.native.get("arrayClass")?;
        let output = if id == array || id == plain {
            let output = if id == array {
                let array = input.as_array().ok_or(rquickjs::Error::Unknown)?;
                let cloned = Array::new(self.ctx.clone())?.into_object();
                cloned.set("length", array.len())?;
                cloned
            } else {
                Object::new(self.ctx.clone())?
            };
            self.seen.insert(input.clone(), output.as_value().clone());
            for property in source.props::<Atom, Value>() {
                let (key, value) = property?;
                output.prop(
                    key,
                    Property::from(self.value(value, depth)?)
                        .writable()
                        .enumerable()
                        .configurable(),
                )?;
            }
            output.into_value()
        } else if let Some(buffer) =
            buffer::array_buffer(&input).map_err(|_| clone_error(self.ctx, "Detached buffer"))?
        {
            self.array_buffer(buffer)?
        } else if buffer::is_source(&input) {
            self.view(input.clone(), depth)?
        } else {
            let info: Option<Object> = self.native.get(id)?;
            let info =
                info.ok_or_else(|| clone_error(self.ctx, "Unsupported structured clone value"))?;
            self.builtin(&info, input.clone(), depth)?
        };
        self.seen.insert(input, output.clone());
        Ok(output)
    }

    fn array_buffer(&self, buffer: ArrayBuffer<'js>) -> Result<Value<'js>> {
        let resizable: Option<Function> = self.native.get("resizable")?;
        if let Some(resizable) = resizable
            && resizable.call::<_, bool>((This(buffer.as_value().clone()),))?
        {
            let max: Function = self.native.get("maxByteLength")?;
            let max: usize = max.call((This(buffer.as_value().clone()),))?;
            let options = Object::new(self.ctx.clone())?;
            options.set("maxByteLength", max)?;
            let len = buffer.len();
            let dest: ArrayBuffer = self
                .native
                .get::<_, Constructor>("ArrayBuffer")?
                .construct((len, options))?;
            let source = buffer
                .as_raw()
                .ok_or_else(|| clone_error(self.ctx, "Detached buffer"))?;
            let target = dest
                .as_raw()
                .ok_or_else(|| clone_error(self.ctx, "Detached buffer"))?;
            // SAFETY: the native pointers refer to distinct rooted buffers of
            // identical length. No JS runs while the pointers are used.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr().cast::<u8>(),
                    target.as_ptr().cast::<u8>(),
                    len,
                )
            };
            return Ok(dest.into_value());
        }
        // SAFETY: the borrowed bytes are copied before invoking QuickJS again.
        let data = unsafe { buffer.as_bytes() }
            .ok_or_else(|| clone_error(self.ctx, "Detached buffer"))?
            .to_vec();
        Ok(ArrayBuffer::new(self.ctx.clone(), data)?.into_value())
    }

    fn view(&mut self, input: Value<'js>, depth: usize) -> Result<Value<'js>> {
        // SAFETY: this only reads the internal typed array tag.
        let kind = unsafe { qjs::JS_GetTypedArrayType(input.as_raw()) };
        let (buffer, offset, bytes) = buffer::view_parts(self.ctx, input)
            .map_err(|_| clone_error(self.ctx, "Detached buffer view"))?;
        let cloned = self.value(buffer.into_value(), depth)?;
        if kind < 0 {
            return self
                .native
                .get::<_, Constructor>("DataView")?
                .construct((cloned, offset, bytes));
        }
        let element_bytes = match kind {
            0..=2 => 1,
            3 | 4 | 9 => 2,
            5 | 6 | 10 => 4,
            _ => 8,
        };
        let mut args = [
            cloned.as_raw(),
            Value::new_number(self.ctx.clone(), offset as f64).as_raw(),
            Value::new_number(self.ctx.clone(), (bytes / element_bytes) as f64).as_raw(),
        ];
        // SAFETY: the context and buffer are live and the arguments describe
        // a validated byte range of the same-sized cloned backing store.
        let raw = unsafe {
            qjs::JS_NewTypedArray(self.ctx.as_raw().as_ptr(), 3, args.as_mut_ptr(), kind as _)
        };
        // SAFETY: this is an owned QuickJS result from the same context.
        let result = unsafe { Value::from_raw(self.ctx.clone(), raw) };
        if result.is_exception() {
            return Err(rquickjs::Error::Exception);
        }
        Ok(result)
    }

    fn builtin(
        &mut self,
        info: &Object<'js>,
        input: Value<'js>,
        depth: usize,
    ) -> Result<Value<'js>> {
        let name: String = info.get("name")?;
        let ctor: Constructor = if name == "BigInt" {
            self.native.get("Object")?
        } else {
            info.get("constructor")?
        };
        match name.as_str() {
            "Date" => ctor.construct((invoke::<f64>(info, "getTime", input)?,)),
            "RegExp" => {
                let source: rquickjs::String = invoke(info, "source", input.clone())?;
                let mut flags = String::new();
                for (property, flag) in [
                    ("hasIndices", 'd'),
                    ("global", 'g'),
                    ("ignoreCase", 'i'),
                    ("multiline", 'm'),
                    ("dotAll", 's'),
                    ("unicode", 'u'),
                    ("unicodeSets", 'v'),
                    ("sticky", 'y'),
                ] {
                    if let Some(getter) = info.get::<_, Option<Function>>(property)?
                        && getter.call::<_, bool>((This(input.clone()),))?
                    {
                        flags.push(flag);
                    }
                }
                ctor.construct((source, flags))
            }
            "DOMException" => ctor.construct((
                invoke::<rquickjs::String>(info, "message", input.clone())?,
                invoke::<rquickjs::String>(info, "name", input)?,
            )),
            "Map" | "Set" => self.collection(info, ctor, input, depth, name == "Map"),
            "Error" => self.error(ctor, input, depth),
            _ => ctor.construct((invoke::<Value>(info, "valueOf", input)?,)),
        }
    }

    fn error(
        &mut self,
        default: Constructor<'js>,
        input: Value<'js>,
        depth: usize,
    ) -> Result<Value<'js>> {
        let source = input.as_object().ok_or(rquickjs::Error::Unknown)?;
        let name: Option<String> = source.get("name")?;
        let ctor = match name.as_deref() {
            Some(
                name @ ("EvalError" | "RangeError" | "ReferenceError" | "SyntaxError" | "TypeError"
                | "URIError"),
            ) => self.native.get(name)?,
            _ => default,
        };
        let descriptor: Function = self.native.get("descriptor")?;
        let message: Option<Object> = descriptor.call((source.clone(), "message"))?;
        let message = message
            .map(|message| message.get::<_, Value>("value"))
            .transpose()?;
        let output: Object = ctor.construct((Opt(message),))?;
        self.seen.insert(input.clone(), output.as_value().clone());
        for name in ["cause", "stack"] {
            let property: Option<Object> = descriptor.call((source.clone(), name))?;
            if let Some(property) = property
                && property.contains_key("value")?
            {
                let value = self.value(property.get("value")?, depth)?;
                output.prop(name, Property::from(value).writable().configurable())?;
            }
        }
        Ok(output.into_value())
    }

    fn collection(
        &mut self,
        info: &Object<'js>,
        ctor: Constructor<'js>,
        input: Value<'js>,
        depth: usize,
        map: bool,
    ) -> Result<Value<'js>> {
        let output: Object = ctor.construct(())?;
        self.seen.insert(input.clone(), output.as_value().clone());
        let items = Rc::new(RefCell::new(Vec::new()));
        let collect = Rc::clone(&items);
        let callback = Function::new(
            self.ctx.clone(),
            move |value: Value<'js>, key: Value<'js>| {
                collect.borrow_mut().push((key, value));
            },
        )?;
        let each: Function = info.get("forEach")?;
        each.call::<_, ()>((This(input), callback))?;
        let set: Function = info.get(if map { "set" } else { "add" })?;
        let items = std::mem::take(&mut *items.borrow_mut());
        for (key, value) in items {
            let value = self.value(value, depth)?;
            if map {
                let key = self.value(key, depth)?;
                set.call::<_, ()>((This(output.clone()), key, value))?;
            } else {
                set.call::<_, ()>((This(output.clone()), value))?;
            }
        }
        Ok(output.into_value())
    }
}

fn invoke<'js, T: rquickjs::FromJs<'js>>(
    info: &Object<'js>,
    method: &str,
    receiver: Value<'js>,
) -> Result<T> {
    info.get::<_, Function>(method)?.call((This(receiver),))
}
