//! Live QuickJS session. Tap handlers are JS values owned by the interpreter.

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use crate::host::HostEvent;
use crate::session::{InputEvent, IoJob, Key, Metrics, ModuleConfig, PluginValue, Session};
use uicore::{Affinity, FocusDirection, NodeId, Rect, Shaper, Tree};

use crate::ui::Error;

mod bootstrap;
mod driver;
mod environment;
mod reactive;
mod scene;

mod bridge;
mod call;
mod frame;
mod keys;
mod mutation;
mod nodes;
mod vm;
use bridge::*;
use frame::*;
use keys::*;
use nodes::*;
use vm::*;

macro_rules! vm_entry {
    ($fallback:expr, $body:block) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(value) => value,
            Err(_) => {
                tracing::error!(target: "impasto::script", "host callback panicked");
                $fallback
            }
        }
    }};
}
pub(super) use vm::{compile_declarations, prepare_compile};
pub(super) use vm_entry;

type SharedConsoleLog = Rc<dyn Fn(u8, &str)>;

struct Inner {
    pub(super) tree: RefCell<Tree>,
    pub(super) host: *mut Host,
    pub(super) log: SharedConsoleLog,
    pub(super) ime: RefCell<ImeSlot>,
    pub(super) text_scale: Cell<f32>,
    pub(super) reduce_motion: Cell<bool>,
    pub(super) platform: RefCell<crate::session::PlatformSettings>,
    pub(super) window_metrics: Cell<Metrics>,
    pub(super) window_display: RefCell<crate::session::WindowDisplay>,
    pub(super) settings_frame: Cell<bool>,
    /// Bytes from the latest `text_edit`. Stable until the next edit.
    pub(super) edit_bytes: RefCell<Vec<u8>>,
    /// Copy or Cut text waiting for the platform clipboard.
    pub(super) clipboard: RefCell<Option<String>>,
    /// Selection rectangles from the latest query, as packed `x,y,w,h` floats.
    pub(super) selection_rects: RefCell<Vec<f32>>,
    pub(super) config: ModuleConfig,
    /// Debug JSON from the latest `debug_json` call, as NUL-terminated UTF-8.
    pub(super) debug_json: RefCell<Option<CString>>,
    pub(super) paused: Cell<bool>,
    pub(super) fonts: RefCell<FontBook>,
    /// Sources passed to `reload` since the last [`Session::take_reloads`].
    pub(super) reloads: RefCell<Vec<String>>,
    pub(super) mutations: RefCell<Vec<mutation::OwnedRec>>,
    pub(super) marks: FrameMarks,
    pub(super) drivers: RefCell<uicore::DriverSet>,
    pub(super) driver_frame: Cell<bool>,
    trace: Cell<bool>,
    /// Script failures waiting for [`crate::session::Session::take_faults`].
    pub(super) faults: RefCell<Vec<crate::session::ScriptFault>>,
    /// The reactive graph owns Persistent values and temporary transfer slots.
    pub(super) graph: reactive::Graph,
    /// Paint bindings drop before the budget so a recorder can refund into it.
    pub(super) canvas: RefCell<CanvasSession>,
    pub(super) canvas_budget: RefCell<uicore::canvas::Budget>,
}

impl Inner {
    fn new(
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
        config: ModuleConfig,
    ) -> Self {
        Self {
            tree: RefCell::new(Tree::new(shaper)),
            host: ptr::null_mut(),
            log: Rc::from(log),
            ime: RefCell::new(ImeSlot::new()),
            text_scale: Cell::new(1.0),
            reduce_motion: Cell::new(false),
            platform: RefCell::new(crate::session::PlatformSettings::default()),
            window_metrics: Cell::new(Metrics::new(0.0, 0.0, 1.0)),
            window_display: RefCell::new(crate::session::WindowDisplay::default()),
            settings_frame: Cell::new(false),
            edit_bytes: RefCell::new(Vec::new()),
            clipboard: RefCell::new(None),
            selection_rects: RefCell::new(Vec::new()),
            config,
            debug_json: RefCell::new(None),
            paused: Cell::new(false),
            fonts: RefCell::new(FontBook::new()),
            reloads: RefCell::new(Vec::new()),
            mutations: RefCell::new(Vec::new()),
            marks: FrameMarks::default(),
            drivers: RefCell::new(uicore::DriverSet::new()),
            driver_frame: Cell::new(false),
            trace: Cell::new(std::env::var_os("IMPASTO_TRACE").is_some()),
            faults: RefCell::new(Vec::new()),
            graph: reactive::Graph::new(),
            canvas: RefCell::new(CanvasSession::new()),
            canvas_budget: RefCell::new(uicore::canvas::Budget::session()),
        }
    }
}

struct ImeSlot {
    shown: bool,
    announced: bool,
    dirty: bool,
    state: crate::session::ImeState,
    /// Last editor revision copied into [`ImeSlot::state`].
    revision: u64,
}

impl ImeSlot {
    fn new() -> Self {
        Self {
            shown: false,
            announced: false,
            dirty: false,
            state: crate::session::ImeState::at_end(""),
            revision: 0,
        }
    }

    /// Recomputes the platform keyboard state when the focused editor moved.
    fn sync_editor(&mut self, tree: &mut Tree) {
        let Some(id) = tree.focused() else {
            return;
        };
        let Some((revision, value)) = tree.editor_snapshot(id) else {
            return;
        };
        let (start, end) = value.selection.ordered();
        let caret = tree
            .caret_for(id, value.selection.extent, Affinity::Downstream)
            .ok();
        let origin = tree.scene_rect(id).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        let caret_rect = caret
            .map(|caret| Rect {
                x: origin.x + caret.x,
                y: origin.y + caret.y,
                w: caret.w,
                h: caret.h,
            })
            .unwrap_or(self.state.caret);
        let composing = value.composing.map(|range| range.start..range.end);
        let changed = revision != self.revision
            || self.state.text != value.text
            || self.state.selection != (start..end)
            || self.state.composing != composing
            || self.state.caret != caret_rect;
        if !changed {
            return;
        }
        self.revision = revision;
        self.state.text = value.text;
        self.state.selection = start..end;
        self.state.composing = composing;
        self.state.caret = caret_rect;
        self.dirty = true;
    }

    fn take(&mut self) -> Option<crate::session::ImeUpdate> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        if !self.shown {
            self.announced = false;
            return Some(crate::session::ImeUpdate::Hide);
        }
        let state = self.state.clone();
        if self.announced {
            Some(crate::session::ImeUpdate::Update(state))
        } else {
            self.announced = true;
            Some(crate::session::ImeUpdate::Show(state))
        }
    }
}

fn app_directory(name: &str) -> Option<&str> {
    let index = name.rfind('/')?;
    let dir = &name[..index];
    if dir.is_empty() { None } else { Some(dir) }
}

pub struct JsSession {
    inner: *mut Inner,
    engine: Engine,
    layer_epoch: u64,
    platform_views: Arc<[uicore::PlatformViewPlacement]>,
    image_generation: u64,
    script_error: RefCell<Option<String>>,
    exit: Cell<bool>,
    /// A failed frame must not schedule another frame by itself.
    suppress_demand: Cell<bool>,
    reload_frame: Option<crate::session::FrameOutput>,
    record_failing: bool,
    record_frames: u32,
    _thread: PhantomData<Rc<()>>,
}

impl JsSession {
    pub fn open(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
    ) -> Result<Self, Error> {
        Self::open_with(
            script,
            name,
            shaper,
            log,
            None,
            ModuleConfig::disconnected(),
        )
    }

    pub fn open_with(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
        http: Option<Arc<dyn crate::session::HttpClient>>,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_source(script.as_bytes(), name, shaper, log, http, config)
    }

    pub fn open_source(
        script: &[u8],
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
        http: Option<Arc<dyn crate::session::HttpClient>>,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_program(ProgramKind::Source(script), name, shaper, log, http, config)
    }

    pub(crate) fn open_trusted(
        program: crate::runtime::TrustedBytecode<'_>,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
        http: Option<Arc<dyn crate::session::HttpClient>>,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_program(
            ProgramKind::Trusted(program),
            name,
            shaper,
            log,
            http,
            config,
        )
    }

    fn open_program(
        program: ProgramKind<'_>,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::ui::ConsoleLog,
        http: Option<Arc<dyn crate::session::HttpClient>>,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        let state = Box::new(Inner::new(shaper, log, config));
        let engine = Engine::new(state, app_directory(name), http)?;
        engine.install(bootstrap::SCENE_JS)?;
        engine.start(program, name)?;
        let inner = engine.inner_ptr();
        // SAFETY: Engine owns Inner for the entire session.
        sweep_detached(unsafe { &*inner });
        Ok(Self {
            inner,
            engine,
            layer_epoch: u64::MAX,
            platform_views: Arc::default(),
            image_generation: u64::MAX,
            script_error: RefCell::new(None),
            exit: Cell::new(false),
            suppress_demand: Cell::new(false),
            reload_frame: None,
            record_failing: false,
            record_frames: 0,
            _thread: PhantomData,
        })
    }

    fn hold_frame(&mut self, message: String) -> crate::session::FrameOutput {
        self.note_script_error(message);
        self.suppress_demand.set(true);
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        sweep_detached(inner);
        inner.marks.reset();
        inner.graph.arena.take_update_count();
        crate::session::FrameOutput::unchanged()
    }

    #[cfg(test)]
    pub(crate) fn picture_writes(&self) -> usize {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.tree.borrow().picture_writes()
    }

    pub fn install_io(&mut self, submit: Box<dyn Fn(IoJob) + Send>) {
        self.engine.install_io(submit);
    }

    pub fn install_routed_io(&mut self, submit: crate::ui::RoutedIoSubmit) {
        self.engine.install_routed_io(submit);
    }

    pub fn semantics(&mut self) -> Option<uicore::SemanticsUpdate> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.tree.borrow().semantics()
    }

    pub fn take_script_error(&self) -> Option<String> {
        self.script_error.borrow_mut().take()
    }

    pub fn take_faults(&mut self) -> Vec<crate::session::ScriptFault> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        std::mem::take(&mut *inner.faults.borrow_mut())
    }

    /// Flush pending graph updates at a frame boundary.
    fn run_reactive(&mut self) -> Result<(), String> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        if !inner.graph.arena.has_updates() {
            return Ok(());
        }
        let mut error = [0u8; 512];
        // SAFETY: the session owns host. The VM arms the interpreter budget before
        // entering Rust's graph, whose callbacks may execute arbitrary JS.
        let failed = unsafe { vm_run_updates(inner.host, error.as_mut_ptr().cast(), error.len()) };
        if failed == 0 {
            Ok(())
        } else {
            Err(pull_error(inner, "reactive", &c_message(&error)))
        }
    }

    fn note_script_error(&self, message: String) {
        tracing::error!(target: "impasto::script", "{message}");
        *self.script_error.borrow_mut() = Some(message);
    }

    fn note_record_ok(&mut self) {
        if !self.record_failing {
            return;
        }
        tracing::info!(
            target: "impasto::render::record",
            frames = self.record_frames,
            "record recovered"
        );
        self.record_failing = false;
        self.record_frames = 0;
    }

    fn push_fault(&self, source: &'static str, message: &str) {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.faults.borrow_mut().push(crate::session::ScriptFault {
            source,
            message: message.to_owned(),
            stack: String::new(),
        });
    }

    fn hold_layout(&mut self, message: String) -> crate::session::FrameOutput {
        tracing::error!(target: "impasto::layout", error = %message, "layout failed");
        self.push_fault("layout", &message);
        self.hold_frame(message)
    }

    fn note_record_err(&mut self, message: &str) {
        self.record_frames = self.record_frames.saturating_add(1);
        if self.record_failing {
            return;
        }
        self.record_failing = true;
        tracing::error!(target: "impasto::render::record", error = message, "record failed");
    }

    pub fn jobs_pending(&self) -> bool {
        // SAFETY: `inner` is owned by this session. The query only reads the runtime flag.
        let inner = unsafe { &*self.inner };
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        unsafe { vm_jobs_pending(inner.host) != 0 }
    }

    pub fn tap_count(&self) -> usize {
        // SAFETY: `inner` is owned by this session. The query only reads the tap table length.
        let inner = unsafe { &*self.inner };
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        unsafe { vm_tap_count(inner.host) as usize }
    }
}

impl Drop for JsSession {
    fn drop(&mut self) {
        self.engine.shutdown();
    }
}

impl Session for JsSession {
    fn set_trace(&mut self, enabled: bool) {
        // SAFETY: Inner belongs to this session on the render thread.
        unsafe { &*self.inner }.trace.set(enabled);
    }

    fn take_script_error(&mut self) -> Option<String> {
        JsSession::take_script_error(self)
    }

    fn take_faults(&mut self) -> Vec<crate::session::ScriptFault> {
        JsSession::take_faults(self)
    }

    fn metrics(&mut self, metrics: &Metrics) {
        // SAFETY: `inner` is owned by this session and is not aliased by a Rust reference.
        let inner = unsafe { &*self.inner };
        let left = metrics.inset_left.max(0.0);
        let top = metrics.inset_top.max(0.0);
        let right = metrics.inset_right.max(0.0);
        let bottom = metrics.inset_bottom.max(0.0);
        let mut tree = inner.tree.borrow_mut();
        let _ = tree.set_viewport(
            left,
            top,
            (metrics.width - left - right).max(0.0),
            (metrics.height - top - bottom).max(0.0),
        );
        let _ = tree.set_safe_area(
            metrics.width,
            metrics.height,
            metrics.padding.left,
            metrics.padding.top,
            metrics.padding.right,
            metrics.padding.bottom,
        );
        if metrics.device_pixel_ratio.is_finite() && metrics.device_pixel_ratio > 0.0 {
            let _ = tree.set_pixel_scale(metrics.device_pixel_ratio);
        }
        drop(tree);
        if inner.window_metrics.replace(*metrics) != *metrics {
            let mut error = [0u8; 512];
            // SAFETY: the session owns Host and the diagnostic buffer outlives the call.
            if unsafe {
                vm_apply_window_metrics(inner.host, error.as_mut_ptr().cast(), error.len())
            } != 0
            {
                self.note_script_error(pull_error(inner, "input", &c_message(&error)));
            }
        }
    }

    fn window_display(&mut self, display: crate::session::WindowDisplay) {
        // SAFETY: the session owns Inner. End the borrow before entering JS.
        let inner = unsafe { &*self.inner };
        {
            let mut current = inner.window_display.borrow_mut();
            if *current == display {
                return;
            }
            *current = display;
        }
        let mut error = [0u8; 512];
        // SAFETY: the VM and diagnostic buffer remain live throughout the turn.
        if unsafe { vm_apply_window_metrics(inner.host, error.as_mut_ptr().cast(), error.len()) }
            != 0
        {
            self.note_script_error(pull_error(inner, "windowMetrics", &c_message(&error)));
        }
    }

    fn pointer(&mut self, event: uicore::PointerEvent) {
        // SAFETY: the tree borrow ends before dispatch. JS handlers may call
        // back into the tree through the `RefCell`.
        let inner = unsafe { &*self.inner };
        let gestures = inner.tree.borrow_mut().pointer(event);
        bump(&inner.marks.pointers);
        if let Err(message) = dispatch_pointer(inner, gestures) {
            self.note_script_error(message);
        }
    }

    fn take_back_enabled(&mut self) -> Option<bool> {
        // SAFETY: the render thread owns this session and its live VM.
        unsafe { vm_take_back_enabled((*self.inner).host) }
    }

    fn back_gesture(&mut self, event: crate::session::BackGesture) -> bool {
        let mut error = [0u8; 512];
        // SAFETY: Inner remains allocated while the session is live.
        let inner = unsafe { &*self.inner };
        // SAFETY: the session owns the VM and error buffer for this synchronous turn.
        let status =
            unsafe { vm_back_gesture(inner.host, event, error.as_mut_ptr().cast(), error.len()) };
        if status < 0 {
            self.note_script_error(pull_error(inner, "backGesture", &c_message(&error)));
        }
        status != 0
    }

    fn input(&mut self, event: InputEvent) {
        // SAFETY: `inner` is owned by this session. The tree borrow ends before
        // the callback, which may re-enter the tree.
        let inner = unsafe { &*self.inner };
        match &event {
            InputEvent::Key(_) | InputEvent::Back => bump(&inner.marks.keys),
            InputEvent::Text(_)
            | InputEvent::Composing(_)
            | InputEvent::Copy
            | InputEvent::Cut
            | InputEvent::Paste { .. }
            | InputEvent::SelectAll
            | InputEvent::Edit(_)
            | InputEvent::Replace { .. } => bump(&inner.marks.edits),
        }
        if let InputEvent::Key(press) = &event {
            let spec = shortcut_spec(press);
            let c_spec = CString::new(spec).ok();
            if let Some(c_spec) = c_spec {
                let mut error = [0u8; 512];
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                let hit = unsafe {
                    vm_try_shortcut(
                        inner.host,
                        c_spec.as_ptr(),
                        error.as_mut_ptr().cast(),
                        error.len(),
                    )
                };
                if hit < 0 {
                    self.note_script_error(pull_error(inner, "listener", &c_message(&error)));
                    return;
                }
                if hit > 0 {
                    return;
                }
            }
        }
        let escape = matches!(event, InputEvent::Key(press) if press.key == Key::Escape);
        if matches!(event, InputEvent::Back) || escape {
            let mut error = [0u8; 512];
            let handled =
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                unsafe { vm_dispatch_back(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if handled < 0 {
                self.note_script_error(pull_error(inner, "input", &c_message(&error)));
            } else if handled == 0 && !escape {
                self.exit.set(true);
            }
            return;
        }
        if let InputEvent::Key(press) = &event
            && press.key == Key::Tab
        {
            let direction = if press.shift {
                FocusDirection::Previous
            } else {
                FocusDirection::Next
            };
            let gestures = inner.tree.borrow_mut().focus_step(direction);
            if let Err(message) = dispatch_gestures(inner, gestures) {
                self.note_script_error(message);
            }
            return;
        }
        let Some(id) = inner.tree.borrow().focused() else {
            return;
        };
        if let InputEvent::Replace {
            text,
            start,
            end,
            compose_start,
            compose_end,
        } = event
        {
            if let Err(message) = dispatch_input(
                inner,
                id,
                16,
                Some(&text),
                Some((start as i32, end as i32, compose_start, compose_end)),
            ) {
                self.note_script_error(message);
            }
            return;
        }
        let (kind, text) = match event {
            InputEvent::Edit(edit) => {
                let value = inner.tree.borrow_mut().text_edit(id, edit.clone());
                if let Ok(value) = value {
                    let _ = dispatch_text(inner, id, &value.text);
                    return;
                }
                // The focused control is the field chrome. The editor lives on
                // its text child, so a buffer edit is delivered as a key or text.
                let (kind, forwarded) = match edit {
                    uicore::Edit::DeleteRange { .. } | uicore::Edit::Backspace => {
                        (8, Some("Backspace".to_string()))
                    }
                    uicore::Edit::ForwardDelete => (8, Some("Delete".to_string())),
                    uicore::Edit::Insert(text) => (9, Some(text)),
                    _ => return,
                };
                if let Err(message) = dispatch_input(inner, id, kind, forwarded.as_deref(), None) {
                    self.note_script_error(message);
                }
                return;
            }
            InputEvent::Key(press) => (8, Some(key_name(press))),
            InputEvent::Text(text) | InputEvent::Paste { plain: text, .. } => (9, Some(text)),
            InputEvent::Composing(text) => (10, text),
            InputEvent::Copy => (8, Some("Copy".to_string())),
            InputEvent::Cut => (8, Some("Cut".to_string())),
            InputEvent::SelectAll => (8, Some("SelectAll".to_string())),
            InputEvent::Back | InputEvent::Replace { .. } => return,
        };
        if let Err(message) = dispatch_input(inner, id, kind, text.as_deref(), None) {
            self.note_script_error(message);
        }
    }

    fn take_ime(&mut self) -> Option<crate::session::ImeUpdate> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.ime.borrow_mut().take()
    }

    fn settings(&mut self, settings: &crate::session::PlatformSettings) {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        let previous = inner.platform.borrow().clone();
        let changed = previous != *settings;
        let scale_changed = (previous.text_scale - settings.text_scale).abs() > f32::EPSILON;
        inner.text_scale.set(settings.text_scale);
        inner.reduce_motion.set(settings.reduce_motion);
        *inner.platform.borrow_mut() = settings.clone();
        if scale_changed {
            let _ = inner.tree.borrow_mut().set_text_scale(settings.text_scale);
        }
        if !changed {
            return;
        }
        inner.settings_frame.set(true);
        if inner.host.is_null() {
            return;
        }
        let mut error = [0u8; 512];
        let dark = i32::from(settings.brightness == crate::session::Brightness::Dark);
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        let failed = unsafe {
            vm_apply_settings(
                inner.host,
                settings.locale.as_ptr().cast(),
                settings.locale.len(),
                f64::from(settings.text_scale),
                dark,
                i32::from(settings.reduce_motion),
                i32::from(settings.bold_text),
                i32::from(settings.high_contrast),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        if failed < 0 {
            self.note_script_error(pull_error(inner, "input", &c_message(&error)));
        }
    }

    fn semantics_action(&mut self, action: uicore::SemanticsAction) {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        let wrapper_edit = match &action {
            uicore::SemanticsAction::SetText { id, text }
            | uicore::SemanticsAction::Autofill { id, text } => {
                let tree = inner.tree.borrow();
                let autofill = matches!(action, uicore::SemanticsAction::Autofill { .. });
                tree.semantics_edit_target(*id, autofill)
                    .filter(|node| tree.is_text(*node).is_ok_and(|is_text| !is_text))
                    .map(|node| (node, text))
            }
            _ => None,
        };
        let result = if let Some((node, text)) = wrapper_edit {
            dispatch_text(inner, node, text)
        } else {
            let gestures = inner.tree.borrow_mut().semantics_action(action);
            dispatch_gestures(inner, gestures)
        };
        if let Err(message) = result {
            self.note_script_error(message);
        }
    }

    fn update_source(
        &mut self,
        source: &str,
        shaper: Box<dyn Shaper>,
        io: crate::ui::RoutedIoSubmit,
        time: crate::session::FrameTime,
    ) -> Result<(), String> {
        // SAFETY: the render thread still owns the live session throughout staging.
        let prior = unsafe { &*self.inner };
        let log = Rc::clone(&prior.log);
        let mut candidate = Self::open_source(
            source.as_bytes(),
            "app.js",
            shaper,
            Box::new(move |level, line| log(level, line)),
            None,
            prior.config.clone(),
        )
        .map_err(|error| error.to_string())?;
        candidate.metrics(&prior.window_metrics.get());
        candidate.settings(&prior.platform.borrow());
        candidate.window_display(prior.window_display.borrow().clone());
        candidate.set_semantics_enabled(prior.tree.borrow().semantics_enabled());
        candidate.set_autofill_enabled(prior.tree.borrow().autofill_enabled());
        candidate.set_trace(prior.trace.get());
        let mut output = candidate.frame(time);
        if let Some(fault) = candidate.take_faults().into_iter().next() {
            return Err(fault.message);
        }
        if output
            .layers
            .as_ref()
            .is_none_or(|tree| tree.layers.is_empty())
            || output.exit
        {
            return Err("updated script did not produce a scene".to_owned());
        }
        if output.platform_views.is_none() {
            // The previous runtime may have placed views with overlapping node IDs.
            output.platform_views = Some(Arc::default());
        }
        if prior.paused.get() {
            // SAFETY: this pointer is owned by the staged candidate.
            unsafe { &*candidate.inner }.paused.set(true);
        }
        candidate.install_routed_io(io);
        candidate.reload_frame = Some(output);
        *self = candidate;
        Ok(())
    }

    fn set_semantics_enabled(&mut self, enabled: bool) {
        // SAFETY: The session owns this Inner until it shuts down the VM.
        let inner = unsafe { &*self.inner };
        inner.tree.borrow_mut().set_semantics_enabled(enabled);
    }

    fn set_autofill_enabled(&mut self, enabled: bool) {
        // SAFETY: the interpreter and its tree are owned by this session.
        let inner = unsafe { &*self.inner };
        inner.tree.borrow_mut().set_autofill_enabled(enabled);
    }

    fn take_fonts(&mut self) -> Vec<(String, Vec<u8>)> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.fonts.borrow_mut().take()
    }

    fn take_reloads(&mut self) -> Vec<String> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        std::mem::take(&mut *inner.reloads.borrow_mut())
    }

    fn take_exit(&mut self) -> bool {
        self.exit.replace(false)
    }

    fn take_clipboard(&mut self) -> Option<String> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        inner.clipboard.borrow_mut().take()
    }

    fn ime_dismissed(&mut self) {
        // SAFETY: `inner` is owned by this session. The ime borrow ends before
        // blur, because the blur callback calls `setImeState` on the same cell.
        let inner = unsafe { &*self.inner };
        bump(&inner.marks.ime);
        {
            let mut ime = inner.ime.borrow_mut();
            ime.shown = false;
            ime.dirty = true;
        }
        let gestures = inner.tree.borrow_mut().blur();
        if let Err(message) = dispatch_gestures(inner, gestures) {
            self.note_script_error(message);
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    fn frame(&mut self, time: crate::session::FrameTime) -> crate::session::FrameOutput {
        if let Some(mut output) = self.reload_frame.take() {
            output.again |= self.wants_frame();
            return output;
        }
        use crate::session::trace::Section;

        let script_started = std::time::Instant::now();
        let script_trace = Section::begin(c"Impasto script");
        // SAFETY: `inner` is owned by this session. Job drain and `vm_frame` may
        // re-enter the tree through the `RefCell`; this function does not hold that borrow yet.
        let inner = unsafe { &*self.inner };
        let stamp = std::time::Instant::now();
        {
            let mut tree = inner.tree.borrow_mut();
            let trace_enabled = Section::enabled() || inner.trace.get();
            inner.marks.trace_enabled.set(trace_enabled);
            tree.trace_text_probes(trace_enabled);
            tree.prepare_sync_stats();
            inner.ime.borrow_mut().sync_editor(&mut tree);
            if inner.reduce_motion.get() {
                tree.finish_motions();
            } else {
                tree.tick_motions(time.target_ms);
            }
            let mut notices = tree.tick(time.target_ms);
            notices.extend(tree.take_scroll_notices());
            drop(tree);
            let motion_ms = elapsed_ms(stamp);
            let stamp = std::time::Instant::now();
            if let Err(message) = dispatch_gestures(inner, notices) {
                return self.hold_frame(message);
            }
            if let Err(message) = drain_jobs(inner) {
                return self.hold_frame(message);
            }
            let dispatch_ms = elapsed_ms(stamp);
            let paused = self_paused(inner);
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            unsafe { vm_set_clock(inner.host, time.mono_ms as i64, i32::from(paused)) };
            if !paused {
                // SAFETY: the session owns Host through the script phase.
                for message in deliver_errors(unsafe { &*inner.host }) {
                    self.note_script_error(message);
                }
            }
            self.engine.drain_local_io();
            let mut error = [0u8; 512];
            let timer_started = std::time::Instant::now();
            let timer_failed =
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                unsafe { vm_fire_timers(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if timer_failed != 0 {
                self.note_script_error(pull_error(inner, "timer", &c_message(&error)));
            }
            let timer_ms = elapsed_ms(timer_started);
            if let Err(message) = drain_jobs(inner) {
                return self.hold_frame(message);
            }
            error = [0u8; 512];
            let raf_started = std::time::Instant::now();
            let raf_failed =
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                unsafe { vm_fire_raf(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if raf_failed != 0 {
                self.note_script_error(pull_error(inner, "timer", &c_message(&error)));
            }
            let raf_ms = elapsed_ms(raf_started);
            if let Err(message) = drain_jobs(inner) {
                return self.hold_frame(message);
            }
            let ends = inner.tree.borrow_mut().take_motion_ends();
            for (id, prop) in ends {
                let name = cstring(prop.script_name());
                error = [0u8; 512];
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                let failed = unsafe {
                    vm_motion_end(
                        inner.host,
                        id.to_raw(),
                        name.as_ptr(),
                        error.as_mut_ptr().cast(),
                        error.len(),
                    )
                };
                if failed != 0 {
                    self.note_script_error(pull_error(inner, "motion", &c_message(&error)));
                }
            }
            // Frame listeners can read the graph updated by input, timers, and rAF.
            {
                let _trace = Section::begin(c"Impasto reactive");
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
            }
            error = [0u8; 512];
            let callback_started = std::time::Instant::now();
            let callback_trace = Section::begin(c"Impasto frame callbacks");
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            let failed = unsafe {
                vm_frame(
                    inner.host,
                    time.target_ms,
                    time.interval_ms,
                    error.as_mut_ptr().cast(),
                    error.len(),
                )
            };
            if failed != 0 {
                return self.hold_frame(pull_error(inner, "frame", &c_message(&error)));
            }
            let callback_ms = elapsed_ms(callback_started);
            drop(callback_trace);
            if let Err(message) = drain_jobs(inner) {
                return self.hold_frame(message);
            }
            // Commit frame listeners' signal writes before layout reads the tree.
            {
                let _trace = Section::begin(c"Impasto reactive after frame callbacks");
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
            }
            let flush_started = std::time::Instant::now();
            if let Err(error) = mutation::flush_mutations(inner) {
                let message = error.to_string();
                self.push_fault("frame", &message);
                return self.hold_frame(message);
            }
            let flush_ms = elapsed_ms(flush_started);
            sweep_detached(inner);
            let script_ms = elapsed_ms(script_started);
            drop(script_trace);
            let style_trace = Section::begin(c"Impasto styles");
            let queued = inner.fonts.borrow().snapshot();
            let mut tree = inner.tree.borrow_mut();
            for (family, bytes) in &queued {
                let _ = tree.register_font(family, bytes);
            }
            if let Err(error) = tree.flush_styles() {
                let message = error.to_string();
                drop(tree);
                return self.hold_layout(message);
            }
            let style_notices = tree.take_style_gestures();
            drop(tree);
            if let Err(message) = dispatch_gestures(inner, style_notices) {
                self.note_script_error(message);
            }
            drop(style_trace);
            let mut tree = inner.tree.borrow_mut();
            let mut ran_layout = tree.needs_layout();
            if ran_layout {
                let _trace = Section::begin(c"Impasto layout");
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_layout(message);
                }
            }
            drop(tree);
            let driver_trace = Section::begin(c"Impasto list and scroll");
            let mut list_ops = 0_u32;
            for event in driver::collect_driver_tick(inner) {
                let _trace = matches!(event, uicore::DriverEvent::ListOp { .. }).then(|| {
                    list_ops = list_ops.saturating_add(1);
                    Section::begin(c"Impasto ListOp")
                });
                if let Err(message) = driver::emit_driver(inner, &event) {
                    self.note_script_error(message);
                }
            }
            if let Err(error) = mutation::flush_mutations(inner) {
                let message = error.to_string();
                self.push_fault("frame", &message);
                return self.hold_frame(message);
            }
            drop(driver_trace);
            {
                let mut tree = inner.tree.borrow_mut();
                if tree.needs_layout() {
                    let _trace = Section::begin(c"Impasto layout after drivers");
                    if let Err(error) = tree.layout() {
                        let message = error.to_string();
                        drop(tree);
                        return self.hold_layout(message);
                    }
                    ran_layout = true;
                }
            }
            let mut layout_cb_ms = 0.0;
            if ran_layout {
                let _trace = Section::begin(c"Impasto layout callbacks");
                let callback_started = std::time::Instant::now();
                error = [0u8; 512];
                let failed =
                    // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                    unsafe { vm_layout(inner.host, error.as_mut_ptr().cast(), error.len()) };
                if failed != 0 {
                    return self.hold_frame(pull_error(inner, "layout", &c_message(&error)));
                }
                // Layout listeners may schedule a further update for this frame.
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
                layout_cb_ms = elapsed_ms(callback_started);
                if let Err(error) = mutation::flush_mutations(inner) {
                    let message = error.to_string();
                    self.push_fault("frame", &message);
                    return self.hold_frame(message);
                }
            }
            let mut tree = inner.tree.borrow_mut();
            if ran_layout && tree.needs_layout() {
                let _trace = Section::begin(c"Impasto layout after callbacks");
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_layout(message);
                }
            }
            drop(tree);
            let canvas_stats = if paused {
                CanvasStats::default()
            } else {
                let _trace = Section::begin(c"Impasto canvas");
                // SAFETY: the session owns Host through the paint turn.
                paint(unsafe { &*inner.host })
            };
            let mut tree = inner.tree.borrow_mut();
            let record_started = std::time::Instant::now();
            if tree.needs_paint() {
                let _trace = Section::begin(c"Impasto record");
                match tree.record_layers() {
                    Ok(()) => self.note_record_ok(),
                    Err(error) => {
                        let message = error.to_string();
                        self.note_record_err(&message);
                    }
                }
                Section::event(|| {
                    format!(
                        "Impasto record work visits={} ops={}",
                        tree.layer_record_visits(),
                        tree.picture_writes()
                    )
                });
                tree.clear_paint();
            }
            // Fresh overflow findings go to the log as warnings; the
            // devtools layout page reads the full report on demand.
            for issue in tree.take_layout_issues() {
                tracing::warn!(
                    target: "impasto::layout",
                    axis = issue.axis.name(),
                    parent = issue.parent.to_raw(),
                    parent_size = issue.parent_size,
                    used = issue.used,
                    excess = issue.excess,
                    child = issue.worst_child.to_raw(),
                    "overflow"
                );
            }
            let record_ms = elapsed_ms(record_started);
            let epoch = tree.layer_epoch();
            let layers = if epoch != self.layer_epoch {
                self.layer_epoch = epoch;
                Some(Arc::new(tree.layer_tree()))
            } else {
                None
            };
            let generation = tree.image_generation();
            let images = if generation != self.image_generation {
                self.image_generation = generation;
                Some(Arc::new(tree.image_census()))
            } else {
                None
            };
            let layout_report = tree.debug_scan_enabled().then(|| tree.layout_issues_json());
            let semantics_started = std::time::Instant::now();
            let semantics = tree.semantics_patch();
            let placements = tree.platform_views();
            let platform_views = if self.platform_views.as_ref() == placements.as_slice() {
                None
            } else {
                self.platform_views = placements.into();
                Some(Arc::clone(&self.platform_views))
            };
            let semantics_ms = elapsed_ms(semantics_started);
            let (layout_ms, sync_ms) = tree.layout_timing();
            let (sync_visited, sync_synced) = tree.sync_stats();
            let scroll_paint_updates = tree.take_scroll_paint_updates();
            let layout_trace = tree.take_layout_trace();
            if let Some(trigger) = layout_trace.root_trigger {
                Section::event(|| {
                    let node = trigger.node.map_or(0, NodeId::to_raw);
                    match trigger.node.and_then(|id| tree.rect(id).ok()) {
                        Some(rect) => format!(
                            "Impasto root cause={} node={node} x={:.0} y={:.0} w={:.0} h={:.0}",
                            trigger.cause, rect.x, rect.y, rect.w, rect.h,
                        ),
                        None => format!("Impasto root cause={} node={node}", trigger.cause),
                    }
                });
            }
            inner.ime.borrow_mut().sync_editor(&mut tree);
            drop(tree);
            let trace = crate::session::FrameTrace {
                laywork_ms: layout_trace.laywork_ms,
                layout_passes: layout_trace.passes,
                root_layouts: layout_trace.root_layouts,
                local_layouts: layout_trace.local_layouts,
                same_text_layouts: layout_trace.same_text_layouts,
                list_ops,
                text_reused: layout_trace.text_reused,
                text_shaped: layout_trace.text_shaped,
                layout_cause: layout_trace.cause.name(),
                pointers: inner.marks.pointers.take(),
                keys: inner.marks.keys.take(),
                edits: inner.marks.edits.take(),
                ime: inner.marks.ime.take(),
                scrolls: inner.marks.scrolls.take(),
                focus: inner.marks.focus.take(),
                blur: inner.marks.blur.take(),
                gestures: inner.marks.gestures.take(),
                updates: inner.graph.arena.take_update_count(),
                sets: inner.marks.sets.take(),
                canvas_record_ms: canvas_stats.record_ms,
                canvas_prepare_ms: canvas_stats.prepare_ms,
                canvas_ops: canvas_stats.ops,
                canvas_segments: canvas_stats.segments,
                canvas_redraws: canvas_stats.redraws,
                canvas_callbacks: canvas_stats.callbacks,
                canvas_preparations: canvas_stats.preparations,
                canvas_reused: canvas_stats.reused,
            };
            Section::event(|| {
                format!(
                    "Impasto work roots={} local={} same_text={} pointer={} updates={} sets={} list_ops={} draw={} prepare={} commit={}",
                    trace.root_layouts,
                    trace.local_layouts,
                    trace.same_text_layouts,
                    trace.pointers,
                    trace.updates,
                    trace.sets,
                    trace.list_ops,
                    trace.canvas_callbacks,
                    trace.canvas_preparations,
                    trace.canvas_redraws,
                )
            });
            if layout_trace.passes != 0 || sync_visited != 0 {
                Section::event(|| {
                    format!(
                        "Impasto layout work taffy_us={:.0} sync_us={:.0} hits={} misses={} shaped={} checked={} synced={}",
                        layout_ms * 1000.0,
                        sync_ms * 1000.0,
                        layout_trace.cache_hits,
                        layout_trace.cache_misses,
                        layout_trace.text_shaped,
                        sync_visited,
                        sync_synced,
                    )
                });
            }
            if scroll_paint_updates != 0 {
                Section::event(|| format!("Impasto scroll work changes={scroll_paint_updates}"));
            }
            let probes = layout_trace.text_probes;
            if !probes.check_time.is_zero() {
                Section::event(|| {
                    format!(
                        "Impasto text probes check_us={} measure_us={} queries={} intrinsic={} repeat={} hit={} miss={}",
                        probes.check_time.as_micros(),
                        probes.measure_time.as_micros(),
                        probes.queries,
                        probes.intrinsic,
                        probes.repeated,
                        probes.cache_hits,
                        probes.cache_misses,
                    )
                });
            }
            if inner.marks.callbacks.get() != 0 || inner.marks.mutation_calls.get() != 0 {
                Section::event(|| {
                    format!(
                        "Impasto script work callbacks={} callback_us={} mutations={} equal={} mutation_us={} text={}",
                        inner.marks.callbacks.get(),
                        inner.marks.callback_time.get().as_micros(),
                        inner.marks.mutation_calls.get(),
                        inner.marks.mutation_equal.get(),
                        inner.marks.mutation_time.get().as_micros(),
                        inner.marks.text_writes.get(),
                    )
                });
            }
            if inner.trace.get() {
                tracing::info!(
                    target: "impasto::script",
                    script_us = ((script_ms + layout_cb_ms) * 1000.0).round() as u64,
                    updates = trace.updates,
                    sets = trace.sets,
                    callbacks = inner.marks.callbacks.get(),
                    text_writes = inner.marks.text_writes.get(),
                    text_probe_us = probes.check_time.as_micros() as u64,
                    text_measure_us = probes.measure_time.as_micros() as u64,
                    text_queries = probes.queries,
                    text_intrinsic = probes.intrinsic,
                    text_queries_repeated = probes.repeated,
                    measure_hits = probes.cache_hits,
                    measure_misses = probes.cache_misses,
                    "frame"
                );
            }
            inner.marks.reset_trace();
            let again = self.wants_frame();
            crate::session::FrameOutput {
                layers,
                again,
                ime: self.take_ime(),
                semantics,
                platform_views,
                fonts: self.take_fonts(),
                reloads: self.take_reloads(),
                clipboard: self.take_clipboard(),
                exit: self.take_exit(),
                images,
                phases: Some(crate::session::FramePhases {
                    script_ms,
                    layout_ms,
                    layout_cb_ms,
                    sync_ms,
                    record_ms,
                    semantics_ms,
                    script: crate::session::ScriptPhases {
                        motion_ms,
                        dispatch_ms,
                        timer_ms,
                        raf_ms,
                        callback_ms,
                        flush_ms,
                    },
                    trace,
                }),
                layout_report,
            }
        }
    }
    fn wants_frame(&mut self) -> bool {
        // SAFETY: `inner` is owned by this session. The call reads and clears one flag.
        let inner = unsafe { &*self.inner };
        if self.suppress_demand.replace(false) {
            inner.settings_frame.set(false);
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            let _ = unsafe { vm_take_wants_frame(inner.host) };
            return false;
        }
        let visual = {
            let tree = inner.tree.borrow();
            tree.needs_layout()
                || tree.needs_paint()
                || tree.motion_active()
                || tree.pending_long_press()
        };
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        let jobs = unsafe { vm_jobs_pending(inner.host) != 0 };
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        let soon = unsafe { vm_needs_frame(inner.host) != 0 };
        let explicit =
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            inner.settings_frame.take() || unsafe { vm_take_wants_frame(inner.host) != 0 };
        let driver = inner.driver_frame.get();
        let paints = !inner.paused.get() && inner.graph.arena.has_paints();
        explicit || visual || jobs || soon || driver || paints
    }

    fn lifecycle(&mut self, event: crate::session::Lifecycle) {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        let paused = matches!(
            event,
            crate::session::Lifecycle::Hidden | crate::session::Lifecycle::Paused
        );
        inner.paused.set(paused);
        let name = match event {
            crate::session::Lifecycle::Resumed => "resumed",
            crate::session::Lifecycle::Inactive => "inactive",
            crate::session::Lifecycle::Hidden => "hidden",
            crate::session::Lifecycle::Paused => "paused",
            crate::session::Lifecycle::Detached => "detached",
        };
        let state = bootstrap::app_state(name);
        crate::session::shared_write(&inner.config.app_state, state.to_owned());
        self.engine.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "change".into(),
            payload: PluginValue::Str(state.into()),
        });
    }

    fn window_focus(&mut self, focused: bool) {
        self.engine.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "focus".into(),
            payload: PluginValue::Str(focused.to_string()),
        });
    }

    fn window_state_changed(&mut self, key: String) {
        self.engine.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "windowState".into(),
            payload: PluginValue::Str(key),
        });
    }

    fn io_event(&mut self, event: crate::session::IoEvent) {
        self.engine.deliver_event(event);
    }

    fn io_delivery(&mut self, delivery: crate::session::Delivery) {
        self.engine.deliver_routed(delivery);
    }

    fn next_wake_ms(&mut self) -> Option<f64> {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        if inner.paused.get() {
            return None;
        }
        // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
        let deadline = unsafe { vm_next_deadline(inner.host) };
        if deadline < 0 {
            None
        } else {
            Some(deadline as f64)
        }
    }

    fn set_launch_url(&mut self, url: &str) {
        // SAFETY: Engine owns this stable Inner allocation through VM shutdown.
        let inner = unsafe { &*self.inner };
        crate::session::shared_write(&inner.config.launch_url, Some(url.to_owned()));
        self.engine.emit_host_event(HostEvent {
            module: "Linking".into(),
            event: "url".into(),
            payload: PluginValue::Map(vec![("url".into(), PluginValue::Str(url.into()))]),
        });
    }
}

fn drain_jobs(inner: &Inner) -> Result<(), String> {
    let mut error = [0u8; 512];
    // SAFETY: `host` is the interpreter for this session. `error` is writable
    // for the length we pass. Drain runs on the render thread, with the engine.
    let failed = unsafe { vm_drain_jobs(inner.host, error.as_mut_ptr().cast(), error.len()) };
    if failed == 0 {
        Ok(())
    } else {
        Err(pull_error(inner, "jobs", &c_message(&error)))
    }
}

pub(super) fn c_message(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Takes the exception saved by the current VM turn.
fn take_stashed(host: *mut Host) -> (String, String) {
    // SAFETY: callers pass the Host owned by their live session.
    unsafe { &*host }.take_error()
}

fn display_fault(message: &str, stack: &str) -> String {
    if stack.is_empty() {
        message.to_owned()
    } else {
        format!("{message}\n{stack}")
    }
}

/// Reads the stashed exception, records a [`crate::session::ScriptFault`], and returns
/// the text `take_script_error` keeps.
fn pull_error(inner: &Inner, source: &'static str, fallback: &str) -> String {
    let (message, stack) = take_stashed(inner.host);
    let message = if message.is_empty() {
        fallback.to_owned()
    } else {
        message
    };
    let display = display_fault(&message, &stack);
    inner.faults.borrow_mut().push(crate::session::ScriptFault {
        source,
        message,
        stack,
    });
    display
}

pub(super) fn publish(out: *mut u64, id: u64) -> i32 {
    if out.is_null() {
        return -1;
    }
    // SAFETY: the VM binding passes a writable `uint64_t` that lives for this call.
    unsafe { *out = id };
    0
}

pub(super) fn status(result: Result<(), uicore::Error>) -> i32 {
    if result.is_ok() { 0 } else { -1 }
}

pub(super) fn queried(result: Result<Option<NodeId>, uicore::Error>, out: *mut u64) -> i32 {
    match result {
        Ok(Some(id)) => publish(out, id.to_raw()),
        Ok(None) => publish(out, 0),
        Err(_) => -1,
    }
}

/// # Safety
/// `session` is the `Inner` installed in [`JsSession::open`] and is still alive.
unsafe fn inner<'a>(session: *mut c_void) -> &'a Inner {
    // SAFETY: session is captured from the live Host and used only on the render thread.
    unsafe { &*session.cast::<Inner>() }
}

/// # Safety
/// `ptr` and `len` describe a byte slice that lives for this call.
pub(super) unsafe fn utf8_of<'a>(ptr: *const c_char, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: the caller guarantees `ptr` points at `len` initialized bytes.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes).ok()
}

/// # Safety
/// `color` points at 4 initialized bytes that live for this call.
pub(super) unsafe fn rgba(color: *const u8) -> Option<[u8; 4]> {
    if color.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a 4-byte color.
    let bytes = unsafe { std::slice::from_raw_parts(color, 4) };
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}
