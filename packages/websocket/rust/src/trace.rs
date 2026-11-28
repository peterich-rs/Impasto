use impasto::network::NetworkEvent;
use std::sync::Arc;

use impasto::{Bytes, CancelToken, DiagnosticsSink, ImpastoType, IntoPluginValue};

use crate::socket::WebSocketEvent;

#[derive(ImpastoType)]
struct Entry {
    token: String,
    url: String,
    direction: String,
    opcode: String,
    bytes_len: u64,
    preview: Option<Bytes>,
    body: Option<Bytes>,
    error: Option<String>,
}

pub(crate) struct SocketTrace {
    sink: Arc<dyn DiagnosticsSink>,
    id: u64,
    canceled: CancelToken,
}

impl SocketTrace {
    pub(crate) fn start(
        sink: Arc<dyn DiagnosticsSink>,
        id: u64,
        url: &str,
        canceled: CancelToken,
    ) -> Self {
        NetworkEvent {
            token: id.to_string(),
            initiator: Some("websocket".into()),
            method: Some("WS".into()),
            url: Some(url.to_owned()),
            ..Default::default()
        }
        .emit(sink.as_ref(), "websocket", "network.start");
        Self { sink, id, canceled }
    }
}

impl Drop for SocketTrace {
    fn drop(&mut self) {
        NetworkEvent {
            token: self.id.to_string(),
            canceled: Some(self.canceled.is_cancelled()),
            ..Default::default()
        }
        .emit(self.sink.as_ref(), "websocket", "network.finish");
    }
}

pub(crate) fn record(sink: &dyn DiagnosticsSink, id: u64, url: &str, event: &WebSocketEvent) {
    match event {
        WebSocketEvent::Open { .. } => emit(sink, id, url, "in", "open", &[], None),
        WebSocketEvent::Message { data, text } => emit(
            sink,
            id,
            url,
            "in",
            if *text { "text" } else { "binary" },
            data,
            None,
        ),
        WebSocketEvent::Close { code, reason, .. } => emit(
            sink,
            id,
            url,
            "in",
            "close",
            &[],
            (!reason.is_empty()).then(|| format!("{code} {reason}")),
        ),
        WebSocketEvent::Error(error) => {
            emit(sink, id, url, "in", "error", &[], Some(error.clone()))
        }
    }
}

pub(crate) fn outbound(sink: &dyn DiagnosticsSink, id: u64, url: &str, text: bool, bytes: &[u8]) {
    emit(
        sink,
        id,
        url,
        "out",
        if text { "text" } else { "binary" },
        bytes,
        None,
    );
}

fn emit(
    sink: &dyn DiagnosticsSink,
    id: u64,
    url: &str,
    direction: &str,
    opcode: &str,
    bytes: &[u8],
    error: Option<String>,
) {
    let entry = Entry {
        token: id.to_string(),
        url: url.to_owned(),
        direction: direction.to_owned(),
        opcode: opcode.to_owned(),
        bytes_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        preview: (!bytes.is_empty()).then(|| Bytes(bytes[..bytes.len().min(512)].to_vec())),
        body: (!bytes.is_empty() && bytes.len() <= 64 * 1024).then(|| Bytes(bytes.to_vec())),
        error,
    };
    if let Ok(payload) = entry.into_value() {
        sink.record("websocket", "network.ws_frame", payload);
    }
}
