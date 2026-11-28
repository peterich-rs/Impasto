//! Ring buffers with a monotonic revision per channel. Producers are the
//! UI, raster and IO queues, external network callbacks and the log hook.

use super::net_bodies::{BodySide, NetBodyPool};
use super::telemetry::{ScheduleTiming, TelemetryProfile};
use super::timeline::{CapturedSpan, SpanLoss, TimelineRecording, TraceDetail};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

/// One filtered log event copied for the devtools console.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub seq: u64,
    pub at_ms: f64,
    pub level: tracing::Level,
    pub source: LogSource,
    pub target: &'static str,
    pub message: String,
    pub fields: Vec<(&'static str, String)>,
    pub frame: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Script,
    Layout,
    Host,
    Net,
}

impl LogSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Layout => "layout",
            Self::Host => "host",
            Self::Net => "net",
        }
    }

    /// Target prefix from the call site. Unknown targets stay on the host page.
    pub fn from_target(target: &str) -> Self {
        if target.starts_with("impasto::script") {
            Self::Script
        } else if target.starts_with("impasto::layout") {
            Self::Layout
        } else if target.starts_with("impasto::net") {
            Self::Net
        } else {
            Self::Host
        }
    }
}

/// One surfaced failure with a stack, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagError {
    pub seq: u64,
    pub at_ms: f64,
    /// `listener`, `rejection`, `boot`, `engine`, or `host`.
    pub source: &'static str,
    pub message: String,
    pub stack: Option<String>,
    pub frame: Option<u64>,
}

/// One HTTP transfer or WebSocket connection.
#[derive(Clone, Debug, Default)]
pub struct NetEntry {
    pub seq: u64,
    pub(crate) changed_at: u64,
    pub initiator: String,
    pub state: NetState,
    pub final_url: Option<String>,
    pub redirected: bool,
    pub head_elapsed_ms: Option<f64>,
    pub timing: Option<impasto::network::NetworkTiming>,
    pub connection: Option<impasto::network::ConnectionMeta>,
    pub redirects: Vec<impasto::network::RedirectHop>,
    pub request_body_truncated: bool,
    pub response_body_truncated: bool,
    pub request_body_evicted: bool,
    pub response_body_evicted: bool,
    pub ws_sent: u64,
    pub ws_received: u64,
    pub ws_bytes_sent: u64,
    pub ws_bytes_received: u64,
    pub at_ms: f64,
    pub kind: NetKind,
    pub token: u64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<Vec<u8>>,
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub response_bytes: Option<usize>,
    pub response_preview: Option<Vec<u8>>,
    pub error: Option<String>,
    pub done_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetKind {
    #[default]
    Http,
    Stream,
    Socket,
}

/// State of a transfer at the latest channel revision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetState {
    #[default]
    Pending,
    Streaming,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct WsFrame {
    pub seq: u64,
    pub token: u64,
    pub at_ms: f64,
    pub direction: String,
    pub opcode: String,
    pub bytes_len: u64,
    pub preview: Vec<u8>,
    pub error: Option<String>,
}

impl NetKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Stream => "stream",
            Self::Socket => "socket",
        }
    }
}

/// Script and layout phase totals for one frame. The performance page
/// reads this off the newest sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhaseSample {
    pub motion_ms: f64,
    pub dispatch_ms: f64,
    pub timer_ms: f64,
    pub raf_ms: f64,
    pub callback_ms: f64,
    pub flush_ms: f64,
    pub layout_cb_ms: f64,
    pub sync_ms: f64,
    pub semantics_ms: f64,
    pub laywork_ms: f64,
    pub layout_passes: u32,
    pub nodes_created: u32,
    pub ffi_batches: u32,
    pub ffi_calls: u32,
    pub style_resolves: u32,
    pub repaint_boundaries_dirtied: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    pub pointers: u32,
    pub keys: u32,
    pub edits: u32,
    pub ime: u32,
    pub scrolls: u32,
    pub focus: u32,
    pub blur: u32,
    pub canvas_record_ms: f64,
    pub canvas_prepare_ms: f64,
    pub canvas_ops: u32,
    pub canvas_segments: u32,
    pub canvas_redraws: u32,
    pub canvas_reused: u32,
}

/// Per-frame timing sample, trimmed from [`crate::FrameTiming`].
#[derive(Clone, Debug)]
pub struct FrameSample {
    pub repainted_view_count: Option<u32>,
    pub seq: u64,
    pub(crate) changed_at: u64,
    pub schedule: ScheduleTiming,
    pub raster: raster::SubmitStats,
    pub gpu: raster::GpuSample,
    pub gpu_timing: Option<Arc<raster::GpuFrameTiming>>,
    pub updates: u32,
    pub sets: u32,
    pub runtime: Option<engine::session::RuntimeSample>,
    pub number: u64,
    pub at_ms: f64,
    pub work_ms: f64,
    pub budget_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub presented: bool,
    pub present_error: Option<raster::Status>,
    pub layers: u32,
    pub atlas_entries: u32,
    pub pictures: u32,
    pub textures: u32,
    pub shapes_built: u32,
    pub shapes_measure_hit: u32,
    pub shapes_measure_miss: u32,
    pub shapes_paint_hit: u32,
    pub shapes_paint_miss: u32,
    pub atlas_hits: u32,
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layout_cause: &'static str,
    pub gestures: u32,
    pub over_budget: bool,
    pub ui_over_budget: bool,
    pub raster_over_budget: bool,
    pub gpu_over_budget: bool,
    pub phases: PhaseSample,
}

impl FrameSample {
    pub(super) fn input_events(&self) -> u32 {
        let phase = self.phases;
        [
            self.gestures,
            phase.pointers,
            phase.keys,
            phase.edits,
            phase.ime,
            phase.scrolls,
            phase.focus,
            phase.blur,
        ]
        .into_iter()
        .fold(0, u32::saturating_add)
    }

    pub(super) fn gpu_span_ms(&self) -> Option<f64> {
        self.gpu_timing
            .as_ref()
            .map(|timing| timing.frame_span_us / 1_000.0)
    }
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
            repainted_view_count: None,
            seq: 0,
            changed_at: 0,
            schedule: ScheduleTiming::default(),
            raster: raster::SubmitStats::default(),
            gpu: raster::GpuSample::default(),
            gpu_timing: None,
            updates: 0,
            sets: 0,
            runtime: None,
            number: 0,
            at_ms: 0.0,
            work_ms: 0.0,
            budget_ms: 0.0,
            session_ms: 0.0,
            gesture_ms: 0.0,
            script_ms: 0.0,
            layout_ms: 0.0,
            record_ms: 0.0,
            submit_ms: 0.0,
            present_ms: 0.0,
            presented: false,
            present_error: None,
            layers: 0,
            atlas_entries: 0,
            pictures: 0,
            textures: 0,
            shapes_built: 0,
            shapes_measure_hit: 0,
            shapes_measure_miss: 0,
            shapes_paint_hit: 0,
            shapes_paint_miss: 0,
            atlas_hits: 0,
            picture_rebuilt: 0,
            picture_reused: 0,
            layout_cause: "none",
            gestures: 0,
            over_budget: false,
            ui_over_budget: false,
            raster_over_budget: false,
            gpu_over_budget: false,
            phases: PhaseSample::default(),
        }
    }
}

/// Runtime knobs shared with the session bindings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DevtoolsFlags {
    pub layout_scan: bool,
    pub pointer_trace: bool,
    pub suppress_dispatch: bool,
}

pub(crate) struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
    seq: u64,
    discarded_through: u64,
}

impl<T: SeqField> Ring<T> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap,
            seq: 0,
            discarded_through: 0,
        }
    }

    pub(crate) fn push(&mut self, make: impl FnOnce(u64) -> T) {
        self.seq = self.seq.saturating_add(1);
        if self.buf.len() == self.cap
            && let Some(oldest) = self.buf.pop_front()
        {
            self.discarded_through = self.discarded_through.max(oldest.cursor());
        }
        let seq = self.seq;
        self.buf.push_back(make(seq));
    }

    pub(crate) fn revision(&self) -> u64 {
        self.seq
    }

    /// Entries with `seq > since`.
    pub(crate) fn tail(&self, since: u64) -> impl Iterator<Item = &T> {
        self.buf.iter().filter(move |entry| entry.cursor() > since)
    }

    pub(crate) fn clear(&mut self) {
        self.seq = self.seq.saturating_add(1);
        self.discarded_through = self.seq;
        self.buf.clear();
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn updates(&self, since: Option<u64>) -> devtools_protocol::ChannelUpdate<T>
    where
        T: Clone,
    {
        let reset = since.is_none_or(|cursor| cursor < self.discarded_through || cursor > self.seq);
        devtools_protocol::ChannelUpdate {
            revision: self.seq,
            reset,
            entries: self
                .tail(if reset { 0 } else { since.unwrap_or_default() })
                .cloned()
                .collect(),
        }
    }
}

/// The most recent channel revision that changed an entry.
pub(crate) trait SeqField {
    fn cursor(&self) -> u64;
}

macro_rules! seq_field {
    ($ty:ty) => {
        impl SeqField for $ty {
            fn cursor(&self) -> u64 {
                self.seq
            }
        }
    };
}

seq_field!(LogEntry);
seq_field!(DiagError);
impl SeqField for NetEntry {
    fn cursor(&self) -> u64 {
        self.changed_at
    }
}
seq_field!(WsFrame);
impl SeqField for FrameSample {
    fn cursor(&self) -> u64 {
        self.changed_at
    }
}

pub const LOG_CAP: usize = 400;
pub const ERROR_CAP: usize = 100;
pub const NET_CAP: usize = 150;
pub const FRAME_CAP: usize = 240;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
#[derive(Clone)]
pub(super) struct RepaintUpdate {
    pub seq: u64,
    pub batch: Option<Arc<engine::tree::FrameRepaintBatch>>,
    pub picked: Option<engine::tree::PickedViewEvent>,
}

#[cfg(any(debug_assertions, feature = "devtools-server"))]
seq_field!(RepaintUpdate);

#[cfg(any(debug_assertions, feature = "devtools-server"))]
#[derive(Clone)]
pub(super) struct RuntimeUpdate {
    pub seq: u64,
    pub at_ms: f64,
    pub observation: Arc<serde_json::Value>,
}

#[cfg(any(debug_assertions, feature = "devtools-server"))]
seq_field!(RuntimeUpdate);

/// Process-local telemetry store, shared by the host and its consumers.
pub struct TelemetryHub {
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) repaints: Mutex<Ring<RepaintUpdate>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    repaint_readers: AtomicU64,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    latest_layers: Mutex<Option<(u64, Arc<engine::scene::LayerTree>)>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) runtime: Mutex<Ring<RuntimeUpdate>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) resources: Mutex<Ring<RuntimeUpdate>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    runtime_readers: AtomicU64,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    resource_readers: AtomicU64,
    profile: AtomicU8,
    capture_phase: AtomicU8,
    capture_gpu: AtomicBool,
    capture: Mutex<Option<CaptureSlot>>,
    capture_changed: Condvar,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    latest_capture: Mutex<Option<Arc<TraceCapture>>>,
    pub(crate) logs: Mutex<Ring<LogEntry>>,
    pub(crate) errors: Mutex<Ring<DiagError>>,
    pub(crate) network: Mutex<Ring<NetEntry>>,
    net_bodies: Mutex<NetBodyPool>,
    ws_frames: Mutex<Ring<WsFrame>>,
    pub(crate) frames: Mutex<Ring<FrameSample>>,
    /// Bumped by any channel so a reader can skip the whole pull.
    revision: AtomicU64,
    flags: Mutex<DevtoolsFlags>,
    /// Latest window metrics. The platform page reads this.
    metrics: Mutex<engine::session::Metrics>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) refresh_rate: Mutex<Option<f32>>,
    /// Overflow report JSON from the last frame that scanned, or `"[]"`.
    layout_report: Mutex<String>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    inspector_clients: AtomicU64,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    inspector: Mutex<Option<(std::time::Instant, Arc<str>)>>,
}

#[cfg(any(debug_assertions, feature = "devtools-server"))]
pub(super) struct InspectorClient<'a>(&'a TelemetryHub);

#[cfg(any(debug_assertions, feature = "devtools-server"))]
impl Drop for InspectorClient<'_> {
    fn drop(&mut self) {
        if self.0.inspector_clients.fetch_sub(1, Ordering::Relaxed) == 1 {
            lock(&self.0.inspector).take();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Default for TelemetryHub {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryHub {
    pub fn new() -> Self {
        Self {
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            repaints: Mutex::new(Ring::new(FRAME_CAP)),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            repaint_readers: AtomicU64::new(0),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            latest_layers: Mutex::new(None),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            runtime: Mutex::new(Ring::new(300)),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            resources: Mutex::new(Ring::new(300)),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            runtime_readers: AtomicU64::new(0),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            resource_readers: AtomicU64::new(0),
            profile: AtomicU8::new(TelemetryProfile::Counters as u8),
            capture_phase: AtomicU8::new(CAPTURE_IDLE),
            capture_gpu: AtomicBool::new(false),
            capture: Mutex::new(None),
            capture_changed: Condvar::new(),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            latest_capture: Mutex::new(None),
            logs: Mutex::new(Ring::new(LOG_CAP)),
            errors: Mutex::new(Ring::new(ERROR_CAP)),
            network: Mutex::new(Ring::new(NET_CAP)),
            net_bodies: Mutex::default(),
            ws_frames: Mutex::new(Ring::new(1024)),
            frames: Mutex::new(Ring::new(FRAME_CAP)),
            revision: AtomicU64::new(1),
            flags: Mutex::new(DevtoolsFlags::default()),
            metrics: Mutex::new(engine::session::Metrics::new(0.0, 0.0, 1.0)),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            refresh_rate: Mutex::new(None),
            layout_report: Mutex::new("[]".to_owned()),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            inspector_clients: AtomicU64::new(0),
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            inspector: Mutex::new(None),
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn inspect_remote(&self) -> InspectorClient<'_> {
        self.inspector_clients.fetch_add(1, Ordering::Relaxed);
        InspectorClient(self)
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn refresh_inspector(
        &self,
        settled: bool,
        snapshot: impl FnOnce() -> Option<String>,
    ) {
        if self.inspector_clients.load(Ordering::Relaxed) == 0
            || (!settled
                && lock(&self.inspector)
                    .as_ref()
                    .is_some_and(|(updated, _)| updated.elapsed() < Duration::from_millis(200)))
        {
            return;
        }
        if let Some(snapshot) = snapshot() {
            let mut stored = lock(&self.inspector);
            if self.inspector_clients.load(Ordering::Relaxed) != 0 {
                *stored = Some((std::time::Instant::now(), snapshot.into()));
            }
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn inspector(&self) -> Option<Arc<str>> {
        lock(&self.inspector)
            .as_ref()
            .map(|(_, snapshot)| Arc::clone(snapshot))
    }

    pub fn note_metrics(&self, metrics: engine::session::Metrics) {
        *lock(&self.metrics) = metrics;
    }

    pub fn metrics(&self) -> engine::session::Metrics {
        *lock(&self.metrics)
    }

    pub fn set_layout_report(&self, json: &str) {
        *lock(&self.layout_report) = json.to_owned();
    }

    pub fn layout_report(&self) -> String {
        lock(&self.layout_report).clone()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn flags(&self) -> DevtoolsFlags {
        *lock(&self.flags)
    }

    pub fn set_flags(&self, flags: DevtoolsFlags) {
        *lock(&self.flags) = flags;
    }

    pub fn push_log(&self, entry: LogEntry) {
        lock(&self.logs).push(|seq| LogEntry { seq, ..entry });
        self.bump();
    }

    /// Surfaced failure with a stack. The console line, if any, is a separate event.
    pub fn error(
        &self,
        source: &'static str,
        message: &str,
        stack: Option<&str>,
        frame: Option<u64>,
    ) {
        let at = crate::timing::mono_ms();
        lock(&self.errors).push(|seq| DiagError {
            seq,
            at_ms: at,
            source,
            message: message.to_owned(),
            stack: stack.map(str::to_owned),
            frame,
        });
        self.bump();
    }

    pub(super) fn record_net(&self, mut entry: NetEntry) {
        lock(&self.network).push(|seq| {
            entry.seq = seq;
            entry.changed_at = seq;
            entry
        });
        self.bump();
    }

    pub(super) fn update_net(&self, token: u64, update: impl FnOnce(&mut NetEntry)) {
        let mut network = lock(&self.network);
        if let Some(index) = network.buf.iter().rposition(|entry| entry.token == token) {
            network.seq = network.seq.saturating_add(1);
            let seq = network.seq;
            let entry = &mut network.buf[index];
            update(entry);
            entry.changed_at = seq;
            self.bump();
        }
    }

    pub(super) fn note_body(&self, token: u64, side: BodySide, body: Vec<u8>) {
        let evicted = lock(&self.net_bodies).insert((token, side), body);
        for (token, side) in evicted {
            match side {
                BodySide::Request => {
                    self.update_net(token, |entry| entry.request_body_evicted = true)
                }
                BodySide::Response => {
                    self.update_net(token, |entry| entry.response_body_evicted = true)
                }
                BodySide::Socket(_) => {}
            }
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn net_body(&self, token: u64, side: BodySide) -> Option<Arc<[u8]>> {
        lock(&self.net_bodies).get((token, side))
    }

    pub(super) fn record_ws(&self, mut frame: WsFrame, body: Option<Vec<u8>>) {
        let mut frames = lock(&self.ws_frames);
        let token = frame.token;
        let mut occurrences = 0;
        if let Some(index) = frames
            .buf
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| {
                if entry.token == token {
                    occurrences += 1;
                }
                (occurrences == 200).then_some(index)
            })
            && let Some(oldest) = frames.buf.remove(index)
        {
            frames.discarded_through = frames.discarded_through.max(oldest.seq);
        }
        frames.push(|seq| {
            frame.seq = seq;
            frame
        });
        let seq = frames.seq;
        drop(frames);
        if let Some(body) = body {
            self.note_body(token, BodySide::Socket(seq), body);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn ws_updates(
        &self,
        token: u64,
        since: Option<u64>,
    ) -> devtools_protocol::ChannelUpdate<WsFrame> {
        let frames = lock(&self.ws_frames);
        let reset =
            since.is_none_or(|since| since < frames.discarded_through || since > frames.seq);
        devtools_protocol::ChannelUpdate {
            revision: frames.seq,
            reset,
            entries: frames
                .tail(if reset { 0 } else { since.unwrap_or_default() })
                .filter(|frame| frame.token == token)
                .cloned()
                .collect(),
        }
    }

    /// Returns the current logs revision and entries newer than the supplied cursor.
    pub fn logs_since(&self, since: u64) -> (u64, Vec<LogEntry>) {
        let ring = self.logs.lock().unwrap_or_else(|error| error.into_inner());
        (ring.revision(), ring.tail(since).cloned().collect())
    }

    /// Returns the current errors revision and entries newer than the supplied cursor.
    pub fn errors_since(&self, since: u64) -> (u64, Vec<DiagError>) {
        let ring = self
            .errors
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (ring.revision(), ring.tail(since).cloned().collect())
    }

    /// Returns the current network revision and entries newer than the supplied cursor.
    pub fn network_since(&self, since: u64) -> (u64, Vec<NetEntry>) {
        let ring = self
            .network
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (ring.revision(), ring.tail(since).cloned().collect())
    }

    /// Returns the current frames revision and entries newer than the supplied cursor.
    pub fn frames_since(&self, since: u64) -> (u64, Vec<FrameSample>) {
        let ring = self
            .frames
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (ring.revision(), ring.tail(since).cloned().collect())
    }

    /// Clears the session's recorded logs and script errors.
    pub fn clear_logs(&self) {
        self.logs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.errors
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.bump();
    }

    pub fn network_snapshot(&self) -> Vec<NetEntry> {
        lock(&self.network).tail(0).cloned().collect()
    }

    pub fn set_profile(&self, profile: TelemetryProfile) {
        if profile != TelemetryProfile::Off {
            let mut frames = lock(&self.frames);
            if frames.buf.capacity() < FRAME_CAP {
                let available = frames.buf.len();
                frames.buf.reserve(FRAME_CAP - available);
            }
        }
        self.profile.store(profile as u8, Ordering::Relaxed);
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        if profile != TelemetryProfile::DeepInspect {
            *lock(&self.latest_layers) = None;
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_refresh_rate(&self, refresh_rate: Option<f32>) {
        *lock(&self.refresh_rate) = refresh_rate;
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn latest_frame(&self) -> Option<FrameSample> {
        lock(&self.frames).buf.back().cloned()
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn frame_detail(&self, id: u64) -> Option<FrameSample> {
        lock(&self.frames)
            .buf
            .iter()
            .find(|frame| frame.number == id)
            .cloned()
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn repaint_reader(&self, add: bool) {
        if add {
            self.repaint_readers.fetch_add(1, Ordering::Relaxed);
        } else {
            self.repaint_readers.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn tracks_repaints(&self) -> bool {
        self.repaint_readers.load(Ordering::Relaxed) != 0
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_repaints(
        &self,
        batch: Option<engine::tree::FrameRepaintBatch>,
        picked: Option<engine::tree::PickedViewEvent>,
    ) {
        if batch.is_some() || picked.is_some() {
            lock(&self.repaints).push(|seq| RepaintUpdate {
                seq,
                batch: batch.map(Arc::new),
                picked,
            });
            self.bump();
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn frame_repaints(&self, id: u64) -> Option<Arc<engine::tree::FrameRepaintBatch>> {
        lock(&self.repaints)
            .buf
            .iter()
            .rev()
            .filter_map(|entry| entry.batch.as_ref())
            .find(|batch| batch.frame_number == id)
            .map(Arc::clone)
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_layers(&self, frame: u64, tree: &Arc<engine::scene::LayerTree>) {
        if self.profile.load(Ordering::Relaxed) == TelemetryProfile::DeepInspect as u8 {
            let mut layers = lock(&self.latest_layers);
            if self.profile.load(Ordering::Relaxed) == TelemetryProfile::DeepInspect as u8 {
                *layers = Some((frame, Arc::clone(tree)));
            }
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn retained_layers(
        &self,
        frame: Option<u64>,
    ) -> Option<(u64, Arc<engine::scene::LayerTree>)> {
        lock(&self.latest_layers)
            .as_ref()
            .filter(|(id, _)| frame.is_none_or(|frame| frame == *id))
            .cloned()
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn runtime_reader(&self, channel: devtools_protocol::Channel, active: bool) {
        use devtools_protocol::Channel;
        let readers = match channel {
            Channel::Runtime => &self.runtime_readers,
            Channel::Resources => &self.resource_readers,
            _ => return,
        };
        if active {
            readers.fetch_add(1, Ordering::Relaxed);
        } else {
            readers.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn reads_runtime(&self) -> bool {
        self.runtime_readers.load(Ordering::Relaxed) != 0
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn reads_resources(&self) -> bool {
        self.resource_readers.load(Ordering::Relaxed) != 0
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_runtime(&self, value: serde_json::Value) {
        self.record_runtime(&self.runtime, value);
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_resources(&self, value: serde_json::Value) {
        self.record_runtime(&self.resources, value);
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    fn record_runtime(&self, ring: &Mutex<Ring<RuntimeUpdate>>, value: serde_json::Value) {
        lock(ring).push(|seq| RuntimeUpdate {
            seq,
            at_ms: crate::timing::mono_ms(),
            observation: Arc::new(value),
        });
        self.bump();
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn note_js_samples(
        &self,
        frame_id: u64,
        samples: Vec<engine::session::StackSample>,
    ) {
        if samples.is_empty() {
            return;
        }
        let mut ring = lock(&self.resources);
        for sample in samples {
            let value = serde_json::json!({
                "kind": "stack", "frame_id": frame_id, "entry": sample.entry, "stack": sample.stack,
            });
            ring.push(|seq| RuntimeUpdate {
                seq,
                at_ms: super::telemetry::mono_ns_at(sample.at) as f64 / 1_000_000.0,
                observation: Arc::new(value),
            });
        }
        self.bump();
    }

    pub(crate) fn collects_frames(&self) -> bool {
        self.profile.load(Ordering::Relaxed) != TelemetryProfile::Off as u8 || self.recording()
    }

    pub(crate) fn collects_gpu(&self) -> bool {
        self.profile.load(Ordering::Relaxed) >= TelemetryProfile::GpuAndTimeline as u8
            || (self.recording() && self.capture_gpu.load(Ordering::Relaxed))
    }

    pub(crate) fn recording(&self) -> bool {
        self.capture_phase.load(Ordering::Acquire) == CAPTURE_RECORDING
    }

    pub fn start_trace(self: &Arc<Self>, path: PathBuf, overwrite: bool) -> Option<u64> {
        let detail = match std::env::var("IMPASTO_TRACE_DETAIL").as_deref() {
            Ok("detailed") => TraceDetail::Detailed,
            _ => TraceDetail::Stages,
        };
        let gpu = std::env::var("IMPASTO_TRACE_GPU").as_deref() != Ok("0");
        self.begin_capture(
            Some((path, overwrite)),
            CaptureOptions {
                detail,
                gpu,
                js_sampling: detail == TraceDetail::Detailed,
            },
        )
    }

    pub(super) fn begin_capture(
        &self,
        destination: Option<(PathBuf, bool)>,
        options: CaptureOptions,
    ) -> Option<u64> {
        static NEXT_TRACE: AtomicU64 = AtomicU64::new(1);
        let mut capture = lock(&self.capture);
        if capture.is_some() {
            return None;
        }
        let id = NEXT_TRACE.fetch_add(1, Ordering::Relaxed);
        let timeline = TimelineRecording::begin(id, options.detail, options.js_sampling)?;
        *capture = Some(CaptureSlot {
            archive: TraceCapture {
                id,
                destination,
                started_ns: timeline.started_ns,
                ..TraceCapture::default()
            },
            timeline,
            in_flight: 0,
        });
        self.capture_gpu.store(options.gpu, Ordering::Relaxed);
        self.capture_phase
            .store(CAPTURE_RECORDING, Ordering::Release);
        Some(id)
    }

    pub(crate) fn track_frame(self: &Arc<Self>, started_ns: u64) -> Option<CaptureFrame> {
        if !self.recording() {
            return None;
        }
        let mut slot = lock(&self.capture);
        let capture = slot.as_mut()?;
        if started_ns < capture.archive.started_ns || !capture.timeline.id_active() {
            return None;
        }
        capture.in_flight += 1;
        Some(CaptureFrame {
            hub: Arc::downgrade(self),
            id: capture.archive.id,
        })
    }

    pub fn finish_trace(&self, id: u64) -> std::io::Result<PathBuf> {
        let capture = self
            .take_capture(id)
            .ok_or_else(|| std::io::Error::other("trace recording expired"))?;
        super::trace_export::write(&capture)
    }

    pub(super) fn take_capture(&self, id: u64) -> Option<Arc<TraceCapture>> {
        let mut slot = lock(&self.capture);
        let capture = slot.as_ref().filter(|capture| capture.archive.id == id)?;
        let cutoff = capture.timeline.stop()?;
        self.capture_phase
            .store(CAPTURE_STOPPING, Ordering::Release);
        (slot, _) = self
            .capture_changed
            .wait_timeout_while(slot, TRACE_DRAIN, |slot| {
                slot.as_ref().is_some_and(|capture| {
                    capture.in_flight > 0 || capture.archive.pending_gpu() > 0
                })
            })
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut capture = slot.take()?;
        self.capture_phase.store(CAPTURE_IDLE, Ordering::Release);
        drop(slot);
        let (spans, loss) = capture.timeline.finish(cutoff);
        capture.archive.spans = spans;
        capture.archive.span_loss = loss;
        capture.archive.stopped_ns = cutoff;
        capture.archive.unfinished_frames = capture.in_flight;
        let archive = Arc::new(capture.archive);
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        {
            let mut latest = lock(&self.latest_capture);
            if latest.as_ref().is_none_or(|old| old.id < id) {
                *latest = Some(Arc::clone(&archive));
            }
        }
        capture.timeline.release();
        Some(archive)
    }

    pub(crate) fn cancel_capture(&self, id: u64) {
        let mut slot = lock(&self.capture);
        if let Some(capture) = slot.as_ref().filter(|capture| capture.archive.id == id)
            && capture.timeline.stop().is_some()
        {
            capture.timeline.release();
            self.capture_phase.store(CAPTURE_IDLE, Ordering::Release);
            slot.take();
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn latest_capture(&self) -> Option<Arc<TraceCapture>> {
        lock(&self.latest_capture).clone()
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn capture_status(&self) -> Option<(u64, bool)> {
        lock(&self.capture)
            .as_ref()
            .map(|capture| (capture.archive.id, capture.timeline.id_active()))
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(super) fn frame_updates(&self, since: u64) -> (u64, bool, Vec<FrameSample>) {
        let frames = lock(&self.frames);
        let reset = since == 0
            || since > frames.seq
            || frames
                .buf
                .front()
                .is_some_and(|oldest| since < oldest.seq.saturating_sub(1));
        let samples = frames
            .tail(if reset { 0 } else { since })
            .cloned()
            .collect();
        (frames.seq, reset, samples)
    }

    pub fn push_frame(&self, mut sample: FrameSample) {
        {
            let mut frames = lock(&self.frames);
            frames.push(|seq| {
                sample.seq = seq;
                sample.changed_at = seq;
                sample.clone()
            });
        }
        if self.capture_phase.load(Ordering::Acquire) != CAPTURE_IDLE
            && let Some(capture) = lock(&self.capture).as_mut()
            && sample.schedule.ui_start_ns >= capture.archive.started_ns
            && sample.schedule.ui_start_ns <= capture.timeline.cutoff()
        {
            if capture.archive.frames.len() == TRACE_FRAME_CAP {
                capture.archive.overwritten_frames += 1;
            }
            push_bounded(&mut capture.archive.frames, TRACE_FRAME_CAP, sample);
        }
        self.bump();
    }

    pub(crate) fn patch_gpu_timing(&self, update: raster::GpuTimingUpdate) {
        let timing = update.timing.map(Arc::new);
        let patch = |frame: &mut FrameSample| {
            let key = update.sample;
            if frame.number == key.frame_id
                && frame.gpu.device_generation == key.device_generation
                && frame.gpu.status == raster::GpuTimingStatus::Pending
            {
                frame.gpu = key;
                frame.gpu_timing.clone_from(&timing);
                frame.gpu_over_budget = frame.budget_ms > 0.0
                    && frame.gpu_span_ms().is_some_and(|ms| ms > frame.budget_ms);
                frame.over_budget |= frame.gpu_over_budget;
                true
            } else {
                false
            }
        };
        let mut frames = lock(&self.frames);
        let revision = frames.seq.saturating_add(1);
        if let Some(frame) = frames
            .buf
            .iter_mut()
            .rev()
            .find(|frame| frame.number == update.sample.frame_id)
            && patch(frame)
        {
            frame.changed_at = revision;
            frames.seq = revision;
            self.bump();
        }
        drop(frames);
        if self.capture_phase.load(Ordering::Acquire) != CAPTURE_IDLE
            && let Some(capture) = lock(&self.capture).as_mut()
            && let Some(frame) = capture
                .archive
                .frames
                .iter_mut()
                .rev()
                .find(|frame| frame.number == update.sample.frame_id)
            && patch(frame)
        {
            self.capture_changed.notify_all();
        }
    }
}

pub type DiagnosticsHub = TelemetryHub;

impl impasto::DiagnosticsSink for TelemetryHub {
    fn record(&self, source: &str, event: &str, payload: impasto::abi::PluginValue) {
        match event {
            "network" | "network.start" | "network.head" | "network.finish" => {
                super::plugin_record::network(self, source, event, payload);
                return;
            }
            "network.ws_frame" => {
                super::plugin_record::websocket(self, payload);
                return;
            }
            _ => {}
        }
        self.push_log(LogEntry {
            seq: 0,
            at_ms: super::telemetry::mono_ns() as f64 / 1_000_000.0,
            level: tracing::Level::INFO,
            source: LogSource::Host,
            target: "impasto::plugin",
            message: format!("{source}: {event}"),
            fields: vec![("payload", format!("{payload:?}"))],
            frame: None,
        });
    }
}

const CAPTURE_IDLE: u8 = 0;
const CAPTURE_RECORDING: u8 = 1;
const CAPTURE_STOPPING: u8 = 2;
#[cfg(test)]
pub(super) static CAPTURE_TEST_LOCK: Mutex<()> = Mutex::new(());

const TRACE_FRAME_CAP: usize = 1024;
const TRACE_DRAIN: Duration = Duration::from_millis(80);

pub(super) struct CaptureOptions {
    pub detail: TraceDetail,
    pub gpu: bool,
    pub js_sampling: bool,
}

struct CaptureSlot {
    archive: TraceCapture,
    timeline: Arc<TimelineRecording>,
    in_flight: usize,
}

/// Holds the capture open while an admitted frame is in the host pipeline.
pub(crate) struct CaptureFrame {
    hub: Weak<TelemetryHub>,
    id: u64,
}

impl Drop for CaptureFrame {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade()
            && let Some(capture) = lock(&hub.capture).as_mut()
            && capture.archive.id == self.id
        {
            capture.in_flight -= 1;
            hub.capture_changed.notify_all();
        }
    }
}

#[derive(Default)]
pub(super) struct TraceCapture {
    pub id: u64,
    pub destination: Option<(PathBuf, bool)>,
    pub started_ns: u64,
    pub stopped_ns: u64,
    pub overwritten_frames: u64,
    pub unfinished_frames: usize,
    pub span_loss: SpanLoss,
    pub frames: VecDeque<FrameSample>,
    pub spans: VecDeque<CapturedSpan>,
}

impl TraceCapture {
    pub fn pending_gpu(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.gpu.status == raster::GpuTimingStatus::Pending)
            .count()
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, capacity: usize, value: T) {
    if queue.len() == capacity {
        queue.pop_front();
    }
    queue.push_back(value);
}
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: u64, message: &str) -> LogEntry {
        LogEntry {
            seq,
            at_ms: 0.0,
            level: tracing::Level::INFO,
            source: LogSource::Script,
            target: "impasto::script",
            message: message.to_owned(),
            fields: Vec::new(),
            frame: None,
        }
    }

    #[test]
    fn stopping_capture_accepts_gpu_readback_from_an_admitted_frame() {
        let _serial = CAPTURE_TEST_LOCK.lock().unwrap();
        let hub = Arc::new(TelemetryHub::new());
        let id = hub
            .begin_capture(
                None,
                CaptureOptions {
                    detail: TraceDetail::Stages,
                    gpu: true,
                    js_sampling: false,
                },
            )
            .unwrap();
        let started = super::super::telemetry::mono_ns();
        let submitted = hub.track_frame(started).unwrap();
        let sample = raster::GpuSample {
            frame_id: 90,
            device_generation: 4,
            status: raster::GpuTimingStatus::Pending,
        };
        hub.push_frame(FrameSample {
            number: 90,
            schedule: ScheduleTiming {
                ui_start_ns: started,
                ..ScheduleTiming::default()
            },
            gpu: sample,
            ..FrameSample::default()
        });
        let stopping = Arc::clone(&hub);
        let worker = std::thread::spawn(move || stopping.take_capture(id).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while hub.recording() {
            assert!(
                std::time::Instant::now() < deadline,
                "capture did not begin stopping"
            );
            std::thread::yield_now();
        }
        hub.patch_gpu_timing(raster::GpuTimingUpdate {
            sample: raster::GpuSample {
                status: raster::GpuTimingStatus::Ready,
                ..sample
            },
            timing: None,
        });
        drop(submitted);
        let capture = worker.join().unwrap();
        assert_eq!(capture.frames[0].gpu.status, raster::GpuTimingStatus::Ready);
        assert_eq!(capture.pending_gpu(), 0);
        assert_eq!(capture.unfinished_frames, 0);
    }

    #[test]
    fn gpu_backfill_marks_over_budget_on_the_same_frame_revision() {
        use raster::{GpuFrameTiming, GpuSample, GpuTimingStatus, GpuTimingUpdate};

        let hub = TelemetryHub::new();
        let sample = GpuSample {
            frame_id: 15,
            device_generation: 3,
            status: GpuTimingStatus::Pending,
        };
        hub.push_frame(FrameSample {
            number: 15,
            budget_ms: 16.,
            presented: true,
            gpu: sample,
            ..FrameSample::default()
        });
        let original = hub.latest_frame().unwrap();
        hub.patch_gpu_timing(GpuTimingUpdate {
            sample: GpuSample {
                status: GpuTimingStatus::Ready,
                ..sample
            },
            timing: Some(GpuFrameTiming {
                clock_start_us: 0.,
                frame_span_us: 25_000.,
                passes_sum_us: 21_000.,
                passes: Vec::new(),
            }),
        });
        let (revision, updated) = hub.frames_since(original.changed_at);
        assert_eq!(revision, original.changed_at + 1);
        assert_eq!(updated[0].seq, original.seq);
        assert_eq!(updated[0].changed_at, revision);
        assert_eq!(updated[0].gpu_span_ms(), Some(25.));
        assert!(updated[0].gpu_over_budget && updated[0].over_budget);
        assert!(!updated[0].ui_over_budget && !updated[0].raster_over_budget);
    }

    #[test]
    fn late_gpu_update_keeps_frame_selection_and_advances_the_cursor() {
        use raster::{GpuSample, GpuTimingStatus, GpuTimingUpdate};

        let hub = TelemetryHub::new();
        hub.push_frame(FrameSample {
            number: 42,
            gpu: GpuSample {
                frame_id: 42,
                device_generation: 7,
                status: GpuTimingStatus::Pending,
            },
            ..FrameSample::default()
        });
        let patch = |generation| GpuTimingUpdate {
            sample: GpuSample {
                frame_id: 42,
                device_generation: generation,
                status: GpuTimingStatus::StaleDevice,
            },
            timing: None,
        };
        hub.patch_gpu_timing(patch(8));
        assert_eq!(lock(&hub.frames).revision(), 1);
        hub.push_frame(FrameSample {
            number: 43,
            ..FrameSample::default()
        });
        hub.patch_gpu_timing(patch(7));
        let frames = lock(&hub.frames);
        assert_eq!(frames.revision(), 3);
        let updated: Vec<_> = frames.tail(2).collect();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].seq, 1);
        assert_eq!(updated[0].number, 42);
        assert_eq!(updated[0].gpu.status, GpuTimingStatus::StaleDevice);
    }

    #[test]
    fn settled_inspector_frames_replace_a_recent_snapshot() {
        let hub = DiagnosticsHub::new();
        let client = hub.inspect_remote();
        hub.refresh_inspector(false, || Some("initial".to_owned()));
        hub.refresh_inspector(true, || Some("settled".to_owned()));
        assert_eq!(hub.inspector().as_deref(), Some("settled"));
        drop(client);
        hub.refresh_inspector(true, || panic!("disconnected inspection should not run"));
        assert!(hub.inspector().is_none());
    }

    #[test]
    fn rings_drop_the_oldest_entry() {
        let mut ring: Ring<LogEntry> = Ring::new(2);
        ring.push(|seq| entry(seq, "a"));
        ring.push(|seq| entry(seq, "b"));
        ring.push(|seq| entry(seq, "c"));
        assert_eq!(ring.revision(), 3);
        let texts: Vec<&str> = ring.tail(0).map(|e| e.message.as_str()).collect();
        assert_eq!(texts, ["b", "c"]);
    }

    #[test]
    fn error_entries_stay_off_the_log_channel() {
        let hub = DiagnosticsHub::new();
        hub.error("listener", "boom", Some("at f"), Some(12));
        assert_eq!(lock(&hub.errors).revision(), 1);
        assert_eq!(lock(&hub.logs).revision(), 0);
        let errors = lock(&hub.errors);
        let reported: Vec<&DiagError> = errors.tail(0).collect();
        assert_eq!(reported[0].stack.as_deref(), Some("at f"));
        assert_eq!(reported[0].frame, Some(12));
        drop(errors);
    }

    #[test]
    fn tail_returns_only_new_entries() {
        let hub = DiagnosticsHub::new();
        hub.push_log(entry(0, "one"));
        hub.push_log(entry(0, "two"));
        let first = {
            let guard = lock(&hub.logs);
            let texts: Vec<String> = guard.tail(0).map(|e| e.message.clone()).collect();
            drop(guard);
            texts
        };
        assert_eq!(first, ["one", "two"]);
        let seen = lock(&hub.logs).revision();
        let guard = lock(&hub.logs);
        let second: Vec<&str> = guard.tail(seen).map(|e| e.message.as_str()).collect();
        assert!(second.is_empty());
        drop(guard);
    }

    #[test]
    fn revision_only_grows() {
        let hub = DiagnosticsHub::new();
        let start = hub.revision();
        hub.push_log(entry(0, "x"));
        hub.push_frame(FrameSample::default());
        let after = hub.revision();
        assert!(after > start);
        assert_eq!(hub.revision(), after);
    }

    #[test]
    fn net_entries_get_their_seq_from_the_ring() {
        let hub = DiagnosticsHub::new();
        hub.record_net(NetEntry {
            seq: 0,
            at_ms: 1.0,
            kind: NetKind::Http,
            token: 7,
            method: "GET".into(),
            url: "https://example.test".into(),
            request_headers: Vec::new(),
            request_body: None,
            status: Some(200),
            response_headers: Vec::new(),
            response_bytes: Some(2),
            response_preview: Some(b"hi".to_vec()),
            error: None,
            done_ms: Some(2.0),
            ..Default::default()
        });
        let entry = lock(&hub.network).tail(0).next().cloned().unwrap();
        assert_eq!(entry.seq, 1);
        assert_eq!(entry.status, Some(200));
    }
}
