use std::sync::Mutex;

use devtools_protocol::{Channel, ChannelCursors, ChannelUpdate};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::hub::{Ring, SeqField};
use super::{TelemetryHub, remote_frame};

#[derive(Deserialize)]
pub(super) struct Subscribe {
    channels: Vec<Channel>,
    #[serde(default)]
    cursors: ChannelCursors,
}

#[derive(Default)]
pub(super) struct Subscription {
    channels: Vec<Channel>,
    cursors: ChannelCursors,
    revision: Option<u64>,
}

impl Subscription {
    pub(super) fn subscribe(&mut self, request: Subscribe, hub: &TelemetryHub) -> Value {
        self.channels.clear();
        for channel in request.channels {
            if !self.channels.contains(&channel) {
                self.channels.push(channel);
            }
        }
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
                Channel::Repaints => ("repaints", empty(since)),
                Channel::Runtime => ("runtime", empty(since)),
                Channel::Resources => ("resources", empty(since)),
            };
            if initial || update.reset || Some(update.revision) != since {
                *cursor = Some(update.revision);
                batch.insert(name.to_owned(), json!(update));
            }
        }
        Value::Object(batch)
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

fn empty(since: Option<u64>) -> ChannelUpdate<Value> {
    ChannelUpdate {
        revision: 0,
        reset: since != Some(0),
        entries: Vec::new(),
    }
}
