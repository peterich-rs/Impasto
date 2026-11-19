//! The `__host` object the legacy prelude calls.
//!
//! Both runtimes install this object. The port only receives owned requests.
//! Promise settlement stays on [`crate::quickjs::Vm`].

use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Value};

use crate::quickjs::Vm;

pub struct HttpCall {
    pub streaming: bool,
    pub token: u64,
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: Vec<u8>,
}

pub trait IoPort {
    fn http(&self, call: HttpCall);
    fn abort(&self, token: u64);
    fn has_module(&self, name: &str) -> bool;
    fn open_socket(&self, url: String, protocols: String) -> u64;
    fn send_socket(&self, id: u64, text: bool, data: Vec<u8>);
    fn close_socket(&self, id: u64);
}

/// # Safety
///
/// `vm` and `port` must stay allocated and unchanged for as long as the
/// functions registered here can run. `vm` is only used while a JavaScript
/// call is on the stack.
pub unsafe fn install<'js>(
    ctx: &Ctx<'js>,
    vm: *mut Vm,
    port: *const dyn IoPort,
) -> rquickjs::Result<()> {
    let bridge = Object::new(ctx.clone())?;
    let http = function(ctx, vm, port, false)?;
    let stream = function(ctx, vm, port, true)?;
    bridge.set("http", http)?;
    bridge.set("httpStream", stream)?;
    bridge.set(
        "abort",
        Function::new(ctx.clone(), move |args: Rest<Value<'js>>| {
            let token = js_id(args.0.first());
            if token != 0 {
                // SAFETY: `port` belongs to the runtime that installed this function.
                unsafe { (*port).abort(token) };
            }
        })?,
    )?;
    bridge.set(
        "hasModule",
        Function::new(
            ctx.clone(),
            move |_ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                let name = match args.0.first() {
                    Some(value) => value
                        .as_string()
                        .and_then(|text| text.to_string().ok())
                        .unwrap_or_default(),
                    None => String::new(),
                };
                // SAFETY: `port` belongs to the runtime that installed this function.
                let present = unsafe { (*port).has_module(&name) };
                rquickjs::Result::Ok(present)
            },
        )?,
    )?;
    bridge.set(
        "socketOpen",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let url = arg_string(&ctx, args.0.first());
            let protocols = arg_string(&ctx, args.0.get(1));
            // SAFETY: `port` belongs to the runtime that installed this function.
            let id = unsafe { (*port).open_socket(url, protocols) };
            rquickjs::Result::Ok(js_number(id))
        })?,
    )?;
    bridge.set(
        "socketSend",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let id = js_id(args.0.first());
            let text = args.0.get(1).is_some_and(|value| value.is_string());
            let data = if text {
                arg_string(&ctx, args.0.get(1)).into_bytes()
            } else {
                Vec::new()
            };
            // SAFETY: `port` belongs to the runtime that installed this function.
            unsafe { (*port).send_socket(id, text, data) };
            rquickjs::Result::Ok(())
        })?,
    )?;
    bridge.set(
        "socketClose",
        Function::new(ctx.clone(), move |args: Rest<Value<'js>>| {
            let id = js_id(args.0.first());
            if id != 0 {
                // SAFETY: `port` belongs to the runtime that installed this function.
                unsafe { (*port).close_socket(id) };
            }
        })?,
    )?;
    ctx.globals().set("__host", bridge)
}

fn function<'js>(
    ctx: &Ctx<'js>,
    vm: *mut Vm,
    port: *const dyn IoPort,
    streaming: bool,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
            // SAFETY: `vm` is the armed runtime that installed this function.
            let (token, promise) = unsafe { (*vm).make_promise(&ctx)? };
            let method = arg_string(&ctx, args.0.first());
            let url = arg_string(&ctx, args.0.get(1));
            let headers = arg_string(&ctx, args.0.get(2));
            let body = if args.0.get(3).is_some_and(|value| value.is_string()) {
                arg_string(&ctx, args.0.get(3)).into_bytes()
            } else {
                Vec::new()
            };
            // SAFETY: `port` belongs to the runtime that installed this function.
            unsafe {
                (*port).http(HttpCall {
                    streaming,
                    token,
                    method,
                    url,
                    headers,
                    body,
                });
            }
            Ok(promise)
        },
    )
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "host ids are finite integers round-tripped through a JavaScript number"
)]
fn js_id(value: Option<&Value<'_>>) -> u64 {
    let number = value.and_then(|value| value.as_number()).unwrap_or(0.0);
    if number.is_finite() && number > 0.0 {
        number as u64
    } else {
        0
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "socket ids fit in the integer range of a JavaScript number"
)]
fn js_number(id: u64) -> f64 {
    id as f64
}

fn arg_string(_ctx: &Ctx<'_>, value: Option<&Value<'_>>) -> String {
    value
        .and_then(|value| value.as_string())
        .and_then(|text| text.to_string().ok())
        .unwrap_or_default()
}
