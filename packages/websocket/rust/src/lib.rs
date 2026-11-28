//! Window-scoped WebSocket resources and their event streams.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use impasto::abi::PluginValue;
use impasto::{
    Bytes, CancelToken, DiagnosticsSink, FromPluginValue, ImpastoError, ImpastoType,
    IntoPluginValue, PluginContext, PluginInitError, PluginRegistrar, TsTypes, TypedSink,
};

#[cfg(target_os = "ios")]
#[expect(unsafe_code, reason = "Foundation WebSocket delegate callbacks")]
mod ios;
mod socket;
mod trace;

use socket::{SocketSession, SocketSlotTracker, WebSocketTransport};

#[derive(Debug)]
struct SocketError(String);

impl ImpastoError for SocketError {
    fn scope(&self) -> &'static str {
        "websocket"
    }
    fn code(&self) -> &'static str {
        "socket"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

enum SocketData {
    Text(String),
    Bytes(Bytes),
}

impl FromPluginValue for SocketData {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        match value {
            PluginValue::Str(text) => Ok(Self::Text(text)),
            value => Bytes::from_value(value).map(Self::Bytes),
        }
    }
}

impl IntoPluginValue for SocketData {
    fn into_value(self) -> Result<PluginValue, String> {
        match self {
            Self::Text(text) => text.into_value(),
            Self::Bytes(bytes) => bytes.into_value(),
        }
    }
}

impl ImpastoType for SocketData {
    fn typescript(_: &mut TsTypes) -> String {
        "string | Uint8Array | ArrayBuffer".into()
    }
}

#[derive(ImpastoType)]
#[impasto(tag = "type")]
enum NativeWebSocketEvent {
    Open {
        protocol: String,
    },
    Message {
        text: bool,
        data: SocketData,
    },
    Close {
        code: u16,
        reason: String,
        clean: bool,
    },
    Error {
        message: String,
    },
}

type Sessions = Arc<Mutex<HashMap<u64, Arc<SocketSession>>>>;

#[impasto::object]
struct NativeWebSocketHandle {
    session: Arc<SocketSession>,
    registry: Sessions,
}

impl Drop for NativeWebSocketHandle {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.session.id());
        self.session.shutdown();
    }
}

#[impasto::export]
impl NativeWebSocketHandle {
    pub fn id(&self) -> u64 {
        self.session.id()
    }
    pub fn ready_state(&self) -> u8 {
        self.session.ready_state()
    }
    pub fn buffered_amount(&self) -> u64 {
        u64::try_from(self.session.buffered_amount()).unwrap_or(u64::MAX)
    }
    pub fn protocol(&self) -> String {
        self.session.protocol()
    }
    pub fn send_text(&self, text: String) -> Result<u64, SocketError> {
        self.session
            .send_text(text)
            .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
            .map_err(SocketError)
    }
    pub fn send_binary(&self, data: Bytes) -> Result<u64, SocketError> {
        self.session
            .send_binary(data.0)
            .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
            .map_err(SocketError)
    }
    pub fn close_socket(
        &self,
        code: Option<u16>,
        reason: Option<String>,
    ) -> Result<(), SocketError> {
        self.session
            .close(code, reason.unwrap_or_default())
            .map_err(SocketError)
    }
}

#[impasto::module(name = "WebSocket", specifier = "@impasto/websocket/native", init = init, object = true)]
struct WebSocketPlugin {
    registry: Sessions,
    tracker: SocketSlotTracker,
    transport: Option<Arc<dyn WebSocketTransport>>,
    diagnostics: Option<Arc<dyn DiagnosticsSink>>,
}

impl WebSocketPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        #[cfg(not(target_os = "ios"))]
        let transport = None;
        #[cfg(target_os = "ios")]
        let transport = Some(Arc::new(ios::WebSocket) as Arc<dyn WebSocketTransport>);
        Ok(Self {
            registry: Arc::default(),
            tracker: SocketSlotTracker::default(),
            transport,
            diagnostics: ctx.diagnostics_sink(),
        })
    }
}

#[impasto::export]
impl WebSocketPlugin {
    #[impasto::method(resource)]
    pub fn open(
        &self,
        url: String,
        protocols: Option<Vec<String>>,
    ) -> Result<NativeWebSocketHandle, SocketError> {
        let session = SocketSession::open(
            url,
            protocols.unwrap_or_default(),
            &self.tracker,
            self.transport.clone(),
            self.diagnostics.clone(),
        )
        .map_err(SocketError)?;
        self.registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(session.id(), Arc::clone(&session));
        Ok(NativeWebSocketHandle {
            session,
            registry: Arc::clone(&self.registry),
        })
    }

    pub async fn events(
        &self,
        id: u64,
        sink: TypedSink<NativeWebSocketEvent>,
        _cancel: CancelToken,
    ) -> Result<(), SocketError> {
        let session = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| SocketError("unknown or closed WebSocket handle".into()))?;
        session.start_stream(sink).await.map_err(SocketError)
    }
}

/// Registers the native WebSocket API for the current application.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(WebSocketPlugin::plugin());
}
