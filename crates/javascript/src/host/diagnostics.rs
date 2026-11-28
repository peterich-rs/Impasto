//! Owned diagnostics reported by a scripting session.

/// The VM allocator's full memory breakdown in bytes and object counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JsMemoryDetail {
    pub malloc_size: u64,
    pub malloc_limit: u64,
    pub memory_used_size: u64,
    pub malloc_count: u64,
    pub memory_used_count: u64,
    pub atom_count: u64,
    pub atom_size: u64,
    pub str_count: u64,
    pub str_size: u64,
    pub obj_count: u64,
    pub obj_size: u64,
    pub prop_count: u64,
    pub prop_size: u64,
    pub shape_count: u64,
    pub shape_size: u64,
    pub js_func_count: u64,
    pub js_func_size: u64,
    pub js_func_code_size: u64,
    pub js_func_pc2line_count: u64,
    pub js_func_pc2line_size: u64,
    pub c_func_count: u64,
    pub array_count: u64,
    pub fast_array_count: u64,
    pub fast_array_elements: u64,
    pub binary_object_count: u64,
    pub binary_object_size: u64,
}

/// Before and after measurements from an explicit collection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JsGcResult {
    pub before: JsMemoryDetail,
    pub after: JsMemoryDetail,
    pub freed_bytes: u64,
    pub duration_ms: f64,
}

/// Scheduler source of an active callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    Timeout,
    Interval,
    AnimationFrame,
}

/// Timer identity and deadline relative to the current runtime clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerSnapshot {
    pub id: u64,
    pub kind: TimerKind,
    pub due_in_ms: i64,
    pub interval_ms: i64,
}

/// Counts, active timers and unsettled host promises retained by the VM.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsResourcesSnapshot {
    pub counts: RuntimeCounts,
    pub active_timers: Vec<TimerSnapshot>,
    pub timers_truncated: usize,
    pub pending_promises: Vec<u64>,
    pub promises_truncated: usize,
    pub pending_modules: Vec<String>,
    pub job_limit_hits: u64,
    pub budget_overruns: u64,
}

/// The generated JavaScript stack observed at a VM checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackSample {
    pub at: std::time::Instant,
    pub entry: &'static str,
    pub stack: String,
}

/// Limits cooperative samples. The default also serves existing detailed traces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SamplerConfig {
    pub enabled: bool,
    pub interval_us: u64,
    pub max_depth: usize,
    pub max_samples: usize,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_us: 1000,
            max_depth: 8,
            max_samples: 32,
        }
    }
}

/// Live and peak async resource counts collected by the runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeCounts {
    pub promises: usize,
    pub timers: usize,
    pub tickets: usize,
    pub rejections: usize,
    pub listeners: usize,
    pub requests: usize,
    pub queued_events: usize,
    pub queued_bytes: usize,
    pub peak_promises: usize,
    pub peak_timers: usize,
    pub peak_listeners: usize,
    pub peak_tickets: usize,
    pub peak_rejections: usize,
    pub peak_requests: usize,
    pub peak_queued_events: usize,
    pub peak_queued_bytes: usize,
    pub discarded_events: usize,
    pub rejection_overflows: usize,
}
