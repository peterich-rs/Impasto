//! Frame counters and timing records independent of renderer or VM types.

use serde::{Deserialize, Serialize};

/// A retained frame, including subsequent revisions from GPU backfill.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FrameRecord {
    pub id: u64,
    pub revision: u64,
    pub at_ms: f64,
    pub ui_ms: f64,
    pub queue_ms: f64,
    pub raster_ms: f64,
    pub e2e_ms: f64,
    pub budget_ms: f64,
    pub work_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    /// Whether the agent's surface presentation call succeeded.
    pub presented: bool,
    pub present_error: Option<String>,
    pub surface_present_ms: f64,
    pub over_budget: bool,
    pub compositor_only: bool,
    pub missed_vsync: bool,
    pub trigger: String,
    pub target_lead_ms: Option<f64>,
    pub deadline_overrun_ms: Option<f64>,
    pub ui_over_budget: bool,
    pub raster_over_budget: bool,
    pub gpu_over_budget: bool,
    pub input_events: u32,
    pub topology: Option<String>,
    pub skip_reason: Option<String>,
    pub compositor_block: Option<CompositorBlock>,
    pub in_flight_depth: Option<u32>,
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
    pub canvas_record_ms: f64,
    pub canvas_prepare_ms: f64,
    pub layout_passes: u32,
    pub nodes_created: u32,
    pub ffi_batches: u32,
    pub ffi_calls: u32,
    pub style_resolves: u32,
    pub repaint_boundaries_dirtied: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    pub canvas_ops: u32,
    pub canvas_segments: u32,
    pub canvas_redraws: u32,
    pub canvas_reused: u32,
    pub layout_cause: String,
    pub updates: u32,
    pub sets: u32,
    pub draws: u32,
    pub passes: u32,
    pub layers: u32,
    pub pictures: u32,
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub textures: u32,
    pub atlas_entries: u32,
    pub atlas_hits: u32,
    pub repainted_view_count: Option<u32>,
    pub vertex_bytes: u64,
    pub uniform_bytes: u64,
    pub stop_bytes: u64,
    pub extra_bytes: u64,
    pub mesh_upload_bytes: u64,
    pub atlas_upload_bytes: u64,
    pub gpu: GpuTiming,
    pub runtime_sample: Option<RuntimeSample>,
    pub picture_prepare_ms: f64,
    pub glyph_prepare_ms: f64,
    pub compose_ms: f64,
    pub encode_ms: f64,
    pub raster_flush_ms: f64,
    pub queue_submit_ms: f64,
    pub acquire_ms: f64,
    pub atlas_misses: u32,
    pub atlas_evictions: u32,
    pub atlas_bytes: u64,
    pub image_resident_bytes: u64,
    pub offscreen_peak_bytes: u64,
    pub offscreen_passes: u32,
    pub direct_present: bool,
}

/// Cause and optional node that blocked a compositor-only frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompositorBlock {
    pub reason: String,
    pub node: Option<String>,
}

/// GPU timing availability and completed pass samples.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GpuTiming {
    pub state: String,
    pub generation: u64,
    pub frame_span_us: Option<f64>,
    pub passes_sum_us: Option<f64>,
    pub passes: Option<Vec<GpuPass>>,
}

/// One GPU pass measured relative to the first pass in the frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GpuPass {
    pub kind: String,
    pub slice: Option<usize>,
    pub draws: u32,
    pub start_us: f64,
    pub duration_us: f64,
    pub shader_id: Option<u64>,
    pub view_id: Option<String>,
}

/// Low-frequency resource counts measured on the JavaScript queue.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuntimeSample {
    pub promises: usize,
    pub timers: usize,
    pub requests: usize,
    pub queued_events: usize,
    pub queued_bytes: usize,
    pub heap_bytes: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_record_round_trips_presentation_and_stage_metrics() -> Result<(), serde_json::Error> {
        let frame = FrameRecord {
            id: 7,
            presented: true,
            trigger: "vsync".to_owned(),
            target_lead_ms: Some(16.5),
            deadline_overrun_ms: Some(2.5),
            missed_vsync: true,
            ui_over_budget: true,
            raster_over_budget: false,
            gpu_over_budget: true,
            input_events: 4,
            surface_present_ms: 0.25,
            raster_flush_ms: 0.5,
            ..FrameRecord::default()
        };
        let wire = serde_json::to_value(frame)?;
        assert_eq!(wire["trigger"], "vsync");
        assert_eq!(wire["presented"], true);
        let decoded: FrameRecord = serde_json::from_value(wire.clone())?;
        assert_eq!(serde_json::to_value(decoded)?, wire);
        Ok(())
    }
}
