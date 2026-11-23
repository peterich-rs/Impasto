//! Chrome Trace Event export for Perfetto.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use super::hub::{FrameSample, TraceCapture};

const PLATFORM: u32 = 1;
const UI: u32 = 2;
const RASTER: u32 = 3;
const IO: u32 = 4;
const GPU: u32 = 5;

struct Trace {
    pid: u32,
    start_ns: u64,
    events: Vec<Value>,
}

impl Trace {
    fn stamp(&self, ns: u64) -> f64 {
        ns.saturating_sub(self.start_ns) as f64 / 1_000.0
    }

    fn cpu_span(&mut self, lane: u32, name: &str, start: u64, end: u64, args: &Value) {
        if end >= start {
            self.span(
                lane,
                name,
                self.stamp(start),
                (end - start) as f64 / 1_000.0,
                args,
            );
        }
    }

    fn span(&mut self, lane: u32, name: &str, ts: f64, duration: f64, args: &Value) {
        self.events.push(json!({
            "ph": "X", "name": name, "pid": self.pid, "tid": lane,
            "ts": ts, "dur": duration, "args": args,
        }));
    }

    fn async_span(
        &mut self,
        lane: u32,
        name: &str,
        category: &str,
        id: &str,
        range_us: [f64; 2],
        args: &Value,
    ) {
        let [start, end] = range_us;
        self.events.push(json!({
            "ph": "b", "cat": category, "name": name, "pid": self.pid, "tid": lane,
            "ts": start, "id": id, "args": args,
        }));
        self.events.push(json!({
            "ph": "e", "cat": category, "name": name, "pid": self.pid, "tid": lane,
            "ts": end, "id": id,
        }));
    }

    fn counter(&mut self, lane: u32, name: &str, ns: u64, args: Value) {
        self.events.push(json!({
            "ph": "C", "name": name, "pid": self.pid, "tid": lane,
            "ts": self.stamp(ns), "args": args,
        }));
    }

    fn frame(&mut self, frame: &FrameSample) {
        let s = frame.schedule;
        let id = json!({ "frame_id": frame.number });
        let cookie = frame.number.to_string();
        if let Some(vsync) = s.vsync_ns {
            self.async_span(
                PLATFORM,
                "Vsync to UI",
                "Platform",
                &cookie,
                [self.stamp(vsync), self.stamp(s.ui_start_ns)],
                &id,
            );
        }
        self.cpu_span(
            UI,
            "UI frame",
            s.ui_start_ns,
            s.ui_done_ns,
            &json!({
                "frame_id": frame.number, "session_ms": frame.session_ms,
                "gesture_ms": frame.gesture_ms, "script_ms": frame.script_ms,
                "layout_ms": frame.layout_ms, "record_ms": frame.record_ms,
            }),
        );
        // Queue wait can overlap another frame's work on the Raster thread.
        self.async_span(
            RASTER,
            "UI to Raster",
            "Raster queue",
            &cookie,
            [self.stamp(s.ui_done_ns), self.stamp(s.raster_start_ns)],
            &id,
        );
        self.cpu_span(RASTER, "Raster frame", s.raster_start_ns, s.present_done_ns, &json!({
            "frame_id": frame.number, "topology": format!("{:?}", s.topology),
            "gpu": frame.gpu.status.name(), "budget_ns": s.budget_ns, "missed_vsync": s.missed_vsync,
            "picture_prepare_us": frame.raster.picture_prepare.as_secs_f64() * 1_000_000.0,
            "acquire_us": frame.raster.acquire.as_secs_f64() * 1_000_000.0,
            "encode_us": frame.raster.encode.as_secs_f64() * 1_000_000.0,
            "flush_us": frame.raster.flush.as_secs_f64() * 1_000_000.0,
            "queue_submit_us": frame.raster.queue_submit.as_secs_f64() * 1_000_000.0,
            "surface_present_us": frame.raster.surface_present.as_secs_f64() * 1_000_000.0,
        }));
        if let Some(present) = s.present_start_ns {
            if let Some(submit) = s.submit_done_ns {
                self.cpu_span(RASTER, "Acquire, encode and submit", present, submit, &id);
                self.cpu_span(RASTER, "Present return", submit, s.present_done_ns, &id);
            } else {
                self.cpu_span(RASTER, "Present attempt", present, s.present_done_ns, &id);
            }
        }
        let r = frame.raster.render;
        self.counter(
            RASTER,
            "Raster work",
            s.present_done_ns,
            json!({
                "draws": r.draws, "passes": r.passes, "offscreen_passes": r.offscreen_passes,
                "compute_passes": r.compute_passes, "buffers_created": r.buffers_created,
                "bind_groups_created": r.bind_groups_created, "copy_pixels": r.copy_pixels,
                "blur_pixels": r.blur_pixels,
            }),
        );
        self.counter(
            RASTER,
            "Raster uploads (bytes)",
            s.present_done_ns,
            json!({
                "vertex": r.vertex_bytes, "uniform": r.uniform_bytes, "stops": r.stop_bytes,
                "mesh": r.mesh_upload_bytes, "atlas": r.atlas_upload_bytes,
            }),
        );
        self.counter(
            RASTER,
            "Offscreen textures (bytes)",
            s.present_done_ns,
            json!({
                "allocated": r.offscreen_alloc_bytes, "peak": r.offscreen_peak_bytes,
                "pooled": r.pooled_texture_bytes,
            }),
        );
        self.counter(UI, "UI work", s.ui_done_ns, json!({
            "layout_passes": frame.phases.layout_passes, "text_shaped": frame.phases.text_shaped,
            "updates": frame.updates, "sets": frame.sets,
        }));
        if let Some(runtime) = frame.runtime {
            if let Some(heap) = runtime.heap_bytes {
                self.counter(
                    UI,
                    "QuickJS heap (bytes)",
                    s.ui_done_ns,
                    json!({ "bytes": heap }),
                );
            }
            self.counter(UI, "Script runtime", s.ui_done_ns, json!({
                "promises": runtime.promises, "timers": runtime.timers, "requests": runtime.requests,
                "queued_events": runtime.queued_events, "queued_bytes": runtime.queued_bytes,
            }));
        }
    }
}

fn document(capture: &TraceCapture) -> Value {
    let mut trace = Trace {
        pid: std::process::id(),
        start_ns: capture.started_ns,
        events: Vec::new(),
    };
    for (tid, name) in [
        (PLATFORM, "Platform"),
        (UI, "UI"),
        (RASTER, "Raster"),
        (IO, "IO"),
        (GPU, "GPU (device-relative clock)"),
    ] {
        trace.events.push(json!({
            "ph": "M", "name": "thread_name", "pid": trace.pid, "tid": tid,
            "args": { "name": name },
        }));
    }
    for (index, span) in capture.io.iter().enumerate() {
        trace.async_span(
            IO,
            span.name,
            "IO",
            &index.to_string(),
            [trace.stamp(span.start_ns), trace.stamp(span.end_ns)],
            &Value::Null,
        );
    }
    let mut devices: BTreeMap<u64, Vec<&FrameSample>> = BTreeMap::new();
    for frame in &capture.frames {
        trace.frame(frame);
        if frame.gpu_timing.is_some() {
            devices
                .entry(frame.gpu.device_generation)
                .or_default()
                .push(frame);
        }
    }
    // Present-relative CPU timing cannot calibrate the GPU clock. Preserve
    // relative offsets and durations inside each generation on its own axis.
    let mut offset = 0.0_f64;
    for (generation, frames) in devices {
        let origin = frames
            .iter()
            .filter_map(|frame| frame.gpu_timing.as_ref())
            .map(|gpu| gpu.clock_start_us)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        let mut end = offset;
        for frame in frames {
            let Some(gpu) = frame.gpu_timing.as_ref() else {
                continue;
            };
            let start = offset + gpu.clock_start_us - origin;
            end = end.max(start + gpu.frame_span_us);
            // Preserve overlapping pass intervals as async slices with a
            // unique ID per pass. Perfetto otherwise enforces nesting per tid.
            for (index, pass) in gpu.passes.iter().enumerate() {
                let args = json!({
                    "frame_id": frame.number, "device_generation": generation,
                    "slice_index": pass.slice_index, "draws": pass.draws,
                    "duration_us": pass.duration_us, "frame_span_us": gpu.frame_span_us,
                    "passes_sum_us": gpu.passes_sum_us,
                });
                let name = format!("{:?}", pass.kind);
                let id = format!("{}.{}", frame.number, index);
                let ts = start + pass.start_us;
                trace.async_span(GPU, &name, "GPU", &id, [ts, ts + pass.duration_us], &args);
            }
        }
        offset = end + 1.0;
    }
    json!({ "traceEvents": trace.events, "displayTimeUnit": "ms" })
}

pub(super) fn write(capture: &TraceCapture) -> io::Result<PathBuf> {
    if let Some(parent) = capture
        .path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = File::options()
        .write(true)
        .create(true)
        .create_new(!capture.overwrite)
        .truncate(capture.overwrite)
        .open(&capture.path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, &document(capture))?;
    writer.flush()?;
    Ok(capture.path.clone())
}
