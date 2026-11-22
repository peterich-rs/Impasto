//! Settlement and bridge hooks on the JavaScript thread.

use crate::host::{Credit, IoEvent, PluginResult};
use rquickjs::{Ctx, IntoJs, Value};

use crate::runtime::quickjs::{Vm, codec};

pub(crate) fn apply(
    vm: &Vm,
    ctx: &Ctx<'_>,
    event: IoEvent,
    credit: Option<Credit>,
) -> rquickjs::Result<()> {
    match event {
        IoEvent::Plugin { token, result } => {
            let (ok, value) = match result {
                PluginResult::Done(value) => (true, codec::encode(ctx, &value)),
                PluginResult::Failed { message } => (false, message.into_js(ctx)),
            };
            settle(vm, ctx, token, ok, value)
        }
        IoEvent::NativeStream { id, event } => {
            crate::standard::native_stream::apply(ctx, id, event, credit)
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
