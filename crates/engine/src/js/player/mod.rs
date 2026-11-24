//! Live QuickJS session. Tap handlers are JS values owned by the interpreter.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use crate::session::{InputEvent, Key, Metrics, ModuleConfig, PluginValue, Session};
use crate::{
    text::{Affinity, Shaper},
    tree::{FocusDirection, NodeId, Rect, Tree},
};
use javascript::host::HostEvent;

use crate::js::Error;

mod bootstrap;
mod driver;
mod environment;
mod reactive;

mod bridge;
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

fn drain_fonts(inner: &Inner, tree: &mut Tree) {
    for (family, bytes) in inner.fonts.borrow_mut().take() {
        tree.register_shared_font(&family, bytes);
    }
}

struct Inner {
    pub(super) tree: RefCell<Tree>,
    pub(super) log: SharedConsoleLog,
    pub(super) ime: RefCell<ImeSlot>,
    pub(super) text_scale: Cell<f32>,
    pub(super) reduce_motion: Cell<bool>,
    pub(super) platform: RefCell<crate::session::PlatformSettings>,
    pub(super) window_metrics: Cell<Metrics>,
    pub(super) window_display: RefCell<crate::session::WindowDisplay>,
    pub(super) settings_frame: Cell<bool>,
    /// Bytes from the latest `text_edit`. Stable until the next edit.
    /// Copy or Cut text waiting for the platform clipboard.
    pub(super) clipboard: RefCell<Option<String>>,
    /// Selection rectangles from the latest query, as packed `x,y,w,h` floats.
    pub(super) config: ModuleConfig,
    pub(super) paused: Cell<bool>,
    pub(super) fonts: RefCell<FontBook>,
    /// Sources passed to `reload` since the last [`Session::take_reloads`].
    pub(super) reloads: RefCell<Vec<String>>,
    pub(super) marks: FrameMarks,
    pub(super) drivers: RefCell<crate::tree::list::DriverSet>,
    pub(super) driver_frame: Cell<bool>,
    pub(super) last_pen: Cell<Option<crate::input::PointerEvent>>,
    trace: Cell<bool>,
    counters: Cell<bool>,
    /// Script failures waiting for [`crate::session::Session::take_faults`].
    pub(super) faults: RefCell<Vec<crate::session::ScriptFault>>,
    /// The reactive graph owns Persistent values and temporary transfer slots.
    pub(super) graph: reactive::Graph,
    /// Paint bindings drop before the budget so a recorder can refund into it.
    pub(super) canvas: RefCell<CanvasSession>,
    pub(super) canvas_budget: RefCell<crate::canvas::Budget>,
}

impl Inner {
    fn new(
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
        config: ModuleConfig,
    ) -> Self {
        Self {
            tree: RefCell::new(Tree::new(shaper)),
            log: Rc::from(log),
            ime: RefCell::new(ImeSlot::new()),
            text_scale: Cell::new(1.0),
            reduce_motion: Cell::new(false),
            platform: RefCell::new(crate::session::PlatformSettings::default()),
            window_metrics: Cell::new(Metrics::new(0.0, 0.0, 1.0)),
            window_display: RefCell::new(crate::session::WindowDisplay::default()),
            settings_frame: Cell::new(false),
            clipboard: RefCell::new(None),
            config,
            paused: Cell::new(false),
            fonts: RefCell::new(FontBook::new()),
            reloads: RefCell::new(Vec::new()),
            marks: FrameMarks::default(),
            drivers: RefCell::new(crate::tree::list::DriverSet::new()),
            driver_frame: Cell::new(false),
            last_pen: Cell::new(None),
            trace: Cell::new(std::env::var("IMPASTO_TRACE").as_deref() == Ok("1")),
            counters: Cell::new(false),
            faults: RefCell::new(Vec::new()),
            graph: reactive::Graph::new(),
            canvas: RefCell::new(CanvasSession::new()),
            canvas_budget: RefCell::new(crate::canvas::Budget::session()),
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
    target: Option<NodeId>,
}

impl ImeSlot {
    fn new() -> Self {
        Self {
            shown: false,
            announced: false,
            dirty: false,
            state: crate::session::ImeState::at_end(""),
            revision: 0,
            target: None,
        }
    }

    /// Recomputes the platform keyboard state when the focused editor moved.
    fn sync_editor(&mut self, tree: &mut Tree) {
        static NEXT_CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let target = tree.focused();
        if target != self.target {
            self.target = target;
            self.revision = 0;
            self.state.client = NEXT_CLIENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.state.ack = 0;
            self.state.node = target.map(NodeId::to_raw);
            self.announced = false;
            self.dirty = self.dirty || self.shown;
        }
        let Some(id) = target else {
            return;
        };
        let autofill = tree.editor_autofill_hint(id);
        if self.state.autofill != autofill {
            self.state.autofill = autofill;
            self.dirty = self.dirty || self.shown;
        }
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
    inner: Rc<Inner>,
    vm: JsVm,
    layer_epoch: u64,
    platform_views: Arc<[crate::tree::PlatformViewPlacement]>,
    image_generation: u64,
    script_error: RefCell<Option<String>>,
    exit: Cell<bool>,
    /// A failed frame must not schedule another frame by itself.
    suppress_demand: Cell<bool>,
    ui_work_pending: bool,
    reload_frame: Option<crate::session::FrameOutput>,
    record_failing: bool,
    record_frames: u32,
}

impl JsSession {
    pub fn open(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
    ) -> Result<Self, Error> {
        Self::open_with(script, name, shaper, log, ModuleConfig::disconnected())
    }

    pub fn open_with(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_source(script.as_bytes(), name, shaper, log, config)
    }

    pub fn open_source(
        script: &[u8],
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_program(ProgramKind::Source(script), name, shaper, log, config)
    }

    pub(crate) fn open_trusted(
        program: javascript::runtime::TrustedBytecode<'_>,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        Self::open_program(ProgramKind::Trusted(program), name, shaper, log, config)
    }

    fn open_program(
        program: ProgramKind<'_>,
        name: &str,
        shaper: impl Shaper + 'static,
        log: crate::js::ConsoleLog,
        config: ModuleConfig,
    ) -> Result<Self, Error> {
        let inner = Rc::new(Inner::new(shaper, log, config));
        let vm = JsVm::new(Rc::clone(&inner), app_directory(name))?;
        vm.install(bootstrap::SCENE_JS)?;
        vm.start(program, name)?;
        sweep_detached(&vm.host);
        Ok(Self {
            inner,
            vm,
            layer_epoch: u64::MAX,
            platform_views: Arc::default(),
            image_generation: u64::MAX,
            script_error: RefCell::new(None),
            exit: Cell::new(false),
            suppress_demand: Cell::new(false),
            ui_work_pending: false,
            reload_frame: None,
            record_failing: false,
            record_frames: 0,
        })
    }

    fn hold_frame(&mut self, message: String) -> crate::session::FrameOutput {
        self.note_script_error(message);
        self.suppress_demand.set(true);
        let inner = &self.inner;
        sweep_detached(&self.vm.host);
        inner.marks.reset();
        inner.graph.take_update_count();
        crate::session::FrameOutput::unchanged()
    }

    #[cfg(test)]
    pub(crate) fn picture_writes(&self) -> usize {
        let inner = &self.inner;
        inner.tree.borrow().picture_writes()
    }

    pub fn semantics(&mut self) -> Option<crate::semantics::SemanticsUpdate> {
        let inner = &self.inner;
        inner.tree.borrow().semantics()
    }

    pub fn take_script_error(&self) -> Option<String> {
        self.script_error.borrow_mut().take()
    }

    pub fn take_faults(&mut self) -> Vec<crate::session::ScriptFault> {
        let inner = &self.inner;
        std::mem::take(&mut *inner.faults.borrow_mut())
    }

    /// Flush pending graph updates at a frame boundary.
    fn run_reactive(&mut self) -> Result<(), String> {
        let inner = &self.inner;
        if !inner.graph.has_updates() {
            return Ok(());
        }
        let _trace =
            tracing::trace_span!(target: "impasto::script::solid", "reactive_flush").entered();
        self.vm.host.run_updates()
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
        let inner = &self.inner;
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
        self.vm.host.jobs_pending()
    }

    pub fn tap_count(&self) -> usize {
        self.vm.host.tap_count()
    }
}

impl Session for JsSession {
    fn register_shader_program(
        &mut self,
        source: &str,
        clock: crate::scene::ShaderClock,
    ) -> Result<crate::scene::ShaderProgramId, crate::scene::ShaderError> {
        self.inner
            .tree
            .borrow_mut()
            .register_shader_program(source, clock)
    }
    fn set_counters(&mut self, enabled: bool) {
        self.inner.counters.set(enabled);
    }

    fn runtime_sample(&self) -> Option<crate::session::RuntimeSample> {
        let counts = self.vm.host.runtime.counts();
        Some(crate::session::RuntimeSample {
            promises: counts.promises,
            timers: counts.timers,
            requests: counts.requests,
            queued_events: counts.queued_events,
            queued_bytes: counts.queued_bytes,
            heap_bytes: self.vm.host.runtime.heap_bytes(),
        })
    }

    fn set_trace(&mut self, enabled: bool) {
        self.inner.trace.set(enabled);
    }

    fn take_script_error(&mut self) -> Option<String> {
        JsSession::take_script_error(self)
    }

    fn take_faults(&mut self) -> Vec<crate::session::ScriptFault> {
        JsSession::take_faults(self)
    }

    fn metrics(&mut self, metrics: &Metrics) {
        let inner = &self.inner;
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
        if inner.window_metrics.replace(*metrics) != *metrics
            && let Err(message) = self.vm.host.apply_window_metrics("input")
        {
            self.note_script_error(message);
        }
    }

    fn window_display(&mut self, display: crate::session::WindowDisplay) {
        let inner = &self.inner;
        {
            let mut current = inner.window_display.borrow_mut();
            if *current == display {
                return;
            }
            *current = display;
        }
        if let Err(message) = self.vm.host.apply_window_metrics("windowMetrics") {
            self.note_script_error(message);
        }
    }

    fn pointer(&mut self, event: crate::input::PointerEvent) {
        let inner = &self.inner;
        if event.pen.is_some() {
            inner.last_pen.set(Some(event));
        }
        let gestures = inner.tree.borrow_mut().pointer(event);
        bump(&inner.marks.pointers);
        if let Err(message) = dispatch_pointer(&self.vm.host, gestures) {
            self.note_script_error(message);
        }
    }

    fn take_back_enabled(&mut self) -> Option<bool> {
        self.vm.host.take_back_enabled()
    }

    fn back_gesture(&mut self, event: crate::session::BackGesture) -> bool {
        use crate::session::BackGesture;
        let progress = if let BackGesture::Progress(value) = event {
            value
        } else {
            0.0
        };
        self.inner.tree.borrow_mut().set_back_progress(progress);
        match self.vm.host.back_gesture(event) {
            Ok(handled) => handled,
            Err(message) => {
                self.note_script_error(message);
                true
            }
        }
    }

    fn input(&mut self, event: InputEvent) {
        let inner = &self.inner;
        let event = match event {
            InputEvent::ReplaceIme {
                client,
                sequence,
                text,
                start,
                end,
                compose_start,
                compose_end,
            } => {
                let focused = inner.tree.borrow().focused();
                let mut ime = inner.ime.borrow_mut();
                if !ime.shown
                    || focused != ime.target
                    || client != ime.state.client
                    || sequence <= ime.state.ack
                {
                    return;
                }
                ime.state.ack = sequence;
                ime.dirty = true;
                InputEvent::Replace {
                    text,
                    start,
                    end,
                    compose_start,
                    compose_end,
                }
            }
            InputEvent::DismissIme(client) => {
                let matches = {
                    let ime = inner.ime.borrow();
                    ime.shown
                        && ime.target == inner.tree.borrow().focused()
                        && client == ime.state.client
                };
                if matches {
                    self.ime_dismissed();
                }
                return;
            }
            event => event,
        };
        match &event {
            InputEvent::Key(_) | InputEvent::Back => bump(&inner.marks.keys),
            InputEvent::Text(_)
            | InputEvent::Composing(_)
            | InputEvent::Copy
            | InputEvent::Cut
            | InputEvent::Paste { .. }
            | InputEvent::SelectAll
            | InputEvent::Edit(_)
            | InputEvent::Replace { .. }
            | InputEvent::ReplaceIme { .. }
            | InputEvent::DismissIme(_) => bump(&inner.marks.edits),
        }
        if let InputEvent::Key(press) = &event {
            let spec = shortcut_spec(press);
            if !spec.contains('\0') {
                match self.vm.host.try_shortcut(&spec) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(message) => {
                        self.note_script_error(message);
                        return;
                    }
                }
            }
        }
        let escape = matches!(event, InputEvent::Key(press) if press.key == Key::Escape);
        if matches!(event, InputEvent::Back) || escape {
            match self.vm.host.dispatch_back("input") {
                Ok(false) if !escape => self.exit.set(true),
                Err(message) => self.note_script_error(message),
                Ok(_) => {}
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
            if let Err(message) = dispatch_gestures(&self.vm.host, gestures) {
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
                &self.vm.host,
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
                    let _ = dispatch_text(&self.vm.host, id, &value.text);
                    return;
                }
                // The focused control is the field chrome. The editor lives on
                // its text child, so a buffer edit is delivered as a key or text.
                let (kind, forwarded) = match edit {
                    crate::text::Edit::DeleteRange { .. } | crate::text::Edit::Backspace => {
                        (8, Some("Backspace".to_string()))
                    }
                    crate::text::Edit::ForwardDelete => (8, Some("Delete".to_string())),
                    crate::text::Edit::Insert(text) => (9, Some(text)),
                    _ => return,
                };
                if let Err(message) =
                    dispatch_input(&self.vm.host, id, kind, forwarded.as_deref(), None)
                {
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
            InputEvent::Back
            | InputEvent::Replace { .. }
            | InputEvent::ReplaceIme { .. }
            | InputEvent::DismissIme(_) => return,
        };
        if let Err(message) = dispatch_input(&self.vm.host, id, kind, text.as_deref(), None) {
            self.note_script_error(message);
        }
    }

    fn take_ime(&mut self) -> Option<crate::session::ImeUpdate> {
        let inner = &self.inner;
        inner.ime.borrow_mut().take()
    }

    fn settings(&mut self, settings: &crate::session::PlatformSettings) {
        let inner = &self.inner;
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
        if let Err(message) = self.vm.host.apply_settings() {
            self.note_script_error(message);
        }
    }

    fn semantics_action(&mut self, action: crate::semantics::SemanticsAction) {
        let inner = &self.inner;
        let wrapper_edit = match &action {
            crate::semantics::SemanticsAction::SetText { id, text }
            | crate::semantics::SemanticsAction::Autofill { id, text } => {
                let tree = inner.tree.borrow();
                let autofill = matches!(action, crate::semantics::SemanticsAction::Autofill { .. });
                tree.semantics_edit_target(*id, autofill)
                    .filter(|node| tree.is_text(*node).is_ok_and(|is_text| !is_text))
                    .map(|node| (node, text))
            }
            _ => None,
        };
        let result = if let Some((node, text)) = wrapper_edit {
            dispatch_text(&self.vm.host, node, text)
        } else {
            let gestures = inner.tree.borrow_mut().semantics_action(action);
            dispatch_gestures(&self.vm.host, gestures)
        };
        if let Err(message) = result {
            self.note_script_error(message);
        }
    }

    fn update_source(
        &mut self,
        source: &str,
        shaper: Box<dyn Shaper>,
        time: crate::session::FrameTime,
    ) -> Result<(), String> {
        let prior = &self.inner;
        let log = Rc::clone(&prior.log);
        let mut candidate = Self::open_source(
            source.as_bytes(),
            "app.js",
            shaper,
            Box::new(move |level, line| log(level, line)),
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
            candidate.inner.paused.set(true);
        }
        candidate.reload_frame = Some(output);
        *self = candidate;
        Ok(())
    }

    fn set_semantics_enabled(&mut self, enabled: bool) {
        let inner = &self.inner;
        inner.tree.borrow_mut().set_semantics_enabled(enabled);
    }

    fn set_autofill_enabled(&mut self, enabled: bool) {
        let inner = &self.inner;
        inner.tree.borrow_mut().set_autofill_enabled(enabled);
    }

    fn take_reloads(&mut self) -> Vec<String> {
        let inner = &self.inner;
        std::mem::take(&mut *inner.reloads.borrow_mut())
    }

    fn take_exit(&mut self) -> bool {
        self.exit.replace(false)
    }

    fn take_clipboard(&mut self) -> Option<String> {
        let inner = &self.inner;
        inner.clipboard.borrow_mut().take()
    }

    fn ime_dismissed(&mut self) {
        let inner = &self.inner;
        bump(&inner.marks.ime);
        {
            let mut ime = inner.ime.borrow_mut();
            ime.shown = false;
            ime.dirty = true;
        }
        let gestures = inner.tree.borrow_mut().blur();
        if let Err(message) = dispatch_gestures(&self.vm.host, gestures) {
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
        self.vm.record_stack_samples();
        let script_started = std::time::Instant::now();
        let script_trace = tracing::debug_span!(target: "impasto::script", "script").entered();
        let vm_host = Rc::clone(&self.vm.host);
        let host = &*vm_host;
        let inner = host.inner();
        let stamp = std::time::Instant::now();
        {
            let motion_span =
                tracing::trace_span!(target: "impasto::script", "motion_tick").entered();
            let mut tree = inner.tree.borrow_mut();
            let trace_enabled = tracing::enabled!(target: "impasto::layout", tracing::Level::DEBUG)
                || inner.trace.get()
                || inner.counters.get();
            inner.marks.trace_enabled.set(trace_enabled);
            tree.trace_text_probes(trace_enabled);
            tree.prepare_sync_stats();
            inner.ime.borrow_mut().sync_editor(&mut tree);
            if inner.reduce_motion.get() {
                tree.finish_motions();
            } else {
                tree.tick_motions(time.target_ms);
            }
            tree.sync_scroll_offsets();
            let mut notices = tree.tick(time.target_ms);
            notices.extend(tree.take_scroll_notices());
            drop(tree);
            drop(motion_span);
            let motion_ms = elapsed_ms(stamp);
            let stamp = std::time::Instant::now();
            {
                let _dispatch_span =
                    tracing::trace_span!(target: "impasto::script", "dispatch_gestures").entered();
                if let Err(message) = dispatch_gestures(host, notices) {
                    return self.hold_frame(message);
                }
                if let Err(message) = host.drain_jobs() {
                    return self.hold_frame(message);
                }
            }
            let dispatch_ms = elapsed_ms(stamp);
            let paused = inner.paused.get();
            host.set_clock(time.mono_ms as i64, paused);
            if !paused {
                for message in deliver_errors(host) {
                    self.note_script_error(message);
                }
            }
            self.vm.drain_local_io();
            let timer_started = std::time::Instant::now();
            {
                let _timer_span =
                    tracing::trace_span!(target: "impasto::script", "timers").entered();
                if let Err(message) = host.fire_timers() {
                    self.note_script_error(message);
                }
            }
            let timer_ms = elapsed_ms(timer_started);
            if let Err(message) = host.drain_jobs() {
                return self.hold_frame(message);
            }
            let raf_started = std::time::Instant::now();
            {
                let _raf_span = tracing::trace_span!(target: "impasto::script", "raf").entered();
                if let Err(message) = host.fire_raf() {
                    self.note_script_error(message);
                }
            }
            let raf_ms = elapsed_ms(raf_started);
            if let Err(message) = host.drain_jobs() {
                return self.hold_frame(message);
            }
            let ends = inner.tree.borrow_mut().take_motion_ends();
            for (id, prop) in ends {
                if let Err(message) = host.motion_end(id.to_raw(), prop.script_name()) {
                    self.note_script_error(message);
                }
            }
            // Frame listeners can read the graph updated by input, timers, and rAF.
            {
                let _trace = tracing::debug_span!(target: "impasto::script", "reactive").entered();
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
            }
            let callback_started = std::time::Instant::now();
            let callback_trace =
                tracing::debug_span!(target: "impasto::script", "frame_callbacks").entered();
            if let Err(message) = host.frame(time.target_ms, time.interval_ms) {
                return self.hold_frame(message);
            }
            let callback_ms = elapsed_ms(callback_started);
            drop(callback_trace);
            if let Err(message) = host.drain_jobs() {
                return self.hold_frame(message);
            }
            // Commit frame listeners' signal writes before layout reads the tree.
            {
                let _trace =
                    tracing::debug_span!(target: "impasto::script", "reactive_post").entered();
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
            }
            sweep_detached(host);
            let script_ms = elapsed_ms(script_started);
            drop(script_trace);
            let style_trace = tracing::debug_span!(target: "impasto::layout", "styles").entered();
            let mut tree = inner.tree.borrow_mut();
            drain_fonts(inner, &mut tree);
            if let Err(error) = tree.flush_styles() {
                let message = error.to_string();
                drop(tree);
                return self.hold_layout(message);
            }
            let style_notices = tree.take_style_gestures();
            drop(tree);
            if let Err(message) = dispatch_gestures(host, style_notices) {
                self.note_script_error(message);
            }
            drop(style_trace);
            let mut tree = inner.tree.borrow_mut();
            let mut ran_layout = tree.needs_layout();
            if ran_layout {
                let _trace = tracing::debug_span!(target: "impasto::layout", "layout").entered();
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_layout(message);
                }
            }
            drop(tree);
            inner.tree.borrow_mut().sync_scroll_offsets();
            let driver_trace =
                tracing::debug_span!(target: "impasto::layout", "list_driver").entered();
            let mut list_ops = 0_u32;
            for event in driver::collect_driver_tick(inner) {
                let _trace =
                    matches!(event, crate::tree::list::DriverEvent::ListOp { .. }).then(|| {
                        list_ops = list_ops.saturating_add(1);
                        tracing::trace_span!(target: "impasto::layout", "list_op").entered()
                    });
                if let Err(message) = host.driver(&event) {
                    self.note_script_error(message);
                }
            }
            drop(driver_trace);
            {
                let mut tree = inner.tree.borrow_mut();
                drain_fonts(inner, &mut tree);
                if tree.needs_layout() {
                    let _trace =
                        tracing::debug_span!(target: "impasto::layout", "layout_after_drivers")
                            .entered();
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
                let _trace =
                    tracing::debug_span!(target: "impasto::script", "layout_callbacks").entered();
                let callback_started = std::time::Instant::now();
                if let Err(message) = host.layout() {
                    return self.hold_frame(message);
                }
                // Layout listeners may schedule a further update for this frame.
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
                layout_cb_ms = elapsed_ms(callback_started);
            }
            let mut tree = inner.tree.borrow_mut();
            drain_fonts(inner, &mut tree);
            if tree.needs_layout() {
                let _trace =
                    tracing::debug_span!(target: "impasto::layout", "layout_after_callbacks")
                        .entered();
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_layout(message);
                }
            }
            drop(tree);
            let thresholds = inner.tree.borrow_mut().update_scroll_thresholds();
            if !thresholds.is_empty() {
                for (id, active) in thresholds {
                    if let Err(message) = host.scroll_threshold(id, active) {
                        self.note_script_error(message);
                    }
                }
                if let Err(error) = self.run_reactive() {
                    return self.hold_frame(error);
                }
                let mut tree = inner.tree.borrow_mut();
                let layout = tree.flush_styles().and_then(|()| {
                    if tree.needs_layout() {
                        tree.layout().map(|()| true)
                    } else {
                        Ok(false)
                    }
                });
                drop(tree);
                match layout {
                    Err(error) => return self.hold_layout(error.to_string()),
                    Ok(true) if !ran_layout => {
                        let started = std::time::Instant::now();
                        if let Err(error) = host.layout().and_then(|()| self.run_reactive()) {
                            return self.hold_frame(error);
                        }
                        layout_cb_ms += elapsed_ms(started);
                        let mut tree = inner.tree.borrow_mut();
                        if tree.needs_layout()
                            && let Err(error) = tree.layout()
                        {
                            let message = error.to_string();
                            drop(tree);
                            return self.hold_layout(message);
                        }
                    }
                    _ => {}
                }
            }
            let canvas_stats = if paused {
                CanvasStats::default()
            } else {
                let _trace = tracing::debug_span!(target: "impasto::canvas", "canvas").entered();
                paint(host)
            };
            let mut tree = inner.tree.borrow_mut();
            let record_started = std::time::Instant::now();
            if tree.needs_paint() {
                let _trace =
                    tracing::debug_span!(target: "impasto::render::record", "record_layers")
                        .entered();
                match tree.record_layers() {
                    Ok(()) => self.note_record_ok(),
                    Err(error) => {
                        let message = error.to_string();
                        self.note_record_err(&message);
                    }
                }
                tracing::debug!(
                    target: "impasto::render::record",
                    visits = tree.layer_record_visits(),
                    ops = tree.picture_writes(),
                    "record_work"
                );
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
            let semantics_span =
                tracing::debug_span!(target: "impasto::layout", "semantics").entered();
            let semantics = tree.semantics_patch();
            let placements = tree.platform_views();
            let platform_views = if self.platform_views.as_ref() == placements.as_slice() {
                None
            } else {
                self.platform_views = placements.into();
                Some(Arc::clone(&self.platform_views))
            };
            drop(semantics_span);
            let semantics_ms = elapsed_ms(semantics_started);
            let (layout_ms, sync_ms) = tree.layout_timing();
            let (sync_visited, sync_synced) = tree.sync_stats();
            let scroll_paint_updates = tree.take_scroll_paint_updates();
            let layout_trace = tree.take_layout_trace();
            if let Some(trigger) = layout_trace.root_trigger {
                let node = trigger.node.map_or(0, NodeId::to_raw);
                if let Some(rect) = trigger.node.and_then(|id| tree.rect(id).ok()) {
                    tracing::debug!(
                        target: "impasto::layout",
                        cause = %trigger.cause,
                        node,
                        x = rect.x,
                        y = rect.y,
                        w = rect.w,
                        h = rect.h,
                        "root_layout_trigger"
                    );
                } else {
                    tracing::debug!(
                        target: "impasto::layout",
                        cause = %trigger.cause,
                        node,
                        "root_layout_trigger"
                    );
                }
            }
            inner.ime.borrow_mut().sync_editor(&mut tree);
            drop(tree);
            self.vm.record_stack_samples();
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
                updates: inner.graph.take_update_count(),
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
            tracing::debug!(
                target: "impasto::layout",
                roots = trace.root_layouts,
                local = trace.local_layouts,
                same_text = trace.same_text_layouts,
                pointer = trace.pointers,
                updates = trace.updates,
                sets = trace.sets,
                list_ops = trace.list_ops,
                draw = trace.canvas_callbacks,
                prepare = trace.canvas_preparations,
                commit = trace.canvas_redraws,
                "frame_work"
            );
            if layout_trace.passes != 0 || sync_visited != 0 {
                tracing::debug!(
                    target: "impasto::layout",
                    taffy_us = layout_ms * 1000.0,
                    sync_us = sync_ms * 1000.0,
                    hits = layout_trace.cache_hits,
                    misses = layout_trace.cache_misses,
                    shaped = layout_trace.text_shaped,
                    checked = sync_visited,
                    synced = sync_synced,
                    "layout_work"
                );
            }
            if scroll_paint_updates != 0 {
                tracing::debug!(
                    target: "impasto::layout",
                    changes = scroll_paint_updates,
                    "scroll_work"
                );
            }
            let probes = layout_trace.text_probes;
            if !probes.check_time.is_zero() {
                tracing::debug!(
                    target: "impasto::layout",
                    check_us = probes.check_time.as_micros() as u64,
                    measure_us = probes.measure_time.as_micros() as u64,
                    queries = probes.queries,
                    intrinsic = probes.intrinsic,
                    repeat = probes.repeated,
                    hit = probes.cache_hits,
                    miss = probes.cache_misses,
                    "text_probes"
                );
            }
            if inner.marks.callbacks.get() != 0 || inner.marks.mutation_calls.get() != 0 {
                tracing::debug!(
                    target: "impasto::script",
                    callbacks = inner.marks.callbacks.get(),
                    callback_us = inner.marks.callback_time.get().as_micros() as u64,
                    mutations = inner.marks.mutation_calls.get(),
                    equal = inner.marks.mutation_equal.get(),
                    mutation_us = inner.marks.mutation_time.get().as_micros() as u64,
                    text = inner.marks.text_writes.get(),
                    "script_work"
                );
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
                        flush_ms: 0.0,
                    },
                    trace,
                }),
                layout_report,
            }
        }
    }
    fn wants_frame(&mut self) -> bool {
        let inner = &self.inner;
        if self.suppress_demand.replace(false) {
            self.ui_work_pending = true;
            inner.settings_frame.set(false);
            self.vm.host.take_wants_frame();
            return false;
        }
        let (visual, motion) = {
            let tree = inner.tree.borrow();
            (
                tree.needs_layout() || tree.needs_paint() || tree.pending_long_press(),
                tree.motion_active(),
            )
        };
        let jobs = self.vm.host.jobs_pending();
        let soon = self.vm.host.needs_frame();
        let explicit = inner.settings_frame.take() || self.vm.host.take_wants_frame();
        let driver = inner.driver_frame.get();
        let paints = !inner.paused.get() && inner.graph.has_paints();
        self.ui_work_pending = explicit || visual || jobs || soon || driver || paints;
        self.ui_work_pending || motion
    }

    fn compositor_eligibility(
        &self,
        tracks: &[crate::scene::CompositorTrack],
    ) -> crate::scene::CompositorEligibility {
        use crate::scene::CompositorBlockReason;

        let inner = &self.inner;
        let host = &self.vm.host;
        if host.observes_frame() {
            return Err(CompositorBlockReason::GlobalFrameHook);
        }
        if inner.canvas.borrow().observes_visibility() {
            return Err(CompositorBlockReason::CanvasVisibilityObserver);
        }
        inner
            .tree
            .borrow()
            .compositor_eligibility(tracks, |node| host.observes_scroll(node.to_raw()))?;
        if self.ui_work_pending || inner.paused.get() || inner.graph.has_updates() {
            return Err(CompositorBlockReason::PendingUiWork);
        }
        Ok(())
    }

    fn commit_compositor_state(&mut self, snapshot: &crate::scene::CompositorSnapshot) {
        self.inner
            .tree
            .borrow_mut()
            .commit_compositor_state(snapshot);
    }

    fn lifecycle(&mut self, event: crate::session::Lifecycle) {
        let inner = &self.inner;
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
        self.vm.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "change".into(),
            payload: PluginValue::Str(state.into()),
        });
    }

    fn window_focus(&mut self, focused: bool) {
        self.vm.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "focus".into(),
            payload: PluginValue::Str(focused.to_string()),
        });
    }

    fn window_state_changed(&mut self, key: String) {
        self.vm.emit_host_event(HostEvent {
            module: "AppState".into(),
            event: "windowState".into(),
            payload: PluginValue::Str(key),
        });
    }

    fn io_event(&mut self, event: crate::session::IoEvent) {
        self.vm.deliver_event(event);
    }

    fn io_delivery(&mut self, delivery: crate::session::Delivery) {
        self.vm.deliver_routed(delivery);
    }

    fn next_wake_ms(&mut self) -> Option<f64> {
        let inner = &self.inner;
        if inner.paused.get() {
            return None;
        }
        if self.inner.paused.get() {
            return None;
        }
        self.vm.host.next_deadline().map(|deadline| deadline as f64)
    }

    fn set_launch_url(&mut self, url: &str) {
        let inner = &self.inner;
        crate::session::shared_write(&inner.config.launch_url, Some(url.to_owned()));
        self.vm.emit_host_event(HostEvent {
            module: "Linking".into(),
            event: "url".into(),
            payload: PluginValue::Map(vec![("url".into(), PluginValue::Str(url.into()))]),
        });
    }
}

fn display_fault(message: &str, stack: &str) -> String {
    if stack.is_empty() {
        message.to_owned()
    } else {
        format!("{message}\n{stack}")
    }
}
