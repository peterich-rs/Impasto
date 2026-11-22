//! QuickJS buffer brands and fixed byte views for codecs and Web APIs.

use rquickjs::class::Trace;
use rquickjs::function::{Constructor, This};
use rquickjs::{
    ArrayBuffer, Class, Ctx, Exception, Function, JsLifetime, Object, Result, TypedArray, Value,
    qjs,
};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct BufferIntrinsics {}

fn intrinsics<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    Class::<BufferIntrinsics>::prototype(ctx)?
        .ok_or_else(|| Exception::throw_internal(ctx, "Missing buffer intrinsics"))
}

pub(crate) fn install(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    let object: Object = globals.get("Object")?;
    let descriptor: Function = object.get("getOwnPropertyDescriptor")?;
    let data_view: Constructor = globals.get("DataView")?;
    let bytes: Constructor = globals.get("Uint8Array")?;
    let bytes_proto: Object = bytes.get("prototype")?;
    let typed = bytes_proto
        .get_prototype()
        .ok_or_else(|| Exception::throw_internal(ctx, "Missing typed array prototype"))?;
    let data: Object = data_view.get("prototype")?;
    // The private class prototype is owned and traced by the context. Its
    // getters read native slots, including after user code changes prototypes.
    let holder = intrinsics(ctx)?;
    holder.set("bytes", bytes)?;
    for (key, proto) in [("typed", typed), ("data", data)] {
        let getters = Object::new_proto(ctx.clone(), None)?;
        for name in ["buffer", "byteOffset", "byteLength"] {
            let desc: Object = descriptor.call((proto.clone(), name))?;
            getters.set(name, desc.get::<_, Function>("get")?)?;
        }
        holder.set(key, getters)?;
    }
    Ok(())
}

pub(crate) fn is_typed_array(value: &Value<'_>) -> bool {
    // SAFETY: the call reads the internal class tag of a live value.
    unsafe { qjs::JS_GetTypedArrayType(value.as_raw()) >= 0 }
}

pub fn array_buffer<'js>(value: &Value<'js>) -> Result<Option<ArrayBuffer<'js>>> {
    // SAFETY: the brand check reads a live value without probing its storage.
    // JS_GetArrayBuffer throws when used to probe a non-buffer object.
    if unsafe { qjs::JS_IsArrayBuffer(value.as_raw()) } {
        ArrayBuffer::from_value(value.clone())
            .map(Some)
            .ok_or_else(|| Exception::throw_type(value.ctx(), "detached buffer"))
    } else {
        Ok(None)
    }
}

pub(crate) fn is_source(value: &Value<'_>) -> bool {
    // SAFETY: the value is live and these operations only inspect internal class tags.
    unsafe {
        qjs::JS_IsArrayBuffer(value.as_raw())
            || qjs::JS_GetTypedArrayType(value.as_raw()) >= 0
            || qjs::JS_IsDataView(value.as_raw())
    }
}

/// Obtain a byte view of exactly the supplied buffer range.
pub(crate) fn view<'js>(ctx: &Ctx<'js>, input: Value<'js>) -> Result<TypedArray<'js, u8>> {
    if let Some(buffer) = array_buffer(&input)? {
        return TypedArray::from_arraybuffer(buffer);
    }
    let (buffer, offset, length) = view_parts(ctx, input)?;
    intrinsics(ctx)?
        .get::<_, Constructor>("bytes")?
        .construct((buffer, offset, length))
}

pub(crate) fn view_parts<'js>(
    ctx: &Ctx<'js>,
    input: Value<'js>,
) -> Result<(ArrayBuffer<'js>, usize, usize)> {
    // SAFETY: these calls inspect the brand of a live value without invoking JS.
    let (typed, data) = unsafe {
        (
            qjs::JS_GetTypedArrayType(input.as_raw()) >= 0,
            qjs::JS_IsDataView(input.as_raw()),
        )
    };
    if !typed && !data {
        return Err(Exception::throw_type(ctx, "Expected a buffer or view"));
    }
    let holder = intrinsics(ctx)?;
    let getters: Object = holder.get(if typed { "typed" } else { "data" })?;
    let buffer: Value = getters
        .get::<_, Function>("buffer")?
        .call((This(input.clone()),))?;
    let buffer = array_buffer(&buffer)?
        .ok_or_else(|| Exception::throw_type(ctx, "Expected a non-shared ArrayBuffer"))?;
    // The engine's raw typed-array API reports the original length for a
    // length-tracking view. The getters expose the current bounds, and this
    // fixed-range Uint8Array lets the Rust reader use that size.
    let offset: usize = getters
        .get::<_, Function>("byteOffset")?
        .call((This(input.clone()),))?;
    let length: usize = getters
        .get::<_, Function>("byteLength")?
        .call((This(input),))?;
    Ok((buffer, offset, length))
}
