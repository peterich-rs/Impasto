//! Fetch objects and promise settlement on the QuickJS owner thread.

use std::collections::HashMap;

use rquickjs::class::Trace;
use rquickjs::function::{Opt, This};
use rquickjs::{
    ArrayBuffer, Class, Coerced, Ctx, Error, Exception, FromJs, Function, JsLifetime, Object,
    Promise, Result, TypedArray, Value,
};

use crate::quickjs::buffer::{is_source as is_buffer_source, view as buffer_bytes};

use super::encoding::usv_string;
use super::events::{abort_reason, named_error, new_signal, on_abort};
use super::headers::{Headers, is_token};
use super::streams::{self, BodyOutput, ReadableStream, ReadableStreamDefaultReader};
use super::url::{form_query, parse};
use super::web::{Charge, bridge, constructor, is_nullish, option, platform};

type Stream<'js> = Class<'js, ReadableStream<'js>>;
type Reader<'js> = Class<'js, ReadableStreamDefaultReader<'js>>;

const BODY_LIMIT: usize = js_host::HTTP_BODY_LIMIT;

#[derive(Trace, JsLifetime)]
struct Body<'js> {
    stream: Stream<'js>,
    #[qjs(skip_trace)]
    media_type: Option<&'static str>,
}

fn body_from_bytes<'js>(
    ctx: &Ctx<'js>,
    bytes: TypedArray<'js, u8>,
    media_type: Option<&'static str>,
) -> Result<Body<'js>> {
    if bytes.len() > BODY_LIMIT {
        return Err(Exception::throw_range(ctx, "Body exceeds byte limit"));
    }
    let stream = streams::create(ctx, None)?;
    if !streams::push(ctx, &stream, bytes, None)? {
        return Err(Exception::throw_range(ctx, "Body exceeds byte limit"));
    }
    streams::finish(ctx, &stream)?;
    Ok(Body { stream, media_type })
}

fn static_body<'js>(ctx: &Ctx<'js>, input: Value<'js>) -> Result<Option<Body<'js>>> {
    if is_nullish(&input) {
        return Ok(None);
    }
    let (bytes, media_type) = if let Some(form) = form_query(&input)? {
        (
            TypedArray::new_copy(ctx.clone(), form.as_bytes())?,
            Some("application/x-www-form-urlencoded;charset=UTF-8"),
        )
    } else if is_buffer_source(&input) {
        let view = buffer_bytes(ctx, input)?;
        if view.len() > BODY_LIMIT {
            return Err(Exception::throw_range(ctx, "Body exceeds byte limit"));
        }
        // SAFETY: buffer_bytes rejects shared backing stores. No user code runs while
        // the view is copied into a new fixed-length QuickJS allocation.
        let bytes = unsafe { view.as_bytes() }
            .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))?;
        (TypedArray::new_copy(ctx.clone(), bytes)?, None)
    } else {
        let text = usv_string(ctx, input)?;
        if text.len() > BODY_LIMIT {
            return Err(Exception::throw_range(ctx, "Body exceeds byte limit"));
        }
        (
            TypedArray::new_copy(ctx.clone(), text.as_bytes())?,
            Some("text/plain;charset=UTF-8"),
        )
    };
    body_from_bytes(ctx, bytes, media_type).map(Some)
}

fn fill_content_type(ctx: &Ctx<'_>, headers: &mut Headers, body: Option<&Body<'_>>) -> Result<()> {
    if let Some(media_type) = body.and_then(|body| body.media_type)
        && !headers.contains("content-type")
    {
        headers.set_valid(ctx, "content-type".into(), media_type.into())?;
    }
    Ok(())
}

fn get_body<'js>(ctx: &Ctx<'js>, body: Option<&Body<'js>>) -> Value<'js> {
    body.map_or_else(
        || Value::new_null(ctx.clone()),
        |body| body.stream.clone().into_value(),
    )
}

fn request_method<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    let Coerced(method) = Coerced::<String>::from_js(ctx, value)?;
    if !is_token(&method) {
        return Err(Exception::throw_type(ctx, "Invalid HTTP method"));
    }
    let normalized = method.to_ascii_uppercase();
    if matches!(normalized.as_str(), "CONNECT" | "TRACE" | "TRACK") {
        return Err(Exception::throw_type(ctx, "Forbidden HTTP method"));
    }
    Ok(
        if !platform(ctx)?.is_portable()
            || matches!(
                normalized.as_str(),
                "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
            )
        {
            normalized
        } else {
            method
        },
    )
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct Request<'js> {
    url: String,
    method: String,
    headers: Class<'js, Headers>,
    signal: Object<'js>,
    body: Option<Body<'js>>,
    #[qjs(skip_trace)]
    _charge: Charge,
}

impl<'js> Request<'js> {
    fn from_input(ctx: &Ctx<'js>, input: Value<'js>, init: Value<'js>) -> Result<Self> {
        let source = Class::<Request>::from_value(&input).ok();
        let url = parse(
            ctx,
            if let Some(source) = &source {
                rquickjs::String::from_str(ctx.clone(), &source.try_borrow()?.url)?.into_value()
            } else {
                input
            },
            None,
        )?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Exception::throw_type(ctx, "Credentials in request URL"));
        }
        let method = option(ctx, &init, "method")?;
        let method = if method.is_undefined() {
            source
                .as_ref()
                .map(|source| source.try_borrow().map(|value| value.method.clone()))
                .transpose()?
                .unwrap_or_else(|| "GET".into())
        } else {
            request_method(ctx, method)?
        };
        let portable = platform(ctx)?.is_portable();
        let header_init = option(ctx, &init, "headers")?;
        let source_headers = source
            .as_ref()
            .map(|source| source.try_borrow().map(|value| value.headers.clone()))
            .transpose()?;
        let mut headers = if let Some(source_headers) = source_headers {
            if header_init.is_undefined() {
                Headers::from_value(ctx, source_headers.into_value(), false)?
            } else if portable {
                Headers::from_value(ctx, header_init, false)?
            } else {
                let supplied = Headers::from_value(ctx, header_init, false)?;
                let mut merged = Headers::from_value(ctx, source_headers.into_value(), false)?;
                for (name, value) in supplied.merged() {
                    merged.set_valid(ctx, name.to_owned(), value)?;
                }
                merged
            }
        } else {
            Headers::from_value(ctx, header_init, false)?
        };
        let signal = option(ctx, &init, "signal")?;
        let signal = if signal.is_undefined() {
            if let Some(source) = &source {
                source.try_borrow()?.signal.clone()
            } else {
                new_signal(ctx)?
            }
        } else if signal.is_null() {
            new_signal(ctx)?
        } else {
            let signal = signal
                .into_object()
                .ok_or_else(|| Exception::throw_type(ctx, "Expected an AbortSignal"))?;
            abort_reason(&signal)?;
            signal
        };
        let body_input = option(ctx, &init, "body")?;
        let inherited = if is_nullish(&body_input) {
            source
                .as_ref()
                .map(|source| {
                    source.try_borrow().map(|source| {
                        source
                            .body
                            .as_ref()
                            .map(|body| (body.stream.clone(), body.media_type))
                    })
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        if portable
            && (!is_nullish(&body_input) || inherited.is_some())
            && matches!(method.as_str(), "GET" | "HEAD")
        {
            return Err(Exception::throw_type(
                ctx,
                "Request method cannot have a body",
            ));
        }
        let body = if let Some((stream, media_type)) = inherited {
            let bytes = streams::take_bytes(ctx, &stream)?;
            Some(body_from_bytes(ctx, bytes, media_type)?)
        } else {
            static_body(ctx, body_input)?
        };
        fill_content_type(ctx, &mut headers, body.as_ref())?;
        let url: String = url.into();
        let mut charge = Charge::new(ctx)?;
        charge.resize(ctx, url.len().saturating_add(method.len()))?;
        Ok(Self {
            url,
            method,
            headers: Class::instance(ctx.clone(), headers)?,
            signal,
            body,
            _charge: charge,
        })
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Request<'js> {
    #[qjs(get)]
    fn url(&self) -> String {
        self.url.clone()
    }
    #[qjs(get)]
    fn method(&self) -> String {
        self.method.clone()
    }
    #[qjs(get)]
    fn headers(&self) -> Class<'js, Headers> {
        self.headers.clone()
    }
    #[qjs(get)]
    fn signal(&self) -> Object<'js> {
        self.signal.clone()
    }
    #[qjs(get)]
    fn body(&self, ctx: Ctx<'js>) -> Value<'js> {
        get_body(&ctx, self.body.as_ref())
    }
    #[qjs(get)]
    fn body_used(&self) -> Result<bool> {
        self.body
            .as_ref()
            .map_or(Ok(false), |body| streams::disturbed(&body.stream))
    }

    fn array_buffer(This(request): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = request
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Buffer)
    }
    fn text(This(request): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = request
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Text)
    }
    fn json(This(request): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = request
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Json)
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct Response<'js> {
    url: String,
    status: u16,
    status_text: String,
    headers: Class<'js, Headers>,
    body: Option<Body<'js>>,
    legacy: Option<LegacyReader<'js>>,
    #[qjs(skip_trace)]
    _charge: Charge,
}

#[derive(Trace, JsLifetime)]
struct LegacyReader<'js> {
    stream: Stream<'js>,
    reader: Option<Reader<'js>>,
}

impl<'js> Response<'js> {
    fn from_input(ctx: &Ctx<'js>, input: Value<'js>, init: Value<'js>) -> Result<Self> {
        let status = option(ctx, &init, "status")?;
        let status = if is_nullish(&status) {
            200.0
        } else {
            Coerced::<f64>::from_js(ctx, status)?.0
        };
        if !(200.0..=599.0).contains(&status) || status.fract() != 0.0 {
            return Err(Exception::throw_range(ctx, "Invalid response status"));
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the status is an integer in 200..=599"
        )]
        let status = status as u16;
        let status_text = option(ctx, &init, "statusText")?;
        let status_text = if is_nullish(&status_text) {
            String::new()
        } else {
            Coerced::<String>::from_js(ctx, status_text)?.0
        };
        if status_text.contains(['\n', '\r', '\0']) {
            return Err(Exception::throw_type(ctx, "Invalid response status text"));
        }
        let mut headers = Headers::from_value(ctx, option(ctx, &init, "headers")?, false)?;
        let body = static_body(ctx, input)?;
        if body.is_some() && no_body("", status) {
            return Err(Exception::throw_type(
                ctx,
                "Response status cannot have a body",
            ));
        }
        fill_content_type(ctx, &mut headers, body.as_ref())?;
        let mut charge = Charge::new(ctx)?;
        charge.resize(ctx, status_text.len())?;
        Ok(Self {
            url: String::new(),
            status,
            status_text,
            headers: Class::instance(ctx.clone(), headers)?,
            body,
            legacy: None,
            _charge: charge,
        })
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Response<'js> {
    #[qjs(get)]
    fn url(&self) -> String {
        self.url.clone()
    }
    #[qjs(get)]
    fn status(&self) -> u16 {
        self.status
    }
    #[qjs(get)]
    fn status_text(&self) -> String {
        self.status_text.clone()
    }
    #[qjs(get)]
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    #[qjs(get)]
    fn headers(&self) -> Class<'js, Headers> {
        self.headers.clone()
    }
    #[qjs(get)]
    fn body(&self, ctx: Ctx<'js>) -> Value<'js> {
        get_body(&ctx, self.body.as_ref())
    }
    #[qjs(get)]
    fn body_used(&self) -> Result<bool> {
        self.body
            .as_ref()
            .map_or(Ok(false), |body| streams::disturbed(&body.stream))
    }

    fn array_buffer(This(response): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = response
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Buffer)
    }
    fn text(This(response): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = response
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Text)
    }
    fn json(This(response): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        let stream = response
            .try_borrow()?
            .body
            .as_ref()
            .map(|body| body.stream.clone());
        streams::consume(&ctx, stream.as_ref(), BodyOutput::Json)
    }
}

fn legacy_reader<'js>(
    This(response): This<Class<'js, Response<'js>>>,
    ctx: Ctx<'js>,
) -> Result<Reader<'js>> {
    let (stream, prior) = {
        let response = response.try_borrow()?;
        let reader = response
            .legacy
            .as_ref()
            .ok_or_else(|| Exception::throw_type(&ctx, "Expected a stream response"))?;
        (reader.stream.clone(), reader.reader.clone())
    };
    if let Some(prior) = prior
        && streams::reader(&stream)?.as_ref() == Some(&prior)
    {
        return Ok(prior);
    }
    let reader = streams::lock(&ctx, &stream)?;
    if let Some(legacy) = response.try_borrow_mut()?.legacy.as_mut() {
        legacy.reader = Some(reader.clone());
    }
    Ok(reader)
}

fn no_body(method: &str, status: u16) -> bool {
    method == "HEAD" || matches!(status, 101 | 103 | 204 | 205 | 304)
}

#[derive(Trace, JsLifetime)]
struct Reply<'js> {
    resolve: Function<'js>,
    reject: Function<'js>,
}

#[derive(Trace, JsLifetime)]
struct StreamRequest<'js> {
    stream: Stream<'js>,
    reply: Option<Reply<'js>>,
    detach: Option<Function<'js>>,
    external: bool,
    no_body: bool,
    received: usize,
    url: String,
    method: String,
}

#[derive(Trace, JsLifetime)]
struct WholeRequest<'js> {
    detach: Option<Function<'js>>,
    url: String,
    method: String,
}

#[derive(Default, Trace, JsLifetime)]
#[rquickjs::class]
struct Fetches<'js> {
    streams: HashMap<u64, StreamRequest<'js>>,
    whole: HashMap<u64, WholeRequest<'js>>,
    legacy_proto: Option<Object<'js>>,
}

fn fetches<'js>(ctx: &Ctx<'js>) -> Result<Class<'js, Fetches<'js>>> {
    bridge(ctx)?.get("_fetches")
}

pub(super) fn caught<'js>(ctx: &Ctx<'js>, error: Error) -> Result<Value<'js>> {
    if error.is_exception() {
        Ok(ctx.catch())
    } else {
        Exception::from_message(ctx.clone(), &error.to_string()).map(Exception::into_value)
    }
}

fn reject<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Promise<'js>> {
    let (promise, _, reject) = ctx.promise()?;
    reject.call::<_, ()>((value,))?;
    Ok(promise)
}

fn abort<'js>(ctx: &Ctx<'js>, token: u64) -> Result<()> {
    let abort: Function = bridge(ctx)?.get("abort")?;
    abort.call((crate::resources::number(token),))
}

fn detach(handler: Option<Function<'_>>) -> Result<()> {
    if let Some(handler) = handler {
        handler.call::<_, ()>(())?;
    }
    Ok(())
}

fn fetch<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    Opt(init): Opt<Value<'js>>,
) -> Result<Promise<'js>> {
    let portable = platform(&ctx)?.is_portable();
    let result = request(&ctx, input, init).and_then(|request| {
        if portable {
            start_stream(&ctx, request, true)
        } else {
            start_whole(&ctx, request)
        }
    });
    match result {
        Err(error) if portable => reject(&ctx, caught(&ctx, error)?),
        result => result,
    }
}

fn fetch_stream<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    Opt(init): Opt<Value<'js>>,
) -> Result<Promise<'js>> {
    let request = request(&ctx, input, init)?;
    start_stream(&ctx, request, false)
}

fn request<'js>(
    ctx: &Ctx<'js>,
    input: Value<'js>,
    init: Option<Value<'js>>,
) -> Result<Request<'js>> {
    let request = Request::from_input(
        ctx,
        input,
        init.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
    )?;
    if !request.url.starts_with("https:") && !request.url.starts_with("http:") {
        return Err(Exception::throw_type(ctx, "Unsupported fetch URL scheme"));
    }
    Ok(request)
}

fn submit<'js>(
    ctx: &Ctx<'js>,
    request: &Request<'js>,
    streaming: bool,
    external: bool,
) -> Result<(u64, Promise<'js>)> {
    let host = bridge(ctx)?;
    let send: Function = host.get(if streaming { "httpStream" } else { "http" })?;
    let headers = request.headers.try_borrow()?.wire();
    let body = request
        .body
        .as_ref()
        .map(|body| streams::take_bytes(ctx, &body.stream))
        .transpose()?;
    let sent: Promise = send.call((&request.method, &request.url, headers, body, external))?;
    let token = sent
        .as_object()
        .ok_or(Error::Unknown)?
        .get::<_, Value>("token")?;
    let token = crate::host::js_id(Some(&token))
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid HTTP request token"))?;
    Ok((token, sent))
}

fn start_whole<'js>(ctx: &Ctx<'js>, request: Request<'js>) -> Result<Promise<'js>> {
    if abort_reason(&request.signal)?.is_some() {
        return reject(ctx, named_error(ctx, "Error", "AbortError")?);
    }
    let store = fetches(ctx)?;
    let (token, sent) = submit(ctx, &request, false, false)?;
    let registered = on_abort(
        &request.signal,
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| abort(&ctx, token))?,
    )?;
    store.try_borrow_mut()?.whole.insert(
        token,
        WholeRequest {
            detach: Some(registered),
            url: request.url,
            method: request.method,
        },
    );
    let fulfilled = Function::new(ctx.clone(), move |ctx: Ctx<'js>, raw: Object<'js>| {
        let request = fetches(&ctx)?
            .try_borrow_mut()?
            .whole
            .remove(&token)
            .ok_or_else(|| Exception::throw_internal(&ctx, "HTTP response has no request"))?;
        detach(request.detach)?;
        let status = raw.get("status")?;
        let headers = raw.get("headers")?;
        let body = if no_body(&request.method, status) {
            None
        } else {
            let buffer: ArrayBuffer = raw.get("body")?;
            let bytes = TypedArray::<u8>::from_arraybuffer(buffer)?;
            Some(body_from_bytes(&ctx, bytes, None)?)
        };
        network_response(&ctx, request.url, status, headers, body, None)
    })?;
    let rejected = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, error: Value<'js>| -> Result<Value<'js>> {
            let request = fetches(&ctx)?.try_borrow_mut()?.whole.remove(&token);
            detach(request.and_then(|request| request.detach))?;
            Err(ctx.throw(error))
        },
    )?;
    streams::promise_then(ctx, &sent, Some(&fulfilled), Some(&rejected))
}

fn start_stream<'js>(
    ctx: &Ctx<'js>,
    request: Request<'js>,
    external: bool,
) -> Result<Promise<'js>> {
    if let Some(reason) = abort_reason(&request.signal)? {
        return reject(
            ctx,
            if external {
                reason
            } else {
                named_error(ctx, "Error", "AbortError")?
            },
        );
    }
    let (promise, resolve, reject) = ctx.promise()?;
    let store = fetches(ctx)?;
    let (token, sent) = submit(ctx, &request, true, external)?;
    let cancel = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        detach(take_stream(&ctx, token)?.and_then(|entry| entry.detach))?;
        abort(&ctx, token)
    })?;
    let stream = streams::create(ctx, Some(cancel))?;
    store.try_borrow_mut()?.streams.insert(
        token,
        StreamRequest {
            stream,
            reply: Some(Reply { resolve, reject }),
            detach: None,
            external,
            no_body: false,
            received: 0,
            url: request.url,
            method: request.method,
        },
    );
    let handler = on_abort(
        &request.signal,
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, reason: Value<'js>| {
            let reason = if external {
                reason
            } else {
                named_error(&ctx, "Error", "AbortError")?
            };
            fail_stream(&ctx, token, reason)?;
            abort(&ctx, token)
        })?,
    )?;
    if let Some(entry) = store.try_borrow_mut()?.streams.get_mut(&token) {
        entry.detach = Some(handler);
    }
    let fallback = Function::new(ctx.clone(), move |ctx: Ctx<'js>, error: Value<'js>| {
        fail_stream(&ctx, token, error)
    })?;
    streams::promise_then(ctx, &sent, None, Some(&fallback))?;
    Ok(promise)
}

fn take_stream<'js>(ctx: &Ctx<'js>, token: u64) -> Result<Option<StreamRequest<'js>>> {
    Ok(fetches(ctx)?.try_borrow_mut()?.streams.remove(&token))
}

fn fail_stream<'js>(ctx: &Ctx<'js>, token: u64, reason: Value<'js>) -> Result<()> {
    if let Some(entry) = take_stream(ctx, token)? {
        detach(entry.detach)?;
        streams::error(ctx, &entry.stream, reason.clone())?;
        if let Some(reply) = entry.reply {
            reply.reject.call::<_, ()>((reason,))?;
        }
    }
    Ok(())
}

fn release(release: Option<Function<'_>>) -> Result<()> {
    if let Some(release) = release {
        release.call::<_, ()>(())?;
    }
    Ok(())
}

fn http_stream<'js>(
    ctx: Ctx<'js>,
    token: Value<'js>,
    kind: String,
    payload: Value<'js>,
    Opt(credit): Opt<Value<'js>>,
) -> Result<()> {
    let credit = credit.and_then(Value::into_function);
    let Some(token) = crate::host::js_id(Some(&token)) else {
        return release(credit);
    };
    let store = fetches(&ctx)?;
    let Some((is_pending, external, stream)) = store
        .try_borrow()?
        .streams
        .get(&token)
        .map(|entry| (entry.reply.is_some(), entry.external, entry.stream.clone()))
    else {
        return release(credit);
    };
    if kind == "head" {
        return if is_pending {
            stream_head(&ctx, &store, token, &stream, payload, external)
        } else {
            Ok(())
        };
    }
    if kind == "done"
        && let Some(message) = payload.as_string()
    {
        release(credit)?;
        let message = message.to_string()?;
        let error = if external {
            named_error(&ctx, &message, &message)?
        } else {
            named_error(&ctx, "Error", &message)?
        };
        return fail_stream(&ctx, token, error);
    }
    if is_pending {
        release(credit)?;
        fail_stream(
            &ctx,
            token,
            named_error(
                &ctx,
                "TypeError",
                "Stream ended before receiving a response head",
            )?,
        )?;
        return abort(&ctx, token);
    }
    let bytes = TypedArray::<u8>::from_arraybuffer(
        ArrayBuffer::from_value(payload)
            .ok_or_else(|| Exception::throw_type(&ctx, "Expected an ArrayBuffer"))?,
    )?;
    let Some((no_body, over_limit)) =
        store
            .try_borrow_mut()?
            .streams
            .get_mut(&token)
            .map(|entry| {
                if !entry.no_body {
                    entry.received = entry.received.saturating_add(bytes.len());
                }
                (entry.no_body, entry.received > BODY_LIMIT)
            })
    else {
        return release(credit);
    };
    if over_limit {
        release(credit)?;
        return fail_limit(&ctx, token, "Stream exceeded its response body limit");
    }
    if no_body {
        release(credit)?;
    } else if !streams::push(&ctx, &stream, bytes, credit)? {
        return fail_limit(&ctx, token, "Stream exceeded its body queue limit");
    }
    if kind == "done" {
        detach(take_stream(&ctx, token)?.and_then(|entry| entry.detach))?;
        streams::finish(&ctx, &stream)?;
    }
    Ok(())
}

fn fail_limit<'js>(ctx: &Ctx<'js>, token: u64, message: &str) -> Result<()> {
    Exception::throw_range(ctx, message);
    fail_stream(ctx, token, ctx.catch())?;
    abort(ctx, token)
}

fn stream_head<'js>(
    ctx: &Ctx<'js>,
    store: &Class<'js, Fetches<'js>>,
    token: u64,
    stream: &Stream<'js>,
    head: Value<'js>,
    external: bool,
) -> Result<()> {
    let (url, method) = {
        let store = store.try_borrow()?;
        let Some(entry) = store.streams.get(&token) else {
            return Ok(());
        };
        (entry.url.clone(), entry.method.clone())
    };
    let build = || {
        let head = head
            .into_object()
            .ok_or_else(|| Exception::throw_type(ctx, "Expected a response head"))?;
        let status = head.get("status")?;
        let empty = no_body(&method, status);
        let body = (!empty).then(|| Body {
            stream: stream.clone(),
            media_type: None,
        });
        let response = network_response(
            ctx,
            url,
            status,
            head.get("headers")?,
            body,
            (!external).then_some(stream.clone()),
        )?;
        Ok::<_, Error>((empty, response))
    };
    let (empty, response) = match build() {
        Ok(result) => result,
        Err(error) => {
            fail_stream(ctx, token, caught(ctx, error)?)?;
            return abort(ctx, token);
        }
    };
    if empty {
        streams::finish(ctx, stream)?;
    }
    let reply = store
        .try_borrow_mut()?
        .streams
        .get_mut(&token)
        .and_then(|entry| {
            entry.no_body = empty;
            entry.reply.take()
        });
    if let Some(reply) = reply {
        reply.resolve.call::<_, ()>((response,))?;
    }
    Ok(())
}

fn network_response<'js>(
    ctx: &Ctx<'js>,
    url: String,
    status: u16,
    wire: Value<'js>,
    body: Option<Body<'js>>,
    legacy: Option<Stream<'js>>,
) -> Result<Class<'js, Response<'js>>> {
    let headers = Class::instance(ctx.clone(), Headers::from_value(ctx, wire, true)?)?;
    let mut charge = Charge::new(ctx)?;
    charge.resize(ctx, url.len())?;
    let legacy = legacy.map(|stream| LegacyReader {
        stream,
        reader: None,
    });
    let response = Response {
        url,
        status,
        status_text: String::new(),
        headers,
        body,
        legacy,
        _charge: charge,
    };
    if response.legacy.is_some() {
        let proto = fetches(ctx)?
            .try_borrow()?
            .legacy_proto
            .clone()
            .ok_or_else(|| {
                Exception::throw_internal(ctx, "Stream response prototype is unavailable")
            })?;
        Class::instance_proto(response, proto)
    } else {
        Class::instance(ctx.clone(), response)
    }
}

pub(super) fn close(ctx: &Ctx<'_>) -> Result<()> {
    let store = fetches(ctx)?;
    let (streams, whole) = {
        let mut store = store.try_borrow_mut()?;
        (
            std::mem::take(&mut store.streams),
            std::mem::take(&mut store.whole),
        )
    };
    for request in whole.into_values() {
        detach(request.detach)?;
    }
    let error = named_error(ctx, "AbortError", "Runtime closed")?;
    for request in streams.into_values() {
        detach(request.detach)?;
        streams::error(ctx, &request.stream, error.clone())?;
        if let Some(reply) = request.reply {
            reply.reject.call::<_, ()>((error.clone(),))?;
        }
    }
    Ok(())
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, bridge: &Object<'js>) -> Result<()> {
    let globals = ctx.globals();
    globals.set(
        "Request",
        constructor::<Request, _, _>(
            ctx,
            |ctx: Ctx<'js>, input: Value<'js>, Opt(init): Opt<Value<'js>>| {
                let init = init.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                Class::instance(ctx.clone(), Request::from_input(&ctx, input, init)?)
            },
        )?,
    )?;
    globals.set(
        "Response",
        constructor::<Response, _, _>(
            ctx,
            |ctx: Ctx<'js>, Opt(input): Opt<Value<'js>>, Opt(init): Opt<Value<'js>>| {
                let input = input.unwrap_or_else(|| Value::new_null(ctx.clone()));
                let init = init.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                Class::instance(ctx.clone(), Response::from_input(&ctx, input, init)?)
            },
        )?,
    )?;
    let profile = platform(ctx)?;
    let legacy_proto = if profile.is_portable() {
        None
    } else {
        let parent = Class::<Response>::prototype(ctx)?
            .ok_or_else(|| Exception::throw_internal(ctx, "Response prototype is unavailable"))?;
        let proto = Object::new_proto(ctx.clone(), Some(&parent))?;
        proto.prop(
            "getReader",
            rquickjs::object::Property::from(Function::new(ctx.clone(), legacy_reader)?)
                .configurable()
                .writable(),
        )?;
        globals.set(
            "fetchStream",
            Function::new(ctx.clone(), fetch_stream)?.with_name("fetchStream")?,
        )?;
        Some(proto)
    };
    if profile.has_http() {
        globals.set(
            "fetch",
            Function::new(ctx.clone(), fetch)?.with_name("fetch")?,
        )?;
    }
    bridge.prop(
        "_fetches",
        Class::instance(
            ctx.clone(),
            Fetches {
                legacy_proto,
                ..Fetches::default()
            },
        )?,
    )?;
    bridge.set("_httpStream", Function::new(ctx.clone(), http_stream)?)
}

pub(super) const BODY_DECLARATIONS: &str = r#"
type BodyInit = string | BufferSource | URLSearchParams;

interface Body {
  readonly body: ReadableStream<Uint8Array> | null;
  readonly bodyUsed: boolean;
  arrayBuffer(): Promise<ArrayBuffer>;
  text(): Promise<string>;
  json(): Promise<unknown>;
}

interface RequestInit {
  method?: string;
  headers?: HeadersInit;
  body?: BodyInit | null;
  signal?: AbortSignal | null;
}

declare class Request implements Body {
  constructor(input: string | URL | Request, init?: RequestInit);
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  readonly signal: AbortSignal;
  readonly body: ReadableStream<Uint8Array> | null;
  readonly bodyUsed: boolean;
  arrayBuffer(): Promise<ArrayBuffer>;
  text(): Promise<string>;
  json(): Promise<unknown>;
}

interface ResponseInit {
  status?: number;
  statusText?: string;
  headers?: HeadersInit;
}

declare class Response implements Body {
  constructor(body?: BodyInit | null, init?: ResponseInit);
  readonly status: number;
  readonly statusText: string;
  readonly headers: Headers;
  readonly url: string;
  readonly ok: boolean;
  readonly body: ReadableStream<Uint8Array> | null;
  readonly bodyUsed: boolean;
  arrayBuffer(): Promise<ArrayBuffer>;
  text(): Promise<string>;
  json(): Promise<unknown>;
}
"#;

pub(super) const FETCH_DECLARATIONS: &str = r#"
declare function fetch(input: string | URL | Request, init?: RequestInit): Promise<Response>;
"#;

pub(super) const LEGACY_DECLARATIONS: &str = r#"
interface StreamResponse extends Response {
  getReader(): ReadableStreamDefaultReader<Uint8Array>;
}

declare function fetchStream(input: string | URL | Request, init?: RequestInit): Promise<StreamResponse>;
"#;
