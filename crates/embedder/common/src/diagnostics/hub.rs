//! Ring buffers with a monotonic revision per channel. Producers are the
//! UI, raster and IO queues, external network callbacks and the log hook.

use super::telemetry::{IoSpan, ScheduleTiming, TelemetryProfile, mono_ns};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// One HTTP call or socket lifecycle step.
#[derive(Clone, Debug)]
pub struct NetEntry {
    pub seq: u64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Http,
    Stream,
    Socket,
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
    pub phases: PhaseSample,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
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
}

impl<T: SeqField> Ring<T> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap,
            seq: 0,
        }
    }

    pub(crate) fn push(&mut self, make: impl FnOnce(u64) -> T) {
        self.seq = self.seq.saturating_add(1);
        if self.buf.len() == self.cap {
            self.buf.pop_front();
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
        self.buf.clear();
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
seq_field!(NetEntry);
impl SeqField for FrameSample {
    fn cursor(&self) -> u64 {
        self.changed_at
    }
}

pub const LOG_CAP: usize = 400;
pub const ERROR_CAP: usize = 100;
pub const NET_CAP: usize = 150;
pub const FRAME_CAP: usize = 240;
/// Body preview cap for network entries.
pub const PREVIEW_CAP: usize = 2048;

/// Process-local telemetry store, shared by the host and its consumers.
pub struct TelemetryHub {
    profile: AtomicU8,
    recording: AtomicBool,
    capture: Mutex<Option<TraceCapture>>,
    pub(crate) logs: Mutex<Ring<LogEntry>>,
    pub(crate) errors: Mutex<Ring<DiagError>>,
    pub(crate) network: Mutex<Ring<NetEntry>>,
    pub(crate) frames: Mutex<Ring<FrameSample>>,
    /// Bumped by any channel so a reader can skip the whole pull.
    revision: AtomicU64,
    flags: Mutex<DevtoolsFlags>,
    /// Latest window metrics. The platform page reads this.
    metrics: Mutex<engine::session::Metrics>,
    /// Overflow report JSON from the last frame that scanned, or `"[]"`.
    layout_report: Mutex<String>,
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
            profile: AtomicU8::new(TelemetryProfile::Counters as u8),
            recording: AtomicBool::new(false),
            capture: Mutex::new(None),
            logs: Mutex::new(Ring::new(LOG_CAP)),
            errors: Mutex::new(Ring::new(ERROR_CAP)),
            network: Mutex::new(Ring::new(NET_CAP)),
            frames: Mutex::new(Ring::new(FRAME_CAP)),
            revision: AtomicU64::new(1),
            flags: Mutex::new(DevtoolsFlags::default()),
            metrics: Mutex::new(engine::session::Metrics::new(0.0, 0.0, 1.0)),
            layout_report: Mutex::new("[]".to_owned()),
        }
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

    pub(crate) fn record_net(&self, mut entry: NetEntry) {
        lock(&self.network).push(|seq| {
            entry.seq = seq;
            entry
        });
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
    }

    pub(crate) fn collects_frames(&self) -> bool {
        self.profile.load(Ordering::Relaxed) != TelemetryProfile::Off as u8
            || self.recording.load(Ordering::Acquire)
    }

    pub(crate) fn collects_gpu(&self) -> bool {
        self.profile.load(Ordering::Relaxed) >= TelemetryProfile::GpuAndTimeline as u8
            || self.recording.load(Ordering::Acquire)
    }

    pub(crate) fn recording(&self) -> bool {
        self.recording.load(Ordering::Acquire)
    }

    pub(crate) fn start_trace(&self, path: PathBuf, overwrite: bool) -> Option<u64> {
        self.begin_capture(Some((path, overwrite)))
    }

    pub(super) fn begin_capture(&self, destination: Option<(PathBuf, bool)>) -> Option<u64> {
        static NEXT_TRACE: AtomicU64 = AtomicU64::new(1);
        let mut capture = lock(&self.capture);
        if capture.is_some() {
            return None;
        }
        let id = NEXT_TRACE.fetch_add(1, Ordering::Relaxed);
        *capture = Some(TraceCapture {
            id,
            destination,
            started_ns: mono_ns(),
            frames: VecDeque::with_capacity(TRACE_FRAME_CAP),
            io: VecDeque::with_capacity(TRACE_IO_CAP),
        });
        self.recording.store(true, Ordering::Release);
        Some(id)
    }

    pub(crate) fn finish_trace(&self, id: u64) -> std::io::Result<PathBuf> {
        let capture = self
            .take_capture(id)
            .ok_or_else(|| std::io::Error::other("trace recording expired"))?;
        super::trace_export::write(&capture)
    }

    pub(super) fn take_capture(&self, id: u64) -> Option<TraceCapture> {
        let mut slot = lock(&self.capture);
        if slot.as_ref().is_none_or(|capture| capture.id != id) {
            return None;
        }
        self.recording.store(false, Ordering::Release);
        slot.take()
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

    pub(crate) fn note_io(&self, span: IoSpan) {
        if self.recording()
            && let Some(capture) = lock(&self.capture).as_mut()
            && span.start_ns >= capture.started_ns
        {
            push_bounded(&mut capture.io, TRACE_IO_CAP, span);
        }
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
        if self.recording()
            && let Some(capture) = lock(&self.capture).as_mut()
            && sample.schedule.ui_start_ns >= capture.started_ns
        {
            push_bounded(&mut capture.frames, TRACE_FRAME_CAP, sample);
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
        if self.recording()
            && let Some(capture) = lock(&self.capture).as_mut()
            && let Some(frame) = capture
                .frames
                .iter_mut()
                .rev()
                .find(|frame| frame.number == update.sample.frame_id)
        {
            patch(frame);
        }
    }
}

pub type DiagnosticsHub = TelemetryHub;

const TRACE_FRAME_CAP: usize = 1024;
const TRACE_IO_CAP: usize = 2048;

pub(super) struct TraceCapture {
    id: u64,
    pub destination: Option<(PathBuf, bool)>,
    pub started_ns: u64,
    pub frames: VecDeque<FrameSample>,
    pub io: VecDeque<IoSpan>,
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
        });
        let entry = lock(&hub.network).tail(0).next().cloned().unwrap();
        assert_eq!(entry.seq, 1);
        assert_eq!(entry.status, Some(200));
    }
}
