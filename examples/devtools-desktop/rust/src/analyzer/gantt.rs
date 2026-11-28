use std::collections::HashMap;

use devtools_protocol::FrameRecord;
use serde_json::{Value, json};

use crate::types::{id, number, string};

pub(crate) const LANES: [&str; 5] = ["Platform", "UI", "Raster", "IO", "GPU · relative clock"];

pub(crate) struct Bar {
    pub id: String,
    pub label: String,
    pub lane: usize,
    pub depth: usize,
    pub start: f64,
    pub duration: f64,
    pub color: u16,
    pub fields: Value,
}

pub(crate) fn bars(frame: &FrameRecord, capture: Option<&Value>) -> Vec<Bar> {
    let mut bars = Vec::new();
    if let Some(spans) = capture
        .and_then(|value| value["spans"].as_array())
        .filter(|spans| !spans.is_empty())
    {
        let mut depth = HashMap::new();
        for span in spans {
            let lane = match string(span, "queue").to_ascii_lowercase().as_str() {
                "platform" => 0,
                "ui" => 1,
                "raster" => 2,
                "gpu" => 4,
                _ => 3,
            };
            let key = id(&span["id"]);
            let level = depth
                .get(&(lane, id(&span["parent_id"])))
                .map_or(0, |parent| parent + 1);
            depth.insert((lane, key.clone()), level);
            bars.push(Bar {
                id: key,
                label: string(span, "name").to_owned(),
                lane,
                depth: usize::min(level, 8),
                start: number(span, "offset_ns") / 1_000_000.,
                duration: (number(span, "duration_ns") / 1_000_000.).max(0.001),
                color: [3, 0, 4, 2, 5][lane],
                fields: span.clone(),
            });
        }
    } else {
        let mut stage = |lane, start: f64, name: &str, duration, color| {
            if duration > 0.0 {
                bars.push(Bar {
                    id: name.to_owned(),
                    label: name.to_owned(),
                    lane,
                    depth: 0,
                    start,
                    duration,
                    color,
                    fields: json!({"phase": name, "duration_ms": duration}),
                });
            }
            start + f64::max(duration, 0.0)
        };
        let start = 0.;
        let mut ui = start;
        for (name, duration, color) in [
            ("Gesture", frame.gesture_ms, 0),
            ("Motion", frame.motion_ms, 0),
            ("Dispatch", frame.dispatch_ms, 0),
            ("Timers", frame.timer_ms, 0),
            ("Animation callbacks", frame.raf_ms, 0),
            ("Callbacks", frame.callback_ms, 0),
            ("Reactive flush", frame.flush_ms, 0),
            ("Layout callbacks", frame.layout_cb_ms, 1),
            ("Layout", frame.laywork_ms, 1),
            ("Canvas record", frame.canvas_record_ms, 2),
            ("Record", frame.record_ms, 2),
            ("Semantics", frame.semantics_ms, 2),
        ] {
            ui = stage(1, ui, name, duration, color);
        }
        if ui <= start {
            ui = stage(1, start, "UI work", frame.ui_ms, 0);
        }
        let ui_end = frame.ui_ms.max(ui);
        let mut raster = stage(0, ui_end, "Raster queue", frame.queue_ms, 3);
        let raster_start = raster;
        let picture_prepare = frame.picture_prepare_ms.max(0.);
        let glyph_prepare = frame.glyph_prepare_ms.clamp(0., picture_prepare);
        raster = stage(
            2,
            raster,
            "Compile pictures",
            picture_prepare - glyph_prepare,
            4,
        );
        raster = stage(2, raster, "Prepare glyphs", glyph_prepare, 4);
        let mut remaining = frame.present_ms.max(0.);
        for (name, duration) in [
            ("Encode", frame.encode_ms),
            ("Acquire surface", frame.acquire_ms),
            ("Raster flush", frame.raster_flush_ms),
            ("Queue submit", frame.queue_submit_ms),
            ("Surface present", frame.surface_present_ms),
        ] {
            // Encode combines the work done on both sides of late acquire.
            let duration = duration.clamp(0., remaining);
            raster = stage(2, raster, name, duration, 4);
            remaining -= duration;
        }
        if let Some(error) = frame.present_error.as_deref() {
            bars.push(Bar {
                id: "Present failed".to_owned(),
                label: "Present failed".to_owned(),
                lane: 2,
                depth: 0,
                start: raster,
                duration: remaining,
                color: 7,
                fields: json!({"phase": "Present failed", "duration_ms": remaining, "reason": error}),
            });
        } else {
            let label = if remaining == frame.present_ms {
                "Present"
            } else {
                "Present overhead"
            };
            raster = stage(2, raster, label, remaining, 4);
            if raster <= raster_start && frame.present_ms == 0.0 {
                stage(2, raster_start, "Raster work", frame.raster_ms, 4);
            }
        }
    }
    if !bars.iter().any(|bar| bar.lane == 4) {
        for (index, pass) in frame.gpu.passes.iter().flatten().enumerate() {
            bars.push(Bar {
                id: format!("gpu-{index}"),
                label: pass.kind.clone(),
                lane: 4,
                depth: 0,
                start: pass.start_us / 1000.,
                duration: pass.duration_us / 1000.,
                color: 5,
                fields: serde_json::to_value(pass).unwrap_or_default(),
            });
        }
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_breakdown_does_not_add_parent_spans_to_their_stages() {
        let mut frame = FrameRecord {
            picture_prepare_ms: 2.,
            glyph_prepare_ms: 0.5,
            compose_ms: 3.5,
            encode_ms: 2.,
            acquire_ms: 0.5,
            raster_flush_ms: 0.5,
            queue_submit_ms: 0.5,
            surface_present_ms: 0.25,
            present_ms: 5.,
            ..FrameRecord::default()
        };
        for error in [None, Some("acquire missed".to_owned())] {
            frame.present_error = error;
            let stages = bars(&frame, None);
            let raster: Vec<_> = stages.iter().filter(|stage| stage.lane == 2).collect();
            assert_eq!(raster.iter().map(|stage| stage.duration).sum::<f64>(), 7.);
            assert_eq!(raster[0].duration, 1.5);
            assert_eq!(raster[1].duration, 0.5);
            assert!(
                raster
                    .windows(2)
                    .all(|pair| pair[0].start + pair[0].duration == pair[1].start)
            );
            assert_eq!(
                raster.iter().any(|stage| stage.label == "Present failed"),
                frame.present_error.is_some(),
            );
        }
    }
}
