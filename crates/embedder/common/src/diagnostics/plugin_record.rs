//! Projection of package telemetry into the host diagnostics channels.

use impasto::abi::PluginValue;
use impasto::network::NetworkEvent;
use impasto::{Bytes, FromPluginValue, ImpastoType};

use super::hub::{NetState, WsFrame};
use super::net_bodies::BodySide;
use super::{NetEntry, NetKind, TelemetryHub};

pub(super) fn network(hub: &TelemetryHub, source: &str, event: &str, value: PluginValue) {
    let kind = match source {
        "http" => NetKind::Stream,
        "websocket" => NetKind::Socket,
        _ => return,
    };
    let Ok(mut record) = NetworkEvent::from_value(value) else {
        return;
    };
    let Ok(token) = record.token.parse::<u64>() else {
        return;
    };
    let now = crate::timing::mono_ms();
    let start = event == "network.start" || event == "network";
    let finish = event == "network.finish" || event == "network";
    if let Some(data) = record.request_body_full.take() {
        hub.note_body(token, BodySide::Request, data.0);
    }
    if let Some(data) = record.response_body_full.take() {
        hub.note_body(token, BodySide::Response, data.0);
    }
    let apply = |entry: &mut NetEntry| {
        if let Some(status) = record.status {
            entry.status = Some(status);
        }
        if let Some(headers) = record.response_headers {
            entry.response_headers = headers;
        }
        if let Some(url) = record.final_url {
            entry.final_url = Some(url);
        }
        if let Some(redirected) = record.redirected {
            entry.redirected = redirected;
        }
        if let Some(elapsed) = record.head_elapsed_ms {
            entry.head_elapsed_ms = Some(elapsed);
        }
        if finish {
            if kind == NetKind::Socket && entry.done_ms.is_some() {
                return;
            }
            entry.response_bytes = record
                .response_bytes
                .and_then(|size| usize::try_from(size).ok());
            entry.response_preview = record.response_preview.map(|bytes| bytes.0);
            entry.response_body_truncated = record.response_body_truncated.unwrap_or_default();
            entry.error = record.error;
            entry.done_ms = Some(now);
            entry.timing = record.timing;
            entry.connection = record.connection;
            entry.redirects = record.redirects.unwrap_or_default();
            entry.state = if record.canceled == Some(true) {
                NetState::Canceled
            } else if entry.error.is_some() {
                NetState::Failed
            } else {
                NetState::Completed
            };
        } else if !start {
            entry.state = NetState::Streaming;
        }
    };
    if start {
        let mut entry = NetEntry {
            at_ms: now - record.elapsed_ms.unwrap_or_default(),
            kind,
            token,
            method: record.method.unwrap_or_default(),
            url: record.url.unwrap_or_default(),
            initiator: record.initiator.unwrap_or_else(|| source.to_owned()),
            request_headers: record.request_headers.unwrap_or_default(),
            request_body: record.request_body.map(|bytes| bytes.0),
            request_body_truncated: record.request_body_truncated.unwrap_or_default(),
            ..Default::default()
        };
        apply(&mut entry);
        hub.record_net(entry);
    } else {
        hub.update_net(token, apply);
    }
}

#[derive(ImpastoType)]
struct SocketRecord {
    token: String,
    direction: String,
    opcode: String,
    bytes_len: u64,
    preview: Option<Bytes>,
    body: Option<Bytes>,
    error: Option<String>,
}

pub(super) fn websocket(hub: &TelemetryHub, value: PluginValue) {
    let Ok(record) = SocketRecord::from_value(value) else {
        return;
    };
    let Ok(token) = record.token.parse::<u64>() else {
        return;
    };
    let at_ms = crate::timing::mono_ms();
    hub.update_net(token, |entry| match record.opcode.as_str() {
        "open" => entry.state = NetState::Streaming,
        "close" => {
            entry.done_ms = Some(at_ms);
            if entry.state != NetState::Failed {
                entry.state = NetState::Completed;
            }
        }
        "error" => {
            entry.done_ms = Some(at_ms);
            entry.state = NetState::Failed;
            entry.error.clone_from(&record.error);
        }
        "text" | "binary" if record.direction == "out" => {
            entry.ws_sent = entry.ws_sent.saturating_add(1);
            entry.ws_bytes_sent = entry.ws_bytes_sent.saturating_add(record.bytes_len);
        }
        "text" | "binary" => {
            entry.ws_received = entry.ws_received.saturating_add(1);
            entry.ws_bytes_received = entry.ws_bytes_received.saturating_add(record.bytes_len);
        }
        _ => {}
    });
    hub.record_ws(
        WsFrame {
            seq: 0,
            token,
            at_ms,
            direction: record.direction,
            opcode: record.opcode,
            bytes_len: record.bytes_len,
            preview: record.preview.map(|bytes| bytes.0).unwrap_or_default(),
            error: record.error,
        },
        record.body.map(|bytes| bytes.0),
    );
}
