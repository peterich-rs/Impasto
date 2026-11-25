//! Settlement and bridge hooks on the JavaScript thread.

use crate::host::{Credit, IoEvent, PluginResult};
use rquickjs::{Ctx, Exception, IntoJs, Value};

use crate::runtime::quickjs::{Vm, codec, native_resource::ResourceTable};

pub(crate) fn apply(
    vm: &Vm,
    ctx: &Ctx<'_>,
    event: IoEvent,
    credit: Option<Credit>,
    resources: &std::rc::Rc<ResourceTable>,
    cap: usize,
) -> rquickjs::Result<()> {
    match event {
        IoEvent::Plugin { token, result } => {
            let mut outcome = result;
            let (ok, value) = loop {
                break match outcome {
                    PluginResult::Done(value) => (true, codec::encode(ctx, &value)),
                    PluginResult::Resource(factory) => {
                        (true, resources.open(ctx, cap, || Ok(factory.open())))
                    }
                    PluginResult::Failed { message } => (false, message.into_js(ctx)),
                    PluginResult::Rejected { message } => (false, error_value(ctx, &message)),
                    PluginResult::Scoped { cancel, result } => {
                        if cancel.is_cancelled() {
                            (false, error_value(ctx, "native resource is closed"))
                        } else {
                            outcome = *result;
                            continue;
                        }
                    }
                };
            };
            settle(vm, ctx, token, ok, value)
        }
        IoEvent::NativeStream { id, event } => {
            crate::standard::native_stream::apply(ctx, id, event, credit)
        }
        IoEvent::Host(_) => Ok(()),
    }
}

fn error_value<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Result<Value<'js>> {
    Exception::from_message(ctx.clone(), message)
        .map(|error| error.as_object().clone().into_value())
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
