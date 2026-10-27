//! Per-frame durations. Recording them does not change the frame.

#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    pub number: u64,
    pub vsync_ms: f64,
    pub target_ms: f64,
    pub session_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layer_count: u32,
}

impl FrameTiming {
    pub fn over_budget(self, interval_ms: f64) -> bool {
        interval_ms > 0.0 && self.session_ms + self.submit_ms + self.present_ms > interval_ms
    }
}
