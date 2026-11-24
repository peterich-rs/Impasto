//! Records tracing activations and events into a bounded, process-wide capture.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;

use super::telemetry::mono_ns;

const BATCH_SIZE: usize = 64;
const PRODUCER_LIMIT: usize = 64;
const SPAN_BYTES: usize = 8 * 1024 * 1024;
const MAX_FIELDS: usize = 12;
const MAX_TEXT: usize = 96;
const RECORDING: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TraceDetail {
    #[default]
    Stages = 1,
    Detailed = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Queue {
    #[default]
    Other,
    Platform,
    Ui,
    Raster,
    Io,
}

impl Queue {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Other => "Thread",
            Self::Platform => "Platform",
            Self::Ui => "UI",
            Self::Raster => "Raster",
            Self::Io => "IO",
        }
    }

    fn from_target(target: &str) -> Self {
        if target.starts_with("impasto::io") {
            Self::Io
        } else if target.starts_with("impasto::platform") {
            Self::Platform
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FieldValue {
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CapturedSpan {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub frame_id: Option<u64>,
    pub queue: Queue,
    pub thread_id: u64,
    pub thread_name: Arc<str>,
    pub target: &'static str,
    pub name: Cow<'static, str>,
    pub file: Option<&'static str>,
    pub line: Option<u32>,
    pub start_ns: u64,
    pub end_ns: u64,
    pub is_instant: bool,
    pub fields: Vec<(&'static str, FieldValue)>,
}

impl CapturedSpan {
    fn bytes(&self) -> usize {
        size_of::<Self>()
            + match &self.name {
                Cow::Owned(name) => name.capacity(),
                Cow::Borrowed(_) => 0,
            }
            + self.fields.capacity() * size_of::<(&str, FieldValue)>()
            + self
                .fields
                .iter()
                .map(|(_, value)| match value {
                    FieldValue::Str(text) => text.capacity(),
                    _ => 0,
                })
                .sum::<usize>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SpanLoss {
    pub overwritten: u64,
    pub open_at_stop: u64,
    pub producer_limit: u64,
}

#[derive(Default)]
struct SpanStore {
    spans: VecDeque<CapturedSpan>,
    bytes: usize,
    overwritten: u64,
}

impl SpanStore {
    fn append(&mut self, batch: &mut Vec<CapturedSpan>, cutoff: u64) {
        for span in batch.drain(..).filter(|span| span.end_ns <= cutoff) {
            let bytes = span.bytes();
            while self.bytes + bytes > SPAN_BYTES {
                let Some(removed) = self.spans.pop_front() else {
                    break;
                };
                self.bytes -= removed.bytes();
                self.overwritten += 1;
            }
            self.bytes += bytes;
            self.spans.push_back(span);
        }
    }
}

#[derive(Default)]
struct Producer {
    batch: Vec<CapturedSpan>,
    open: usize,
}

pub(super) struct TimelineRecording {
    pub id: u64,
    pub started_ns: u64,
    cutoff_ns: AtomicU64,
    finalized: AtomicBool,
    next_span: AtomicU64,
    producer_limit: AtomicU64,
    producers: Mutex<Vec<Arc<Mutex<Producer>>>>,
    store: Mutex<SpanStore>,
}

static RECORDING_ID: AtomicU64 = AtomicU64::new(0);
static DETAIL: AtomicU8 = AtomicU8::new(0);
static OWNER: Mutex<Option<Weak<TimelineRecording>>> = Mutex::new(None);

impl TimelineRecording {
    pub fn begin(id: u64, detail: TraceDetail) -> Option<Arc<Self>> {
        let mut owner = lock(&OWNER);
        if owner
            .as_ref()
            .is_some_and(|owner| owner.strong_count() != 0)
        {
            return None;
        }
        let recording = Arc::new(Self {
            id,
            started_ns: mono_ns(),
            cutoff_ns: AtomicU64::new(RECORDING),
            finalized: AtomicBool::new(false),
            next_span: AtomicU64::new(1),
            producer_limit: AtomicU64::new(0),
            producers: Mutex::new(Vec::new()),
            store: Mutex::default(),
        });
        *owner = Some(Arc::downgrade(&recording));
        RECORDING_ID.store(id, Ordering::Release);
        DETAIL.store(detail as u8, Ordering::Release);
        drop(owner);
        tracing::callsite::rebuild_interest_cache();
        Some(recording)
    }

    /// Stops admitting new work. Keeping ownership until finalization makes cutover atomic.
    pub fn stop(&self) -> Option<u64> {
        let cutoff = mono_ns();
        self.cutoff_ns
            .compare_exchange(RECORDING, cutoff, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        RECORDING_ID.store(0, Ordering::Release);
        DETAIL.store(0, Ordering::Release);
        tracing::callsite::rebuild_interest_cache();
        Some(cutoff)
    }

    pub fn cutoff(&self) -> u64 {
        self.cutoff_ns.load(Ordering::Acquire)
    }

    pub fn finish(&self, cutoff: u64) -> (VecDeque<CapturedSpan>, SpanLoss) {
        self.finalized.store(true, Ordering::Release);
        let producers = std::mem::take(&mut *lock(&self.producers));
        let mut open_at_stop = 0;
        for producer in producers {
            let mut producer = lock(&producer);
            open_at_stop += producer.open as u64;
            lock(&self.store).append(&mut producer.batch, cutoff);
        }
        let mut store = lock(&self.store);
        let mut spans = std::mem::take(&mut store.spans);
        spans.retain(|span| span.end_ns <= cutoff);
        let loss = SpanLoss {
            overwritten: store.overwritten,
            open_at_stop,
            producer_limit: self.producer_limit.load(Ordering::Relaxed),
        };
        (spans, loss)
    }

    fn producer(&self) -> Option<Arc<Mutex<Producer>>> {
        let mut producers = lock(&self.producers);
        if self.cutoff() != RECORDING {
            return None;
        }
        if producers.len() >= PRODUCER_LIMIT {
            self.producer_limit.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let producer = Arc::new(Mutex::new(Producer {
            batch: Vec::with_capacity(BATCH_SIZE),
            open: 0,
        }));
        producers.push(Arc::clone(&producer));
        Some(producer)
    }

    pub fn id_active(&self) -> bool {
        RECORDING_ID.load(Ordering::Acquire) == self.id
    }

    pub fn release(&self) {
        let mut owner = lock(&OWNER);
        if owner
            .as_ref()
            .is_some_and(|weak| std::ptr::eq(weak.as_ptr(), self))
        {
            *owner = None;
            RECORDING_ID.store(0, Ordering::Release);
            DETAIL.store(0, Ordering::Release);
        }
    }
}

impl Drop for TimelineRecording {
    fn drop(&mut self) {
        self.release();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy)]
struct Activation {
    tracing_id: u64,
    id: u64,
    parent_id: Option<u64>,
    frame_id: Option<u64>,
    queue: Queue,
    start_ns: u64,
}

struct ThreadCapture {
    id: u64,
    recording: Weak<TimelineRecording>,
    producer: Option<Arc<Mutex<Producer>>>,
    open: Vec<Activation>,
}

struct ThreadState {
    id: u64,
    name: Arc<str>,
    capture: Option<ThreadCapture>,
}

impl ThreadState {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let thread = std::thread::current();
        Self {
            id,
            name: Arc::from(thread.name().unwrap_or("unnamed")),
            capture: None,
        }
    }

    fn capture(&mut self) -> Option<&mut ThreadCapture> {
        let id = RECORDING_ID.load(Ordering::Acquire);
        if id == 0 {
            return None;
        }
        if self.capture.as_ref().is_none_or(|capture| capture.id != id) {
            let recording = lock(&OWNER).as_ref().and_then(Weak::upgrade)?;
            if recording.id != id {
                return None;
            }
            self.capture = Some(ThreadCapture {
                id,
                recording: Arc::downgrade(&recording),
                producer: recording.producer(),
                open: Vec::new(),
            });
        }
        self.capture.as_mut()
    }
}

thread_local! {
    static THREAD: RefCell<ThreadState> = RefCell::new(ThreadState::new());
}

#[derive(Default)]
struct SpanFields {
    frame_id: Option<u64>,
    queue: Option<Queue>,
    label: Option<String>,
    age_ns: Option<u64>,
    duration_ns: Option<u64>,
    fields: Vec<(&'static str, FieldValue)>,
}

impl SpanFields {
    fn set(&mut self, key: &'static str, value: FieldValue) {
        if let Some((_, slot)) = self.fields.iter_mut().find(|(name, _)| *name == key) {
            *slot = value;
        } else if self.fields.len() < MAX_FIELDS {
            self.fields.push((key, value));
        }
    }
}

fn allowed(key: &str) -> bool {
    matches!(
        key,
        "frame"
            | "frame_id"
            | "topology"
            | "vsync_ms"
            | "target_ms"
            | "cause"
            | "pass"
            | "node"
            | "count"
            | "request_id"
            | "surface"
            | "serial"
            | "submitted"
            | "ack_us"
            | "draws"
            | "slice"
            | "kind"
            | "status"
            | "updates"
            | "modules"
            | "passes"
            | "shaped"
            | "ok"
            | "cached"
            | "generation"
            | "components"
            | "computed"
            | "active_tracks"
            | "ui_woken"
            | "missed_vsync"
    )
}

fn limited(value: &str) -> String {
    let end = value.floor_char_boundary(MAX_TEXT.min(value.len()));
    value[..end].to_owned()
}

impl Visit for SpanFields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        if allowed(field.name()) {
            self.set(field.name(), FieldValue::I64(value));
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        let key = field.name();
        match key {
            "frame" | "frame_id" => self.frame_id = Some(value),
            "age_ns" => self.age_ns = Some(value),
            "duration_ns" => self.duration_ns = Some(value),
            _ => {}
        }
        if allowed(key) {
            self.set(key, FieldValue::U64(value));
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if allowed(field.name()) && value.is_finite() {
            self.set(field.name(), FieldValue::F64(value));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if allowed(field.name()) {
            self.set(field.name(), FieldValue::Bool(value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let key = field.name();
        match key {
            "label" => self.label = Some(limited(value)),
            "queue" => {
                self.queue = match value {
                    "platform" => Some(Queue::Platform),
                    "ui" => Some(Queue::Ui),
                    "raster" => Some(Queue::Raster),
                    "io" => Some(Queue::Io),
                    _ => None,
                }
            }
            "modules" => {
                let end = value.floor_char_boundary(640.min(value.len()));
                self.set(key, FieldValue::Str(value[..end].to_owned()));
            }
            _ if allowed(key) => self.set(key, FieldValue::Str(limited(value))),
            _ => {}
        }
    }

    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {
        // Trace fields must opt in as typed values. Debug output can contain arbitrary payloads.
    }
}

struct TimelineFilter;

fn enabled(metadata: &Metadata<'_>) -> bool {
    let detail = DETAIL.load(Ordering::Acquire);
    detail != 0
        && metadata.target().starts_with("impasto::")
        && (metadata.is_span() || metadata.fields().field("trace_event").is_some())
        && (*metadata.level() <= Level::DEBUG || detail == TraceDetail::Detailed as u8)
}

impl<S> Filter<S> for TimelineFilter {
    fn enabled(&self, metadata: &Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        enabled(metadata)
    }

    fn callsite_enabled(&self, metadata: &'static Metadata<'static>) -> Interest {
        if enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
}

struct TimelineLayer;

fn parent_of(open: &[Activation], parent: Option<&Id>) -> Option<Activation> {
    parent.and_then(|parent| {
        open.iter()
            .rev()
            .find(|entry| entry.tracing_id == parent.into_u64())
            .copied()
    })
}

fn submit(recording: &TimelineRecording, producer: &mut Producer, span: CapturedSpan) {
    if recording.finalized.load(Ordering::Acquire) || span.end_ns > recording.cutoff() {
        return;
    }
    let scope_finished = !span.is_instant && producer.open == 0;
    producer.batch.push(span);
    if scope_finished || producer.batch.len() >= BATCH_SIZE {
        lock(&recording.store).append(&mut producer.batch, recording.cutoff());
    }
}

impl<S> Layer<S> for TimelineLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut fields = SpanFields::default();
            attrs.record(&mut fields);
            span.extensions_mut().insert(fields);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(fields) = span.extensions_mut().get_mut::<SpanFields>()
        {
            values.record(fields);
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if RECORDING_ID.load(Ordering::Acquire) == 0 {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        let extensions = span.extensions();
        let fields = extensions.get::<SpanFields>();
        let parent = span.parent().map(|parent| parent.id());
        THREAD.with_borrow_mut(|thread| {
            let Some(capture) = thread.capture() else {
                return;
            };
            let (Some(recording), Some(producer)) =
                (capture.recording.upgrade(), &capture.producer)
            else {
                return;
            };
            let mut producer = lock(producer);
            let start_ns = mono_ns();
            if !recording.id_active() || start_ns > recording.cutoff() {
                return;
            }
            let parent = parent_of(&capture.open, parent.as_ref());
            let frame_id = fields.and_then(|state| state.frame_id);
            let queue = fields.and_then(|state| state.queue);
            capture.open.push(Activation {
                tracing_id: id.into_u64(),
                id: recording.next_span.fetch_add(1, Ordering::Relaxed),
                parent_id: parent.map(|entry| entry.id),
                frame_id: frame_id.or_else(|| parent.and_then(|entry| entry.frame_id)),
                queue: queue
                    .or_else(|| parent.map(|entry| entry.queue))
                    .unwrap_or_else(|| Queue::from_target(span.metadata().target())),
                start_ns,
            });
            producer.open += 1;
        });
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let _ = THREAD.try_with(|thread| {
            let mut thread = thread.borrow_mut();
            let thread_id = thread.id;
            let thread_name = Arc::clone(&thread.name);
            let Some(capture) = thread.capture.as_mut() else {
                return;
            };
            let Some(index) = capture
                .open
                .iter()
                .rposition(|entry| entry.tracing_id == id.into_u64())
            else {
                return;
            };
            let entry = capture.open.remove(index);
            let (Some(recording), Some(producer)) =
                (capture.recording.upgrade(), &capture.producer)
            else {
                return;
            };
            let end_ns = mono_ns();
            let mut producer = lock(producer);
            if end_ns > recording.cutoff() {
                // Finalization accounts for intervals open at the cutoff.
                return;
            }
            producer.open -= 1;
            let Some(span) = ctx.span(id) else { return };
            let metadata = span.metadata();
            let fields = span
                .extensions()
                .get::<SpanFields>()
                .map_or_else(Vec::new, |state| state.fields.clone());
            submit(
                &recording,
                &mut producer,
                CapturedSpan {
                    id: entry.id,
                    parent_id: entry.parent_id,
                    frame_id: entry.frame_id,
                    queue: entry.queue,
                    thread_id,
                    thread_name,
                    target: metadata.target(),
                    name: Cow::Borrowed(metadata.name()),
                    file: metadata.file(),
                    line: metadata.line(),
                    start_ns: entry.start_ns,
                    end_ns,
                    is_instant: false,
                    fields,
                },
            );
        });
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if RECORDING_ID.load(Ordering::Acquire) == 0 {
            return;
        }
        let now = mono_ns();
        let mut fields = SpanFields::default();
        event.record(&mut fields);
        let parent = ctx.event_span(event).map(|span| span.id());
        THREAD.with_borrow_mut(|thread| {
            let thread_id = thread.id;
            let thread_name = Arc::clone(&thread.name);
            let Some(capture) = thread.capture() else {
                return;
            };
            let (Some(recording), Some(producer)) =
                (capture.recording.upgrade(), &capture.producer)
            else {
                return;
            };
            let start_ns = now.saturating_sub(fields.age_ns.unwrap_or(0));
            if start_ns < recording.started_ns || now > recording.cutoff() {
                return;
            }
            let current = parent_of(&capture.open, parent.as_ref());
            let parent = current.filter(|entry| entry.start_ns <= start_ns);
            let metadata = event.metadata();
            let historical = fields.duration_ns.is_some();
            submit(
                &recording,
                &mut lock(producer),
                CapturedSpan {
                    id: recording.next_span.fetch_add(1, Ordering::Relaxed),
                    parent_id: parent.map(|entry| entry.id),
                    frame_id: fields
                        .frame_id
                        .or_else(|| current.and_then(|entry| entry.frame_id)),
                    queue: fields
                        .queue
                        .or_else(|| current.map(|entry| entry.queue))
                        .unwrap_or_else(|| Queue::from_target(metadata.target())),
                    thread_id,
                    thread_name,
                    target: metadata.target(),
                    name: fields
                        .label
                        .map_or(Cow::Borrowed(metadata.name()), Cow::Owned),
                    file: metadata.file(),
                    line: metadata.line(),
                    start_ns,
                    end_ns: start_ns
                        .saturating_add(fields.duration_ns.unwrap_or(0))
                        .min(now),
                    is_instant: !historical,
                    fields: fields.fields,
                },
            );
        });
    }
}

pub(crate) fn layer<S>() -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    TimelineLayer.with_filter(TimelineFilter)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::diagnostics::TelemetryHub;
    use crate::diagnostics::hub::{CAPTURE_TEST_LOCK, CaptureOptions};

    #[test]
    fn cutoff_collects_completed_children_on_other_threads() {
        let _serial = CAPTURE_TEST_LOCK.lock().unwrap();
        let hub = Arc::new(TelemetryHub::new());
        let id = hub
            .begin_capture(
                None,
                CaptureOptions {
                    detail: TraceDetail::Stages,
                    gpu: false,
                },
            )
            .unwrap();
        let other = TelemetryHub::new();
        assert!(!hub.collects_gpu());
        assert!(
            other
                .begin_capture(
                    None,
                    CaptureOptions {
                        detail: TraceDetail::Detailed,
                        gpu: false
                    }
                )
                .is_none()
        );
        assert!(hub.take_capture(id + 1).is_none());

        let (done, observed) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let subscriber = tracing_subscriber::registry().with(layer());
            tracing::subscriber::with_default(subscriber, || {
                let _root = tracing::info_span!(target: "impasto::frame", "frame", frame = 8_u64, queue = "ui").entered();
                {
                    let _child = tracing::debug_span!(target: "impasto::layout", "layout", source = "private path", cause = "metrics").entered();
                    tracing::event!(name: "metrics", target: "impasto::layout", Level::DEBUG,
                        trace_event = true, passes = 2_u64);
                    tracing::event!(name: "deep", target: "impasto::layout", Level::TRACE,
                        trace_event = true);
                }
                done.send(()).unwrap();
                wait.recv().unwrap();
            });
        });
        observed.recv().unwrap();
        let capture = hub.take_capture(id).unwrap();
        release.send(()).unwrap();
        thread.join().unwrap();

        assert_eq!(capture.span_loss.open_at_stop, 1);
        assert_eq!(capture.spans.len(), 2);
        let metrics = &capture.spans[0];
        let layout = &capture.spans[1];
        assert_eq!(metrics.parent_id, Some(layout.id));
        assert_eq!(layout.frame_id, Some(8));
        assert_eq!(layout.queue, Queue::Ui);
        assert_eq!(
            layout.fields,
            [("cause", FieldValue::Str("metrics".to_owned()))]
        );

        let next = other
            .begin_capture(
                None,
                CaptureOptions {
                    detail: TraceDetail::Detailed,
                    gpu: false,
                },
            )
            .unwrap();
        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(name: "deep", target: "impasto::layout", Level::TRACE,
                trace_event = true);
        });
        let next = other.take_capture(next).unwrap();
        assert_eq!(next.spans.len(), 1);
        assert_eq!(next.spans[0].name, "deep");
    }
}
