//! Pure-Rust Perfetto `.pftrace` protobuf exporter.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::PathBuf;

use prost::encoding::message;

use super::hub::TraceCapture;
use super::pftrace::{
    CHILD_ORDERING_EXPLICIT, COUNTER_UNIT_COUNT, COUNTER_UNIT_SIZE_BYTES, CounterDescriptor,
    DebugAnnotation, DebugAnnotationName, EVENT_TYPE_COUNTER, EVENT_TYPE_INSTANT,
    EVENT_TYPE_SLICE_BEGIN, EVENT_TYPE_SLICE_END, EventCategory, EventName, InternedData,
    ProcessDescriptor, SEQ_INCREMENTAL_STATE_CLEARED, SEQ_NEEDS_INCREMENTAL_STATE, SourceLocation,
    TracePacket, TrackDescriptor, TrackEvent,
};
use super::timeline::{CapturedSpan, FieldValue, Queue};

const SEQUENCE_ID: u32 = 1;
const PROCESS_TRACK_UUID: u64 = 1;
const PLATFORM_TRACK_UUID: u64 = 10;
const QUEUE_TRACK_UUID: u64 = 11;
const FALLBACK_UI_TRACK_UUID: u64 = 12;
const FALLBACK_RASTER_TRACK_UUID: u64 = 13;
const GPU_SUMMARY_TRACK_UUID: u64 = 20;
const COUNTER_TRACK_BASE_UUID: u64 = 100;
const FIRST_SPAN_TRACK_UUID: u64 = 1_000;

struct InternTable {
    categories: HashMap<String, u64>,
    names: HashMap<String, u64>,
    annotations: HashMap<String, u64>,
    locations: HashMap<(String, String, u32), u64>,
    pending: InternedData,
}

impl InternTable {
    fn new() -> Self {
        Self {
            categories: HashMap::new(),
            names: HashMap::new(),
            annotations: HashMap::new(),
            locations: HashMap::new(),
            pending: InternedData::default(),
        }
    }

    fn category(&mut self, name: &str) -> u64 {
        if let Some(&iid) = self.categories.get(name) {
            return iid;
        }
        let iid = (self.categories.len() + 1) as u64;
        self.categories.insert(name.to_owned(), iid);
        self.pending.event_categories.push(EventCategory {
            iid: Some(iid),
            name: Some(name.to_owned()),
        });
        iid
    }

    fn name(&mut self, name: &str) -> u64 {
        if let Some(&iid) = self.names.get(name) {
            return iid;
        }
        let iid = (self.names.len() + 1) as u64;
        self.names.insert(name.to_owned(), iid);
        self.pending.event_names.push(EventName {
            iid: Some(iid),
            name: Some(name.to_owned()),
        });
        iid
    }

    fn annotation(&mut self, name: &str) -> u64 {
        if let Some(&iid) = self.annotations.get(name) {
            return iid;
        }
        let iid = (self.annotations.len() + 1) as u64;
        self.annotations.insert(name.to_owned(), iid);
        self.pending
            .debug_annotation_names
            .push(DebugAnnotationName {
                iid: Some(iid),
                name: Some(name.to_owned()),
            });
        iid
    }

    fn source_location(&mut self, file: &str, function: &str, line: u32) -> u64 {
        let key = (file.to_owned(), function.to_owned(), line);
        if let Some(&iid) = self.locations.get(&key) {
            return iid;
        }
        let iid = (self.locations.len() + 1) as u64;
        self.locations.insert(key, iid);
        self.pending.source_locations.push(SourceLocation {
            iid: Some(iid),
            file_name: Some(file.to_owned()),
            function_name: Some(function.to_owned()),
            line_number: Some(line),
        });
        iid
    }

    fn take_pending(&mut self) -> Option<InternedData> {
        if self.pending.event_categories.is_empty()
            && self.pending.event_names.is_empty()
            && self.pending.debug_annotation_names.is_empty()
            && self.pending.source_locations.is_empty()
        {
            return None;
        }
        Some(std::mem::take(&mut self.pending))
    }
}

struct Builder {
    start_ns: u64,
    intern: InternTable,
    descriptors: Vec<u8>,
    event_bytes: Vec<u8>,
    events: Vec<EncodedEvent>,
    next_track: u64,
    next_flow: u64,
}

struct EncodedEvent {
    timestamp: u64,
    bytes: Range<usize>,
}

struct SliceSpec<'a> {
    track_uuid: u64,
    category: &'a str,
    name: &'a str,
    start: u64,
    end: u64,
    annotations: Vec<DebugAnnotation>,
    flow_ids: Vec<u64>,
    terminating_flow_ids: Vec<u64>,
}

impl Builder {
    fn new(start_ns: u64) -> Self {
        let pid = i32::try_from(std::process::id()).unwrap_or(1);
        let mut builder = Self {
            start_ns,
            intern: InternTable::new(),
            descriptors: Vec::new(),
            event_bytes: Vec::new(),
            events: Vec::new(),
            next_track: FIRST_SPAN_TRACK_UUID,
            next_flow: 1,
        };
        builder.push_descriptor(TrackDescriptor {
            uuid: Some(PROCESS_TRACK_UUID),
            name: Some("Impasto".to_owned()),
            process: Some(ProcessDescriptor {
                pid: Some(pid),
                process_name: Some("Impasto".to_owned()),
            }),
            child_ordering: Some(CHILD_ORDERING_EXPLICIT),
            ..Default::default()
        });
        builder.push_named_track(PLATFORM_TRACK_UUID, PROCESS_TRACK_UUID, "Platform", 1);
        builder.push_named_track(
            QUEUE_TRACK_UUID,
            PROCESS_TRACK_UUID,
            "UI to Raster Queue",
            2,
        );
        builder
    }

    fn rel_ns(&self, ns: u64) -> u64 {
        ns.saturating_sub(self.start_ns)
    }

    fn push_descriptor(&mut self, descriptor: TrackDescriptor) {
        let packet = TracePacket {
            track_descriptor: Some(descriptor),
            ..Default::default()
        };
        message::encode(1, &packet, &mut self.descriptors);
    }

    fn push_named_track(&mut self, uuid: u64, parent_uuid: u64, name: &str, rank: i32) {
        self.push_descriptor(TrackDescriptor {
            uuid: Some(uuid),
            parent_uuid: Some(parent_uuid),
            name: Some(name.to_owned()),
            sibling_order_rank: Some(rank),
            child_ordering: Some(CHILD_ORDERING_EXPLICIT),
            ..Default::default()
        });
    }

    fn push_counter_track(&mut self, uuid: u64, name: &str, unit: i32, rank: i32) {
        self.push_descriptor(TrackDescriptor {
            uuid: Some(uuid),
            parent_uuid: Some(PROCESS_TRACK_UUID),
            name: Some(name.to_owned()),
            sibling_order_rank: Some(rank),
            counter: Some(CounterDescriptor {
                unit: Some(unit),
                unit_name: None,
            }),
            ..Default::default()
        });
    }

    fn push_event(&mut self, timestamp: u64, event: TrackEvent) {
        let packet = TracePacket {
            timestamp: Some(timestamp),
            track_event: Some(event),
            trusted_packet_sequence_id: Some(SEQUENCE_ID),
            sequence_flags: Some(SEQ_NEEDS_INCREMENTAL_STATE),
            ..Default::default()
        };
        let start = self.event_bytes.len();
        message::encode(1, &packet, &mut self.event_bytes);
        self.events.push(EncodedEvent {
            timestamp,
            bytes: start..self.event_bytes.len(),
        });
    }

    fn debug_annotations(&mut self, fields: &[(&'static str, FieldValue)]) -> Vec<DebugAnnotation> {
        fields
            .iter()
            .map(|(key, value)| {
                // Keep the JavaScript sample annotation self-describing so a
                // host tool can enrich it without rewriting interning state.
                if let ("js_stack", FieldValue::Str(stack)) = (*key, value) {
                    return DebugAnnotation {
                        name: Some((*key).to_owned()),
                        string_value: Some(stack.clone()),
                        ..Default::default()
                    };
                }
                let name_iid = Some(self.intern.annotation(key));
                match value {
                    FieldValue::I64(v) => DebugAnnotation {
                        name_iid,
                        int_value: Some(*v),
                        ..Default::default()
                    },
                    FieldValue::U64(v) => DebugAnnotation {
                        name_iid,
                        uint_value: Some(*v),
                        ..Default::default()
                    },
                    FieldValue::F64(v) => DebugAnnotation {
                        name_iid,
                        double_value: Some(*v),
                        ..Default::default()
                    },
                    FieldValue::Bool(v) => DebugAnnotation {
                        name_iid,
                        bool_value: Some(*v),
                        ..Default::default()
                    },
                    FieldValue::Str(v) => DebugAnnotation {
                        name_iid,
                        string_value: Some(v.clone()),
                        ..Default::default()
                    },
                }
            })
            .collect()
    }

    fn emit_slice(&mut self, spec: SliceSpec<'_>) {
        let cat_iid = self.intern.category(spec.category);
        let name_iid = self.intern.name(spec.name);
        self.push_event(
            spec.start,
            TrackEvent {
                category_iids: vec![cat_iid],
                name_iid: Some(name_iid),
                r#type: Some(EVENT_TYPE_SLICE_BEGIN),
                track_uuid: Some(spec.track_uuid),
                debug_annotations: spec.annotations,
                flow_ids: spec.flow_ids,
                terminating_flow_ids: spec.terminating_flow_ids,
                ..Default::default()
            },
        );
        self.push_event(
            spec.end.max(spec.start),
            TrackEvent {
                r#type: Some(EVENT_TYPE_SLICE_END),
                track_uuid: Some(spec.track_uuid),
                ..Default::default()
            },
        );
    }

    fn emit_counter(&mut self, track_uuid: u64, ts_ns: u64, value: i64) {
        self.push_event(
            ts_ns,
            TrackEvent {
                r#type: Some(EVENT_TYPE_COUNTER),
                track_uuid: Some(track_uuid),
                counter_value: Some(value),
                ..Default::default()
            },
        );
    }

    fn end_slice(&mut self, track_uuid: u64, at: u64) {
        self.push_event(
            self.rel_ns(at),
            TrackEvent {
                r#type: Some(EVENT_TYPE_SLICE_END),
                track_uuid: Some(track_uuid),
                ..Default::default()
            },
        );
    }

    fn handoff_flows<'a>(
        &mut self,
        spans: impl IntoIterator<Item = &'a CapturedSpan>,
    ) -> HashMap<u64, (Vec<u64>, Vec<u64>)> {
        #[derive(Default)]
        struct Handoff<'a> {
            start: Option<&'a CapturedSpan>,
            end: Option<&'a CapturedSpan>,
        }
        let mut handoffs = BTreeMap::new();
        for span in spans {
            let field = |name| {
                span.fields.iter().find_map(|(key, value)| {
                    if *key == name
                        && let FieldValue::U64(id) = value
                    {
                        Some(*id)
                    } else {
                        None
                    }
                })
            };
            let (kind, start, id, owner) = match span.name.as_ref() {
                "image_requested" => (0, true, field("request_id"), 0),
                "image_job" => (0, false, field("request_id"), 0),
                "image_ready" => (1, true, field("request_id"), 0),
                "image_cache_apply" => (1, false, field("request_id"), 0),
                "uikit_publish" => (2, true, field("serial"), field("surface").unwrap_or(0)),
                "uikit_ack" => (2, false, field("serial"), field("surface").unwrap_or(0)),
                _ => continue,
            };
            let Some(id) = id else { continue };
            let handoff: &mut Handoff<'_> = handoffs.entry((kind, owner, id)).or_default();
            if start {
                handoff.start = Some(span);
            } else {
                handoff.end = Some(span);
            }
        }
        let mut flows: HashMap<u64, (Vec<u64>, Vec<u64>)> = HashMap::new();
        for handoff in handoffs.values() {
            if let (Some(start), Some(end)) = (handoff.start, handoff.end)
                && start.start_ns <= end.start_ns
            {
                let flow = self.flow();
                flows.entry(start.id).or_default().0.push(flow);
                flows.entry(end.id).or_default().1.push(flow);
            }
        }
        flows
    }

    fn emit_spans(&mut self, spans: &VecDeque<CapturedSpan>) -> HashMap<(u64, Queue), u64> {
        #[derive(Default)]
        struct Lane {
            track: u64,
            open: Vec<(u64, u64)>,
        }

        let mut flows = self.handoff_flows(spans);
        let mut by_queue: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for span in spans {
            by_queue
                .entry((span.thread_id, span.queue))
                .or_default()
                .push(span);
        }
        let mut roots = HashMap::new();
        for ((_, queue), mut entries) in by_queue {
            entries.sort_by_key(|span| (span.start_ns, span.id));
            let mut lanes: Vec<Lane> = Vec::new();
            let name = format!("{} · {}", queue.name(), entries[0].thread_name);
            for span in entries {
                for lane in &mut lanes {
                    while let Some(&(id, end)) = lane.open.last() {
                        if end > span.start_ns || Some(id) == span.parent_id && end == span.start_ns
                        {
                            break;
                        }
                        lane.open.pop();
                        self.end_slice(lane.track, end);
                    }
                }
                // A historical interval or explicit sibling can overlap an open slice.
                // Perfetto stacks are LIFO, so place that interval on a separate track.
                let lane_index = lanes
                    .iter()
                    .position(|lane| {
                        lane.open.last().is_some_and(|&(id, end)| {
                            Some(id) == span.parent_id && end >= span.end_ns
                        })
                    })
                    .or_else(|| lanes.iter().position(|lane| lane.open.is_empty()));
                let index = lane_index.unwrap_or_else(|| {
                    let index = lanes.len();
                    let track = self.next_track;
                    self.next_track += 1;
                    let label = if index == 0 {
                        name.clone()
                    } else {
                        format!("{name} · {}", index + 1)
                    };
                    self.push_named_track(track, PROCESS_TRACK_UUID, &label, 10);
                    lanes.push(Lane {
                        track,
                        ..Lane::default()
                    });
                    index
                });
                let lane = &mut lanes[index];
                if let Some(frame) = span.frame_id
                    && !span.is_instant
                    && matches!(
                        (queue, span.target, span.name.as_ref()),
                        (Queue::Ui, "impasto::frame", "frame")
                            | (Queue::Raster, "impasto::render", "raster")
                    )
                {
                    roots.insert((frame, queue), lane.track);
                }

                let cat_iid = self.intern.category(span.target);
                let name_iid = self.intern.name(span.name.as_ref());
                let source_location_iid = span.file.map(|file| {
                    self.intern
                        .source_location(file, span.name.as_ref(), span.line.unwrap_or(0))
                });
                let mut debug_annotations = self.debug_annotations(&span.fields);
                debug_annotations
                    .extend(self.debug_annotations(&[("span_id", FieldValue::U64(span.id))]));
                if let Some(frame_id) = span.frame_id {
                    debug_annotations
                        .extend(self.debug_annotations(&[("frame_id", FieldValue::U64(frame_id))]));
                }
                if let Some(parent_id) = span.parent_id {
                    debug_annotations.extend(
                        self.debug_annotations(&[("parent_span_id", FieldValue::U64(parent_id))]),
                    );
                }
                let (flow_ids, terminating_flow_ids) = flows.remove(&span.id).unwrap_or_default();
                self.push_event(
                    self.rel_ns(span.start_ns),
                    TrackEvent {
                        category_iids: vec![cat_iid],
                        name_iid: Some(name_iid),
                        source_location_iid,
                        r#type: Some(if span.is_instant {
                            EVENT_TYPE_INSTANT
                        } else {
                            EVENT_TYPE_SLICE_BEGIN
                        }),
                        track_uuid: Some(lane.track),
                        debug_annotations,
                        flow_ids,
                        terminating_flow_ids,
                        ..Default::default()
                    },
                );
                if !span.is_instant {
                    if span.start_ns == span.end_ns {
                        self.end_slice(lane.track, span.end_ns);
                    } else {
                        lane.open.push((span.id, span.end_ns));
                    }
                }
            }
            for lane in lanes {
                for (_, end) in lane.open.into_iter().rev() {
                    self.end_slice(lane.track, end);
                }
            }
        }
        roots
    }

    fn instant(
        &mut self,
        track: u64,
        at: u64,
        name: &str,
        fields: &[(&'static str, FieldValue)],
        flows: (Option<u64>, Option<u64>),
    ) {
        let name_iid = self.intern.name(name);
        let category = if track == GPU_SUMMARY_TRACK_UUID {
            "impasto::gpu"
        } else {
            "impasto::schedule"
        };
        let category_iid = self.intern.category(category);
        let debug_annotations = self.debug_annotations(fields);
        self.push_event(
            self.rel_ns(at),
            TrackEvent {
                category_iids: vec![category_iid],
                name_iid: Some(name_iid),
                r#type: Some(EVENT_TYPE_INSTANT),
                track_uuid: Some(track),
                debug_annotations,
                flow_ids: flows.0.into_iter().collect(),
                terminating_flow_ids: flows.1.into_iter().collect(),
                ..TrackEvent::default()
            },
        );
    }

    fn flow(&mut self) -> u64 {
        let flow = self.next_flow;
        self.next_flow += 1;
        flow
    }

    fn emit_frames(&mut self, capture: &TraceCapture, roots: &HashMap<(u64, Queue), u64>) {
        if capture.frames.is_empty() {
            return;
        }
        if capture.frames.iter().any(|frame| {
            !frame.schedule.compositor_only && !roots.contains_key(&(frame.number, Queue::Ui))
        }) {
            self.push_named_track(FALLBACK_UI_TRACK_UUID, PROCESS_TRACK_UUID, "UI summary", 3);
        }
        if capture
            .frames
            .iter()
            .any(|frame| !roots.contains_key(&(frame.number, Queue::Raster)))
        {
            self.push_named_track(
                FALLBACK_RASTER_TRACK_UUID,
                PROCESS_TRACK_UUID,
                "Raster summary",
                4,
            );
        }
        self.push_named_track(
            GPU_SUMMARY_TRACK_UUID,
            PROCESS_TRACK_UUID,
            "GPU measurements (frame-relative)",
            50,
        );

        let counter_specs: [(&str, i32); 18] = [
            ("Raster draws", COUNTER_UNIT_COUNT),
            ("Raster passes", COUNTER_UNIT_COUNT),
            ("Raster offscreen passes", COUNTER_UNIT_COUNT),
            ("Raster compute passes", COUNTER_UNIT_COUNT),
            ("Raster buffers created", COUNTER_UNIT_COUNT),
            ("Raster bind groups created", COUNTER_UNIT_COUNT),
            ("Upload vertex (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Upload uniform (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Upload stops (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Upload extra (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Upload mesh (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Upload atlas (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Offscreen allocated (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Offscreen peak (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Offscreen pooled (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("Offscreen transient (bytes)", COUNTER_UNIT_SIZE_BYTES),
            ("UI layout passes", COUNTER_UNIT_COUNT),
            ("UI text shaped", COUNTER_UNIT_COUNT),
        ];
        for (idx, (name, unit)) in counter_specs.iter().enumerate() {
            let uuid = COUNTER_TRACK_BASE_UUID + idx as u64;
            let rank = i32::try_from(100 + idx).unwrap_or(i32::MAX);
            self.push_counter_track(uuid, name, *unit, rank);
        }
        let ui_updates_uuid = COUNTER_TRACK_BASE_UUID + 18;
        let ui_sets_uuid = COUNTER_TRACK_BASE_UUID + 19;
        self.push_counter_track(ui_updates_uuid, "UI updates", COUNTER_UNIT_COUNT, 118);
        self.push_counter_track(ui_sets_uuid, "UI sets", COUNTER_UNIT_COUNT, 119);

        let has_runtime = capture.frames.iter().any(|f| f.runtime.is_some());
        let has_heap = capture
            .frames
            .iter()
            .any(|f| f.runtime.and_then(|r| r.heap_bytes).is_some());
        if has_heap {
            self.push_counter_track(
                COUNTER_TRACK_BASE_UUID + 20,
                "QuickJS heap (bytes)",
                COUNTER_UNIT_SIZE_BYTES,
                120,
            );
        }
        if has_runtime {
            self.push_counter_track(
                COUNTER_TRACK_BASE_UUID + 21,
                "JS promises",
                COUNTER_UNIT_COUNT,
                121,
            );
            self.push_counter_track(
                COUNTER_TRACK_BASE_UUID + 22,
                "JS timers",
                COUNTER_UNIT_COUNT,
                122,
            );
            self.push_counter_track(
                COUNTER_TRACK_BASE_UUID + 23,
                "JS requests",
                COUNTER_UNIT_COUNT,
                123,
            );
        }

        for frame in &capture.frames {
            let s = frame.schedule;
            let fields = [("frame_id", FieldValue::U64(frame.number))];
            let ui = roots.get(&(frame.number, Queue::Ui)).copied();
            let raster = roots.get(&(frame.number, Queue::Raster)).copied();
            let ui_track = ui.unwrap_or(FALLBACK_UI_TRACK_UUID);
            let raster_track = raster.unwrap_or(FALLBACK_RASTER_TRACK_UUID);
            if ui.is_none() && !s.compositor_only {
                let annotations = self.debug_annotations(&[
                    fields[0].clone(),
                    ("script_ms", FieldValue::F64(frame.script_ms)),
                    ("layout_ms", FieldValue::F64(frame.layout_ms)),
                    ("record_ms", FieldValue::F64(frame.record_ms)),
                ]);
                self.emit_slice(SliceSpec {
                    track_uuid: ui_track,
                    category: "impasto::frame",
                    name: "UI frame",
                    start: self.rel_ns(s.ui_start_ns),
                    end: self.rel_ns(s.ui_done_ns),
                    annotations,
                    flow_ids: Vec::new(),
                    terminating_flow_ids: Vec::new(),
                });
            }
            if raster.is_none() {
                let annotations = self.debug_annotations(&fields);
                self.emit_slice(SliceSpec {
                    track_uuid: raster_track,
                    category: "impasto::render",
                    name: "Raster frame",
                    start: self.rel_ns(s.raster_start_ns),
                    end: self.rel_ns(s.present_done_ns),
                    annotations,
                    flow_ids: Vec::new(),
                    terminating_flow_ids: Vec::new(),
                });
            }

            if let Some(vsync) = s.vsync_ns {
                let flow = self.flow();
                self.instant(
                    PLATFORM_TRACK_UUID,
                    vsync,
                    "Vsync received",
                    &fields,
                    (Some(flow), None),
                );
                let (track, timestamp, name) = if s.compositor_only {
                    (raster_track, s.raster_start_ns, "Raster started")
                } else {
                    (ui_track, s.ui_start_ns, "UI started")
                };
                self.instant(track, timestamp, name, &fields, (None, Some(flow)));
            }
            if !s.compositor_only {
                let enqueue = self.flow();
                let dequeue = self.flow();
                self.instant(
                    ui_track,
                    s.ui_done_ns,
                    "UI submitted",
                    &fields,
                    (Some(enqueue), None),
                );
                let queue_fields = [
                    fields[0].clone(),
                    (
                        "wait_ns",
                        FieldValue::U64(s.raster_start_ns.saturating_sub(s.ui_done_ns)),
                    ),
                ];
                self.instant(
                    QUEUE_TRACK_UUID,
                    s.ui_done_ns,
                    "Submission queued",
                    &queue_fields,
                    (Some(dequeue), Some(enqueue)),
                );
                self.instant(
                    raster_track,
                    s.raster_start_ns,
                    "Raster started",
                    &fields,
                    (None, Some(dequeue)),
                );
            }
            if s.present_start_ns.is_some() {
                self.instant(
                    raster_track,
                    s.present_done_ns,
                    "Present returned",
                    &fields,
                    (None, None),
                );
            }
            let status = [
                fields[0].clone(),
                (
                    "gpu_status",
                    FieldValue::Str(frame.gpu.status.name().to_owned()),
                ),
            ];
            self.instant(
                GPU_SUMMARY_TRACK_UUID,
                s.present_done_ns,
                "GPU timing status",
                &status,
                (None, None),
            );
            if let Some(timing) = &frame.gpu_timing {
                for pass in &timing.passes {
                    let mut fields = vec![
                        fields[0].clone(),
                        (
                            "device_generation",
                            FieldValue::U64(frame.gpu.device_generation),
                        ),
                        ("offset_us", FieldValue::F64(pass.start_us)),
                        ("duration_us", FieldValue::F64(pass.duration_us)),
                        ("draws", FieldValue::U64(u64::from(pass.draws))),
                    ];
                    if let Some(slice) = pass.slice_index {
                        fields.push(("slice_index", FieldValue::U64(slice as u64)));
                    }
                    if let Some(id) = pass.shader_id() {
                        fields.push(("shader_id", FieldValue::U64(id.0)));
                    }
                    if let Some(id) = pass.view_id() {
                        fields.push(("view_id", FieldValue::Str(id.to_owned())));
                    }
                    self.instant(
                        GPU_SUMMARY_TRACK_UUID,
                        s.present_done_ns,
                        &format!("{:?}", pass.kind),
                        &fields,
                        (None, None),
                    );
                }
            }

            let ui_ts = self.rel_ns(s.ui_done_ns);
            let raster_ts = self.rel_ns(s.present_done_ns);
            let r = frame.raster.render;
            let values: [i64; 18] = [
                i64::from(r.draws),
                i64::from(r.passes),
                i64::from(r.offscreen_passes),
                i64::from(r.compute_passes),
                i64::from(r.buffers_created),
                i64::from(r.bind_groups_created),
                i64::try_from(r.vertex_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.uniform_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.stop_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.extra_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.mesh_upload_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.atlas_upload_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.offscreen_alloc_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.offscreen_peak_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.pooled_texture_bytes).unwrap_or(i64::MAX),
                i64::try_from(r.transient_bytes).unwrap_or(i64::MAX),
                i64::from(frame.phases.layout_passes),
                i64::from(frame.phases.text_shaped),
            ];
            for (idx, value) in values.into_iter().enumerate() {
                let ts = if idx < 16 { raster_ts } else { ui_ts };
                self.emit_counter(COUNTER_TRACK_BASE_UUID + idx as u64, ts, value);
            }
            self.emit_counter(ui_updates_uuid, ui_ts, i64::from(frame.updates));
            self.emit_counter(ui_sets_uuid, ui_ts, i64::from(frame.sets));
            if let Some(runtime) = frame.runtime {
                if let Some(heap) = runtime.heap_bytes {
                    self.emit_counter(
                        COUNTER_TRACK_BASE_UUID + 20,
                        ui_ts,
                        i64::try_from(heap).unwrap_or(i64::MAX),
                    );
                }
                self.emit_counter(
                    COUNTER_TRACK_BASE_UUID + 21,
                    ui_ts,
                    i64::try_from(runtime.promises).unwrap_or(i64::MAX),
                );
                self.emit_counter(
                    COUNTER_TRACK_BASE_UUID + 22,
                    ui_ts,
                    i64::try_from(runtime.timers).unwrap_or(i64::MAX),
                );
                self.emit_counter(
                    COUNTER_TRACK_BASE_UUID + 23,
                    ui_ts,
                    i64::try_from(runtime.requests).unwrap_or(i64::MAX),
                );
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        // TrackEvent field values and annotations can be freed as each packet
        // is built. Only encoded bytes and the timestamp sort index are kept.
        let interned = TracePacket {
            trusted_packet_sequence_id: Some(SEQUENCE_ID),
            sequence_flags: Some(SEQ_INCREMENTAL_STATE_CLEARED),
            interned_data: self.intern.take_pending(),
            ..Default::default()
        };
        let mut trace = Vec::with_capacity(
            message::encoded_len(1, &interned) + self.descriptors.len() + self.event_bytes.len(),
        );
        message::encode(1, &interned, &mut trace);
        trace.extend(self.descriptors);
        self.events
            .sort_unstable_by_key(|event| (event.timestamp, event.bytes.start));
        for event in self.events {
            trace.extend_from_slice(&self.event_bytes[event.bytes]);
        }
        trace
    }
}

pub(super) fn encode(capture: &TraceCapture) -> Vec<u8> {
    let mut builder = Builder::new(capture.started_ns);
    let roots = builder.emit_spans(&capture.spans);
    builder.emit_frames(capture, &roots);
    builder.instant(
        PLATFORM_TRACK_UUID,
        capture.stopped_ns,
        "Capture summary",
        &[
            ("capture_id", FieldValue::U64(capture.id)),
            (
                "overwritten_spans",
                FieldValue::U64(capture.span_loss.overwritten),
            ),
            (
                "open_spans_at_stop",
                FieldValue::U64(capture.span_loss.open_at_stop),
            ),
            (
                "producer_limit",
                FieldValue::U64(capture.span_loss.producer_limit),
            ),
            (
                "overwritten_frames",
                FieldValue::U64(capture.overwritten_frames),
            ),
            (
                "unfinished_frames",
                FieldValue::U64(capture.unfinished_frames as u64),
            ),
            ("pending_gpu", FieldValue::U64(capture.pending_gpu() as u64)),
        ],
        (None, None),
    );
    builder.finish()
}

pub(super) fn write(capture: &TraceCapture) -> io::Result<PathBuf> {
    let Some((path, overwrite)) = &capture.destination else {
        return Err(io::Error::other("trace file destination missing"));
    };
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "use a .pftrace destination",
        ));
    }
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(format!(
        ".impasto-{}-{}-{}.pftrace.tmp",
        std::process::id(),
        capture.id,
        capture.started_ns,
    ));
    let mut writer = BufWriter::new(
        File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)?,
    );
    let bytes = encode(capture);
    let written = writer.write_all(&bytes).and_then(|()| writer.flush());
    drop(writer);
    let committed = written.and_then(|()| {
        if *overwrite {
            fs::rename(&temporary, path)
        } else {
            fs::hard_link(&temporary, path)
        }
    });
    let _ = fs::remove_file(&temporary);
    committed?;
    Ok(path.clone())
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;

    use prost::Message;
    use raster::{GpuFrameTiming, GpuPassKind, GpuPassTiming, GpuSample, GpuTimingStatus};

    use super::*;
    use crate::diagnostics::hub::FrameSample;
    use crate::diagnostics::pftrace::Trace;
    use crate::diagnostics::telemetry::ScheduleTiming;

    fn sample_frame(number: u64, with_gpu: bool) -> FrameSample {
        let mut frame = FrameSample {
            number,
            schedule: ScheduleTiming {
                vsync_ns: Some(1_000_000),
                ui_start_ns: 1_050_000,
                ui_done_ns: 1_500_000,
                raster_start_ns: 1_550_000,
                submit_done_ns: Some(1_850_000),
                present_start_ns: Some(1_600_000),
                present_done_ns: 1_900_000,
                budget_ns: 8_333_333,
                missed_vsync: false,
                ..ScheduleTiming::default()
            },
            updates: 3,
            sets: 5,
            runtime: Some(engine::session::RuntimeSample {
                promises: 1,
                timers: 2,
                requests: 0,
                queued_events: 0,
                queued_bytes: 0,
                heap_bytes: Some(65536),
            }),
            ..FrameSample::default()
        };
        frame.raster.render.draws = 12;
        frame.raster.render.passes = 2;
        frame.phases.layout_passes = 2;
        frame.phases.text_shaped = 4;
        if with_gpu {
            frame.gpu = GpuSample {
                frame_id: number,
                device_generation: 7,
                status: GpuTimingStatus::Ready,
            };
            frame.gpu_timing = Some(Arc::new(GpuFrameTiming {
                clock_start_us: 100.0,
                frame_span_us: 400.0,
                passes_sum_us: 450.0,
                passes: vec![
                    GpuPassTiming {
                        kind: GpuPassKind::BackdropBlur,
                        source: None,
                        slice_index: Some(0),
                        draws: 4,
                        start_us: 0.0,
                        duration_us: 250.0,
                    },
                    GpuPassTiming {
                        kind: GpuPassKind::Main,
                        source: None,
                        slice_index: Some(0),
                        draws: 8,
                        start_us: 100.0,
                        duration_us: 200.0,
                    },
                ],
            }));
        }
        frame
    }

    fn fixture() -> TraceCapture {
        let span = |id, parent_id, name, start_ns, end_ns| CapturedSpan {
            id,
            parent_id,
            frame_id: Some(42),
            queue: Queue::Ui,
            thread_id: 1,
            thread_name: "main".into(),
            target: if name == "frame" {
                "impasto::frame"
            } else {
                "impasto::layout"
            },
            name: Cow::Borrowed(name),
            file: Some("layout.rs"),
            line: Some(1),
            start_ns,
            end_ns,
            ..CapturedSpan::default()
        };

        let mut queued_behind_42 = sample_frame(43, false);
        queued_behind_42.schedule = ScheduleTiming {
            ui_start_ns: 1_510_000,
            ui_done_ns: 1_530_000,
            raster_start_ns: 1_950_000,
            present_start_ns: Some(2_000_000),
            present_done_ns: 2_100_000,
            ..ScheduleTiming::default()
        };
        TraceCapture {
            id: 1,
            started_ns: 1_000_000,
            stopped_ns: 2_200_000,
            frames: VecDeque::from([sample_frame(42, true), queued_behind_42]),
            spans: VecDeque::from([
                span(3, Some(1), "layout", 1_200_000, 1_245_000),
                span(2, Some(1), "layout", 1_105_000, 1_150_000),
                span(5, Some(4), "taffy_compute", 1_300_000, 1_305_000),
                span(4, Some(1), "layout", 1_300_000, 1_345_000),
                span(1, None, "frame", 1_050_000, 1_500_000),
            ]),
            ..TraceCapture::default()
        }
    }

    #[test]
    fn encodes_nested_spans_flows_counters_and_frame_relative_gpu() {
        let bytes = encode(&fixture());
        let decoded = Trace::decode(&bytes[..]).expect("valid protobuf");
        let mut depth: HashMap<u64, i32> = HashMap::new();
        let mut starts = 0;
        let mut ends = 0;
        let mut counters = 0;
        for ev in decoded
            .packet
            .iter()
            .filter_map(|packet| packet.track_event.as_ref())
        {
            let track = ev.track_uuid.unwrap();
            starts += ev.flow_ids.len();
            ends += ev.terminating_flow_ids.len();
            match ev.r#type {
                Some(EVENT_TYPE_SLICE_BEGIN) => *depth.entry(track).or_default() += 1,
                Some(EVENT_TYPE_SLICE_END) => {
                    let entry = depth.entry(track).or_default();
                    *entry -= 1;
                    assert!(*entry >= 0, "unbalanced slice end on track {track}");
                }
                Some(EVENT_TYPE_COUNTER) => counters += 1,
                _ => {}
            }
        }
        assert!(depth.values().all(|depth| *depth == 0));
        assert_eq!((starts, ends, counters), (5, 5, 48));

        if let Some(path) = std::env::var_os("IMPASTO_TEST_TRACE") {
            fs::write(path, bytes).expect("write Perfetto integration fixture");
        }
    }
}
