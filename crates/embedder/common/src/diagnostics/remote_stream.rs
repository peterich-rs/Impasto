use std::sync::{Arc, Mutex};

use devtools_protocol::{Channel, ChannelCursors, ChannelUpdate};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::hub::{Ring, SeqField};
use super::{TelemetryHub, remote_frame, remote_views};

#[derive(Deserialize)]
pub(super) struct Subscribe {
    channels: Vec<Channel>,
    #[serde(default)]
    cursors: ChannelCursors,
}

pub(super) struct Subscription {
    hub: Arc<TelemetryHub>,
    repaint_requested: bool,
    repaint_active: bool,
    channels: Vec<Channel>,
    cursors: ChannelCursors,
    revision: Option<u64>,
}

impl Subscription {
    pub(super) fn new(hub: Arc<TelemetryHub>) -> Self {
        Self {
            hub,
            channels: Vec::new(),
            cursors: ChannelCursors::default(),
            revision: None,
            repaint_requested: false,
            repaint_active: false,
        }
    }

    pub(super) fn set_repaint_tracking(&mut self, enabled: bool) {
        self.repaint_requested = enabled;
        self.sync_repaints();
    }

    fn sync_repaints(&mut self) {
        let active = self.repaint_requested || self.channels.contains(&Channel::Repaints);
        if self.repaint_active != active {
            self.hub.repaint_reader(active);
            self.repaint_active = active;
        }
    }

    pub(super) fn subscribe(&mut self, request: Subscribe, hub: &TelemetryHub) -> Value {
        for channel in [Channel::Runtime, Channel::Resources] {
            let previous = self.channels.contains(&channel);
            let next = request.channels.contains(&channel);
            if previous != next {
                self.hub.runtime_reader(channel, next);
            }
        }
        self.channels.clear();
        for channel in request.channels {
            if !self.channels.contains(&channel) {
                self.channels.push(channel);
            }
        }
        self.sync_repaints();
        self.cursors = request.cursors;
        self.revision = Some(hub.revision());
        self.collect(hub, true)
    }

    pub(super) fn update(&mut self, hub: &TelemetryHub) -> Option<Value> {
        let revision = hub.revision();
        if self.channels.is_empty() || self.revision == Some(revision) {
            return None;
        }
        self.revision = Some(revision);
        let Value::Object(channels) = self.collect(hub, false) else {
            return None;
        };
        (!channels.is_empty()).then(|| {
            json!({
                "jsonrpc": "2.0", "method": "stream.batch", "params": channels,
            })
        })
    }

    fn collect(&mut self, hub: &TelemetryHub, initial: bool) -> Value {
        let mut batch = Map::new();
        for &channel in &self.channels {
            let cursor = self.cursors.cursor(channel);
            let since = *cursor;
            let (name, update) = match channel {
                Channel::Frames => (
                    "frames",
                    changes(&hub.frames, since, |frame| {
                        json!(remote_frame::record(frame))
                    }),
                ),
                Channel::Network => (
                    "network",
                    changes(&hub.network, since, |entry| {
                        json!({
                            "seq": entry.seq, "at_ms": entry.at_ms, "kind": entry.kind.name(),
                            "token": entry.token, "method": entry.method, "url": entry.url,
                        "revision": entry.changed_at, "initiator": entry.initiator, "state": entry.state,
                        "final_url": entry.final_url, "redirected": entry.redirected, "head_elapsed_ms": entry.head_elapsed_ms,
                        "timing": entry.timing, "connection": entry.connection, "redirects": entry.redirects,
                        "request_body_preview": entry.request_body, "response_preview": entry.response_preview,
                        "request_body_truncated": entry.request_body_truncated, "request_body_evicted": entry.request_body_evicted,
                        "response_body_truncated": entry.response_body_truncated, "response_body_evicted": entry.response_body_evicted,
                        "ws_sent": entry.ws_sent, "ws_received": entry.ws_received,
                        "ws_bytes_sent": entry.ws_bytes_sent, "ws_bytes_received": entry.ws_bytes_received,
                            "request_headers": entry.request_headers, "response_headers": entry.response_headers,
                            "status": entry.status, "response_bytes": entry.response_bytes,
                            "error": entry.error, "done_ms": entry.done_ms,
                        })
                    }),
                ),
                Channel::Logs => (
                    "logs",
                    changes(&hub.logs, since, |entry| {
                        json!({
                            "seq": entry.seq, "at_ms": entry.at_ms, "level": entry.level.as_str(),
                            "source": entry.source.name(), "target": entry.target, "message": entry.message,
                            "fields": entry.fields, "frame": entry.frame,
                        })
                    }),
                ),
                Channel::Errors => (
                    "errors",
                    changes(&hub.errors, since, |entry| {
                        json!({
                            "seq": entry.seq, "at_ms": entry.at_ms, "source": entry.source,
                            "message": entry.message, "stack": entry.stack, "frame": entry.frame,
                        })
                    }),
                ),
                Channel::Repaints => (
                    "repaints",
                    changes(&hub.repaints, since, |entry| {
                        json!({
                            "seq": entry.seq,
                            "repaints": entry.batch.as_deref().map(remote_views::repaints),
                            "view.picked": entry.picked.as_ref().map(remote_views::picked),
                        })
                    }),
                ),
                Channel::Runtime => ("runtime", changes(&hub.runtime, since, runtime)),
                Channel::Resources => ("resources", changes(&hub.resources, since, runtime)),
            };
            if initial || update.reset || Some(update.revision) != since {
                *cursor = Some(update.revision);
                batch.insert(name.to_owned(), json!(update));
            }
        }
        Value::Object(batch)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        for channel in [Channel::Runtime, Channel::Resources] {
            if self.channels.contains(&channel) {
                self.hub.runtime_reader(channel, false);
            }
        }
        if self.repaint_active {
            self.hub.repaint_reader(false);
        }
    }
}

fn changes<T: Clone + SeqField>(
    ring: &Mutex<Ring<T>>,
    since: Option<u64>,
    format: impl Fn(&T) -> Value,
) -> ChannelUpdate<Value> {
    let update = ring
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .updates(since);
    ChannelUpdate {
        revision: update.revision,
        reset: update.reset,
        entries: update.entries.iter().map(format).collect(),
    }
}

fn runtime(entry: &super::hub::RuntimeUpdate) -> Value {
    json!({"seq": entry.seq, "at_ms": entry.at_ms, "value": entry.observation.as_ref()})
}
