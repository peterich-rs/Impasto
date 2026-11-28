//! Readable streams on the QuickJS owner thread.

mod source;

use std::collections::VecDeque;

use rquickjs::class::Trace;
use rquickjs::function::{Constructor, Opt, This};
use rquickjs::object::Property;
use rquickjs::{
    Class, Ctx, Error, Exception, Function, IntoJs, JsLifetime, Object, Promise, Result, Value,
};

use super::web::constructor;

const PENDING_READ_LIMIT: usize = 32;
const STREAM_CHUNK_LIMIT: usize = 1024;
const STREAM_BYTE_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Trace, JsLifetime)]
struct Deferred<'js> {
    promise: Promise<'js>,
    resolve: Function<'js>,
    reject: Function<'js>,
    anchor: Option<ReadAnchor>,
}

impl<'js> Deferred<'js> {
    fn new(ctx: &Ctx<'js>) -> Result<Self> {
        let (promise, resolve, reject) = ctx.promise()?;
        Ok(Self {
            promise,
            resolve,
            reject,
            anchor: None,
        })
    }

    fn retain(
        &mut self,
        stream: &Class<'js, ReadableStream<'js>>,
        anchor: ReadAnchor,
    ) -> Result<()> {
        let holder = anchor.holder(self.promise.ctx())?;
        let map: Object = holder.get("map")?;
        holder.get::<_, Function>("set")?.call::<_, ()>((
            This(map),
            self.promise.clone(),
            stream.clone(),
        ))?;
        self.anchor = Some(anchor);
        Ok(())
    }

    fn release(&self) -> Result<()> {
        if let Some(anchor) = self.anchor {
            let holder = anchor.holder(self.promise.ctx())?;
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

#[derive(Clone, Copy, Trace, JsLifetime)]
enum ReadAnchor {
    Script,
    Native,
}

impl ReadAnchor {
    fn key(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Native => "native",
        }
    }

    fn holder<'js>(self, ctx: &Ctx<'js>) -> Result<Object<'js>> {
        read_anchors(ctx)?.get(self.key())
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

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ReadableStream<'js> {
    phase: Phase<'js>,
    lock: Option<Class<'js, ReadableStreamDefaultReader<'js>>>,
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
        state.lock = Some(reader.clone());
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
            let anchor = if state.cancel.is_some() {
                ReadAnchor::Native
            } else {
                ReadAnchor::Script
            };
            drop(state);
            deferred.retain(stream, anchor)?;
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
            Some(reader) => Some(reader.try_borrow()?.closed.clone()),
            None => None,
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
    run_cleanup(stream)
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
    run_cleanup(stream)
}

fn cancel<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, ReadableStream<'js>>,
    reason: Value<'js>,
) -> Result<Promise<'js>> {
    let (work, chunks, cancel, source) = {
        let mut state = stream.try_borrow_mut()?;
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
            return Ok(true);
        }
    };
    let bytes = chunk.release()?;
    if let Some(pending) = pending {
        pending.resolve(iteration(ctx, Some(bytes))?)?;
    }
    Ok(accepted)
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
    let anchors = read_anchors(ctx)?;
    // A script read keeps its stream alive while the read promise is reachable.
    // Native I/O can complete an awaited read even when QuickJS no longer roots
    // the promise itself. Retain those reads until they settle or are canceled.
    for (kind, constructor) in [(ReadAnchor::Script, "WeakMap"), (ReadAnchor::Native, "Map")] {
        let holder = Object::new(ctx.clone())?;
        let map: Object = globals.get::<_, Constructor>(constructor)?.construct(())?;
        holder.set("set", map.get::<_, Function>("set")?)?;
        holder.set("delete", map.get::<_, Function>("delete")?)?;
        holder.set("map", map)?;
        anchors.set(kind.key(), holder)?;
    }
    globals.set(
        "ReadableStream",
        constructor::<ReadableStream<'js>, _, _>(ctx, source::construct)?,
    )?;
    ctx.eval::<(), _>(include_str!("streams/tee.js"))?;
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
  tee(): [ReadableStream<T>, ReadableStream<T>];
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
            let bytes = rquickjs::TypedArray::new_copy(ctx.clone(), [42_u8]).unwrap();
            assert!(push_value(&ctx, &stream, bytes.into_value(), 1, Some(callback)).unwrap());
            let _reader = lock(&ctx, &stream).unwrap();
            assert_eq!(dropped.get(), 0);
        });
        vm.run_gc();
        assert_eq!(dropped.get(), 1);
    }
}
