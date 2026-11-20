//! Settlement and bridge hooks on the JavaScript thread.

use std::cell::RefCell;

use crate::host::{Credit, HttpResponse, IoEvent, PluginResult};
use rquickjs::{ArrayBuffer, Ctx, Function, IntoJs, Object, Value};

use crate::runtime::quickjs::{Vm, codec};
use crate::runtime::resources::number;

pub(crate) fn apply(
    vm: &Vm,
    ctx: &Ctx<'_>,
    event: IoEvent,
    credit: Option<Credit>,
) -> rquickjs::Result<()> {
    match event {
        IoEvent::Http { token, result } => {
            let (ok, value) = match result {
                Ok(response) => (true, response_value(ctx, response)),
                Err(error) => (false, error.dom_name().into_js(ctx)),
            };
            settle(vm, ctx, token, ok, value)
        }
        IoEvent::Plugin { token, result } => {
            let (ok, value) = match result {
                PluginResult::Done(value) => (true, codec::encode(ctx, &value)),
                PluginResult::Failed { message } => (false, message.into_js(ctx)),
            };
            settle(vm, ctx, token, ok, value)
        }
        IoEvent::Socket {
            id,
            event,
            text,
            data,
        } => {
            let payload = if text {
                String::from_utf8_lossy(&data).into_js(ctx)?
            } else {
                ArrayBuffer::new_copy(ctx.clone(), data)?.into_value()
            };
            call_hook(
                ctx,
                "_socket",
                [number(id).into_js(ctx)?, event.into_js(ctx)?, payload],
            )
        }
        IoEvent::HttpHead {
            token,
            status,
            headers,
        } => {
            let head = Object::new(ctx.clone())?;
            head.set("status", status)?;
            head.set("headers", header_block(&headers))?;
            call_hook(
                ctx,
                "_httpStream",
                [
                    number(token).into_js(ctx)?,
                    "head".into_js(ctx)?,
                    head.into_value(),
                ],
            )
        }
        IoEvent::HttpChunk {
            token,
            bytes,
            done,
            error,
        } => {
            let terminal = done || error.is_some();
            let payload = match error {
                Some(error) => error.dom_name().into_js(ctx)?,
                None => ArrayBuffer::new_copy(ctx.clone(), bytes)?.into_value(),
            };
            let release = match credit {
                Some(credit) => {
                    let credit = RefCell::new(Some(credit));
                    Function::new(ctx.clone(), move || drop(credit.borrow_mut().take()))?
                        .into_value()
                }
                None => Value::new_undefined(ctx.clone()),
            };
            call_hook(
                ctx,
                "_httpStream",
                [
                    number(token).into_js(ctx)?,
                    (if terminal { "done" } else { "chunk" }).into_js(ctx)?,
                    payload,
                    release,
                ],
            )?;
            if terminal {
                vm.promises.borrow_mut().remove(&token);
            }
            Ok(())
        }
        IoEvent::Host(_) => Ok(()),
    }
}

fn settle<'js>(
    vm: &Vm,
    ctx: &Ctx<'js>,
    token: u64,
    ok: bool,
    value: rquickjs::Result<Value<'js>>,
) -> rquickjs::Result<()> {
    match value {
        Ok(value) => vm.settle(ctx, token, ok, value),
        Err(error) => {
            if error.is_exception() {
                let _ = ctx.catch();
            }
            reject_bad_completion(vm, ctx, token)
        }
    }
}

pub(crate) fn reject_bad_completion(vm: &Vm, ctx: &Ctx<'_>, token: u64) -> rquickjs::Result<()> {
    let rejection = "host completion could not be converted"
        .into_js(ctx)
        .unwrap_or_else(|_| {
            let _ = ctx.catch();
            Value::new_undefined(ctx.clone())
        });
    vm.settle(ctx, token, false, rejection)
}

fn response_value<'js>(ctx: &Ctx<'js>, response: HttpResponse) -> rquickjs::Result<Value<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("status", response.status)?;
    object.set("headers", header_block(&response.headers))?;
    object.set("body", ArrayBuffer::new_copy(ctx.clone(), response.body)?)?;
    Ok(object.into_value())
}

fn call_hook<'js, const N: usize>(
    ctx: &Ctx<'js>,
    name: &str,
    args: [Value<'js>; N],
) -> rquickjs::Result<()> {
    let bridge: Object = ctx.globals().get("__host")?;
    let hook: Value = bridge.get(name)?;
    if let Some(hook) = hook.into_function() {
        let mut values = rquickjs::function::Args::new(ctx.clone(), N);
        values.push_args(args)?;
        hook.call_arg::<()>(values)?;
    }
    Ok(())
}

fn header_block(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}
