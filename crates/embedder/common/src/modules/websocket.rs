//! Native `@impasto/websocket` module built on `Method::resource` and `Method::stream_with_exec`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use engine::session::{CancelToken, Method, Module, ModuleExports, NativeResource, PluginValue};

use crate::diagnostics::DiagnosticsHub;
use crate::websocket::{SocketSession, SocketSlotTracker, WebSocketTransport};

const WS_TYPES: &str = "\
export interface NativeWebSocketHandle {
  call(method: \"id\"): number;
  call(method: \"bufferedAmount\" | \"readyState\"): number;
  call(method: \"protocol\"): string;
  call(method: \"sendText\", text: string): number;
  call(method: \"sendBinary\", data: Uint8Array | ArrayBuffer): number;
  call(method: \"close\", code?: number | null, reason?: string): void;
  close(): void;
}

export type NativeWebSocketEvent =
  | { type: \"open\"; protocol: string }
  | { type: \"message\"; text: true; data: string }
  | { type: \"message\"; text: false; data: ArrayBuffer | Uint8Array }
  | { type: \"close\"; code: number; reason: string; clean: boolean }
  | { type: \"error\"; message: string };";

struct WebSocketResource {
    session: Arc<SocketSession>,
    registry: Arc<Mutex<HashMap<u64, Arc<SocketSession>>>>,
}

impl Drop for WebSocketResource {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.session.id);
        self.session.shutdown();
    }
}

impl NativeResource for WebSocketResource {
    fn call(
        &mut self,
        method: &str,
        args: &[PluginValue],
        _cancel: &CancelToken,
    ) -> Result<PluginValue, String> {
        match method {
            "id" =>
            {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "monotonic WebSocket handle ids stay within safe f64 integers"
                )]
                Ok(PluginValue::F64(self.session.id as f64))
            }
            "readyState" => Ok(PluginValue::F64(f64::from(self.session.ready_state()))),
            "bufferedAmount" =>
            {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "MAX_OUTBOUND_BUFFERED is 256 KiB and fits losslessly in f64"
                )]
                Ok(PluginValue::F64(self.session.buffered_amount() as f64))
            }
            "protocol" => Ok(PluginValue::Str(self.session.protocol())),
            "sendText" => {
                let text = match args.first() {
                    Some(PluginValue::Str(text)) => text.clone(),
                    _ => return Err("sendText requires a string payload".to_owned()),
                };
                let buffered = self.session.send_text(text)?;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "MAX_OUTBOUND_BUFFERED is 256 KiB and fits losslessly in f64"
                )]
                Ok(PluginValue::F64(buffered as f64))
            }
            "sendBinary" => {
                let data = match args.first() {
                    Some(PluginValue::Bytes(bytes)) => bytes.clone(),
                    _ => return Err("sendBinary requires a Uint8Array or ArrayBuffer".to_owned()),
                };
                let buffered = self.session.send_binary(data)?;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "MAX_OUTBOUND_BUFFERED is 256 KiB and fits losslessly in f64"
                )]
                Ok(PluginValue::F64(buffered as f64))
            }
            "close" => {
                let code = match args.first() {
                    None | Some(PluginValue::Null) => None,
                    Some(PluginValue::F64(n))
                        if n.is_finite() && *n >= 0.0 && *n <= f64::from(u16::MAX) =>
                    {
                        #[expect(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "checked finite and within 0..=u16::MAX"
                        )]
                        let code = *n as u16;
                        Some(code)
                    }
                    _ => {
                        return Err("close code must be a valid unsigned 16-bit integer".to_owned());
                    }
                };
                let reason = match args.get(1) {
                    None | Some(PluginValue::Null) => String::new(),
                    Some(PluginValue::Str(text)) => text.clone(),
                    _ => return Err("close reason must be a string".to_owned()),
                };
                self.session.close(code, reason)?;
                Ok(PluginValue::Null)
            }
            _ => Err(format!("unknown WebSocket handle method \"{method}\"")),
        }
    }
}

pub fn websocket(
    transport: Option<Arc<dyn WebSocketTransport>>,
    devtools: Option<Arc<DiagnosticsHub>>,
) -> Module {
    let tracker = Arc::new(SocketSlotTracker::default());
    let registry: Arc<Mutex<HashMap<u64, Arc<SocketSession>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let open_tracker = Arc::clone(&tracker);
    let open_registry = Arc::clone(&registry);
    let open_transport = transport;
    let open_devtools = devtools;

    Module {
        name: "WebSocket",
        specifier: "@impasto/websocket",
        exports: ModuleExports {
            object: true,
            typescript: WS_TYPES,
            ..Default::default()
        },
        methods: vec![
            Method::resource("open", move |args| {
                let url = match args.first() {
                    Some(PluginValue::Str(url)) => url.clone(),
                    _ => return Err("WebSocket.open requires a URL string".to_owned()),
                };
                let protocols = parse_protocols(args.get(1))?;
                let session = SocketSession::open(
                    url,
                    protocols,
                    &open_tracker,
                    open_transport.clone(),
                    open_devtools.clone(),
                )?;
                open_registry
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(session.id, Arc::clone(&session));
                Ok(Box::new(WebSocketResource {
                    session,
                    registry: Arc::clone(&open_registry),
                }))
            })
            .with_types("url: string, protocols?: string[]", "NativeWebSocketHandle"),
            Method::stream_with_exec("events", move |args, sink, _exec| {
                let Some(id) = parse_handle_id(args.first()) else {
                    sink.fail("WebSocket.events requires a numeric handle id".to_owned());
                    return;
                };
                let session = registry
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&id)
                    .cloned();
                let Some(session) = session else {
                    sink.fail("unknown or closed WebSocket handle".to_owned());
                    return;
                };
                if let Err(err) = session.start_stream(sink) {
                    let (sink, message) = *err;
                    sink.fail(message);
                }
            })
            .with_types(
                "id: number, signal?: AbortSignal",
                "ReadableStream<NativeWebSocketEvent>",
            ),
        ],
    }
}

fn parse_protocols(arg: Option<&PluginValue>) -> Result<Vec<String>, String> {
    match arg {
        None | Some(PluginValue::Null) => Ok(Vec::new()),
        Some(PluginValue::Str(single)) => Ok(vec![single.clone()]),
        Some(PluginValue::List(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let PluginValue::Str(proto) = item else {
                    return Err("WebSocket protocols must be strings".to_owned());
                };
                out.push(proto.clone());
            }
            Ok(out)
        }
        _ => Err("WebSocket protocols must be a string or string[]".to_owned()),
    }
}

fn parse_handle_id(arg: Option<&PluginValue>) -> Option<u64> {
    let Some(PluginValue::F64(n)) = arg else {
        return None;
    };
    if !n.is_finite() || *n <= 0.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked finite positive id"
    )]
    Some(*n as u64)
}
