//! Default readers and controllers for in-memory JavaScript sources.

use rquickjs::class::{ClassKind, JsCell, JsClass, Trace, Writable};
use rquickjs::function::{Constructor, Opt, Params, This};
use rquickjs::{
    Class, Ctx, Error, Exception, Function, JsLifetime, Object, Promise, Result, Value,
};

use super::{
    Deferred, Phase, ReadableStream, create, error, exception_value, finish, promise_then,
    push_chunk,
};

type Stream<'js> = Class<'js, ReadableStream<'js>>;

#[derive(Trace, JsLifetime)]
pub(super) struct Source<'js> {
    object: Object<'js>,
    controller: Class<'js, Controller<'js>>,
    pull: Option<Function<'js>>,
    cancel: Option<Function<'js>>,
    size: Option<Function<'js>>,
    high_water_mark: f64,
    pub(super) queued: f64,
    started: bool,
    pulling: bool,
    pull_again: bool,
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ReadableStreamDefaultController")]
pub(super) struct Controller<'js> {
    stream: Stream<'js>,
}

#[rquickjs::methods]
impl<'js> Controller<'js> {
    #[qjs(get, rename = "desiredSize")]
    fn desired_size(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let stream = self.stream.try_borrow()?;
        let size = match stream.phase {
            Phase::Errored(_) => return Ok(Value::new_null(ctx)),
            Phase::Closed | Phase::Ending => 0.0,
            Phase::Readable => stream
                .source
                .as_ref()
                .map_or(0.0, |source| source.high_water_mark - source.queued),
        };
        Ok(Value::new_number(ctx, size))
    }

    fn close(&self, ctx: Ctx<'js>) -> Result<()> {
        self.require_readable(&ctx)?;
        finish(&ctx, &self.stream)
    }

    fn error(&self, ctx: Ctx<'js>, Opt(reason): Opt<Value<'js>>) -> Result<()> {
        error(
            &ctx,
            &self.stream,
            reason.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
        )
    }

    fn enqueue(&self, ctx: Ctx<'js>, Opt(value): Opt<Value<'js>>) -> Result<()> {
        self.require_readable(&ctx)?;
        let size = {
            let state = self.stream.try_borrow()?;
            state
                .pending
                .is_empty()
                .then(|| state.source.as_ref().and_then(|source| source.size.clone()))
                .flatten()
        };
        let value = value.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let weight = size.map_or(Ok(1.0), |size| {
            size.call::<_, rquickjs::Coerced<f64>>((value.clone(),))
                .map(|size| size.0)
        });
        let weight = match weight {
            Ok(weight) if weight.is_finite() && weight >= 0.0 => weight,
            result => {
                let cause = match result {
                    Err(cause) => cause,
                    Ok(_) => Exception::throw_range(&ctx, "Invalid stream chunk size"),
                };
                let cause = exception_value(&ctx, cause)?;
                error(&ctx, &self.stream, cause.clone())?;
                return Err(ctx.throw(cause));
            }
        };
        if !push_chunk(&ctx, &self.stream, value, 0, weight, None)? {
            let cause = Exception::throw_range(&ctx, "Stream queue limit exceeded");
            let cause = exception_value(&ctx, cause)?;
            error(&ctx, &self.stream, cause.clone())?;
            return Err(ctx.throw(cause));
        }
        pull_if_needed(&ctx, &self.stream)
    }
}

impl Controller<'_> {
    fn require_readable(&self, ctx: &Ctx<'_>) -> Result<()> {
        if !matches!(self.stream.try_borrow()?.phase, Phase::Readable) {
            return Err(Exception::throw_type(ctx, "Stream is closing or closed"));
        }
        Ok(())
    }
}

pub(super) fn construct<'js>(
    ctx: Ctx<'js>,
    Opt(underlying): Opt<Value<'js>>,
    Opt(strategy): Opt<Object<'js>>,
) -> Result<Stream<'js>> {
    let object = match underlying.filter(|value| !value.is_undefined()) {
        Some(value) => value
            .into_object()
            .ok_or_else(|| Exception::throw_type(&ctx, "Expected a stream source"))?,
        None => Object::new(ctx.clone())?,
    };
    if !object.get::<_, Value>("type")?.is_undefined() {
        return Err(Exception::throw_type(
            &ctx,
            "Only default streams are supported",
        ));
    }
    let high_water_mark = strategy
        .as_ref()
        .map(|strategy| strategy.get::<_, Option<rquickjs::Coerced<f64>>>("highWaterMark"))
        .transpose()?
        .flatten()
        .map_or(1.0, |mark| mark.0);
    if high_water_mark.is_nan() || high_water_mark < 0.0 {
        return Err(Exception::throw_range(
            &ctx,
            "Invalid stream high-water mark",
        ));
    }
    let stream = create(&ctx, None)?;
    let controller = Class::instance(
        ctx.clone(),
        Controller {
            stream: stream.clone(),
        },
    )?;
    let start = callback(&ctx, &object, "start")?;
    let pull = callback(&ctx, &object, "pull")?;
    let cancel = callback(&ctx, &object, "cancel")?;
    let size = strategy
        .as_ref()
        .map(|strategy| callback(&ctx, strategy, "size"))
        .transpose()?
        .flatten();
    stream.try_borrow_mut()?.source = Some(Source {
        object: object.clone(),
        controller: controller.clone(),
        pull,
        cancel,
        size,
        high_water_mark,
        queued: 0.0,
        started: false,
        pulling: false,
        pull_again: false,
    });
    let started = match start {
        Some(start) => start.call((This(object), controller))?,
        None => Value::new_undefined(ctx.clone()),
    };
    react(&ctx, &stream, started, true)?;
    Ok(stream)
}

fn callback<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &str,
) -> Result<Option<Function<'js>>> {
    let method: Value = object.get(name)?;
    if method.is_undefined() {
        return Ok(None);
    }
    method
        .into_function()
        .map(Some)
        .ok_or_else(|| Exception::throw_type(ctx, "Expected a stream callback"))
}

pub(super) fn pull_if_needed<'js>(ctx: &Ctx<'js>, stream: &Stream<'js>) -> Result<()> {
    let (pull, source, controller) = {
        let mut state = stream.try_borrow_mut()?;
        if !matches!(state.phase, Phase::Readable) {
            return Ok(());
        }
        let has_read = !state.pending.is_empty();
        let Some(source) = state.source.as_mut() else {
            return Ok(());
        };
        if !source.started || (!has_read && source.high_water_mark <= source.queued) {
            return Ok(());
        }
        let Some(pull) = source.pull.clone() else {
            return Ok(());
        };
        if source.pulling {
            source.pull_again = true;
            return Ok(());
        }
        source.pulling = true;
        (pull, source.object.clone(), source.controller.clone())
    };
    match pull.call((This(source), controller)) {
        Ok(pulled) => react(ctx, stream, pulled, false),
        Err(reason) => error(ctx, stream, exception_value(ctx, reason)?),
    }
}

pub(super) fn cancel<'js>(
    ctx: &Ctx<'js>,
    source: Option<Source<'js>>,
    reason: Value<'js>,
) -> Result<Promise<'js>> {
    let deferred = Deferred::new(ctx)?;
    if let Some(source) = source
        && let Some(cancel) = source.cancel
    {
        match cancel.call::<_, Value>((This(source.object), reason)) {
            Ok(result) => {
                deferred.resolve(result)?;
                let ignore_value = Function::new(ctx.clone(), || ())?;
                return promise_then(ctx, &deferred.promise, Some(&ignore_value), None);
            }
            Err(cause) => deferred.reject(exception_value(ctx, cause)?)?,
        }
    } else {
        deferred.resolve(())?;
    }
    Ok(deferred.promise)
}

fn react<'js>(ctx: &Ctx<'js>, stream: &Stream<'js>, value: Value<'js>, start: bool) -> Result<()> {
    let react = |rejected| -> Result<Function<'js>> {
        Class::instance(
            ctx.clone(),
            Reaction {
                stream: stream.clone(),
                start,
                rejected,
            },
        )?
        .into_value()
        .into_function()
        .ok_or(Error::Unknown)
    };
    let deferred = Deferred::new(ctx)?;
    deferred.resolve(value)?;
    promise_then(
        ctx,
        &deferred.promise,
        Some(&react(false)?),
        Some(&react(true)?),
    )?;
    Ok(())
}

#[derive(Trace, JsLifetime)]
struct Reaction<'js> {
    stream: Stream<'js>,
    start: bool,
    rejected: bool,
}

impl<'js> JsClass<'js> for Reaction<'js> {
    const NAME: &'static str = "StreamReaction";
    const KIND: ClassKind = ClassKind::Callable;
    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        Ok(Some(Function::prototype(ctx.clone())))
    }

    fn constructor(_: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }

    fn call(this: &JsCell<'js, Self>, params: Params<'_, 'js>) -> Result<Value<'js>> {
        let this = this.try_borrow().map_err(Error::ClassBorrow)?;
        let ctx = params.ctx();
        let undefined = Value::new_undefined(ctx.clone());
        if this.rejected {
            error(
                ctx,
                &this.stream,
                params.arg(0).unwrap_or_else(|| undefined.clone()),
            )?;
        } else {
            let pull_again = {
                let mut stream = this.stream.try_borrow_mut()?;
                if let Some(source) = stream.source.as_mut() {
                    if this.start {
                        source.started = true;
                        true
                    } else {
                        source.pulling = false;
                        std::mem::take(&mut source.pull_again)
                    }
                } else {
                    false
                }
            };
            if pull_again {
                pull_if_needed(ctx, &this.stream)?;
            }
        }
        Ok(undefined)
    }
}
