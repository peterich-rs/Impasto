//! Fetch objects and promise settlement on the QuickJS owner thread.

use std::collections::HashMap;

use rquickjs::class::Trace;
use rquickjs::function::{Opt, This};
use rquickjs::{
    ArrayBuffer, Class, Coerced, Ctx, Error, Exception, FromJs, Function, JsLifetime, Object,
    Promise, Result, TypedArray, Value,
};

use crate::runtime::quickjs::buffer::{is_source as is_buffer_source, view as buffer_bytes};

use super::encoding::usv_string;
use super::events::{abort_reason, named_error, new_signal, on_abort};
use super::headers::{Headers, is_token};
use super::streams::{self, BodyOutput, ReadableStream};
use super::url::{form_query, parse};
use super::web::{Charge, bridge, constructor, is_nullish, option, platform};

type Stream<'js> = Class<'js, ReadableStream<'js>>;

const BODY_LIMIT: usize = crate::host::HTTP_BODY_LIMIT;

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
        if matches!(
            normalized.as_str(),
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
        ) {
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
        let header_init = option(ctx, &init, "headers")?;
        let source_headers = source
            .as_ref()
            .map(|source| source.try_borrow().map(|value| value.headers.clone()))
            .transpose()?;
        let header_init = if header_init.is_undefined() {
            source_headers.map_or(header_init, Class::into_value)
        } else {
            header_init
        };
        let mut headers = Headers::from_value(ctx, header_init, false)?;
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
        if (!is_nullish(&body_input) || inherited.is_some())
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
    #[qjs(skip_trace)]
    _charge: Charge,
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
    no_body: bool,
    received: usize,
    url: String,
    method: String,
}

#[derive(Default, Trace, JsLifetime)]
#[rquickjs::class]
struct Fetches<'js> {
    streams: HashMap<u64, StreamRequest<'js>>,
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
    abort.call((crate::runtime::resources::number(token),))
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
    match request(&ctx, input, init).and_then(|request| start_stream(&ctx, request)) {
        Ok(promise) => Ok(promise),
        Err(error) => reject(&ctx, caught(&ctx, error)?),
    }
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

fn submit<'js>(ctx: &Ctx<'js>, request: &Request<'js>) -> Result<(u64, Promise<'js>)> {
    let host = bridge(ctx)?;
    let send: Function = host.get("httpStream")?;
    let headers = request.headers.try_borrow()?.wire();
    let body = request
        .body
        .as_ref()
        .map(|body| streams::take_bytes(ctx, &body.stream))
        .transpose()?;
    let sent: Promise = send.call((&request.method, &request.url, headers, body))?;
    let token = sent
        .as_object()
        .ok_or(Error::Unknown)?
        .get::<_, Value>("token")?;
    let token = crate::runtime::io::js_id(Some(&token))
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid HTTP request token"))?;
    Ok((token, sent))
}

fn start_stream<'js>(ctx: &Ctx<'js>, request: Request<'js>) -> Result<Promise<'js>> {
    if let Some(reason) = abort_reason(&request.signal)? {
        return reject(ctx, reason);
    }
    let (promise, resolve, reject) = ctx.promise()?;
    let store = fetches(ctx)?;
    let (token, sent) = submit(ctx, &request)?;
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
            no_body: false,
            received: 0,
            url: request.url,
            method: request.method,
        },
    );
    let handler = on_abort(
        &request.signal,
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, reason: Value<'js>| {
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
    let Some(token) = crate::runtime::io::js_id(Some(&token)) else {
        return release(credit);
    };
    let store = fetches(&ctx)?;
    let Some((is_pending, stream)) = store
        .try_borrow()?
        .streams
        .get(&token)
        .map(|entry| (entry.reply.is_some(), entry.stream.clone()))
    else {
        return release(credit);
    };
    if kind == "head" {
        return if is_pending {
            stream_head(&ctx, &store, token, &stream, payload)
        } else {
            Ok(())
        };
    }
    if kind == "done"
        && let Some(message) = payload.as_string()
    {
        release(credit)?;
        let message = message.to_string()?;
        let error = named_error(&ctx, &message, &message)?;
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
        let response = network_response(ctx, url, status, head.get("headers")?, body)?;
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
) -> Result<Class<'js, Response<'js>>> {
    let headers = Class::instance(ctx.clone(), Headers::from_value(ctx, wire, true)?)?;
    let mut charge = Charge::new(ctx)?;
    charge.resize(ctx, url.len())?;
    Class::instance(
        ctx.clone(),
        Response {
            url,
            status,
            status_text: String::new(),
            headers,
            body,
            _charge: charge,
        },
    )
}

pub(super) fn close(ctx: &Ctx<'_>) -> Result<()> {
    let store = fetches(ctx)?;
    let streams = std::mem::take(&mut store.try_borrow_mut()?.streams);
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
    if profile.has_http() {
        globals.set(
            "fetch",
            Function::new(ctx.clone(), fetch)?.with_name("fetch")?,
        )?;
    }
    bridge.prop(
        "_fetches",
        Class::instance(ctx.clone(), Fetches::default())?,
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
