//! Ring buffers with a monotonic revision per channel. Producers are the
//! render thread, the net thread, and the log hook; the only reader is the
//! devtools host module (render thread).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// One console line or host log line. Level 0 log, 1 info, 2 warn, 3 error,
/// 4 debug; the numbering matches the QuickJS console binding.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub seq: u64,
    pub at_ms: f64,
    pub level: u8,
    pub source: LogSource,
    pub text: String,
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

/// One HTTP call (fetch or fetchStream) or socket lifecycle step.
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
}

/// Per-frame timing sample, trimmed from [`crate::FrameTiming`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSample {
    pub seq: u64,
    pub number: u64,
    pub at_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub layers: u32,
    pub paragraphs: u32,
    pub pictures: u32,
    pub textures: u32,
    pub over_budget: bool,
    pub phases: PhaseSample,
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
        self.buf.iter().filter(move |entry| entry.seq() > since)
    }

    pub(crate) fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Every ring entry carries its sequence number as the `seq` field.
pub(crate) trait SeqField {
    fn seq(&self) -> u64;
}

macro_rules! seq_field {
    ($ty:ty) => {
        impl SeqField for $ty {
            fn seq(&self) -> u64 {
                self.seq
            }
        }
    };
}

seq_field!(LogEntry);
seq_field!(DiagError);
seq_field!(NetEntry);
seq_field!(FrameSample);

pub const LOG_CAP: usize = 400;
pub const ERROR_CAP: usize = 100;
pub const NET_CAP: usize = 150;
pub const FRAME_CAP: usize = 240;
/// Body preview cap for network entries.
pub const PREVIEW_CAP: usize = 2048;

/// Process-wide diagnostics sink. Every method takes the lock only to clone.
pub struct DiagnosticsHub {
    pub(crate) logs: Mutex<Ring<LogEntry>>,
    pub(crate) errors: Mutex<Ring<DiagError>>,
    pub(crate) network: Mutex<Ring<NetEntry>>,
    pub(crate) frames: Mutex<Ring<FrameSample>>,
    /// Bumped by any channel so a reader can skip the whole pull.
    revision: AtomicU64,
    flags: Mutex<DevtoolsFlags>,
    /// Latest window metrics. The platform page reads this.
    metrics: Mutex<session::Metrics>,
    /// Overflow report JSON from the last frame that scanned, or `"[]"`.
    layout_report: Mutex<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Default for DiagnosticsHub {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsHub {
    pub fn new() -> Self {
        Self {
            logs: Mutex::new(Ring::new(LOG_CAP)),
            errors: Mutex::new(Ring::new(ERROR_CAP)),
            network: Mutex::new(Ring::new(NET_CAP)),
            frames: Mutex::new(Ring::new(FRAME_CAP)),
            revision: AtomicU64::new(1),
            flags: Mutex::new(DevtoolsFlags::default()),
            metrics: Mutex::new(session::Metrics::new(0.0, 0.0, 1.0)),
            layout_report: Mutex::new("[]".to_owned()),
        }
    }

    pub fn note_metrics(&self, metrics: session::Metrics) {
        *lock(&self.metrics) = metrics;
    }

    pub fn metrics(&self) -> session::Metrics {
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

    pub fn log(&self, level: u8, source: LogSource, text: &str) {
        let at = crate::net::mono_ms();
        lock(&self.logs).push(|seq| LogEntry {
            seq,
            at_ms: at,
            level,
            source,
            text: text.to_owned(),
        });
        self.bump();
    }

    /// Errors also land in the log channel so the console page shows them
    /// without a second subscription.
    pub fn error(
        &self,
        source: &'static str,
        message: &str,
        stack: Option<&str>,
        frame: Option<u64>,
    ) {
        let at = crate::net::mono_ms();
        lock(&self.errors).push(|seq| DiagError {
            seq,
            at_ms: at,
            source,
            message: message.to_owned(),
            stack: stack.map(str::to_owned),
            frame,
        });
        lock(&self.logs).push(|seq| LogEntry {
            seq,
            at_ms: at,
            level: 3,
            source: LogSource::Host,
            text: message.to_owned(),
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

    pub fn push_frame(&self, sample: FrameSample) {
        lock(&self.frames).push(|seq| FrameSample { seq, ..sample });
        self.bump();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_drop_the_oldest_entry() {
        let mut ring: Ring<LogEntry> = Ring::new(2);
        ring.push(|seq| LogEntry {
            seq,
            at_ms: 0.0,
            level: 0,
            source: LogSource::Script,
            text: "a".into(),
        });
        ring.push(|seq| LogEntry {
            seq,
            at_ms: 0.0,
            level: 0,
            source: LogSource::Script,
            text: "b".into(),
        });
        ring.push(|seq| LogEntry {
            seq,
            at_ms: 0.0,
            level: 0,
            source: LogSource::Script,
            text: "c".into(),
        });
        assert_eq!(ring.revision(), 3);
        let texts: Vec<&str> = ring.tail(0).map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["b", "c"]);
    }

    #[test]
    fn error_entries_also_land_in_the_log_channel() {
        let hub = DiagnosticsHub::new();
        hub.error("listener", "boom", Some("at f"), Some(12));
        assert_eq!(lock(&hub.errors).revision(), 1);
        assert_eq!(lock(&hub.logs).revision(), 1);
        let logs: Vec<String> = lock(&hub.logs).tail(0).map(|e| e.text.clone()).collect();
        assert_eq!(logs, ["boom"]);
        let errors = lock(&hub.errors);
        let reported: Vec<&DiagError> = errors.tail(0).collect();
        assert_eq!(reported[0].stack.as_deref(), Some("at f"));
        assert_eq!(reported[0].frame, Some(12));
        drop(errors);
    }

    #[test]
    fn tail_returns_only_new_entries() {
        let hub = DiagnosticsHub::new();
        hub.log(1, LogSource::Script, "one");
        hub.log(2, LogSource::Host, "two");
        let first = {
            let guard = lock(&hub.logs);
            let texts: Vec<String> = guard.tail(0).map(|e| e.text.clone()).collect();
            drop(guard);
            texts
        };
        assert_eq!(first, ["one", "two"]);
        let seen = lock(&hub.logs).revision();
        let guard = lock(&hub.logs);
        let second: Vec<&str> = guard.tail(seen).map(|e| e.text.as_str()).collect();
        assert!(second.is_empty());
        drop(guard);
    }

    #[test]
    fn revision_only_grows() {
        let hub = DiagnosticsHub::new();
        let start = hub.revision();
        hub.log(0, LogSource::Script, "x");
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
