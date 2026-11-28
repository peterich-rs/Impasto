pub(crate) mod frames;
pub(crate) mod network;
pub(crate) mod resources;
pub(crate) mod views;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use devtools_protocol::{Channel, ChannelCursors, ChannelUpdate, FrameRecord, SessionInfo};
use devtools_symbols::SymbolTable;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analyzer::{audits, treemap::Bundle};
use crate::discovery::TargetSummary;
use crate::types::{TickSummary, id, string};

pub(crate) const CHANNELS: [(Channel, &str); 7] = [
    (Channel::Frames, "frames"),
    (Channel::Repaints, "repaints"),
    (Channel::Network, "network"),
    (Channel::Logs, "logs"),
    (Channel::Errors, "errors"),
    (Channel::Runtime, "runtime"),
    (Channel::Resources, "resources"),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    #[default]
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Offline,
}

impl ConnectionState {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Offline => "offline_snapshot",
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Recording {
    pub info: Option<SessionInfo>,
    pub frames: frames::Frames,
    pub repaints: BTreeMap<u64, Value>,
    pub network: BTreeMap<u64, Value>,
    pub logs: VecDeque<Value>,
    pub errors: VecDeque<Value>,
    pub runtime: VecDeque<Value>,
    pub resources: VecDeque<Value>,
    pub resource_samples: VecDeque<resources::Sample>,
    pub stacks: VecDeque<Value>,
    pub spans: BTreeMap<u64, Value>,
    pub view: views::ViewIndex,
    pub layouts: Value,
    pub leaderboard: Value,
    pub layers: Value,
    pub memory: Value,
    pub async_resources: Value,
    pub gc: VecDeque<Value>,
    pub ws: BTreeMap<u64, VecDeque<Value>>,
    pub bundle: Option<Bundle>,
    pub capture: Value,
    pub resolved_errors: BTreeMap<String, String>,
}

impl Recording {
    pub(crate) fn frame(&self, id: u64) -> Option<std::borrow::Cow<'_, FrameRecord>> {
        if let Some(frame) = self.frames.get(id) {
            return Some(std::borrow::Cow::Borrowed(frame));
        }
        let item = self.captured_frame(id)?;
        let mut full = serde_json::to_value(FrameRecord::default()).ok()?;
        full.as_object_mut()?.extend(item.as_object()?.clone());
        let record = serde_json::from_value(full).ok()?;
        Some(std::borrow::Cow::Owned(record))
    }

    fn captured_frame(&self, id: u64) -> Option<&Value> {
        self.spans
            .get(&id)
            .and_then(|span| span.get("frame"))
            .or_else(|| {
                self.capture["frames"]
                    .as_array()?
                    .iter()
                    .find(|frame| frame["id"].as_u64() == Some(id))
            })
    }

    pub(crate) fn frame_revision(&self, id: u64) -> Option<u64> {
        self.frames.get(id).map(|frame| frame.revision).or_else(|| {
            Some(
                self.captured_frame(id)?["revision"]
                    .as_u64()
                    .unwrap_or_default(),
            )
        })
    }

    pub(crate) fn frame_budget(&self) -> f64 {
        frames::refresh_budget(self.info.as_ref().and_then(|info| info.refresh_rate))
    }

    pub(crate) fn clip(&mut self) {
        self.frames.restore();
        if self.resource_samples.is_empty() {
            let (first, second) = self.frames.entries().as_slices();
            resources::note(&mut self.resource_samples, first);
            resources::note(&mut self.resource_samples, second);
        }
        trim_map(&mut self.repaints, 480);
        trim_map(&mut self.network, 2000);
        trim_map(&mut self.spans, 96);
        trim_map(&mut self.ws, 32);
        for frames in self.ws.values_mut() {
            trim(frames, 2000);
        }
        trim(&mut self.logs, 4000);
        trim(&mut self.errors, 500);
        trim(&mut self.runtime, 300);
        trim(&mut self.resources, 300);
        trim(&mut self.resource_samples, 300);
        trim(&mut self.stacks, 8192);
        trim(&mut self.gc, 16);
        self.view.reindex();
    }
}

pub(crate) enum Incoming {
    Frames {
        update: ChannelUpdate<FrameRecord>,
        sampled_at_ms: f64,
    },
    Channel(Channel, ChannelUpdate<Value>),
}

pub(crate) fn decode_batch(batch: Value) -> Vec<Incoming> {
    let Value::Object(updates) = batch else {
        return Vec::new();
    };
    updates
        .into_iter()
        .filter_map(|(name, update)| {
            let &(channel, _) = CHANNELS.iter().find(|(_, key)| *key == name)?;
            if channel == Channel::Frames {
                let sampled_at_ms = update["sampled_at_ms"].as_f64()?;
                let update = serde_json::from_value(update).ok()?;
                Some(Incoming::Frames {
                    update,
                    sampled_at_ms,
                })
            } else {
                serde_json::from_value(update)
                    .ok()
                    .map(|update| Incoming::Channel(channel, update))
            }
        })
        .collect()
}

fn advance<T>(cursor: &mut Option<u64>, update: &ChannelUpdate<T>) -> Option<Option<u64>> {
    if cursor.is_some_and(|previous| {
        update.revision < previous || update.revision == previous && !update.reset
    }) {
        return None;
    }
    Some(cursor.replace(update.revision))
}

pub(crate) type Shared = Arc<RwLock<Store>>;

#[derive(Default)]
pub(crate) struct Store {
    pub generation: u64,
    clock: Option<(f64, Instant)>,
    frame_clock: Option<(f64, Instant)>,
    pub sequence: u64,
    pub state: ConnectionState,
    pub paused: bool,
    pub target_id: Option<String>,
    pub targets: Vec<TargetSummary>,
    pub capabilities: Vec<String>,
    pub message: Option<String>,
    pub reconnect_attempt: u32,
    pub reconnected_new_pid: bool,
    pub rtt_ms: Option<f64>,
    pub profile: String,
    pub capture_id: Option<u64>,
    pub capture_sampler: bool,
    pub sampler_enabled: bool,
    pub picked: Option<(String, String)>,
    pub project_root: Option<PathBuf>,
    pub framework_root: Option<PathBuf>,
    pub symbols: Option<Arc<SymbolTable>>,
    pub data: Recording,
    pub findings: Vec<audits::Finding>,
    pub bodies: network::Bodies,
    pub trace: Option<(u64, Arc<[u8]>)>,
    pub imported_trace: Option<Arc<[u8]>>,
    pub audits_rev: u32,
    session_rev: u32,
    pub targets_rev: u32,
    pub inspecting_self: bool,
    network_records_rev: u32,
    revisions: [u32; 7],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameWindowKey {
    idle: bool,
    count: usize,
    window_step: u64,
}

impl Store {
    fn observe_time(&mut self, time: f64) {
        if time.is_finite() && self.clock.is_none_or(|(current, _)| time > current) {
            self.clock = Some((time, Instant::now()));
        }
    }

    pub(crate) fn now_ms(&self) -> f64 {
        self.clock.map_or_else(
            || {
                self.data
                    .frames
                    .entries()
                    .back()
                    .map_or(0., |record| record.at_ms)
                    .max(
                        self.data
                            .network
                            .values()
                            .map(|record| crate::types::number(record, "at_ms"))
                            .fold(0., f64::max),
                    )
            },
            |(time, since)| {
                time + if self.state == ConnectionState::Connected && !self.paused {
                    since.elapsed().as_secs_f64() * 1000.
                } else {
                    0.
                }
            },
        )
    }

    pub(crate) fn profile_revision(&self) -> (u32, u32) {
        (self.revisions[0], self.revisions[6])
    }

    fn live_frame_now(&self) -> Option<f64> {
        (self.state == ConnectionState::Connected && !self.paused).then(|| {
            self.frame_clock.map_or(0., |(at_ms, received)| {
                at_ms + received.elapsed().as_secs_f64() * 1_000.
            })
        })
    }

    fn frames_idle_at(&self, now: f64) -> bool {
        let last_presented = self
            .data
            .frames
            .entries()
            .iter()
            .filter(|frame| frame.presented)
            .map(|frame| frame.at_ms)
            .max_by(f64::total_cmp);
        frames::is_idle(now, last_presented)
    }

    pub(crate) fn frame_window_key(&self) -> Option<FrameWindowKey> {
        let now = self.live_frame_now()?;
        let count = self
            .data
            .frames
            .entries()
            .iter()
            .filter(|frame| frame.at_ms >= now - frames::LIVE_WINDOW_MS)
            .count();
        let window_step = (self.live_window_ms(now) / 250.) as u64;
        Some(FrameWindowKey {
            idle: self.frames_idle_at(now),
            count,
            window_step,
        })
    }

    fn live_window_ms(&self, now: f64) -> f64 {
        let first = self
            .data
            .frames
            .entries()
            .iter()
            .map(|frame| frame.at_ms)
            .min_by(f64::total_cmp)
            .unwrap_or(now);
        (now - first).clamp(250., frames::LIVE_WINDOW_MS)
    }

    pub(crate) fn set_session_info(&mut self, info: SessionInfo) {
        if self.data.info.as_ref() != Some(&info) {
            self.data.info = Some(info);
            self.session_rev = self.session_rev.wrapping_add(1);
            self.touch();
        }
    }

    pub(crate) fn set_capabilities(&mut self, capabilities: Vec<String>) {
        if self.capabilities != capabilities {
            self.capabilities = capabilities;
            self.session_rev = self.session_rev.wrapping_add(1);
            self.touch();
        }
    }

    pub(crate) fn clear_frame_clock(&mut self) {
        self.frame_clock = None;
    }

    pub(crate) fn touch(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
    }

    pub(crate) fn changed(&mut self, channel: Channel) {
        if channel == Channel::Network {
            self.network_records_rev = self.network_records_rev.wrapping_add(1);
        }
        self.invalidate_channel(channel);
    }

    pub(crate) fn refresh_network_ages(&mut self) {
        self.invalidate_channel(Channel::Network);
    }

    fn invalidate_channel(&mut self, channel: Channel) {
        if let Some(index) = CHANNELS.iter().position(|(key, _)| *key == channel) {
            self.revisions[index] = self.revisions[index].wrapping_add(1);
        }
        self.touch();
    }

    pub(crate) fn replace(&mut self, data: Recording) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            sequence: self.sequence.wrapping_add(1),
            audits_rev: self.audits_rev.wrapping_add(1),
            session_rev: self.session_rev.wrapping_add(1),
            targets_rev: self.targets_rev,
            targets: std::mem::take(&mut self.targets),
            data,
            ..Default::default()
        };
    }

    pub(crate) fn ingest(
        &mut self,
        batch: Vec<Incoming>,
        cursors: &mut ChannelCursors,
        mut resolved: BTreeMap<String, String>,
    ) {
        if self.paused {
            return;
        }
        for update in batch {
            match update {
                Incoming::Frames {
                    update,
                    sampled_at_ms,
                } => {
                    if advance(cursors.cursor(Channel::Frames), &update).is_some() {
                        self.observe_time(sampled_at_ms);
                        if update.reset
                            || self
                                .frame_clock
                                .is_none_or(|(at_ms, _)| sampled_at_ms > at_ms)
                        {
                            self.frame_clock = Some((sampled_at_ms, Instant::now()));
                        }
                        resources::note(&mut self.data.resource_samples, &update.entries);
                        self.data.frames.update(update.reset, update.entries);
                        self.changed(Channel::Frames);
                    }
                }
                Incoming::Channel(channel, mut update) => {
                    let Some(previous) = advance(cursors.cursor(channel), &update) else {
                        continue;
                    };
                    if !update.reset
                        && let Some(previous) = previous
                    {
                        // A subscribe reply may overlap notifications already read from the old subscription.
                        let sequence = if channel == Channel::Network {
                            "revision"
                        } else {
                            "seq"
                        };
                        update.entries.retain(|entry| {
                            entry[sequence].as_u64().is_none_or(|seq| seq > previous)
                        });
                    }
                    if let Some(time) = update
                        .entries
                        .iter()
                        .filter_map(|entry| entry["at_ms"].as_f64())
                        .max_by(f64::total_cmp)
                    {
                        self.observe_time(time);
                    }
                    self.update(channel, update, &mut resolved);
                    self.changed(channel);
                }
            }
        }
    }

    fn update(
        &mut self,
        channel: Channel,
        update: ChannelUpdate<Value>,
        resolved: &mut BTreeMap<String, String>,
    ) {
        let data = &mut self.data;
        if update.reset {
            match channel {
                Channel::Repaints => data.repaints.clear(),
                Channel::Network => data.network.clear(),
                Channel::Logs => data.logs.clear(),
                Channel::Errors => {
                    data.errors.clear();
                    data.resolved_errors.clear();
                }
                Channel::Runtime => data.runtime.clear(),
                Channel::Resources => {
                    data.resources.clear();
                    data.stacks.clear();
                }
                Channel::Frames => {}
            }
        }
        for entry in update.entries {
            match channel {
                Channel::Repaints => {
                    if let Some(pick) = entry.get("view.picked")
                        && pick["final"] == true
                        && let Some(node) = pick["node_id"].as_str()
                    {
                        self.picked = Some((id(&entry["seq"]), node.to_owned()));
                    }
                    if let Some(frame) = entry["repaints"]["frame_number"].as_u64() {
                        data.repaints.insert(frame, entry["repaints"].clone());
                    }
                }
                Channel::Network => {
                    if let Some(token) = entry["token"].as_u64() {
                        data.network.insert(token, entry);
                    }
                }
                Channel::Logs => data.logs.push_back(entry),
                Channel::Errors => {
                    let seq = id(&entry["seq"]);
                    if let Some(stack) = resolved.remove(&seq) {
                        data.resolved_errors.insert(seq, stack);
                    }
                    data.errors.push_back(entry);
                }
                Channel::Runtime => {
                    if let Some(detail) = entry["value"].get("detail") {
                        data.memory = detail.clone();
                    }
                    data.runtime.push_back(entry);
                }
                Channel::Resources => match string(&entry["value"], "kind") {
                    "async" => {
                        data.async_resources = entry["value"]["detail"].clone();
                        data.resources.push_back(entry);
                    }
                    "stack" | "js_stack_samples" => data.stacks.push_back(entry),
                    _ => data.resources.push_back(entry),
                },
                Channel::Frames => {}
            }
        }
        trim_map(&mut data.repaints, 480);
        trim_map(&mut data.network, 2000);
        trim(&mut data.logs, 4000);
        while data.errors.len() > 500 {
            if let Some(error) = data.errors.pop_front() {
                data.resolved_errors.remove(&id(&error["seq"]));
            }
        }
        trim(&mut data.runtime, 300);
        trim(&mut data.resources, 300);
        trim(&mut data.stacks, 8192);
    }

    pub(crate) fn install_symbols(&mut self, symbols: Arc<SymbolTable>) {
        let resolver = symbols.resolver();
        self.data.resolved_errors = self
            .data
            .errors
            .iter()
            .map(|entry| (id(&entry["seq"]), resolver.remap(string(entry, "stack"))))
            .collect();
        self.symbols = Some(symbols);
        self.changed(Channel::Errors);
        self.changed(Channel::Resources);
    }

    pub(crate) fn tick(&self) -> TickSummary {
        let data = &self.data;
        let [
            frames_rev,
            repaints_rev,
            network_rev,
            logs_rev,
            errors_rev,
            runtime_rev,
            resources_rev,
        ] = self.revisions.map(f64::from);
        let frames = data.frames.entries();
        let stats = if let Some(now) = self.live_frame_now() {
            frames::live_stats(
                frames
                    .iter()
                    .filter(|frame| frame.at_ms >= now - frames::LIVE_WINDOW_MS),
                self.frames_idle_at(now),
                self.live_window_ms(now),
                data.frame_budget(),
            )
        } else {
            let end = frames.iter().map(|frame| frame.at_ms).fold(0., f64::max);
            frames::stats(
                frames
                    .iter()
                    .filter(|frame| frame.at_ms >= end - frames::LIVE_WINDOW_MS),
                data.frame_budget(),
            )
        };
        let count = |len| u32::try_from(len).unwrap_or(u32::MAX);
        TickSummary {
            generation: self.generation.to_string(),
            state: self.state.name().to_owned(),
            session: data.info.as_ref().map(Into::into),
            targets: self.targets.clone(),
            target_id: self.target_id.clone(),
            capabilities: self.capabilities.clone(),
            message: self.message.clone(),
            reconnect_attempt: self.reconnect_attempt,
            reconnected_new_pid: self.reconnected_new_pid,
            paused: self.paused,
            rtt_ms: self.rtt_ms,
            profile: self.profile.clone(),
            frames_rev,
            repaints_rev,
            network_rev,
            network_records_rev: f64::from(self.network_records_rev),
            logs_rev,
            errors_rev,
            runtime_rev,
            resources_rev,
            audits_rev: f64::from(self.audits_rev),
            session_rev: f64::from(self.session_rev),
            targets_rev: f64::from(self.targets_rev),
            inspecting_self: self.inspecting_self,
            frame_count: count(frames.len()),
            latest_frame: frames.back().map(|frame| frame.id.to_string()),
            network_count: count(data.network.len()),
            log_count: count(data.logs.len()),
            error_count: count(data.errors.len()),
            latest_error_seq: data.errors.back().map(|entry| id(&entry["seq"])),
            active_net_count: count(
                data.network
                    .values()
                    .filter(|record| network::active(record))
                    .count(),
            ),
            js_heap_bytes: data.memory["memory_used_size"].as_f64().unwrap_or_default(),
            stats,
            audit_count: count(self.findings.len()),
            picked_node_id: self.picked.as_ref().map(|(_, node)| node.clone()),
            picked_seq: self.picked.as_ref().map(|(seq, _)| seq.clone()),
            active_capture: self.capture_id.map(|id| id.to_string()),
            latest_capture: data.capture["id"].as_u64().map(|id| id.to_string()),
            bundle_loaded: data.bundle.is_some(),
            symbols_loaded: self.symbols.is_some(),
        }
    }
}

pub(crate) fn trim<T>(entries: &mut VecDeque<T>, capacity: usize) {
    entries.drain(..entries.len().saturating_sub(capacity));
}

pub(crate) fn trim_map<K: Ord, T>(entries: &mut BTreeMap<K, T>, capacity: usize) {
    while entries.len() > capacity {
        entries.pop_first();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cross_channel_timestamps_do_not_age_the_live_frame_window() {
        let mut store = Store {
            state: ConnectionState::Connected,
            ..Store::default()
        };
        let frame = |id, at_ms| FrameRecord {
            id,
            at_ms,
            presented: true,
            ..FrameRecord::default()
        };
        let mut cursors = ChannelCursors::default();
        store.ingest(
            vec![Incoming::Frames {
                update: ChannelUpdate {
                    revision: 1,
                    reset: true,
                    entries: vec![frame(1, 1_000.), frame(2, 1_010.)],
                },
                sampled_at_ms: 1_010.,
            }],
            &mut cursors,
            BTreeMap::new(),
        );
        store.observe_time(80_000.);
        assert!(store.now_ms() >= 80_000.);
        let stats = store.tick().stats;
        assert!(!stats.idle);
        assert_eq!(stats.present_rate_hz, 8.);
    }

    #[test]
    fn attaching_to_an_idle_target_does_not_treat_old_frames_as_live() {
        let mut store = Store {
            state: ConnectionState::Connected,
            ..Store::default()
        };
        let mut cursors = ChannelCursors::default();
        store.ingest(
            vec![Incoming::Frames {
                update: ChannelUpdate {
                    revision: 1,
                    reset: true,
                    entries: vec![FrameRecord {
                        id: 1,
                        at_ms: 500.,
                        presented: true,
                        ..FrameRecord::default()
                    }],
                },
                sampled_at_ms: 10_000.,
            }],
            &mut cursors,
            BTreeMap::new(),
        );
        let stats = store.tick().stats;
        assert!(stats.idle);
        assert_eq!(stats.count, 0);
        assert_eq!(store.data.frames.entries().len(), 1);
    }

    #[test]
    fn refreshing_network_ages_does_not_invalidate_overview_network_records() {
        let mut store = Store::default();
        store.changed(Channel::Network);
        let before = store.tick();
        store.refresh_network_ages();
        let after = store.tick();
        assert_ne!(before.network_rev, after.network_rev);
        assert_eq!(before.network_records_rev, after.network_records_rev);
    }

    #[test]
    fn overlapping_subscription_replies_do_not_repeat_or_rewind_streams() {
        let mut store = Store::default();
        let mut cursors = ChannelCursors::default();
        for update in [
            json!({"logs":{"revision":2,"reset":true,"entries":[{"seq":1},{"seq":2}]} }),
            json!({"logs":{"revision":3,"reset":false,"entries":[{"seq":2},{"seq":3}]} }),
            json!({"logs":{"revision":2,"reset":false,"entries":[{"seq":2}]} }),
        ] {
            store.ingest(decode_batch(update), &mut cursors, BTreeMap::new());
        }
        assert_eq!(cursors.logs, Some(3));
        let sequences = store
            .data
            .logs
            .iter()
            .filter_map(|entry| entry["seq"].as_u64())
            .collect::<Vec<_>>();
        assert_eq!(sequences, [1, 2, 3]);
    }
}
