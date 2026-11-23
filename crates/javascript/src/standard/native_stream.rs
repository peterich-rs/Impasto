//! JavaScript bindings for credit-backed native extension streams.

use rquickjs::class::Trace;
use rquickjs::function::{Constructor, This};
use rquickjs::{Class, Ctx, Function, JsLifetime, Object, Result, Value};

use crate::host::{Credit, Route, StreamEvent};
use crate::runtime::quickjs::codec;

use super::{events, streams};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct NativeStreams {}

fn holder<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    Class::<NativeStreams>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)
}

struct CancelOnDrop(Route);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) fn signal<'js>(args: &mut Vec<Value<'js>>) -> Option<Object<'js>> {
    if args.last().is_some_and(events::is_abort_signal) {
        args.pop().and_then(Value::into_object)
    } else {
        None
    }
}

fn key(id: u64) -> String {
    format!("stream{id}")
}

pub(crate) fn create<'js>(
    ctx: &Ctx<'js>,
    route: Route,
    signal: Option<Object<'js>>,
) -> Result<Value<'js>> {
    let id = route.key.sequence;
    if let Some(signal) = &signal
        && let Some(reason) = events::abort_reason(signal)?
    {
        let stream = streams::create(ctx, None)?;
        streams::error(ctx, &stream, reason)?;
        route.cancel();
        return Ok(stream.into_value());
    }
    let cancel = CancelOnDrop(route);
    let cancel = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        cancel.0.cancel();
        forget(&ctx, id)
    })?;
    let stream = streams::create(ctx, Some(cancel))?;
    let holder = holder(ctx)?;
    let weak: Object = holder
        .get::<_, Constructor>("weak")?
        .construct((stream.clone(),))?;
    holder.set(key(id), weak)?;
    if let Some(signal) = signal {
        let abort = Function::new(ctx.clone(), move |ctx: Ctx<'js>, reason: Value<'js>| {
            let result = find(&ctx, id)?;
            forget(&ctx, id)?;
            if let Some(stream) = result {
                streams::error(&ctx, &stream, reason)?;
            }
            Ok::<_, rquickjs::Error>(())
        })?;
        // The detach token belongs to the stream, not the weak index.
        let detach = events::on_abort(&signal, abort)?;
        streams::attach_cleanup(&stream, detach)?;
    }
    Ok(stream.into_value())
}

fn forget(ctx: &Ctx<'_>, id: u64) -> Result<()> {
    holder(ctx)?.remove(key(id))
}

fn find<'js>(ctx: &Ctx<'js>, id: u64) -> Result<Option<Class<'js, streams::ReadableStream<'js>>>> {
    let holder = holder(ctx)?;
    let Some(reference) = holder.get::<_, Option<Object>>(key(id))? else {
        return Ok(None);
    };
    let deref: Function = holder.get("deref")?;
    let stream: Option<Class<'js, streams::ReadableStream>> = deref.call((This(reference),))?;
    Ok(stream)
}

pub(crate) fn apply(
    ctx: &Ctx<'_>,
    id: u64,
    event: StreamEvent,
    credit: Option<Credit>,
) -> Result<()> {
    let Some(stream) = find(ctx, id)? else {
        return forget(ctx, id);
    };
    match event {
        StreamEvent::Item(item) => {
            let bytes = item.payload_bytes();
            let value = codec::encode(ctx, &item)?;
            let release = credit
                .map(|credit| {
                    let reservation = std::cell::RefCell::new(Some(credit));
                    Function::new(ctx.clone(), move || drop(reservation.borrow_mut().take()))
                })
                .transpose()?;
            if !streams::push_value(ctx, &stream, value, bytes, release)? {
                forget(ctx, id)?;
                let error = events::named_error(
                    ctx,
                    "QuotaExceededError",
                    "Native stream queue limit exceeded",
                )?;
                streams::error(ctx, &stream, error)?;
            }
        }
        StreamEvent::Finished => {
            forget(ctx, id)?;
            streams::finish(ctx, &stream)?;
        }
        StreamEvent::Failed(message) => {
            forget(ctx, id)?;
            let error = events::named_error(ctx, "Error", &message)?;
            streams::error(ctx, &stream, error)?;
        }
    }
    Ok(())
}

pub(super) fn install(ctx: &Ctx<'_>) -> Result<()> {
    let holder = holder(ctx)?;
    let weak: Constructor = ctx.globals().get("WeakRef")?;
    let prototype: Object = weak.get("prototype")?;
    holder.set("deref", prototype.get::<_, Function>("deref")?)?;
    holder.set("weak", weak)
}

pub(crate) fn close(ctx: &Ctx<'_>) -> Result<()> {
    let ids = holder(ctx)?.keys::<String>().collect::<Result<Vec<_>>>()?;
    let reason = events::named_error(ctx, "AbortError", "Runtime closed")?;
    for id in ids
        .iter()
        .filter_map(|key| key.strip_prefix("stream")?.parse::<u64>().ok())
    {
        let stream = find(ctx, id)?;
        forget(ctx, id)?;
        if let Some(stream) = stream {
            streams::error(ctx, &stream, reason.clone())?;
        }
    }
    Ok(())
}

pub(crate) fn reap(ctx: &Ctx<'_>, ids: Vec<u64>) {
    for id in ids {
        let _ = forget(ctx, id);
    }
}
