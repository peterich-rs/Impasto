//! The WebSocket API over the existing host transport.

use std::collections::{HashMap, HashSet};

use rquickjs::class::Trace;
use rquickjs::function::This;
use rquickjs::object::{Accessor, Property};
use rquickjs::{Class, Coerced, Ctx, Exception, Function, JsLifetime, Object, Result, Value};

use crate::runtime::quickjs::buffer::{is_source as is_buffer_source, view as buffer_bytes};

use super::encoding::usv_string;
use super::events;
use super::headers::is_token;
use super::url::parse;
use super::web::{Charge, array_from, bridge, platform};

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
}

#[derive(JsLifetime, Trace)]
#[rquickjs::class]
struct Socket {
    id: u64,
    url: String,
    protocol: String,
    #[qjs(skip_trace)]
    state: State,
    #[qjs(skip_trace)]
    charge: Charge,
}

#[derive(Default, JsLifetime, Trace)]
#[rquickjs::class]
struct Sockets<'js> {
    entries: HashMap<u64, Object<'js>>,
}

fn sockets<'js>(ctx: &Ctx<'js>) -> Result<Class<'js, Sockets<'js>>> {
    bridge(ctx)?.get("_sockets")
}

fn socket<'js>(object: &Object<'js>) -> Result<Class<'js, Socket>> {
    events::target_attachment(object)?
        .and_then(|state| Class::<Socket>::from_object(&state))
        .ok_or_else(|| Exception::throw_type(object.ctx(), "Expected a WebSocket"))
}

fn socket_protocols<'js>(ctx: &Ctx<'js>, input: Option<Value<'js>>) -> Result<String> {
    let Some(input) = input.filter(|input| !input.is_undefined()) else {
        return Ok(String::new());
    };
    let protocols: Vec<String> = if let Some(input) = input.as_string() {
        vec![input.to_string()?]
    } else {
        array_from(ctx, input)?
            .iter::<Coerced<String>>()
            .map(|value| value.map(|Coerced(value)| value))
            .collect::<Result<_>>()?
    };
    let mut seen = HashSet::new();
    for value in &protocols {
        if !is_token(value) || !seen.insert(value) {
            return Err(Exception::throw_type(ctx, "Invalid WebSocket subprotocol"));
        }
    }
    Ok(protocols.join(", "))
}

fn open<'js>(
    ctx: &Ctx<'js>,
    input: Value<'js>,
    protocols: Option<Value<'js>>,
) -> Result<Value<'js>> {
    let mut url = parse(ctx, input, None)?;
    let upgraded = match url.scheme() {
        "http" => Some("ws"),
        "https" => Some("wss"),
        _ => None,
    };
    if let Some(scheme) = upgraded {
        url.set_scheme(scheme)
            .map_err(|()| Exception::throw_type(ctx, "Invalid WebSocket URL"))?;
    }
    if !matches!(url.scheme(), "ws" | "wss")
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Exception::throw_type(ctx, "Invalid WebSocket URL"));
    }
    let protocols = socket_protocols(ctx, protocols)?;
    let url: String = url.into();
    let store = sockets(ctx)?;
    let mut charge = Charge::new(ctx)?;
    charge.resize(ctx, url.len())?;
    let socket = Class::instance(
        ctx.clone(),
        Socket {
            id: 0,
            url: url.clone(),
            protocol: String::new(),
            state: State::Connecting,
            charge,
        },
    )?;
    let object = events::target_with_attachment(
        &events::target_prototype(ctx)?,
        socket.clone().into_inner(),
    )?;
    let open: Function = bridge(ctx)?.get("socketOpen")?;
    let value: Value = open.call((url, protocols))?;
    let id = crate::runtime::io::js_id(Some(&value))
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid WebSocket token"))?;
    socket.try_borrow_mut()?.id = id;
    store.try_borrow_mut()?.entries.insert(id, object.clone());
    Ok(object.into_value())
}

fn buffered_amount<'js>(This(object): This<Object<'js>>) -> Result<u32> {
    let socket = socket(&object)?;
    let (id, state) = {
        let socket = socket.try_borrow()?;
        (socket.id, socket.state)
    };
    if state == State::Closed {
        return Ok(0);
    }
    let function: Function = bridge(object.ctx())?.get("socketBufferedAmount")?;
    function.call((crate::runtime::resources::number(id),))
}

fn send<'js>(This(object): This<Object<'js>>, data: Value<'js>) -> Result<()> {
    let ctx = object.ctx();
    let (id, state) = {
        let socket = socket(&object)?;
        let socket = socket.try_borrow()?;
        (socket.id, socket.state)
    };
    if state == State::Connecting {
        return Err(ctx.throw(events::named_error(
            ctx,
            "InvalidStateError",
            "WebSocket is connecting",
        )?));
    }
    if state != State::Open {
        return Ok(());
    }
    let send: Function = bridge(ctx)?.get("socketSend")?;
    let id = crate::runtime::resources::number(id);
    if is_buffer_source(&data) {
        send.call((id, buffer_bytes(ctx, data)?))
    } else {
        send.call((id, usv_string(ctx, data)?))
    }
}

fn request_close<'js>(This(object): This<Object<'js>>) -> Result<()> {
    let socket = socket(&object)?;
    let (id, state) = {
        let socket = socket.try_borrow()?;
        (socket.id, socket.state)
    };
    if matches!(state, State::Closing | State::Closed) {
        return Ok(());
    }
    let close: Function = bridge(object.ctx())?.get("socketClose")?;
    close.call::<_, ()>((crate::runtime::resources::number(id),))?;
    let mut socket = socket.try_borrow_mut()?;
    if socket.state != State::Closed {
        socket.state = State::Closing;
    }
    Ok(())
}

fn transport_event<'js>(
    ctx: Ctx<'js>,
    token: Value<'js>,
    kind: String,
    data: Value<'js>,
) -> Result<()> {
    let Some(id) = crate::runtime::io::js_id(Some(&token)) else {
        return Ok(());
    };
    let store = sockets(&ctx)?;
    let object = store.try_borrow()?.entries.get(&id).cloned();
    let Some(object) = object else { return Ok(()) };
    let socket = socket(&object)?;
    match kind.as_str() {
        "open" => {
            let protocol = data
                .as_string()
                .map(rquickjs::String::to_string)
                .transpose()?
                .unwrap_or_default();
            let mut state = socket.try_borrow_mut()?;
            if state.state != State::Connecting {
                return Ok(());
            }
            let size = state.url.len().saturating_add(protocol.len());
            state.charge.resize(&ctx, size)?;
            state.state = State::Open;
            state.protocol = protocol;
            drop(state);
            events::dispatch(&object, &events::event(&ctx, "open")?)?;
        }
        "message" if socket.try_borrow()?.state == State::Open => {
            let event = events::event(&ctx, "message")?;
            event.prop("data", Property::from(data).enumerable())?;
            events::dispatch(&object, &event)?;
        }
        "error" | "close" => {
            store.try_borrow_mut()?.entries.remove(&id);
            socket.try_borrow_mut()?.state = State::Closed;
            let clean = kind == "close";
            if !clean {
                events::dispatch(&object, &events::event(&ctx, "error")?)?;
            }
            let event = events::event(&ctx, "close")?;
            event.prop(
                "code",
                Property::from(if clean { 1000 } else { 1006 }).enumerable(),
            )?;
            event.prop("reason", Property::from("").enumerable())?;
            event.prop("wasClean", Property::from(clean).enumerable())?;
            events::dispatch(&object, &event)?;
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn close(ctx: &Ctx<'_>) -> Result<()> {
    let store = sockets(ctx)?;
    let outstanding = std::mem::take(&mut store.try_borrow_mut()?.entries);
    for object in outstanding.into_values() {
        socket(&object)?.try_borrow_mut()?.state = State::Closed;
    }
    Ok(())
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, bridge: &Object<'js>) -> Result<()> {
    bridge.prop(
        "_sockets",
        Class::instance(ctx.clone(), Sockets::default())?,
    )?;
    bridge.set("_socket", Function::new(ctx.clone(), transport_event)?)?;
    if !platform(ctx)?.has_socket() {
        return Ok(());
    }
    let prototype = Object::new_proto(ctx.clone(), Some(&events::target_prototype(ctx)?))?;
    let constructor = events::constructor(ctx, prototype.clone(), "WebSocket", 1, |args| {
        let ctx = args.ctx();
        let input = args
            .arg(0)
            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        open(ctx, input, args.arg(1))
    })?;
    let class = constructor.as_object().ok_or(rquickjs::Error::Unknown)?;
    for (name, value) in [
        ("CONNECTING", 0),
        ("OPEN", 1),
        ("CLOSING", 2),
        ("CLOSED", 3),
    ] {
        for target in [class, &prototype] {
            target.prop(name, Property::from(value).enumerable())?;
        }
    }
    prototype.prop(
        "url",
        Accessor::new_get(|This(object): This<Object<'js>>| {
            Ok::<_, rquickjs::Error>(socket(&object)?.try_borrow()?.url.clone())
        })
        .configurable(),
    )?;
    prototype.prop(
        "protocol",
        Accessor::new_get(|This(object): This<Object<'js>>| {
            Ok::<_, rquickjs::Error>(socket(&object)?.try_borrow()?.protocol.clone())
        })
        .configurable(),
    )?;
    prototype.prop(
        "readyState",
        Accessor::new_get(|This(object): This<Object<'js>>| {
            Ok::<_, rquickjs::Error>(socket(&object)?.try_borrow()?.state as u8)
        })
        .configurable(),
    )?;
    prototype.prop(
        "bufferedAmount",
        Accessor::new_get(buffered_amount).configurable(),
    )?;
    prototype.prop(
        "binaryType",
        Accessor::new(|| "arraybuffer", |_: Value<'js>| {}).configurable(),
    )?;
    prototype.prop(
        "send",
        Property::from(Function::new(ctx.clone(), send)?)
            .configurable()
            .writable(),
    )?;
    prototype.prop(
        "close",
        Property::from(Function::new(ctx.clone(), request_close)?)
            .configurable()
            .writable(),
    )?;
    for kind in ["open", "message", "error", "close"] {
        events::define_handler(&prototype, kind)?;
    }
    let parent: Object = ctx.globals().get("EventTarget")?;
    constructor.set_prototype(Some(&parent))?;
    ctx.globals().set("WebSocket", constructor)
}

pub(super) const DECLARATIONS: &str = r#"
interface WebSocketMessageEvent extends Event {
  readonly data: string | ArrayBuffer;
}

interface WebSocketCloseEvent extends Event {
  readonly code: number;
  readonly reason: string;
  readonly wasClean: boolean;
}

interface WebSocketEventMap {
  open: Event;
  message: WebSocketMessageEvent;
  error: Event;
  close: WebSocketCloseEvent;
}

declare class WebSocket extends EventTarget {
  constructor(url: string | URL, protocols?: string | string[]);
  static readonly CONNECTING: 0;
  static readonly OPEN: 1;
  static readonly CLOSING: 2;
  static readonly CLOSED: 3;
  readonly CONNECTING: 0;
  readonly OPEN: 1;
  readonly CLOSING: 2;
  readonly CLOSED: 3;
  readonly url: string;
  readonly readyState: 0 | 1 | 2 | 3;
  readonly protocol: string;
  readonly bufferedAmount: number;
  binaryType: "arraybuffer";
  onopen: ((this: WebSocket, event: Event) => void) | null;
  onmessage: ((this: WebSocket, event: WebSocketMessageEvent) => void) | null;
  onerror: ((this: WebSocket, event: Event) => void) | null;
  onclose: ((this: WebSocket, event: WebSocketCloseEvent) => void) | null;
  addEventListener<K extends keyof WebSocketEventMap>(type: K, listener: (this: WebSocket, event: WebSocketEventMap[K]) => void, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: string, listener: EventListener | null, options?: boolean | AddEventListenerOptions): void;
  send(value: string | BufferSource): void;
  close(): void;
}
"#;
