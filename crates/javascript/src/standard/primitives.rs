use std::sync::Arc;

use rquickjs::{Ctx, Exception, Function, Object, Result, Value, qjs};

use crate::runtime::quickjs::buffer;

pub trait EntropySource: Send + Sync {
    fn fill(&self, bytes: &mut [u8]) -> std::io::Result<()>;
}

pub struct SystemEntropy;

impl EntropySource for SystemEntropy {
    fn fill(&self, bytes: &mut [u8]) -> std::io::Result<()> {
        getrandom::fill(bytes).map_err(std::io::Error::other)
    }
}

pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    globals: &Object<'js>,
    entropy: Arc<dyn EntropySource>,
) -> Result<()> {
    // Context::full installs queueMicrotask, DOMException, atob and btoa.
    super::structured_clone::install(ctx, globals)?;
    let crypto = Object::new(ctx.clone())?;
    let random = Arc::clone(&entropy);
    crypto.set(
        "getRandomValues",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, input: Value<'js>| {
            // SAFETY: this reads the internal brand of a live JS value.
            let kind = unsafe { qjs::JS_GetTypedArrayType(input.as_raw()) };
            // The integer kinds occupy the contiguous part before the floats.
            if kind < 0 || kind >= qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_FLOAT16 as i32 {
                return Err(ctx.throw(super::events::named_error(
                    &ctx,
                    "TypeMismatchError",
                    "Expected an integer typed array",
                )?));
            }
            let view = buffer::view(&ctx, input.clone())?;
            if view.len() > 65_536 {
                return Err(ctx.throw(super::events::named_error(
                    &ctx,
                    "QuotaExceededError",
                    "getRandomValues is limited to 65536 bytes",
                )?));
            }
            let mut bytes = vec![0; view.len()];
            random
                .fill(&mut bytes)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            let target = view
                .as_raw()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached typed array"))?;
            // SAFETY: the view is rooted in this context and its byte span has
            // this exact length. No JavaScript runs while the pointer is live.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    target.as_ptr().cast::<u8>(),
                    bytes.len(),
                )
            };
            Ok(input)
        })?
        .with_name("getRandomValues")?,
    )?;
    crypto.set(
        "randomUUID",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            let mut bytes = [0; 16];
            entropy
                .fill(&mut bytes)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let mut uuid = String::with_capacity(36);
            for (index, byte) in bytes.into_iter().enumerate() {
                if matches!(index, 4 | 6 | 8 | 10) {
                    uuid.push('-');
                }
                use std::fmt::Write;
                let _ = write!(uuid, "{byte:02x}");
            }
            Ok::<_, rquickjs::Error>(uuid)
        })?
        .with_name("randomUUID")?,
    )?;
    globals.set("crypto", crypto)
}

pub(super) const DECLARATIONS: &str = r#"
declare function queueMicrotask(callback: () => void): void;
declare function atob(data: string): string;
declare function btoa(data: string): string;
declare class DOMException extends Error {
  constructor(message?: string, name?: string);
  readonly name: string;
  readonly message: string;
  readonly code: number;
  readonly [Symbol.toStringTag]: string;
  static readonly INDEX_SIZE_ERR: 1;
  readonly INDEX_SIZE_ERR: 1;
  static readonly DOMSTRING_SIZE_ERR: 2;
  readonly DOMSTRING_SIZE_ERR: 2;
  static readonly HIERARCHY_REQUEST_ERR: 3;
  readonly HIERARCHY_REQUEST_ERR: 3;
  static readonly WRONG_DOCUMENT_ERR: 4;
  readonly WRONG_DOCUMENT_ERR: 4;
  static readonly INVALID_CHARACTER_ERR: 5;
  readonly INVALID_CHARACTER_ERR: 5;
  static readonly NO_DATA_ALLOWED_ERR: 6;
  readonly NO_DATA_ALLOWED_ERR: 6;
  static readonly NO_MODIFICATION_ALLOWED_ERR: 7;
  readonly NO_MODIFICATION_ALLOWED_ERR: 7;
  static readonly NOT_FOUND_ERR: 8;
  readonly NOT_FOUND_ERR: 8;
  static readonly NOT_SUPPORTED_ERR: 9;
  readonly NOT_SUPPORTED_ERR: 9;
  static readonly INUSE_ATTRIBUTE_ERR: 10;
  readonly INUSE_ATTRIBUTE_ERR: 10;
  static readonly INVALID_STATE_ERR: 11;
  readonly INVALID_STATE_ERR: 11;
  static readonly SYNTAX_ERR: 12;
  readonly SYNTAX_ERR: 12;
  static readonly INVALID_MODIFICATION_ERR: 13;
  readonly INVALID_MODIFICATION_ERR: 13;
  static readonly NAMESPACE_ERR: 14;
  readonly NAMESPACE_ERR: 14;
  static readonly INVALID_ACCESS_ERR: 15;
  readonly INVALID_ACCESS_ERR: 15;
  static readonly VALIDATION_ERR: 16;
  readonly VALIDATION_ERR: 16;
  static readonly TYPE_MISMATCH_ERR: 17;
  readonly TYPE_MISMATCH_ERR: 17;
  static readonly SECURITY_ERR: 18;
  readonly SECURITY_ERR: 18;
  static readonly NETWORK_ERR: 19;
  readonly NETWORK_ERR: 19;
  static readonly ABORT_ERR: 20;
  readonly ABORT_ERR: 20;
  static readonly URL_MISMATCH_ERR: 21;
  readonly URL_MISMATCH_ERR: 21;
  static readonly QUOTA_EXCEEDED_ERR: 22;
  readonly QUOTA_EXCEEDED_ERR: 22;
  static readonly TIMEOUT_ERR: 23;
  readonly TIMEOUT_ERR: 23;
  static readonly INVALID_NODE_TYPE_ERR: 24;
  readonly INVALID_NODE_TYPE_ERR: 24;
  static readonly DATA_CLONE_ERR: 25;
  readonly DATA_CLONE_ERR: 25;
}
interface StructuredSerializeOptions {
  transfer?: Iterable<ArrayBuffer>;
}
declare function structuredClone<T>(value: T, options?: StructuredSerializeOptions): T;
declare const crypto: {
  getRandomValues<T extends Int8Array | Uint8Array | Uint8ClampedArray | Int16Array | Uint16Array
    | Int32Array | Uint32Array | BigInt64Array | BigUint64Array>(array: T): T;
  randomUUID(): string;
};
"#;
