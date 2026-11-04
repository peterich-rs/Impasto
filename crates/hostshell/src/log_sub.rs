//! Process log dispatcher. Call sites emit `tracing` events. This subscriber
//! filters once and hands the same record to every sink.

use std::collections::HashMap;
use std::fmt::Write as _;
#[cfg(not(target_os = "android"))]
use std::io::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::filter::{LevelFilter, Targets};

use crate::diagnostics::{DiagnosticsHub, LogEntry, LogSource};

const XLOG_QUEUE: usize = 256;

/// One filtered event. Sinks must not filter again.
pub struct LogRecord {
    pub level: Level,
    pub target: &'static str,
    pub message: String,
    pub fields: Vec<(&'static str, String)>,
    pub frame: Option<u64>,
}

/// A place an already-filtered event is written.
pub trait LogSink: Send + Sync {
    /// `line` is the same text every sink sees. Do not block the caller.
    fn write(&self, record: &LogRecord, line: &str);
}

struct SpanState {
    frame: Option<u64>,
}

/// Filters with the process [`Targets`] and fans out to the installed sinks.
pub struct ImpastoSubscriber {
    sinks: Vec<Arc<dyn LogSink>>,
    spans: Mutex<HashMap<u64, SpanState>>,
}

impl ImpastoSubscriber {
    pub fn new(sinks: Vec<Arc<dyn LogSink>>) -> Self {
        Self {
            sinks,
            spans: Mutex::new(HashMap::new()),
        }
    }
}

impl Subscriber for ImpastoSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        // `register_callsite` already dropped disabled call sites.
        true
    }

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        let targets = lock(shared_filter());
        if targets.would_enable(metadata.target(), metadata.level()) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(max_level(&lock(shared_filter())))
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        let id = Id::from_u64(next_span_id());
        let mut spans = lock(&self.spans);
        spans.insert(
            id.into_u64(),
            SpanState {
                frame: visitor.frame,
            },
        );
        id
    }

    fn record(&self, id: &Id, values: &Record<'_>) {
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        if let Some(frame) = visitor.frame {
            lock(&self.spans)
                .entry(id.into_u64())
                .or_insert(SpanState { frame: None })
                .frame = Some(frame);
        }
    }

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let record = LogRecord {
            level: *event.metadata().level(),
            target: event.metadata().target(),
            message: visitor.message,
            fields: visitor.fields,
            frame: current_frame(&self.spans),
        };
        let line = format_line(&record);
        for sink in &self.sinks {
            sink.write(&record, &line);
        }
    }

    fn enter(&self, id: &Id) {
        STACK.with(|stack| stack.borrow_mut().push(id.into_u64()));
    }

    fn exit(&self, id: &Id) {
        STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.last() == Some(&id.into_u64()) {
                stack.pop();
            } else {
                stack.retain(|open| *open != id.into_u64());
            }
        });
    }

    fn try_close(&self, id: Id) -> bool {
        lock(&self.spans).remove(&id.into_u64());
        true
    }
}

fn current_frame(spans: &Mutex<HashMap<u64, SpanState>>) -> Option<u64> {
    let stack = STACK.with(|stack| stack.borrow().clone());
    let spans = lock(spans);
    stack
        .iter()
        .rev()
        .find_map(|id| spans.get(id).and_then(|span| span.frame))
}

fn next_span_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static STACK: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: Vec<(&'static str, String)>,
    frame: Option<u64>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.store(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.store(field, format!("{value:?}"));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.store(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "frame" {
            self.frame = Some(value);
        }
        self.store(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.store(field, value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.store(field, format!("{value}"));
    }
}

impl FieldVisitor {
    fn store(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
            return;
        }
        if field.name() == "frame" {
            return;
        }
        self.fields.push((field.name(), value));
    }
}

pub fn format_line(record: &LogRecord) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{} {} {}",
        record.level.as_str(),
        record.target,
        format_body(record)
    );
    out
}

/// Message, fields, and frame. Android logcat already has a level column and uses the target as the tag.
fn format_body(record: &LogRecord) -> String {
    let mut out = record.message.clone();
    for (name, value) in &record.fields {
        out.push(' ');
        out.push_str(name);
        out.push('=');
        push_value(&mut out, value);
    }
    if let Some(frame) = record.frame {
        let _ = write!(out, " frame={frame}");
    }
    out
}

fn push_value(out: &mut String, raw: &str) {
    let quote = raw.is_empty()
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '=');
    if !quote {
        out.push_str(raw);
        return;
    }
    out.push('"');
    for c in raw.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// stdout / stderr, or Android logcat. Does not filter.
pub struct ConsoleSink;

impl LogSink for ConsoleSink {
    #[cfg(target_os = "android")]
    fn write(&self, record: &LogRecord, _line: &str) {
        android_log(record);
    }

    #[cfg(not(target_os = "android"))]
    fn write(&self, record: &LogRecord, line: &str) {
        desktop_log(record.level, line);
    }
}

#[cfg(not(target_os = "android"))]
fn desktop_log(level: Level, line: &str) {
    if level <= Level::WARN {
        let _ = writeln!(io::stderr(), "{line}");
    } else {
        let _ = writeln!(io::stdout(), "{line}");
    }
}

#[cfg(target_os = "android")]
#[allow(unsafe_code, reason = "logcat is the NDK __android_log_write entry")]
fn android_log(record: &LogRecord) {
    let priority: i32 = match record.level {
        Level::ERROR => 6,
        Level::WARN => 5,
        Level::INFO => 4,
        Level::DEBUG => 3,
        Level::TRACE => 2,
    };
    let mut tag = c_bytes(record.target);
    if tag.len() == 1 {
        tag.splice(0..0, b"impasto".iter().copied());
    }
    let text = c_bytes(&format_body(record));
    unsafe extern "C" {
        fn __android_log_write(priority: i32, tag: *const i8, text: *const i8) -> i32;
    }
    // SAFETY: both buffers are NUL-terminated and live until this call returns.
    unsafe {
        __android_log_write(priority, tag.as_ptr().cast(), text.as_ptr().cast());
    }
}

#[cfg(target_os = "android")]
fn c_bytes(text: &str) -> Vec<u8> {
    let mut bytes = text.replace('\0', " ").into_bytes();
    bytes.push(0);
    bytes
}

/// Copies records into the devtools log ring. Absent when devtools are off.
pub struct DevtoolSink {
    devtools: Arc<DiagnosticsHub>,
}

impl DevtoolSink {
    pub fn new(devtools: Arc<DiagnosticsHub>) -> Self {
        Self { devtools }
    }
}

impl LogSink for DevtoolSink {
    fn write(&self, record: &LogRecord, _line: &str) {
        self.devtools.push_log(LogEntry {
            seq: 0,
            at_ms: crate::net::mono_ms(),
            level: record.level,
            source: LogSource::from_target(record.target),
            target: record.target,
            message: record.message.clone(),
            fields: record.fields.clone(),
            frame: record.frame,
        });
    }
}

/// File sink. The render thread only enqueues. `mars-xlog` runs on a worker.
pub struct XlogSink {
    tx: SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

impl XlogSink {
    pub fn open(dir: &std::path::Path) -> Option<Self> {
        let dir = dir.to_str()?;
        let config =
            mars_xlog::XlogConfig::new(dir, "impasto").mode(mars_xlog::AppenderMode::Async);
        let layout =
            mars_xlog::Layout::pieces([mars_xlog::Piece::Message, mars_xlog::Piece::Literal("\n")]);
        let logger =
            mars_xlog::Xlog::init_with_layout(config, mars_xlog::LogLevel::Verbose, layout).ok()?;
        let (tx, rx) = mpsc::sync_channel(XLOG_QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_worker = Arc::clone(&dropped);
        let spawned = thread::Builder::new()
            .name("impasto-xlog".into())
            .spawn(move || {
                while let Ok(line) = rx.recv() {
                    let missed = dropped_worker.swap(0, Ordering::Relaxed);
                    if missed > 0 {
                        logger.log(
                            mars_xlog::LogLevel::Verbose,
                            None,
                            format!("WARN impasto::host xlog queue dropped {missed}"),
                        );
                    }
                    logger.log(mars_xlog::LogLevel::Verbose, None, line);
                }
            });
        spawned.ok()?;
        Some(Self { tx, dropped })
    }
}

impl LogSink for XlogSink {
    fn write(&self, _record: &LogRecord, line: &str) {
        match self.tx.try_send(line.to_owned()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for XlogSink {
    fn drop(&mut self) {
        // Dropping `tx` closes the queue. The worker exits after the remaining lines.
        // Detach: the render thread must not join or flush.
        let _ = Duration::from_millis(0);
    }
}

static INSTALLED: OnceLock<()> = OnceLock::new();

/// Installs the process subscriber. A second call is ignored.
pub fn install(log_dir: Option<&std::path::Path>, devtools: Option<Arc<DiagnosticsHub>>) {
    if INSTALLED.set(()).is_err() {
        return;
    }
    let mut sinks: Vec<Arc<dyn LogSink>> = vec![Arc::new(ConsoleSink)];
    if let Some(dir) = log_dir
        && std::fs::create_dir_all(dir).is_ok()
        && let Some(file) = XlogSink::open(dir)
    {
        sinks.push(Arc::new(file));
    }
    if let Some(devtools) = devtools {
        sinks.push(Arc::new(DevtoolSink::new(devtools)));
    }
    let subscriber = ImpastoSubscriber::new(sinks);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// Replaces the process filter and rebuilds callsite interest.
///
/// Pass a whole [`Targets`] (`with_default` plus any `with_target`). The lock
/// is dropped before the rebuild, because `register_callsite` reads it.
pub fn set_log_targets(targets: Targets) {
    {
        let mut slot = lock(shared_filter());
        *slot = targets;
    }
    tracing::callsite::rebuild_interest_cache();
}

/// `info`, or `info,impasto::render=warn` when a prefix is set. For display.
pub(crate) fn log_filter_text() -> String {
    format_targets(&lock(shared_filter()))
}

fn shared_filter() -> &'static Mutex<Targets> {
    static FILTER: OnceLock<Mutex<Targets>> = OnceLock::new();
    FILTER.get_or_init(|| Mutex::new(Targets::new().with_default(default_level())))
}

fn default_level() -> Level {
    if cfg!(debug_assertions) {
        Level::TRACE
    } else {
        Level::INFO
    }
}

fn max_level(targets: &Targets) -> LevelFilter {
    let mut max = targets.default_level().unwrap_or(LevelFilter::OFF);
    for (_, level) in targets.iter() {
        if level > max {
            max = level;
        }
    }
    max
}

fn format_targets(targets: &Targets) -> String {
    let mut specific: Vec<(&str, LevelFilter)> = targets.iter().collect();
    specific.sort_by(|left, right| left.0.cmp(right.0));
    let mut parts = Vec::new();
    if let Some(level) = targets.default_level() {
        parts.push(level.to_string());
    }
    for (target, level) in specific {
        parts.push(format!("{target}={level}"));
    }
    if parts.is_empty() {
        "off".to_owned()
    } else {
        parts.join(",")
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_quotes_values_with_spaces() {
        let record = LogRecord {
            level: Level::WARN,
            target: "impasto::layout",
            message: "overflow".to_owned(),
            fields: vec![
                ("axis", "horizontal".to_owned()),
                ("note", "a b".to_owned()),
            ],
            frame: Some(12),
        };
        assert_eq!(
            format_line(&record),
            "WARN impasto::layout overflow axis=horizontal note=\"a b\" frame=12"
        );
    }

    #[test]
    fn tighter_target_hides_info_under_that_prefix() {
        let targets = Targets::new()
            .with_default(Level::INFO)
            .with_target("impasto::render", LevelFilter::WARN);
        assert!(targets.would_enable("impasto::host", &Level::INFO));
        assert!(!targets.would_enable("impasto::host", &Level::DEBUG));
        assert!(targets.would_enable("impasto::render", &Level::WARN));
        assert!(!targets.would_enable("impasto::render", &Level::INFO));
        assert!(!targets.would_enable("impasto::render::gpu", &Level::INFO));
        assert_eq!(format_targets(&targets), "info,impasto::render=warn");
    }

    #[test]
    fn set_log_targets_replaces_the_process_filter() {
        let previous = lock(shared_filter()).clone();
        let _restore = Restore(previous);
        set_log_targets(
            Targets::new()
                .with_default(Level::DEBUG)
                .with_target("impasto::host", LevelFilter::ERROR),
        );
        let targets = lock(shared_filter());
        assert!(targets.would_enable("impasto::render", &Level::DEBUG));
        assert!(!targets.would_enable("impasto::host", &Level::WARN));
        assert_eq!(format_targets(&targets), "debug,impasto::host=error");
    }

    struct Restore(Targets);

    impl Drop for Restore {
        fn drop(&mut self) {
            set_log_targets(self.0.clone());
        }
    }
}
