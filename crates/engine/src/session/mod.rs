//! 帧调度、输入交付和宿主服务的契约。

pub use javascript::host::{
    JsGcResult, JsMemoryDetail, JsResourcesSnapshot, RuntimeCounts, SamplerConfig, StackSample,
    TimerKind, TimerSnapshot,
};
use std::sync::Arc;

pub use javascript::host::{Credit, CreditPool, Delivery, RequestKey, Route, RuntimeKey};

mod input;
mod io;
mod metrics;
mod modules;

pub use input::*;
pub use io::{document_relative, read_asset_bytes};
pub use metrics::*;

pub use javascript::host::{IoEvent, PluginResult, PluginValue, module_not_linked};
pub use modules::{
    AssetReader, CancelToken, EventType, HostFuture, ImmediateExec, Invoke, Method, MethodExec,
    MethodTypes, Module, ModuleConfig, ModuleExports, ModuleRegistry, NativeExtension,
    NativeResource, PromiseFn, ScriptExecution, Settle, Shared, StreamSendError, StreamSink,
    SyncFn, Work, shared, shared_read, shared_write,
};

/// Work required after running jobs and timers while Raster owns vsync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptInvalidation {
    Unchanged,
    Content,
    Ui,
}

/// A low-frequency snapshot of script runtime resources.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeSample {
    pub promises: usize,
    pub timers: usize,
    pub requests: usize,
    pub queued_events: usize,
    pub queued_bytes: usize,
    pub heap_bytes: Option<usize>,
}

/// Durations inside one [`Session::frame`]. The host adds gesture time itself.
///
/// `script` splits the script interval. `trace` counts layout work and the
/// input events already handled before `frame` returned.
#[derive(Clone, Copy, Debug, Default)]
pub struct FramePhases {
    pub script_ms: f64,
    pub layout_ms: f64,
    /// Script that ran after the first `layout` and before `record`.
    pub layout_cb_ms: f64,
    pub sync_ms: f64,
    pub record_ms: f64,
    pub semantics_ms: f64,
    pub script: ScriptPhases,
    pub trace: FrameTrace,
}

/// Pieces of [`FramePhases::script_ms`], in call order.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptPhases {
    pub motion_ms: f64,
    pub dispatch_ms: f64,
    pub timer_ms: f64,
    pub raf_ms: f64,
    pub callback_ms: f64,
    pub flush_ms: f64,
}

/// Layout and input counts for one frame. Zeros mean that stage did not run.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTrace {
    /// Nodes allocated by the script, including allocations in compiled templates.
    pub nodes_created: u32,
    /// Batched JavaScript scene mounts and range updates.
    pub ffi_batches: u32,
    /// JavaScript calls into the native scene bindings.
    pub ffi_calls: u32,
    /// Source cascades resolved against the node environment.
    pub style_resolves: u32,
    /// Explicit repaint boundaries newly marked dirty.
    pub repaint_boundaries_dirtied: u32,
    /// All `layout` calls this frame, including those made from script.
    pub laywork_ms: f64,
    pub layout_passes: u32,
    pub root_layouts: u32,
    pub local_layouts: u32,
    pub same_text_layouts: u32,
    pub list_ops: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    /// `none` when nothing marked layout.
    pub layout_cause: &'static str,
    pub pointers: u32,
    pub keys: u32,
    pub edits: u32,
    pub ime: u32,
    pub scrolls: u32,
    pub focus: u32,
    pub blur: u32,
    pub gestures: u32,
    /// Nonempty reactive update passes this frame.
    pub updates: u32,
    /// Scene setter calls (style, text, image and recognizers) this frame.
    pub sets: u32,
    /// Time spent recording canvas commands this frame.
    pub canvas_record_ms: f64,
    /// Time spent preparing canvas lists this frame.
    pub canvas_prepare_ms: f64,
    pub canvas_ops: u32,
    pub canvas_segments: u32,
    pub canvas_redraws: u32,
    pub canvas_callbacks: u32,
    pub canvas_preparations: u32,
    pub canvas_reused: u32,
}

/// What one `frame` produced. `layers == None` means the host may `PresentCached`.
#[derive(Clone, Debug)]
pub struct FrameOutput {
    /// `Some` when the layer epoch changed. The host submits that tree.
    /// `None` means the host may present the installed tree again.
    pub layers: Option<Arc<crate::scene::LayerTree>>,
    /// Optional incremental description of the new layer snapshot.
    pub scene_delta: Option<crate::scene::SceneDelta>,
    pub again: bool,
    pub ime: Option<ImeUpdate>,
    pub semantics: Option<crate::semantics::SemanticsPatch>,
    pub platform_views: Option<Arc<[crate::tree::PlatformViewPlacement]>>,
    pub reloads: Vec<String>,
    pub clipboard: Option<String>,
    pub exit: bool,
    /// `None` means the image census generation did not change.
    pub images: Option<Arc<crate::semantics::ImageCensus>>,
    /// `None` when the session did not time the frame.
    pub phases: Option<FramePhases>,
    /// Current overflow report as JSON, when the debug scan is on.
    pub layout_report: Option<String>,
    /// Optional per-view recording work for the current UI frame.
    pub repaints: Option<crate::tree::FrameRepaintBatch>,
}

impl FrameOutput {
    pub fn unchanged() -> Self {
        Self {
            layers: None,
            scene_delta: None,
            again: false,
            ime: None,
            semantics: None,
            platform_views: None,
            reloads: Vec::new(),
            clipboard: None,
            exit: false,
            images: None,
            phases: None,
            layout_report: None,
            repaints: None,
        }
    }
}

/// One script failure, with the stack kept apart from the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFault {
    /// `listener`, `rejection`, `frame`, `layout`, `timer`, `jobs`, `driver`,
    /// `scroll`, `input`, `motion`, or `canvas`.
    pub source: &'static str,
    pub message: String,
    pub stack: String,
}

/// UI session driven by the embedder on its render thread.
///
/// `frame` runs in this order: input has already been delivered through
/// [`Session::pointer`] and [`Session::input`], due timers, animation callbacks,
/// reactive updates, layout, and layer recording.
pub trait Session {
    /// Quiesces a session created with the host's migration contract.
    fn prepare_platform_handoff(&mut self) -> bool {
        false
    }

    /// Resumes an idle session after installing it on the platform queue.
    fn finish_platform_handoff(&mut self) {}

    /// Enable opt-in per-frame script counters on this render-thread session.
    fn set_trace(&mut self, _enabled: bool) {}
    fn set_counters(&mut self, _enabled: bool) {}
    fn runtime_sample(&self) -> Option<RuntimeSample> {
        None
    }
    /// Resource counts without an allocator walk.
    fn runtime_counters(&self) -> Option<RuntimeSample> {
        self.runtime_sample()
    }

    /// Detailed memory breakdown available between script turns.
    fn js_memory_detail(&self) -> Option<JsMemoryDetail> {
        None
    }

    /// Snapshot of active asynchronous resources.
    fn js_resources_snapshot(&self) -> Option<JsResourcesSnapshot> {
        None
    }

    /// Performs an owner-thread garbage collection.
    fn run_js_gc(&mut self) -> Option<JsGcResult> {
        None
    }

    /// Configures the cooperative stack sampler.
    fn set_js_sampler_config(&mut self, _config: SamplerConfig) -> bool {
        false
    }

    /// Drains live profiler samples collected in the current UI turn.
    fn take_js_stack_samples(&mut self) -> Vec<StackSample> {
        Vec::new()
    }

    /// Registers an effect source for sessions with a paint tree.
    ///
    /// # Errors
    /// Returns a shader registration error or an unsupported-session error.
    fn register_shader_program(
        &mut self,
        _source: &str,
        _clock: crate::scene::ShaderClock,
    ) -> Result<crate::scene::ShaderProgramId, crate::scene::ShaderError> {
        Err(crate::scene::ShaderError::Unsupported)
    }
    fn metrics(&mut self, metrics: &Metrics);
    fn window_display(&mut self, _display: WindowDisplay) {}
    fn pointer(&mut self, event: crate::input::PointerEvent);

    /// Dispatches the latest move, with optional earlier positions for velocity estimation.
    fn pointer_moves(
        &mut self,
        event: crate::input::PointerEvent,
        _history: &[crate::input::PointerSample],
    ) {
        self.pointer(event);
    }
    fn frame(&mut self, time: FrameTime) -> FrameOutput;
    /// Pumps a due script deadline without advancing compositor-owned motion.
    fn advance_script_deadline(&mut self, _mono_ms: i64) -> ScriptInvalidation {
        ScriptInvalidation::Ui
    }
    /// Prepares a replacement list window while the installed scene keeps presenting.
    fn prefetch_compositor_window(&mut self, time: FrameTime) -> FrameOutput {
        self.frame(time)
    }
    /// Whether the current tree may animate on the compositor while UI is idle.
    fn compositor_eligibility(
        &self,
        _tracks: &[crate::scene::CompositorTrack],
    ) -> crate::scene::CompositorEligibility {
        Err(crate::scene::CompositorBlockReason::PendingUiWork)
    }
    /// Restores the properties of the last compositor frame before input dispatch.
    fn commit_compositor_state(&mut self, _snapshot: &crate::scene::CompositorSnapshot) {}
    /// True after the session asked for another frame, until the host takes it.
    ///
    /// Animation asks again inside each `frame`. One request schedules one frame.
    fn wants_frame(&mut self) -> bool {
        false
    }
    /// The most recent script failure, if the session recorded one and nobody
    /// took it yet. The host consumes this for diagnostics each frame.
    fn take_script_error(&mut self) -> Option<String> {
        None
    }
    /// Script failures since the last take, message and stack kept separate.
    /// Report-mode listener errors land here and leave [`Session::take_script_error`]
    /// empty.
    fn take_faults(&mut self) -> Vec<ScriptFault> {
        Vec::new()
    }
    /// Releases application-owned caches in response to system memory pressure.
    fn memory_pressure(&mut self) {}
    fn lifecycle(&mut self, _event: Lifecycle) {}
    fn settings(&mut self, _settings: &PlatformSettings) {}
    /// The window became or resigned key. This is not text focus.
    fn window_focus(&mut self, _focused: bool) {}
    fn window_state_changed(&mut self, _key: String) {}
    /// Keyboard or IME input for the focused node. No focus means the session
    /// drops the event.
    fn input(&mut self, _event: InputEvent) {}
    /// A platform return gesture owned by the application.
    fn back_gesture(&mut self, event: BackGesture) -> bool {
        event != BackGesture::Committed
    }
    /// Ownership advertised to the platform before it begins a back gesture.
    fn take_back_enabled(&mut self) -> Option<bool> {
        None
    }
    /// Takes a changed pointer cursor after processing input and style updates.
    fn take_cursor(&mut self) -> Option<crate::style::CursorStyle> {
        None
    }

    /// Keyboard visibility or a new editing state. `None` means nothing changed.
    fn take_ime(&mut self) -> Option<ImeUpdate> {
        None
    }
    /// The user hid the keyboard. The session blurs.
    fn ime_dismissed(&mut self) {}
    /// The session asked the process to leave. A back key with no handler sets this.
    fn take_exit(&mut self) -> bool {
        false
    }
    /// Text a Copy or Cut put aside for the platform clipboard. `None` means nothing new.
    fn take_clipboard(&mut self) -> Option<String> {
        None
    }
    /// An accessibility action. The default ignores it.
    fn semantics_action(&mut self, _action: crate::semantics::SemanticsAction) {}
    /// Replaces the development error picture for the current session.
    #[cfg(debug_assertions)]
    fn set_dev_error(&mut self, _overlay: Option<crate::tree::DevErrorOverlay>) -> bool {
        false
    }

    /// Reloads an already-registered packaged font from the active asset provider.
    #[cfg(debug_assertions)]
    fn reload_dev_font(
        &mut self,
        _relative: &str,
        _bytes: Option<Arc<[u8]>>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Attempts to apply compiler-generated changes within the current runtime.
    fn apply_source_patch(&mut self, _source: &str) -> Result<bool, String> {
        Ok(false)
    }
    /// Builds a candidate script runtime and swaps it in after a valid first frame.
    fn update_source(
        &mut self,
        _source: &str,
        _shaper: Box<dyn crate::text::Shaper>,
        _time: FrameTime,
    ) -> Result<(), String> {
        Err("this session does not support debug source updates".to_owned())
    }
    /// Whether the host currently needs semantics. Re-enabling must publish a fresh snapshot.
    fn set_semantics_enabled(&mut self, _enabled: bool) {}
    fn set_autofill_enabled(&mut self, _enabled: bool) {}
    /// Image sources the session asked to fetch again. The host drops those cache
    /// keys on the render thread before it reads the cache. The default is empty.
    fn take_reloads(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Resolves one host IO completion. Called on the render thread.
    fn io_event(&mut self, _event: IoEvent) {}

    fn io_delivery(&mut self, delivery: Delivery) {
        self.io_event(delivery.event);
    }
    /// Absolute monotonic milliseconds of the next timer, if one is armed.
    fn next_wake_ms(&mut self) -> Option<f64> {
        None
    }
    /// A bounded scene snapshot for the host's developer inspector.
    /// Sets repaint demand and the frame identity before the session advances.
    fn begin_repaint_frame(&mut self, _enabled: bool, _frame_number: u64) {}

    /// Updates the diagnostic overlay and picker.
    fn set_remote_overlay(&mut self, _config: crate::tree::RemoteOverlayConfig) {}

    /// Returns retained per-view repaint statistics for this session.
    fn repaint_leaderboard(&self, _limit: usize) -> Vec<crate::tree::NodeRepaintStats> {
        Vec::new()
    }

    /// Takes the most recent remote pointer selection.
    fn take_picked_view(&mut self) -> Option<crate::tree::PickedViewEvent> {
        None
    }

    /// Returns a fresh snapshot bounded by the caller's node budget.
    fn inspector_snapshot_limited(&self, _max_nodes: usize) -> Option<String> {
        self.inspector_snapshot()
    }

    /// Scans the last completed layout for overflow on the owning UI queue.
    fn inspect_layout_issues(&mut self) -> Option<String> {
        None
    }

    /// Returns the hit-test route at a point in scene coordinates.
    fn inspector_probe(&self, _x: f32, _y: f32) -> Option<String> {
        None
    }

    fn inspector_snapshot(&self) -> Option<String> {
        None
    }
    /// The URL that opened this process, if the host has one.
    fn set_launch_url(&mut self, _url: &str) {}
}
