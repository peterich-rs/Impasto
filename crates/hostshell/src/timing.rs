//! Per-frame durations. Recording them does not change the frame.

#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    pub number: u64,
    pub vsync_ms: f64,
    pub target_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub layout_cb_ms: f64,
    pub sync_ms: f64,
    pub record_ms: f64,
    pub semantics_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub shapes_built: u32,
    pub shapes_measure_hit: u32,
    pub shapes_measure_miss: u32,
    pub shapes_paint_hit: u32,
    pub shapes_paint_miss: u32,
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layer_count: u32,
    /// Live engine cache entries after the submit: paragraphs, pictures,
    /// textures.
    pub paragraphs: u32,
    pub pictures: u32,
    pub textures: u32,
}

impl FrameTiming {
    pub fn over_budget(self, interval_ms: f64) -> bool {
        interval_ms > 0.0 && self.session_ms + self.submit_ms + self.present_ms > interval_ms
    }
}
