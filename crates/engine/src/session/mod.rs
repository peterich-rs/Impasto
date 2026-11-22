//! 帧调度、输入交付和宿主服务的契约。

use std::sync::Arc;

pub use javascript::host::{Credit, CreditPool, Delivery, RequestKey, Route, RuntimeKey};

mod input;
mod io;
mod metrics;
mod modules;

pub use input::*;
pub use io::document_relative;
pub use metrics::*;
#[allow(
    unsafe_code,
    reason = "Android trace sections call Android NDK tracing APIs"
)]
pub mod trace;

pub use javascript::host::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    IoEvent, IoJob, PluginResult, PluginValue, module_not_linked,
};
pub use modules::{
    CancelToken, EventType, ImmediateExec, Invoke, Method, MethodExec, MethodTypes, Module,
    ModuleConfig, ModuleExports, ModuleRegistry, NativeExtension, NativeResource, PromiseFn,
    ScriptExecution, Settle, Shared, StreamSendError, StreamSink, SyncFn, Work, shared,
    shared_read, shared_write,
};

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
    pub again: bool,
    pub ime: Option<ImeUpdate>,
    pub semantics: Option<crate::semantics::SemanticsPatch>,
    pub platform_views: Option<Arc<[crate::tree::PlatformViewPlacement]>>,
    pub fonts: Vec<(String, Arc<[u8]>)>,
    pub reloads: Vec<String>,
    pub clipboard: Option<String>,
    pub exit: bool,
    /// `None` means the image census generation did not change.
    pub images: Option<Arc<crate::semantics::ImageCensus>>,
    /// `None` when the session did not time the frame.
    pub phases: Option<FramePhases>,
    /// Current overflow report as JSON, when the debug scan is on.
    pub layout_report: Option<String>,
}

impl FrameOutput {
    pub fn unchanged() -> Self {
        Self {
            layers: None,
            again: false,
            ime: None,
            semantics: None,
            platform_views: None,
            fonts: Vec::new(),
            reloads: Vec::new(),
            clipboard: None,
            exit: false,
            images: None,
            phases: None,
            layout_report: None,
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
    /// Enable opt-in per-frame script counters on this render-thread session.
    fn set_trace(&mut self, _enabled: bool) {}
    fn metrics(&mut self, metrics: &Metrics);
    fn window_display(&mut self, _display: WindowDisplay) {}
    fn pointer(&mut self, event: crate::input::PointerEvent);
    fn frame(&mut self, time: FrameTime) -> FrameOutput;
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
    /// Builds a candidate script runtime and swaps it in after a valid first frame.
    fn update_source(
        &mut self,
        _source: &str,
        _shaper: Box<dyn crate::text::Shaper>,
        _io: Box<dyn Fn(IoJob) -> Result<(), Box<IoJob>> + Send>,
        _time: FrameTime,
    ) -> Result<(), String> {
        Err("this session does not support debug source updates".to_owned())
    }
    /// Whether the host currently needs semantics. Re-enabling must publish a fresh snapshot.
    fn set_semantics_enabled(&mut self, _enabled: bool) {}
    fn set_autofill_enabled(&mut self, _enabled: bool) {}
    /// Font faces queued by the session since the last take.
    ///
    /// The host registers them on the render thread before `submit`. The default
    /// is empty, so a session that does not load faces stays out of the cap.
    fn take_fonts(&mut self) -> Vec<(String, Arc<[u8]>)> {
        Vec::new()
    }
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
    /// The URL that opened this process, if the host has one.
    fn set_launch_url(&mut self, _url: &str) {}
}
