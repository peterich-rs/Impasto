//! Readable streams and byte materialization on the QuickJS owner thread.

mod source;

use std::collections::VecDeque;

use rquickjs::class::Trace;
use rquickjs::function::{Constructor, Opt, This};
use rquickjs::object::Property;
use rquickjs::{
    ArrayBuffer, Class, Ctx, Error, Exception, Function, IntoJs, JsLifetime, Object, Promise,
    Result, TypedArray, Value,
};

use super::encoding::decode_utf8;
use super::web::{Charge, constructor};

const PENDING_READ_LIMIT: usize = 32;
const STREAM_CHUNK_LIMIT: usize = 1024;
const STREAM_BYTE_LIMIT: usize = 4 * 1024 * 1024;
const MATERIALIZED_BYTE_LIMIT: usize = 24 * 1024 * 1024;

#[derive(Clone, Trace, JsLifetime)]
struct Deferred<'js> {
    promise: Promise<'js>,
    resolve: Function<'js>,
    reject: Function<'js>,
    retained: bool,
}

impl<'js> Deferred<'js> {
    fn new(ctx: &Ctx<'js>) -> Result<Self> {
        let (promise, resolve, reject) = ctx.promise()?;
        Ok(Self {
            promise,
            resolve,
            reject,
            retained: false,
        })
    }

    fn retain(&mut self, stream: &Class<'js, ReadableStream<'js>>) -> Result<()> {
        let holder = read_anchors(self.promise.ctx())?;
        let map: Object = holder.get("map")?;
        holder.get::<_, Function>("set")?.call::<_, ()>((
            This(map),
            self.promise.clone(),
            stream.clone(),
        ))?;
        self.retained = true;
        Ok(())
    }

    fn release(&self) -> Result<()> {
        if self.retained {
            let holder = read_anchors(self.promise.ctx())?;
            let map: Object = holder.get("map")?;
            holder
                .get::<_, Function>("delete")?
                .call::<_, ()>((This(map), self.promise.clone()))?;
        }
        Ok(())
    }

    fn resolve(&self, value: impl IntoJs<'js>) -> Result<()> {
        self.release()?;
        self.resolve.call((value,))
    }

    fn reject(&self, error: Value<'js>) -> Result<()> {
        self.release()?;
        self.reject.call((error,))
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct ReadAnchors {}

fn read_anchors<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    Class::<ReadAnchors>::prototype(ctx)?.ok_or(Error::Unknown)
}

#[derive(Trace, JsLifetime)]
struct Chunk<'js> {
    value: Value<'js>,
    release: Option<Function<'js>>,
    len: usize,
    weight: f64,
}

impl<'js> Chunk<'js> {
    fn bytes(&self, ctx: &Ctx<'js>) -> Result<TypedArray<'js, u8>> {
        TypedArray::from_value(self.value.clone())
            .map_err(|_| Exception::throw_type(ctx, "Expected a byte stream"))
    }

    fn release(self) -> Result<Value<'js>> {
        if let Some(release) = self.release {
            release.call::<_, ()>(())?;
        }
        Ok(self.value)
    }
}

#[derive(Trace, JsLifetime)]
enum Phase<'js> {
    Readable,
    Ending,
    Closed,
    Errored(Value<'js>),
}

#[derive(Clone, Copy)]
pub(super) enum BodyOutput {
    Buffer,
    Text,
    Json,
}

#[derive(Trace, JsLifetime)]
struct Materializing<'js> {
    reply: Deferred<'js>,
    bytes: Vec<u8>,
    #[qjs(skip_trace)]
    charge: Charge,
    scheduled: bool,
    #[qjs(skip_trace)]
    output: BodyOutput,
}

#[derive(Trace, JsLifetime)]
enum Lock<'js> {
    Reader(Class<'js, ReadableStreamDefaultReader<'js>>),
    Body(Materializing<'js>),
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ReadableStream<'js> {
    phase: Phase<'js>,
    lock: Option<Lock<'js>>,
    disturbed: bool,
    queue: VecDeque<Chunk<'js>>,
    queued_bytes: usize,
    pending: VecDeque<Deferred<'js>>,
    cancel: Option<Function<'js>>,
    cleanup: Option<Function<'js>>,
    source: Option<source::Source<'js>>,
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ReadableStreamDefaultReader<'js> {
    stream: Option<Class<'js, ReadableStream<'js>>>,
    closed: Deferred<'js>,
}

#[rquickjs::methods]
impl<'js> ReadableStream<'js> {
    #[qjs(get)]
    fn locked(&self) -> bool {
        self.lock.is_some()
    }

    #[qjs(rename = "getReader")]
    fn get_reader(
        ctx: Ctx<'js>,
        This(stream): This<Class<'js, Self>>,
        Opt(options): Opt<Value<'js>>,
    ) -> Result<Class<'js, ReadableStreamDefaultReader<'js>>> {
        if let Some(options) = options.as_ref().and_then(Value::as_object)
            && !options.get::<_, Value<'js>>("mode")?.is_undefined()
        {
            return Err(Exception::throw_type(
                &ctx,
                "Only the default reader is supported",
            ));
        }
        lock(&ctx, &stream)
    }

    fn cancel(
        ctx: Ctx<'js>,
        This(stream): This<Class<'js, Self>>,
        Opt(reason): Opt<Value<'js>>,
    ) -> Result<Promise<'js>> {
        let result = if stream.try_borrow()?.locked() {
            Err(Exception::throw_type(&ctx, "Stream is locked"))
        } else {
            cancel(
                &ctx,
                &stream,
                reason.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
            )
        };
        result.or_else(|error| settlement::<()>(&ctx, Err(error)))
    }
}

#[rquickjs::methods]
impl<'js> ReadableStreamDefaultReader<'js> {
    #[qjs(get)]
    fn closed(&self) -> Promise<'js> {
        self.closed.promise.clone()
    }

    fn read(ctx: Ctx<'js>, This(reader): This<Class<'js, Self>>) -> Result<Promise<'js>> {
        let stream = reader.try_borrow()?.stream.clone();
        let Some(stream) = stream else {
            return settlement::<()>(
                &ctx,
                Err(Exception::throw_type(&ctx, "Reader has no stream")),
            );
        };
        read(&ctx, &stream)
    }

    fn cancel(
        ctx: Ctx<'js>,
        This(reader): This<Class<'js, Self>>,
        Opt(reason): Opt<Value<'js>>,
    ) -> Result<Promise<'js>> {
        let stream = reader.try_borrow()?.stream.clone();
        let result = stream.map_or_else(
            || Err(Exception::throw_type(&ctx, "Reader has no stream")),
            |stream| {
                cancel(
                    &ctx,
                    &stream,
                    reason.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                )
            },
        );
        result.or_else(|error| settlement::<()>(&ctx, Err(error)))
    }

    #[qjs(rename = "releaseLock")]
    fn release_lock(ctx: Ctx<'js>, This(reader): This<Class<'js, Self>>) -> Result<()> {
        release_reader(&ctx, &reader)
    }
}

pub(super) fn create<'js>(
    ctx: &Ctx<'js>,
    cancel: Option<Function<'js>>,
) -> Result<Class<'js, ReadableStream<'js>>> {
    Class::instance(
        ctx.clone(),
        ReadableStream {
            phase: Phase::Readable,
            lock: None,
            disturbed: false,
            queue: VecDeque::new(),
            queued_bytes: 0,
            pending: VecDeque::new(),
            cancel,
            cleanup: None,
            source: None,
        },
    )
}

pub(super) fn attach_cleanup<'js>(
    stream: &Class<'js, ReadableStream<'js>>,
    detach: Function<'js>,
) -> Result<()> {
    stream.try_borrow_mut()?.cleanup = Some(detach);
    Ok(())
}

fn run_cleanup<'js>(stream: &Class<'js, ReadableStream<'js>>) -> Result<()> {
    let cleanup = stream.try_borrow_mut()?.cleanup.take();
    if let Some(cleanup) = cleanup {
        cleanup.call::<_, ()>(())?;
    }
    Ok(())
}

pub(super) fn unavailable<'js>(stream: &Class<'js, ReadableStream<'js>>) -> Result<bool> {
    let stream = stream.try_borrow()?;
    Ok(stream.disturbed || stream.locked())
}

pub(super) fn disturbed<'js>(stream: &Class<'js, ReadableStream<'js>>) -> Result<bool> {
    Ok(stream.try_borrow()?.disturbed)
}

fn handled_deferred<'js>(ctx: &Ctx<'js>) -> Result<Deferred<'js>> {
    let deferred = Deferred::new(ctx)?;
    let ignore = Function::new(ctx.clone(), || {})?;
    promise_then(ctx, &deferred.promise, None, Some(&ignore))?;
    Ok(deferred)
}

/// Attach an internal reaction without consulting user-overridable Promise properties.
pub(super) fn promise_then<'js>(
    ctx: &Ctx<'js>,
    promise: &Promise<'js>,
    resolve: Option<&Function<'js>>,
    reject: Option<&Function<'js>>,
) -> Result<Promise<'js>> {
    let undefined = Value::new_undefined(ctx.clone());
    let operand = |callback: Option<&Function<'js>>| {
        callback.map_or_else(|| undefined.as_raw(), |callback| callback.as_raw())
    };
    // SAFETY: the promise, callback values and undefined live in the current context.
    // JS_PromiseThen borrows the operands and returns a new owned reference.
    let reaction = unsafe {
        let raw = rquickjs::qjs::JS_PromiseThen(
            ctx.as_raw().as_ptr(),
            promise.as_raw(),
            operand(resolve),
            operand(reject),
        );
        Value::from_raw(ctx.clone(), raw)
    };
    if reaction.is_exception() {
        return Err(Error::Exception);
    }
    reaction.into_promise().ok_or(Error::Unknown)
}

pub(super) fn lock<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
) -> Result<Class<'js, ReadableStreamDefaultReader<'js>>> {
    if stream.try_borrow()?.locked() {
        return Err(Exception::throw_type(ctx, "Stream is locked"));
    }
    let closed = handled_deferred(ctx)?;
    let reader = Class::instance(
        ctx.clone(),
        ReadableStreamDefaultReader {
            stream: Some(stream.clone()),
            closed: closed.clone(),
        },
    )?;
    let terminal = {
        let mut state = stream.try_borrow_mut()?;
        state.lock = Some(Lock::Reader(reader.clone()));
        match &state.phase {
            Phase::Closed => Some(Ok(())),
            Phase::Errored(reason) => Some(Err(reason.clone())),
            _ => None,
        }
    };
    if let Some(result) = terminal {
        settle_closed(&closed, result)?;
    }
    Ok(reader)
}

fn settle_closed<'js>(
    deferred: &Deferred<'js>,
    outcome: std::result::Result<(), Value<'js>>,
) -> Result<()> {
    match outcome {
        Ok(()) => deferred.resolve(()),
        Err(reason) => deferred.reject(reason),
    }
}

fn release_reader<'js>(
    ctx: &Ctx<'js>,
    reader: &Class<'js, ReadableStreamDefaultReader<'js>>,
) -> Result<()> {
    let Some(stream) = reader.try_borrow()?.stream.clone() else {
        return Ok(());
    };
    let terminated = !matches!(stream.try_borrow()?.phase, Phase::Readable | Phase::Ending);
    let replacement = if terminated {
        Some(handled_deferred(ctx)?)
    } else {
        None
    };
    let reason = exception_value(ctx, Exception::throw_type(ctx, "Reader lock was released"))?;
    let pending = {
        let mut state = stream.try_borrow_mut()?;
        state.lock = None;
        std::mem::take(&mut state.pending)
    };
    let closed = {
        let mut state = reader.try_borrow_mut()?;
        state.stream = None;
        if let Some(closed) = replacement {
            state.closed = closed;
        }
        state.closed.clone()
    };
    closed.reject(reason.clone())?;
    for promise in pending {
        promise.reject(reason.clone())?;
    }
    Ok(())
}

fn iteration<'js>(ctx: &Ctx<'js>, bytes: Option<Value<'js>>) -> Result<Object<'js>> {
    let result = Object::new(ctx.clone())?;
    let done = bytes.is_none();
    result.prop(
        "value",
        Property::from(bytes).writable().enumerable().configurable(),
    )?;
    result.prop(
        "done",
        Property::from(done).writable().enumerable().configurable(),
    )?;
    Ok(result)
}

fn read<'js>(ctx: &Ctx<'js>, stream: &Class<'js, ReadableStream<'js>>) -> Result<Promise<'js>> {
    let mut deferred = Deferred::new(ctx)?;
    let (phase, chunk, closing) = {
        let mut state = stream.try_borrow_mut()?;
        state.disturbed = true;
        if let Phase::Errored(error) = &state.phase {
            (Some(Err(error.clone())), None, None)
        } else if let Some(chunk) = state.queue.pop_front() {
            state.queued_bytes -= chunk.len;
            if let Some(source) = state.source.as_mut() {
                source.queued = (source.queued - chunk.weight).max(0.0);
            }
            let closing = if state.queue.is_empty() && matches!(state.phase, Phase::Ending) {
                Some(state.close()?)
            } else {
                None
            };
            (None, Some(chunk), closing)
        } else if matches!(state.phase, Phase::Closed) {
            (Some(Ok(())), None, None)
        } else if state.pending.len() >= PENDING_READ_LIMIT {
            drop(state);
            return settlement::<()>(
                ctx,
                Err(Exception::throw_range(ctx, "Too many pending reads")),
            );
        } else {
            drop(state);
            deferred.retain(stream)?;
            stream.try_borrow_mut()?.pending.push_back(deferred.clone());
            source::pull_if_needed(ctx, stream)?;
            return Ok(deferred.promise);
        }
    };
    if let Some(closing) = closing {
        closing.settle(ctx, Ok(()))?;
    }
    if let Some(chunk) = chunk {
        deferred.resolve(iteration(ctx, Some(chunk.release()?))?)?;
    } else if let Some(Err(reason)) = phase {
        deferred.reject(reason)?;
    } else {
        deferred.resolve(iteration(ctx, None)?)?;
    }
    source::pull_if_needed(ctx, stream)?;
    Ok(deferred.promise)
}

struct ReaderWork<'js> {
    closed: Option<Deferred<'js>>,
    pending: VecDeque<Deferred<'js>>,
}

impl<'js> ReaderWork<'js> {
    fn settle(self, ctx: &Ctx<'js>, outcome: std::result::Result<(), Value<'js>>) -> Result<()> {
        if let Some(closed) = self.closed {
            settle_closed(&closed, outcome.clone())?;
        }
        for promise in self.pending {
            match &outcome {
                Ok(()) => promise.resolve(iteration(ctx, None)?)?,
                Err(reason) => promise.reject(reason.clone())?,
            }
        }
        Ok(())
    }
}

impl<'js> ReadableStream<'js> {
    fn readers(&mut self) -> Result<ReaderWork<'js>> {
        let closed = match &self.lock {
            Some(Lock::Reader(reader)) => Some(reader.try_borrow()?.closed.clone()),
            _ => None,
        };
        Ok(ReaderWork {
            closed,
            pending: std::mem::take(&mut self.pending),
        })
    }

    fn close(&mut self) -> Result<ReaderWork<'js>> {
        self.phase = Phase::Closed;
        self.readers()
    }

    fn discard(&mut self) -> VecDeque<Chunk<'js>> {
        self.queued_bytes = 0;
        std::mem::take(&mut self.queue)
    }
}

fn release_chunks(chunks: VecDeque<Chunk<'_>>) -> Result<()> {
    for chunk in chunks {
        chunk.release()?;
    }
    Ok(())
}

pub(super) fn finish<'js>(ctx: &Ctx<'js>, stream: &Class<'js, ReadableStream<'js>>) -> Result<()> {
    let closing = {
        let mut state = stream.try_borrow_mut()?;
        if !matches!(state.phase, Phase::Readable) {
            return Ok(());
        }
        state.phase = Phase::Ending;
        state.cancel = None;
        state.source = None;
        if state.queue.is_empty() {
            Some(state.close()?)
        } else {
            None
        }
    };
    if let Some(closing) = closing {
        closing.settle(ctx, Ok(()))?;
    }
    run_cleanup(stream)?;
    schedule_body(ctx, stream)
}

pub(super) fn error<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    reason: Value<'js>,
) -> Result<()> {
    let (work, chunks) = {
        let mut state = stream.try_borrow_mut()?;
        if matches!(state.phase, Phase::Closed | Phase::Errored(_)) {
            return Ok(());
        }
        state.phase = Phase::Errored(reason.clone());
        state.cancel = None;
        state.source = None;
        (state.readers()?, state.discard())
    };
    release_chunks(chunks)?;
    work.settle(ctx, Err(reason))?;
    run_cleanup(stream)?;
    schedule_body(ctx, stream)
}

fn cancel<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    reason: Value<'js>,
) -> Result<Promise<'js>> {
    let (work, chunks, cancel, source) = {
        let mut state = stream.try_borrow_mut()?;
        state.disturbed = true;
        match &state.phase {
            Phase::Closed => return settlement(ctx, Ok(())),
            Phase::Errored(error) => return Err(ctx.throw(error.clone())),
            _ => {}
        }
        (
            state.close()?,
            state.discard(),
            state.cancel.take(),
            state.source.take(),
        )
    };
    release_chunks(chunks)?;
    work.settle(ctx, Ok(()))?;
    run_cleanup(stream)?;
    if let Some(cancel) = cancel {
        cancel.call::<_, ()>((reason.clone(),))?;
    }
    source::cancel(ctx, source, reason)
}

pub(super) fn push<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    bytes: TypedArray<'js, u8>,
    release: Option<Function<'js>>,
) -> Result<bool> {
    let len = bytes
        .as_raw()
        .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))?
        .len();
    if len == 0 {
        if let Some(release) = release {
            release.call::<_, ()>(())?;
        }
        return Ok(true);
    }
    push_value(ctx, stream, bytes.into_value(), len, release)
}

pub(super) fn push_value<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    value: Value<'js>,
    len: usize,
    release: Option<Function<'js>>,
) -> Result<bool> {
    push_chunk(ctx, stream, value, len, 0.0, release)
}

fn push_chunk<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    value: Value<'js>,
    len: usize,
    weight: f64,
    release: Option<Function<'js>>,
) -> Result<bool> {
    let chunk = Chunk {
        value,
        release,
        len,
        weight,
    };
    let (accepted, pending) = {
        let mut state = stream.try_borrow_mut()?;
        if !matches!(state.phase, Phase::Readable) {
            (true, None)
        } else if let Some(read) = state.pending.pop_front() {
            (true, Some(read))
        } else if len > STREAM_BYTE_LIMIT.saturating_sub(state.queued_bytes)
            || state.queue.len() >= STREAM_CHUNK_LIMIT
        {
            (false, None)
        } else {
            state.queued_bytes += len;
            if let Some(source) = state.source.as_mut() {
                source.queued += weight;
            }
            state.queue.push_back(chunk);
            drop(state);
            schedule_body(ctx, stream)?;
            return Ok(true);
        }
    };
    let bytes = chunk.release()?;
    if let Some(pending) = pending {
        pending.resolve(iteration(ctx, Some(bytes))?)?;
    }
    Ok(accepted)
}

pub(super) fn take_bytes<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
) -> Result<TypedArray<'js, u8>> {
    if unavailable(stream)? {
        return Err(Exception::throw_type(ctx, "Body is locked or already used"));
    }
    let mut chunks = {
        let mut state = stream.try_borrow_mut()?;
        state.disturbed = true;
        state.phase = Phase::Closed;
        state.cancel = None;
        state.discard()
    };
    if chunks.len() == 1
        && let Some(chunk) = chunks.pop_front()
    {
        let bytes = chunk.bytes(ctx)?;
        bytes.arraybuffer()?;
        chunk.release()?;
        return Ok(bytes);
    }
    let mut bytes = Vec::new();
    for chunk in chunks {
        append_bytes(ctx, &mut bytes, &chunk)?;
        chunk.release()?;
    }
    TypedArray::new_copy(ctx.clone(), bytes)
}

pub(super) fn consume<'js>(
    ctx: &Ctx<'js>,
    stream: Option<&Class<'js, ReadableStream<'js>>>,
    output: BodyOutput,
) -> Result<Promise<'js>> {
    let Some(stream) = stream else {
        return settlement(ctx, materialized_value(ctx, output, Vec::new()));
    };
    if unavailable(stream)? {
        return settlement::<()>(
            ctx,
            Err(Exception::throw_type(ctx, "Body is locked or already used")),
        );
    }
    let reply = Deferred::new(ctx)?;
    let charge = Charge::new(ctx)?;
    {
        let mut state = stream.try_borrow_mut()?;
        state.disturbed = true;
        state.lock = Some(Lock::Body(Materializing {
            reply: reply.clone(),
            bytes: Vec::new(),
            charge,
            scheduled: false,
            output,
        }));
    }
    if let Err(error) = schedule_body(ctx, stream) {
        stream.try_borrow_mut()?.lock = None;
        reply.reject(exception_value(ctx, error)?)?;
    }
    Ok(reply.promise)
}

fn schedule_body<'js>(ctx: &Ctx<'js>, stream: &Class<'js, ReadableStream<'js>>) -> Result<()> {
    {
        let mut state = stream.try_borrow_mut()?;
        let Some(Lock::Body(body)) = &mut state.lock else {
            return Ok(());
        };
        if body.scheduled {
            return Ok(());
        }
        body.scheduled = true;
    }
    if let Err(error) =
        Function::new(ctx.clone(), drain_body).and_then(|job| job.defer((stream.clone(),)))
    {
        let mut state = stream.try_borrow_mut()?;
        if let Some(Lock::Body(body)) = &mut state.lock {
            body.scheduled = false;
        }
        return Err(error);
    }
    Ok(())
}

fn drain_body<'js>(ctx: Ctx<'js>, stream: Class<'js, ReadableStream<'js>>) -> Result<()> {
    let (chunks, outcome, completed, cancel) = {
        let mut state = stream.try_borrow_mut()?;
        if !matches!(state.lock, Some(Lock::Body(_))) {
            return Ok(());
        }
        let chunks = state.discard();
        let Some(Lock::Body(mut body)) = state.lock.take() else {
            return Ok(());
        };
        body.scheduled = false;

        let outcome = if let Phase::Errored(reason) = &state.phase {
            BodyResult::Error(reason.clone())
        } else if let Err(error) = append_chunks(&ctx, &chunks, &mut body) {
            BodyResult::Invalid(error)
        } else if matches!(state.phase, Phase::Readable) {
            BodyResult::Pending
        } else {
            BodyResult::Done
        };
        let cancel = if matches!(outcome, BodyResult::Invalid(_)) {
            state.phase = Phase::Closed;
            state.cancel.take()
        } else {
            None
        };
        let completed = if matches!(outcome, BodyResult::Pending) {
            state.lock = Some(Lock::Body(body));
            None
        } else {
            if matches!(outcome, BodyResult::Done) {
                state.phase = Phase::Closed;
            }
            Some(body)
        };
        (chunks, outcome, completed, cancel)
    };
    release_chunks(chunks)?;
    let Some(body) = completed else { return Ok(()) };
    match outcome {
        BodyResult::Done => match materialized_value(&ctx, body.output, body.bytes) {
            Ok(value) => body.reply.resolve(value),
            Err(error) => body.reply.reject(exception_value(&ctx, error)?),
        },
        BodyResult::Error(reason) => body.reply.reject(reason),
        BodyResult::Invalid(error) => {
            let reason = exception_value(&ctx, error)?;
            if let Some(cancel) = cancel
                && let Err(error) = cancel.call::<_, ()>((reason.clone(),))
            {
                return body.reply.reject(exception_value(&ctx, error)?);
            }
            body.reply.reject(reason)
        }
        BodyResult::Pending => Ok(()),
    }
}

enum BodyResult<'js> {
    Pending,
    Done,
    Error(Value<'js>),
    Invalid(Error),
}

fn append_chunks<'js>(
    ctx: &Ctx<'js>,
    chunks: &VecDeque<Chunk<'js>>,
    body: &mut Materializing<'js>,
) -> Result<()> {
    let factor = match body.output {
        BodyOutput::Buffer => 2,
        BodyOutput::Text | BodyOutput::Json => 6,
    };
    for chunk in chunks {
        let size = body.bytes.len().saturating_add(chunk.len);
        if size > STREAM_BYTE_LIMIT || size.saturating_mul(factor) > MATERIALIZED_BYTE_LIMIT {
            return Err(Exception::throw_range(
                ctx,
                "Body exceeds materialization limit",
            ));
        }
        if size > body.bytes.capacity() {
            let capacity = size
                .max(body.bytes.capacity().saturating_mul(2))
                .min(STREAM_BYTE_LIMIT);
            body.charge.resize(ctx, capacity)?;
            body.bytes
                .try_reserve_exact(capacity - body.bytes.len())
                .map_err(|_| Exception::throw_range(ctx, "Body allocation failed"))?;
        }
        append_bytes(ctx, &mut body.bytes, chunk)?;
    }
    Ok(())
}

fn append_bytes<'js>(ctx: &Ctx<'js>, dest: &mut Vec<u8>, chunk: &Chunk<'js>) -> Result<()> {
    let bytes = chunk.bytes(ctx)?;
    bytes.arraybuffer()?;
    // SAFETY: the backing buffer was checked above. No JS runs before the copy.
    let bytes = unsafe { bytes.as_bytes() }
        .filter(|bytes| bytes.len() == chunk.len)
        .ok_or_else(|| Exception::throw_type(ctx, "Body buffer detached or resized"))?;
    dest.extend_from_slice(bytes);
    Ok(())
}

fn materialized_value<'js>(
    ctx: &Ctx<'js>,
    output: BodyOutput,
    bytes: Vec<u8>,
) -> Result<Value<'js>> {
    if matches!(output, BodyOutput::Buffer) {
        return ArrayBuffer::new_copy(ctx.clone(), bytes).map(ArrayBuffer::into_value);
    }
    let text = decode_utf8(&bytes);
    match output {
        BodyOutput::Json => {
            // JS_ParseJSON takes UTF-8 bytes without a leading BOM. A raw NUL
            // must produce a SyntaxError before C-string conversion.
            if text.contains('\0') {
                return Err(Exception::throw_syntax(ctx, "Invalid JSON"));
            }
            ctx.json_parse(text.into_bytes())
        }
        _ => text.into_js(ctx),
    }
}

fn settlement<'js, T: IntoJs<'js>>(ctx: &Ctx<'js>, result: Result<T>) -> Result<Promise<'js>> {
    let deferred = Deferred::new(ctx)?;
    match result {
        Ok(value) => deferred.resolve(value)?,
        Err(error) => deferred.reject(exception_value(ctx, error)?)?,
    }
    Ok(deferred.promise)
}

fn exception_value<'js>(ctx: &Ctx<'js>, error: Error) -> Result<Value<'js>> {
    if error.is_exception() {
        Ok(ctx.catch())
    } else {
        Exception::from_message(ctx.clone(), &error.to_string()).map(Exception::into_value)
    }
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    // The pending read promise keeps its stream alive even when the caller
    // discards the reader after calling read. The weak key permits GC of both.
    let holder = read_anchors(ctx)?;
    let weak: Constructor = globals.get("WeakMap")?;
    let map: Object = weak.construct(())?;
    holder.set("set", map.get::<_, Function>("set")?)?;
    holder.set("delete", map.get::<_, Function>("delete")?)?;
    holder.set("map", map)?;
    globals.set(
        "ReadableStream",
        constructor::<ReadableStream<'js>, _, _>(ctx, source::construct)?,
    )?;
    globals.set(
        "ReadableStreamDefaultController",
        constructor::<source::Controller<'js>, _, _>(ctx, |ctx: Ctx<'js>| {
            Err::<Class<'js, source::Controller<'js>>, _>(Exception::throw_type(
                &ctx,
                "Illegal constructor",
            ))
        })?,
    )?;
    globals.set(
        "ReadableStreamDefaultReader",
        constructor::<ReadableStreamDefaultReader<'js>, _, _>(
            ctx,
            |ctx: Ctx<'js>, stream: Class<'js, ReadableStream<'js>>| lock(&ctx, &stream),
        )?,
    )
}

pub(super) const DECLARATIONS: &str = r#"
type ReadableStreamReadResult<T> = { done: false; value: T } | { done: true; value: undefined };

interface UnderlyingSource<T> {
  start?(controller: ReadableStreamDefaultController<T>): void | PromiseLike<void>;
  pull?(controller: ReadableStreamDefaultController<T>): void | PromiseLike<void>;
  cancel?(reason?: unknown): void | PromiseLike<void>;
}
interface QueuingStrategy<T> {
  highWaterMark?: number;
  size?(chunk: T): number;
}
declare class ReadableStreamDefaultController<T = unknown> {
  private constructor();
  readonly desiredSize: number | null;
  enqueue(chunk?: T): void;
  close(): void;
  error(reason?: unknown): void;
}
declare class ReadableStream<T = unknown> {
  constructor(underlyingSource?: UnderlyingSource<T>, strategy?: QueuingStrategy<T>);
  readonly locked: boolean;
  getReader(): ReadableStreamDefaultReader<T>;
  cancel(reason?: unknown): Promise<void>;
}

declare class ReadableStreamDefaultReader<T = Uint8Array> {
  constructor(stream: ReadableStream<T>);
  readonly closed: Promise<void>;
  read(): Promise<ReadableStreamReadResult<T>>;
  cancel(reason?: unknown): Promise<void>;
  releaseLock(): void;
}
"#;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use rquickjs::{Context, Runtime};

    use super::*;

    struct DropProbe(Rc<Cell<u8>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn unreachable_reader_cycle_releases_queued_credit() {
        let vm = Runtime::new().unwrap();
        let context = Context::full(&vm).unwrap();
        let dropped = Rc::new(Cell::new(0));
        context.with(|ctx| {
            let stream = create(&ctx, None).unwrap();
            let callback = Function::new(ctx.clone(), {
                let probe = DropProbe(Rc::clone(&dropped));
                move || assert_eq!(probe.0.get(), 0)
            })
            .unwrap();
            let bytes = TypedArray::new_copy(ctx.clone(), [42_u8]).unwrap();
            assert!(push(&ctx, &stream, bytes, Some(callback)).unwrap());
            let _reader = lock(&ctx, &stream).unwrap();
            assert_eq!(dropped.get(), 0);
        });
        vm.run_gc();
        assert_eq!(dropped.get(), 1);
    }
}
