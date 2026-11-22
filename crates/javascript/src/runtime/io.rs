//! Shared native side of the standard transport API.

use std::{rc::Rc, sync::Arc};

use crate::host::{HTTP_BODY_LIMIT, HttpClient, IoJob, MAX_SAFE_ID};
use rquickjs::function::Rest;
use rquickjs::{Ctx, Exception, Function, Object, Value};

use crate::runtime::quickjs::{Vm, codec};

pub struct HttpCall {
    pub streaming: bool,
    pub token: u64,
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: Vec<u8>,
}

pub type IoSubmit = Box<dyn Fn(IoJob) + Send>;
type Submit = Rc<dyn Fn(IoJob) -> Result<(), Box<IoJob>>>;

#[derive(Clone)]
pub(crate) enum External {
    Jobs,
    Legacy(Submit),
    Routed(Submit),
}

#[derive(Clone)]
pub(crate) enum HttpMode {
    Inline(Arc<dyn HttpClient>),
    External(External),
    Disconnected,
}

#[derive(Clone)]
pub struct IoBindings {
    pub(crate) http: HttpMode,
    pub(crate) socket: Option<External>,
}

impl IoBindings {
    pub fn jobs() -> Self {
        Self {
            http: HttpMode::External(External::Jobs),
            socket: Some(External::Jobs),
        }
    }

    pub fn disconnected() -> Self {
        Self {
            http: HttpMode::Disconnected,
            socket: None,
        }
    }

    pub fn inline(client: Arc<dyn HttpClient>) -> Self {
        Self {
            http: HttpMode::Inline(client),
            socket: None,
        }
    }

    pub fn with_socket_jobs(mut self) -> Self {
        self.socket = Some(External::Jobs);
        self
    }

    pub fn external(submit: IoSubmit) -> Self {
        let external = External::Legacy(Rc::new(move |job| {
            submit(job);
            Ok(())
        }));
        Self {
            http: HttpMode::External(external.clone()),
            socket: Some(external),
        }
    }

    pub fn routed(submit: impl Fn(IoJob) -> Result<(), Box<IoJob>> + 'static) -> Self {
        let external = External::Routed(Rc::new(submit));
        Self {
            http: HttpMode::External(external.clone()),
            socket: Some(external),
        }
    }
}

pub trait OperationPolicy {
    fn before(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()>;
}

pub struct Allow;

impl OperationPolicy for Allow {
    fn before(&self, _ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        Ok(())
    }
}

pub(crate) trait IoPort {
    fn http(&self, call: HttpCall) -> bool;
    fn abort(&self, ctx: &Ctx<'_>, token: u64) -> rquickjs::Result<()>;
    fn open_socket(&self, url: String, protocols: String) -> Option<u64>;
    fn send_socket(&self, id: u64, text: bool, data: Vec<u8>) -> bool;
    fn close_socket(&self, id: u64);
    fn buffered_amount(&self, id: u64) -> usize;
}

/// # Safety
///
/// The VM, port and policy are owned by the extension installing this bridge,
/// and stay at the same address until the functions and their context are dropped.
pub(crate) unsafe fn install<'js>(
    ctx: &Ctx<'js>,
    vm: *const Vm,
    port: *const dyn IoPort,
    policy: *const dyn OperationPolicy,
    has_module: impl Fn(&str) -> bool + 'static,
    stop_after_http: bool,
) -> rquickjs::Result<()> {
    let bridge = Object::new(ctx.clone())?;
    for (name, streaming) in [("http", false), ("httpStream", true)] {
        bridge.set(
            name,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    // SAFETY: all three refer to the live owners that installed this bridge.
                    let (vm, port, policy) = unsafe { (&*vm, &*port, &*policy) };
                    let _active = vm.enter(&ctx);
                    policy.before(&ctx)?;
                    let method = string_arg(&ctx, args.0.first(), 128)?;
                    let url = string_arg(&ctx, args.0.get(1), 16 << 10)?;
                    let headers = string_arg(&ctx, args.0.get(2), 64 << 10)?;
                    let body = data_arg(&ctx, args.0.get(3), HTTP_BODY_LIMIT)?;
                    let (token, promise) = vm.make_promise(&ctx)?;
                    let admitted = port.http(HttpCall {
                        streaming,
                        token,
                        method,
                        url,
                        headers,
                        body,
                    });
                    if !admitted {
                        vm.promises.borrow_mut().remove(&token);
                        return Err(Exception::throw_range(&ctx, "HTTP request limit exceeded"));
                    }
                    Ok(promise)
                },
            )?,
        )?;
    }
    if stop_after_http {
        // This object already owns the two registered native function values.
        ctx.globals().set("__host", bridge)?;
        return Err(Exception::throw_type(
            ctx,
            "partial host binding installation",
        ));
    }
    bridge.set(
        "abort",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: all three refer to the live owners that installed this bridge.
            let (vm, port, policy) = unsafe { (&*vm, &*port, &*policy) };
            let _active = vm.enter(&ctx);
            policy.before(&ctx)?;
            if let Some(token) = js_id(args.0.first()) {
                port.abort(&ctx, token)?;
            }
            rquickjs::Result::Ok(())
        })?,
    )?;
    bridge.set(
        "hasModule",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: vm is the live owner of the context invoking this binding.
            let _active = unsafe { &*vm }.enter(&ctx);
            // SAFETY: the policy stays alive while this callback can run.
            unsafe { (*policy).before(&ctx)? };
            Ok::<_, rquickjs::Error>(has_module(&string_arg(&ctx, args.0.first(), 16 << 10)?))
        })?,
    )?;
    bridge.set(
        "socketOpen",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: vm is the live owner of the context invoking this binding.
            let _active = unsafe { &*vm }.enter(&ctx);
            // SAFETY: the policy and port stay alive while this callback can run.
            unsafe { (*policy).before(&ctx)? };
            let url = string_arg(&ctx, args.0.first(), 16 << 10)?;
            let protocols = string_arg(&ctx, args.0.get(1), 16 << 10)?;
            // SAFETY: port belongs to the context that installed this callback.
            unsafe { (*port).open_socket(url, protocols) }
                .map(crate::runtime::resources::number)
                .ok_or_else(|| Exception::throw_range(&ctx, "socket admission limit exceeded"))
        })?,
    )?;
    bridge.set(
        "socketSend",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: vm is the live owner of the context invoking this binding.
            let _active = unsafe { &*vm }.enter(&ctx);
            // SAFETY: the policy and port stay alive while this callback can run.
            unsafe { (*policy).before(&ctx)? };
            let Some(id) = js_id(args.0.first()) else {
                return Ok(());
            };
            let value = args.0.get(1);
            let text = value.is_some_and(Value::is_string);
            let data = data_arg(&ctx, value, HTTP_BODY_LIMIT)?;
            // SAFETY: port belongs to the context that installed this callback.
            if !unsafe { (*port).send_socket(id, text, data) } {
                return Err(Exception::throw_range(
                    &ctx,
                    "socket send queue is full or closed",
                ));
            }
            Ok(())
        })?,
    )?;
    bridge.set(
        "socketClose",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: vm is the live owner of the context invoking this binding.
            let _active = unsafe { &*vm }.enter(&ctx);
            // SAFETY: the policy and port stay alive while this callback can run.
            unsafe { (*policy).before(&ctx)? };
            if let Some(id) = js_id(args.0.first()) {
                // SAFETY: port belongs to the context that installed this callback.
                unsafe { (*port).close_socket(id) };
            }
            rquickjs::Result::Ok(())
        })?,
    )?;
    bridge.set(
        "socketBufferedAmount",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            // SAFETY: vm is the live owner of the context invoking this binding.
            let _active = unsafe { &*vm }.enter(&ctx);
            // SAFETY: the policy and port stay alive while this callback can run.
            unsafe { (*policy).before(&ctx)? };
            // SAFETY: port belongs to the context that installed this callback.
            let port = unsafe { &*port };
            let bytes = js_id(args.0.first()).map_or(0, |id| port.buffered_amount(id));
            Ok::<_, rquickjs::Error>(u32::try_from(bytes).unwrap_or(u32::MAX))
        })?,
    )?;
    ctx.globals().set("__host", bridge)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "only positive integral JavaScript-safe IDs reach the cast"
)]
pub(crate) fn js_id(value: Option<&Value<'_>>) -> Option<u64> {
    let number = value.and_then(Value::as_number)?;
    (number >= 1.0
        && number <= crate::runtime::resources::number(MAX_SAFE_ID)
        && number.fract() == 0.0)
        .then_some(number as u64)
}

fn string_arg(ctx: &Ctx<'_>, value: Option<&Value<'_>>, max: usize) -> rquickjs::Result<String> {
    let Some(string) = value.and_then(Value::as_string) else {
        return Ok(String::new());
    };
    let raw = string.clone().to_cstring()?;
    if raw.len() > max {
        return Err(Exception::throw_range(
            ctx,
            "transport text exceeds byte limit",
        ));
    }
    drop(raw);
    string.to_string()
}

fn data_arg<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    max: usize,
) -> rquickjs::Result<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_string() {
        return Ok(string_arg(ctx, Some(value), max)?.into_bytes());
    }
    Ok(codec::read_bytes(ctx, value, max)?.unwrap_or_default())
}
