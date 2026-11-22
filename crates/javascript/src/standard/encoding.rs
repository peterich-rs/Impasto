//! Native UTF-8 codecs and Web IDL conversions shared by the Web bindings.

use std::{ptr::NonNull, str};

use rquickjs::class::Trace;
use rquickjs::function::{Opt, This};
use rquickjs::{
    Class, Coerced, Ctx, Error, Exception, FromJs, JsLifetime, Object, Result, String as JsString,
    TypedArray, Value, qjs,
};

use crate::runtime::quickjs::buffer::view as buffer_bytes;

use super::web::constructor;

pub(super) fn install<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    globals.set(
        "TextEncoder",
        constructor::<TextEncoder, _, _>(ctx, || TextEncoder {})?,
    )?;
    globals.set(
        "TextDecoder",
        constructor::<TextDecoder, _, _>(ctx, move |ctx, label, options| {
            TextDecoder::new(&ctx, label, options)
        })?,
    )
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct TextEncoder {}

#[rquickjs::methods]
impl TextEncoder {
    #[qjs(get, rename = "encoding")]
    fn encoding(&self) -> &'static str {
        "utf-8"
    }

    fn encode<'js>(
        _: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        Opt(input): Opt<Value<'js>>,
    ) -> Result<TypedArray<'js, u8>> {
        let text = match input.filter(|value| !value.is_undefined()) {
            Some(value) => usv_string(&ctx, value)?,
            None => String::new(),
        };
        TypedArray::new_copy(ctx, text.as_bytes())
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct TextDecoder {
    fatal: bool,
    ignore_bom: bool,
    pending: Vec<u8>,
    bom_seen: bool,
}

#[rquickjs::methods]
impl TextDecoder {
    #[qjs(skip)]
    fn new<'js>(
        ctx: &Ctx<'js>,
        Opt(label): Opt<Value<'js>>,
        Opt(options): Opt<Value<'js>>,
    ) -> Result<Self> {
        if let Some(label) = label.filter(|label| !label.is_undefined()) {
            let label = usv_string(ctx, label)?;
            let label = label
                .trim_matches(['\t', '\n', '\x0c', '\r', ' '])
                .to_ascii_lowercase();
            if !matches!(
                label.as_str(),
                "unicode-1-1-utf-8"
                    | "unicode11utf8"
                    | "unicode20utf8"
                    | "utf-8"
                    | "utf8"
                    | "x-unicode20utf8"
            ) {
                return Err(Exception::throw_range(ctx, "Only UTF-8 is supported"));
            }
        }
        Ok(Self {
            fatal: option_flag(options.as_ref(), "fatal")?,
            ignore_bom: option_flag(options.as_ref(), "ignoreBOM")?,
            pending: Vec::new(),
            bom_seen: false,
        })
    }

    #[qjs(get, rename = "encoding")]
    fn encoding(&self) -> &'static str {
        "utf-8"
    }

    #[qjs(get, rename = "fatal")]
    fn fatal(&self) -> bool {
        self.fatal
    }

    #[qjs(get, rename = "ignoreBOM")]
    fn ignore_bom(&self) -> bool {
        self.ignore_bom
    }

    fn decode<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        Opt(input): Opt<Value<'js>>,
        Opt(options): Opt<Value<'js>>,
    ) -> Result<String> {
        let streaming = option_flag(options.as_ref(), "stream")?;
        let view = input
            .filter(|value| !value.is_undefined())
            .map(|value| buffer_bytes(&ctx, value))
            .transpose()?;
        let mut state = this.try_borrow_mut()?;
        let bytes = match &view {
            Some(view) => {
                // SAFETY: buffer_bytes rejects shared memory, and no JS can
                // resize, detach or write the buffer before decoding finishes.
                unsafe { view.as_bytes() }
                    .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
            }
            None => &[],
        };
        let result = decode_chunk(&state.pending, bytes, state.fatal, streaming);
        let Some((mut text, pending)) = result else {
            state.pending.clear();
            state.bom_seen = false;
            return Err(Exception::throw_type(&ctx, "Invalid UTF-8 data"));
        };
        if !state.bom_seen && !text.is_empty() {
            state.bom_seen = true;
            if !state.ignore_bom && text.starts_with('\u{feff}') {
                text.drain(..'\u{feff}'.len_utf8());
            }
        }
        if streaming {
            state.pending = pending;
        } else {
            state.pending.clear();
            state.bom_seen = false;
        }
        Ok(text)
    }
}

fn option_flag(options: Option<&Value<'_>>, name: &str) -> Result<bool> {
    options
        .and_then(Value::as_object)
        .map_or(Ok(false), |options| {
            options.get::<_, Coerced<bool>>(name).map(|flag| flag.0)
        })
}

/// Own the engine's temporary UTF-8 representation until the Rust copy is ready.
struct EncodedString<'a, 'js> {
    ctx: &'a Ctx<'js>,
    pointer: NonNull<qjs::c_char>,
}

impl Drop for EncodedString<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: JS_ToCStringLen2 allocated this reference for the same live context.
        unsafe { qjs::JS_FreeCString(self.ctx.as_raw().as_ptr(), self.pointer.as_ptr()) };
    }
}

/// Web IDL USVString conversion. QuickJS combines paired surrogates in UTF-8
/// mode but preserves unpaired surrogates as three-byte sequences.
pub(super) fn usv_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    let Coerced(value) = Coerced::<JsString<'js>>::from_js(ctx, value)?;
    let mut length: qjs::size_t = 0;
    // SAFETY: value is an owned string in ctx and length points to a valid output slot.
    let raw =
        unsafe { qjs::JS_ToCStringLen2(ctx.as_raw().as_ptr(), &mut length, value.as_raw(), false) };
    let pointer = NonNull::new(raw.cast_mut()).ok_or(Error::Exception)?;
    let encoded = EncodedString { ctx, pointer };
    let length =
        usize::try_from(length).map_err(|_| Exception::throw_range(ctx, "String is too large"))?;
    // SAFETY: the string reference owns these initialized bytes until encoded is dropped.
    let mut input = unsafe { std::slice::from_raw_parts(encoded.pointer.as_ptr().cast(), length) };
    if let Ok(valid) = str::from_utf8(input) {
        return Ok(valid.to_owned());
    }
    let mut text = String::with_capacity(input.len());
    loop {
        match str::from_utf8(input) {
            Ok(valid) => {
                text.push_str(valid);
                return Ok(text);
            }
            Err(error) => {
                let (valid, tail) = input.split_at(error.valid_up_to());
                if let Ok(valid) = str::from_utf8(valid) {
                    text.push_str(valid);
                }
                text.push('\u{fffd}');
                // A preserved surrogate is encoded as ED A0..BF 80..BF.
                let length = if matches!(tail, [0xed, 0xa0..=0xbf, 0x80..=0xbf, ..]) {
                    3
                } else {
                    error.error_len().unwrap_or(tail.len())
                };
                input = &tail[length..];
            }
        }
    }
}

/// Response and request bodies decode with a fresh, nonfatal UTF-8 decoder.
pub(super) fn decode_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).into_owned()
}

fn decode_chunk(
    pending: &[u8],
    bytes: &[u8],
    fatal: bool,
    streaming: bool,
) -> Option<(String, Vec<u8>)> {
    let input;
    let mut remaining = if pending.is_empty() {
        bytes
    } else {
        input = [pending, bytes].concat();
        input.as_slice()
    };
    let mut text = String::new();
    loop {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                text.push_str(valid);
                return Some((text, Vec::new()));
            }
            Err(error) => {
                let (valid, tail) = remaining.split_at(error.valid_up_to());
                text.push_str(str::from_utf8(valid).ok()?);
                let malformed = match error.error_len() {
                    Some(length) => length,
                    None if streaming => return Some((text, tail.to_vec())),
                    None => tail.len(),
                };
                if fatal {
                    return None;
                }
                text.push('\u{fffd}');
                remaining = &tail[malformed..];
            }
        }
    }
}

pub(super) const DECLARATIONS: &str = r#"
type BufferSource = ArrayBuffer | ArrayBufferView;
type Utf8Label = "utf-8" | "utf8" | "unicode-1-1-utf-8" | "unicode11utf8" | "unicode20utf8" | "x-unicode20utf8";

declare class TextEncoder {
  readonly encoding: "utf-8";
  encode(input?: string): Uint8Array;
}

declare class TextDecoder {
  constructor(label?: Utf8Label, options?: { fatal?: boolean; ignoreBOM?: boolean });
  readonly encoding: "utf-8";
  readonly fatal: boolean;
  readonly ignoreBOM: boolean;
  decode(input?: BufferSource, options?: { stream?: boolean }): string;
}
"#;
