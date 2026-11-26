//! Projection of package telemetry into the host diagnostics channels.

use impasto::abi::PluginValue;
use impasto::{Bytes, FromPluginValue, ImpastoType};

use super::{NetEntry, NetKind, TelemetryHub};

#[derive(ImpastoType)]
struct NetworkRecord {
    token: String,
    method: String,
    url: String,
    request_headers: Option<Vec<(String, String)>>,
    request_body: Option<Bytes>,
    status: Option<u16>,
    response_headers: Option<Vec<(String, String)>>,
    response_bytes: Option<u64>,
    response_preview: Option<Bytes>,
    error: Option<String>,
    elapsed_ms: Option<f64>,
}

pub(super) fn network(hub: &TelemetryHub, source: &str, value: PluginValue) {
    let kind = match source {
        "http" => NetKind::Stream,
        "websocket" => NetKind::Socket,
        _ => return,
    };
    let Ok(record) = NetworkRecord::from_value(value) else {
        return;
    };
    let Ok(token) = record.token.parse::<u64>() else {
        return;
    };
    let now = crate::timing::mono_ms();
    hub.record_net(NetEntry {
        seq: 0,
        at_ms: now - record.elapsed_ms.unwrap_or_default(),
        kind,
        token,
        method: record.method,
        url: record.url,
        request_headers: record.request_headers.unwrap_or_default(),
        request_body: record.request_body.map(|data| data.0),
        status: record.status,
        response_headers: record.response_headers.unwrap_or_default(),
        response_bytes: record
            .response_bytes
            .and_then(|value| usize::try_from(value).ok()),
        response_preview: record.response_preview.map(|data| data.0),
        error: record.error,
        done_ms: Some(now),
    });
}
