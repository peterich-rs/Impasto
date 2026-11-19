//! One completion path for owned [`IoEvent`] values.
//!
//! Headless `drive` and the UI adapter both call [`apply`]. The UI adapter
//! still chooses the moment inside a frame. This function only builds the
//! prelude values and settles the promise.

use js_host::{HttpResponse, IoEvent};
use rquickjs::{ArrayBuffer, Ctx, IntoJs, Object};

use crate::quickjs::Vm;

pub fn apply(vm: &Vm, ctx: &Ctx<'_>, event: IoEvent) -> rquickjs::Result<()> {
    match event {
        IoEvent::Http { token, result } => match result {
            Ok(response) => settle_http(vm, ctx, token, &response),
            Err(error) => vm.settle(ctx, token, false, error.dom_name().into_js(ctx)?),
        },
        IoEvent::Plugin { token, result } => match result {
            js_host::PluginResult::Done(value) => {
                vm.settle(ctx, token, true, plugin_value(ctx, &value)?)
            }
            js_host::PluginResult::Failed { message } => {
                vm.settle(ctx, token, false, message.into_js(ctx)?)
            }
        },
        IoEvent::Socket {
            id,
            event,
            text,
            data,
        } => {
            let payload = if text {
                String::from_utf8_lossy(&data).into_owned().into_js(ctx)?
            } else {
                ArrayBuffer::new(ctx.clone(), data)?.into_js(ctx)?
            };
            call_hook(
                ctx,
                "_socket",
                vec![js_number(id).into_js(ctx)?, event.into_js(ctx)?, payload],
            )
        }
        IoEvent::HttpHead {
            token,
            status,
            headers,
        } => {
            let head = Object::new(ctx.clone())?;
            head.set("status", i32::from(status))?;
            head.set("headers", header_block(&headers))?;
            call_hook(
                ctx,
                "_httpStream",
                vec![
                    js_number(token).into_js(ctx)?,
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
            let payload = match error {
                Some(error) => error.dom_name().into_js(ctx)?,
                None => ArrayBuffer::new(ctx.clone(), bytes)?.into_value(),
            };
            call_hook(
                ctx,
                "_httpStream",
                vec![
                    js_number(token).into_js(ctx)?,
                    (if done { "done" } else { "chunk" }).into_js(ctx)?,
                    payload,
                ],
            )?;
            if done {
                vm.promises.borrow_mut().remove(&token);
            }
            Ok(())
        }
    }
}

fn settle_http(
    vm: &Vm,
    ctx: &Ctx<'_>,
    token: u64,
    response: &HttpResponse,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    object.set("status", i32::from(response.status))?;
    object.set("headers", header_block(&response.headers))?;
    object.set(
        "body",
        ArrayBuffer::new(ctx.clone(), response.body.clone())?,
    )?;
    vm.settle(ctx, token, true, object.into_value())
}

fn plugin_value<'js>(
    ctx: &Ctx<'js>,
    value: &js_host::PluginValue,
) -> rquickjs::Result<rquickjs::Value<'js>> {
    use js_host::PluginValue;
    Ok(match value {
        PluginValue::Null => rquickjs::Value::new_null(ctx.clone()),
        PluginValue::Bool(value) => (*value).into_js(ctx)?,
        PluginValue::F64(value) => (*value).into_js(ctx)?,
        PluginValue::Str(value) => value.into_js(ctx)?,
        PluginValue::Bytes(value) => ArrayBuffer::new(ctx.clone(), value.clone())?.into_js(ctx)?,
        PluginValue::List(_) | PluginValue::Map(_) => rquickjs::Value::new_null(ctx.clone()),
    })
}

fn call_hook<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    args: Vec<rquickjs::Value<'js>>,
) -> rquickjs::Result<()> {
    let bridge: Object = ctx.globals().get("__host")?;
    let hook: rquickjs::Value = bridge.get(name)?;
    if let Some(hook) = hook.into_function() {
        let mut values = rquickjs::function::Args::new(ctx.clone(), args.len());
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

#[allow(
    clippy::cast_precision_loss,
    reason = "host tokens are finite ids and fit in a JavaScript number"
)]
fn js_number(id: u64) -> f64 {
    id as f64
}
