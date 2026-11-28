use impasto::ImpastoType;

#[derive(ImpastoType)]
pub(super) struct LogLine {
    pub(super) seq: f64,
    pub(super) at: f64,
    pub(super) level: LogLevel,
    pub(super) source: LogSource,
    pub(super) target: String,
    pub(super) text: String,
    #[impasto(nullable)]
    pub(super) frame: Option<f64>,
}

#[derive(ImpastoType)]
pub(super) struct DiagError {
    pub(super) seq: f64,
    pub(super) at: f64,
    pub(super) source: String,
    pub(super) message: String,
    #[impasto(nullable)]
    pub(super) stack: Option<String>,
    #[impasto(nullable)]
    pub(super) frame: Option<f64>,
}

#[derive(ImpastoType)]
pub(super) enum NetEntryKind {
    Http,
    Stream,
    Socket,
}

#[derive(ImpastoType)]
pub(super) struct NetEntry {
    pub(super) seq: f64,
    pub(super) at: f64,
    pub(super) kind: NetEntryKind,
    pub(super) token: f64,
    pub(super) method: String,
    pub(super) url: String,
    pub(super) request_headers: Vec<(String, String)>,
    #[impasto(nullable)]
    pub(super) request_body: Option<String>,
    #[impasto(nullable)]
    pub(super) status: Option<f64>,
    pub(super) response_headers: Vec<(String, String)>,
    #[impasto(nullable)]
    pub(super) response_bytes: Option<f64>,
    #[impasto(nullable)]
    pub(super) response_preview: Option<String>,
    #[impasto(nullable)]
    pub(super) error: Option<String>,
    #[impasto(nullable)]
    pub(super) done_at: Option<f64>,
}

#[derive(ImpastoType)]
pub(super) struct PhaseSample {
    pub(super) motion_ms: f64,
    pub(super) dispatch_ms: f64,
    pub(super) timer_ms: f64,
    pub(super) raf_ms: f64,
    pub(super) callback_ms: f64,
    pub(super) flush_ms: f64,
    pub(super) layout_cb_ms: f64,
    pub(super) sync_ms: f64,
    pub(super) semantics_ms: f64,
    pub(super) laywork_ms: f64,
    pub(super) passes: f64,
    pub(super) reuse: f64,
    pub(super) shaped: f64,
    pub(super) pointers: f64,
    pub(super) keys: f64,
    pub(super) edits: f64,
    pub(super) ime: f64,
    pub(super) scrolls: f64,
    pub(super) focus: f64,
    pub(super) blur: f64,
    pub(super) canvas_record_ms: f64,
    pub(super) canvas_prepare_ms: f64,
    pub(super) canvas_ops: f64,
    pub(super) canvas_segments: f64,
    pub(super) canvas_redraws: f64,
    pub(super) canvas_reused: f64,
}

#[derive(ImpastoType)]
pub(super) struct GpuPassTiming {
    pub(super) kind: String,
    #[impasto(nullable)]
    pub(super) slice_index: Option<f64>,
    pub(super) draws: f64,
    pub(super) start_us: f64,
    pub(super) duration_us: f64,
}

#[derive(ImpastoType)]
pub(super) struct GpuFrameTiming {
    pub(super) frame_span_us: f64,
    pub(super) passes_sum_us: f64,
    pub(super) passes: Vec<GpuPassTiming>,
}

#[derive(Default, ImpastoType)]
pub(super) struct FramePage {
    pub(super) revision: f64,
    pub(super) entries: Vec<FrameSample>,
    pub(super) sampled_at: f64,
    pub(super) first_frame_at: Option<f64>,
    pub(super) refresh_rate: Option<f64>,
}

#[derive(ImpastoType)]
pub(super) struct FrameSample {
    pub(super) seq: f64,
    pub(super) number: f64,
    pub(super) at: f64,
    pub(super) work_ms: f64,
    pub(super) ui_ms: f64,
    pub(super) raster_ms: f64,
    pub(super) queue_ms: f64,
    pub(super) compositor_only: Option<bool>,
    pub(super) topology: Option<String>,
    pub(super) skip_reason: Option<String>,
    pub(super) presented: bool,
    pub(super) missed_vsync: bool,
    pub(super) trigger: String,
    pub(super) target_lead_ms: Option<f64>,
    pub(super) deadline_overrun_ms: Option<f64>,
    pub(super) gpu_span_ms: Option<f64>,
    pub(super) ui_over_budget: bool,
    pub(super) raster_over_budget: bool,
    pub(super) gpu_over_budget: bool,
    pub(super) compositor_block: Option<String>,
    pub(super) compositor_block_node: Option<String>,
    pub(super) e2e_ms: f64,
    pub(super) draws: f64,
    pub(super) passes: f64,
    pub(super) gpu_status: GpuTimingStatus,
    pub(super) budget_ms: f64,
    pub(super) session_ms: f64,
    pub(super) gesture_ms: f64,
    pub(super) script_ms: f64,
    pub(super) layout_ms: f64,
    pub(super) record_ms: f64,
    pub(super) submit_ms: f64,
    pub(super) present_ms: f64,
    pub(super) layers: f64,
    pub(super) atlas_entries: f64,
    pub(super) pictures: f64,
    pub(super) textures: f64,
    pub(super) shapes_built: Option<f64>,
    pub(super) atlas_hits: Option<f64>,
    pub(super) paint_hit: Option<f64>,
    pub(super) paint_miss: Option<f64>,
    pub(super) picture_rebuilt: Option<f64>,
    pub(super) picture_reused: Option<f64>,
    pub(super) layout_cause: Option<String>,
    pub(super) gestures: Option<f64>,
    pub(super) over_budget: bool,
    pub(super) phases: Option<PhaseSample>,
}

#[derive(ImpastoType)]
pub(super) struct Flags {
    pub(super) layout_scan: bool,
    pub(super) pointer_trace: bool,
    pub(super) suppress_dispatch: bool,
}

#[derive(ImpastoType)]
pub(super) struct Insets {
    pub(super) left: f64,
    pub(super) top: f64,
    pub(super) right: f64,
    pub(super) bottom: f64,
}

#[derive(ImpastoType)]
pub(super) struct Platform {
    pub(super) os: String,
    pub(super) os_version: String,
    pub(super) scale: f64,
    pub(super) viewport: Viewport,
    pub(super) insets: Insets,
    pub(super) padding: Insets,
    pub(super) view_insets: Insets,
    pub(super) app_state: String,
    pub(super) log: String,
    pub(super) devtools: bool,
    #[impasto(nullable)]
    pub(super) dump: Option<String>,
}

#[derive(ImpastoType)]
pub(super) struct Versions {
    pub(super) shell: String,
    pub(super) wgpu: String,
    pub(super) parley: String,
    pub(super) quickjs_pin: String,
}

#[derive(ImpastoType)]
pub(super) struct Viewport {
    pub(super) w: f64,
    pub(super) h: f64,
    pub(super) dpr: f64,
}

#[derive(ImpastoType)]
pub(super) enum LogLevel {
    #[impasto(rename = "ERROR")]
    Error,
    #[impasto(rename = "WARN")]
    Warn,
    #[impasto(rename = "INFO")]
    Info,
    #[impasto(rename = "DEBUG")]
    Debug,
    #[impasto(rename = "TRACE")]
    Trace,
}

#[derive(ImpastoType)]
pub(super) enum LogSource {
    Script,
    Layout,
    Host,
    Net,
}

#[derive(ImpastoType)]
pub(super) enum GpuTimingStatus {
    Disabled,
    Unsupported,
    DroppedSlotFull,
    DroppedPassLimit,
    Pending,
    StaleDevice,
    ReadbackFailed,
    Ready,
}

#[derive(ImpastoType)]
pub(super) struct FlagsPatch {
    pub(super) layout_scan: Option<bool>,
    pub(super) pointer_trace: Option<bool>,
    pub(super) suppress_dispatch: Option<bool>,
}
