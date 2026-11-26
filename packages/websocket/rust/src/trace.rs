use impasto::{Bytes, DiagnosticsSink, ImpastoType, IntoPluginValue};

use crate::socket::WebSocketEvent;

#[derive(ImpastoType)]
struct Entry {
    token: String,
    method: String,
    url: String,
    response_bytes: Option<u64>,
    response_preview: Option<Bytes>,
    error: Option<String>,
}

pub(crate) fn record(sink: &dyn DiagnosticsSink, id: u64, url: &str, event: &WebSocketEvent) {
    let (method, error, data) = match event {
        WebSocketEvent::Open { .. } => ("WS OPEN", None, None),
        WebSocketEvent::Message { data, .. } => ("WS MSG", None, Some(data)),
        WebSocketEvent::Close { code, reason, .. } => (
            "WS CLOSE",
            (!reason.is_empty()).then(|| format!("{code} {reason}")),
            None,
        ),
        WebSocketEvent::Error(error) => ("WS ERROR", Some(error.clone()), None),
    };
    let entry = Entry {
        token: id.to_string(),
        method: method.into(),
        url: url.into(),
        response_bytes: data.and_then(|data| u64::try_from(data.len()).ok()),
        response_preview: data.map(|data| Bytes(data.iter().copied().take(2048).collect())),
        error,
    };
    if let Ok(payload) = entry.into_value() {
        sink.record("websocket", "network", payload);
    }
}
