//! Live QuickJS session. Tap handlers are JS values owned by the interpreter.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use protocol::Command;
use session::{
    HostConfig, HttpError, HttpResponse, InputEvent, IoJob, Key, Metrics, PluginResult, Session,
};
use uicore::{Affinity, FocusDirection, NodeId, Rect, Shaper, Tree};

use crate::Error;

mod driver;
mod host;

mod args;
mod bridge;
mod ffi;
mod frame;
mod keys;
mod nodes;
use args::*;
use bridge::*;
use ffi::*;
use frame::*;
use keys::*;
use nodes::*;

#[allow(private_interfaces)]
pub(super) struct Inner {
    pub(super) tree: RefCell<Tree>,
    pub(super) host: *mut c_void,
    pub(super) log: Box<dyn Fn(u8, &str)>,
    pub(super) ime: RefCell<ImeSlot>,
    pub(super) text_scale: Cell<f32>,
    pub(super) reduce_motion: Cell<bool>,
    pub(super) platform: RefCell<session::PlatformSettings>,
    pub(super) settings_frame: Cell<bool>,
    /// Bytes from the latest `text_edit`. Stable until the next edit.
    pub(super) edit_bytes: RefCell<Vec<u8>>,
    /// Copy or Cut text waiting for the platform clipboard.
    pub(super) clipboard: RefCell<Option<String>>,
    /// Selection rectangles from the latest query, as packed `x,y,w,h` floats.
    pub(super) selection_rects: RefCell<Vec<f32>>,
    pub(super) io: RefCell<Option<Box<dyn Fn(IoJob) + Send>>>,
    pub(super) http: RefCell<Option<Arc<dyn session::HttpClient>>>,
    pub(super) config: HostConfig,
    pub(super) last_error: RefCell<Option<CString>>,
    pub(super) paused: Cell<bool>,
    pub(super) cancels: RefCell<HashMap<u64, Arc<AtomicBool>>>,
    pub(super) pending: RefCell<Pending>,
    /// Zero-copy payloads handed to QuickJS. The handle counter never repeats
    /// within a session, so a stale free from a freed view cannot drop live
    /// bytes.
    pub(super) out_bytes: RefCell<(u64, HashMap<u64, Vec<u8>>)>,
    pub(super) fonts: RefCell<FontBook>,
    /// Sources passed to `reload` since the last [`Session::take_reloads`].
    pub(super) reloads: RefCell<Vec<String>>,
    pub(super) mutations: RefCell<Vec<PendingMutation>>,
    pub(super) prepared: RefCell<Option<PreparedCall>>,
    pub(super) marks: FrameMarks,
    pub(super) drivers: RefCell<uicore::DriverSet>,
    pub(super) driver_frame: Cell<bool>,
}

pub(super) struct Pending {
    pub(super) http: Vec<(u64, Result<HttpResponse, HttpError>)>,
    pub(super) plugin: Vec<(u64, PluginResult)>,
    pub(super) socket: Vec<(u64, String, bool, Vec<u8>)>,
}

impl Pending {
    fn new() -> Self {
        Self {
            http: Vec::new(),
            plugin: Vec::new(),
            socket: Vec::new(),
        }
    }
}

struct ImeSlot {
    shown: bool,
    announced: bool,
    dirty: bool,
    state: session::ImeState,
    /// Last editor revision copied into [`ImeSlot::state`].
    revision: u64,
}

impl ImeSlot {
    fn new() -> Self {
        Self {
            shown: false,
            announced: false,
            dirty: false,
            state: session::ImeState::at_end(""),
            revision: 0,
        }
    }

    /// Recomputes the platform keyboard state when the focused editor moved.
    fn sync_editor(&mut self, tree: &Tree) {
        let Some(id) = tree.focused() else {
            return;
        };
        let Some((revision, value)) = tree.editor_snapshot(id) else {
            return;
        };
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        let (start, end) = value.selection.ordered();
        let caret = tree
            .caret_for(id, value.selection.extent, Affinity::Downstream)
            .ok();
        let origin = tree.rect(id).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        self.state.text = value.text;
        self.state.selection = start..end;
        self.state.composing = value.composing.map(|range| range.start..range.end);
        if let Some(caret) = caret {
            self.state.caret = Rect {
                x: origin.x + caret.x,
                y: origin.y + caret.y,
                w: caret.w,
                h: caret.h,
            };
        }
        self.dirty = true;
    }

    fn take(&mut self) -> Option<session::ImeUpdate> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        if !self.shown {
            self.announced = false;
            return Some(session::ImeUpdate::Hide);
        }
        let state = self.state.clone();
        if self.announced {
            Some(session::ImeUpdate::Update(state))
        } else {
            self.announced = true;
            Some(session::ImeUpdate::Show(state))
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
    paint_commands: Vec<Command>,
    layer_epoch: u64,
    image_generation: u64,
    script_error: RefCell<Option<String>>,
    exit: Cell<bool>,
    /// A failed frame must not schedule another frame by itself.
    suppress_demand: Cell<bool>,
    _thread: PhantomData<Rc<()>>,
}

impl JsSession {
    pub fn open(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: Box<dyn Fn(u8, &str)>,
    ) -> Result<Self, Error> {
        Self::open_with(script, name, shaper, log, None, HostConfig::disconnected())
    }

    pub fn open_with(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: Box<dyn Fn(u8, &str)>,
        http: Option<Arc<dyn session::HttpClient>>,
        config: HostConfig,
    ) -> Result<Self, Error> {
        let inner = Box::into_raw(Box::new(Inner {
            tree: RefCell::new(Tree::new(shaper)),
            host: ptr::null_mut(),
            log,
            ime: RefCell::new(ImeSlot::new()),
            text_scale: Cell::new(1.0),
            reduce_motion: Cell::new(false),
            platform: RefCell::new(session::PlatformSettings::default()),
            settings_frame: Cell::new(false),
            edit_bytes: RefCell::new(Vec::new()),
            clipboard: RefCell::new(None),
            selection_rects: RefCell::new(Vec::new()),
            io: RefCell::new(None),
            http: RefCell::new(None),
            config,
            last_error: RefCell::new(None),
            paused: Cell::new(false),
            cancels: RefCell::new(HashMap::new()),
            pending: RefCell::new(Pending::new()),
            out_bytes: RefCell::new((0, HashMap::new())),
            fonts: RefCell::new(FontBook::new()),
            reloads: RefCell::new(Vec::new()),
            mutations: RefCell::new(Vec::new()),
            prepared: RefCell::new(None),
            marks: FrameMarks::default(),
            drivers: RefCell::new(uicore::DriverSet::new()),
            driver_frame: Cell::new(false),
        }));
        let fns = QjsFns {
            session: inner.cast(),
            column,
            row,
            stack,
            padding,
            pointer_region,
            decorated,
            text,
            append,
            insert,
            remove,
            detach,
            discard,
            free_ids,
            parent,
            first_child,
            next_sibling,
            is_text,
            set_text,
            set_font_size,
            set_color,
            set_gap,
            set_padding,
            clip,
            opacity,
            set_grow,
            image,
            set_radius,
            set_alpha,
            set_align,
            set_size,
            set_image,
            set_max_lines,
            set_ellipsis,
            set_text_align,
            set_font_weight,
            set_text_direction,
            set_line_height,
            set_family,
            scroll,
            set_scroll_offset,
            scroll_offset,
            rect,
            transform,
            set_transform,
            set_justify,
            set_wrap,
            set_fill,
            log: log_message,
            sized,
            set_focusable,
            focus: focus_node,
            blur: blur_node,
            set_ime_enabled,
            set_ime_text,
            set_ime_state,
            set_semantics,
            text_scale,
            reduce_motion,
            measure_text,
            grid,
            set_padding_edges,
            set_margin,
            set_min_size,
            set_max_size,
            set_shrink,
            set_basis,
            set_align_self,
            set_drag_claim,
            caret_at,
            word_range,
            focus_step,
            push_focus_scope,
            pop_focus_scope,
            pointer_position,
            text_edit,
            text_edit_bytes,
            set_text_scale,
            set_soft_wrap,
            set_overflow,
            caret_at_unit,
            selection_rects,
            selection_rect_bytes,
            set_ime_config,
            http: http_call,
            http_stream: http_stream_call,
            abort_token,
            module_has,
            module_prepare,
            socket_open,
            socket_send,
            socket_close,
            focused: focused_id,
            is_focusable,
            grapheme_index: grapheme_index_of,
            grapheme_count: grapheme_count_of,
            grapheme_unit: grapheme_unit_of,
            set_reverse,
            set_axis_size,
            set_min_axis,
            set_max_axis,
            measure_panel,
            set_panel,
            set_aspect_ratio,
            set_position,
            set_inset,
            set_grid_placement,
            set_scale_xy,
            register_font,
            set_image_mode,
            reload,
            push_mutation,
            release_bytes: release_bytes_cb,
            layer_stats,
            scroll_into_view,
            animate,
            fling_target,
            set_recognizers,
            set_stroke,
            module_push_null,
            module_push_bool,
            module_push_f64,
            module_push_str,
            module_push_bytes,
            module_begin_list,
            module_end_list,
            module_begin_map,
            module_map_key,
            module_end_map,
            module_set_token,
            module_finish,
            module_error,
            module_count,
            module_method_at,
            module_name_of,
            align,
            set_cross,
            viewport,
            set_scroll_physics,
            set_scroll_chain,
            overscroll,
            arm_fling,
            stop_scroll,
            scroll_to,
            list_create: driver::list_create,
            list_release: driver::list_release,
            list_bind: driver::list_bind,
            list_configure: driver::list_configure,
            list_begin: driver::list_begin,
            list_item: driver::list_item,
            list_end: driver::list_end,
            list_note_row: driver::list_note_row,
            scroll_create: driver::scroll_create,
            scroll_release: driver::scroll_release,
            scroll_bind: driver::scroll_bind,
            scroll_configure: driver::scroll_configure,
            scroll_drag_by: driver::scroll_drag_by,
            scroll_finish: driver::scroll_finish,
            scroll_drag_end: driver::scroll_drag_end,
            scroll_wheel: driver::scroll_wheel,
            scroll_query: driver::scroll_query,
            scroll_consume: driver::scroll_consume,
            set_scroll_axis,
        };
        // SAFETY: `inner` stays at this address until `Drop`. `qjs_open` copies
        // the function table. The copied session pointer is only used while
        // this `Inner` is alive, which is until `qjs_close` returns in `Drop`.
        let host = unsafe { qjs_open(&fns) };
        if host.is_null() {
            unsafe { drop(Box::from_raw(inner)) };
            return Err(Error::Runtime);
        }
        // SAFETY: `inner` is the allocation from `Box::into_raw` above, and `host`
        // is the pointer `qjs_open` just returned.
        unsafe { (*inner).host = host };
        if let Some(client) = http {
            unsafe { (*inner).http = RefCell::new(Some(client)) };
        }
        let mut error = [0u8; 512];
        let prelude_name = CString::new("prelude.js").map_err(|_| Error::Runtime)?;
        let prelude_failed = unsafe {
            qjs_eval(
                host,
                host::PRELUDE.as_ptr().cast(),
                host::PRELUDE.len(),
                prelude_name.as_ptr(),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        if prelude_failed != 0 {
            let message = c_message(&error);
            unsafe { qjs_close(host) };
            unsafe { (*inner).host = ptr::null_mut() };
            unsafe { drop(Box::from_raw(inner)) };
            return Err(Error::Script(message));
        }
        error = [0u8; 512];
        if let Some(dir) = app_directory(name) {
            let dir = match CString::new(dir) {
                Ok(dir) => dir,
                Err(_) => {
                    unsafe { qjs_close(host) };
                    unsafe { (*inner).host = ptr::null_mut() };
                    unsafe { drop(Box::from_raw(inner)) };
                    return Err(Error::Runtime);
                }
            };
            unsafe { qjs_set_app_dir(host, dir.as_ptr()) };
        }
        let name = match CString::new(name) {
            Ok(name) => name,
            Err(_) => {
                unsafe { qjs_close(host) };
                unsafe { (*inner).host = ptr::null_mut() };
                unsafe { drop(Box::from_raw(inner)) };
                return Err(Error::Runtime);
            }
        };
        // SAFETY: `script` is borrowed for the call. `name` is NUL-terminated and
        // lives until `qjs_eval` returns. `error` is a writable buffer of the length we pass.
        let failed = unsafe {
            qjs_eval(
                host,
                script.as_ptr().cast(),
                script.len(),
                name.as_ptr(),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        if failed != 0 {
            let message = c_message(&error);
            unsafe { qjs_close(host) };
            unsafe { (*inner).host = ptr::null_mut() };
            unsafe { drop(Box::from_raw(inner)) };
            return Err(Error::Script(message));
        }
        // SAFETY: `inner` is the session just opened. Eval has returned, so ids
        // removed and not inserted again can be retired before the caller observes them.
        sweep_detached(unsafe { &*inner });
        Ok(Self {
            inner,
            paint_commands: Vec::new(),
            layer_epoch: u64::MAX,
            image_generation: u64::MAX,
            script_error: RefCell::new(None),
            exit: Cell::new(false),
            suppress_demand: Cell::new(false),
            _thread: PhantomData,
        })
    }

    fn hold_frame(&mut self, message: String) -> session::FrameOutput {
        self.note_script_error(message);
        self.suppress_demand.set(true);
        let inner = unsafe { &*self.inner };
        sweep_detached(inner);
        inner.marks.reset();
        session::FrameOutput::unchanged()
    }

    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn picture_writes(&self) -> usize {
        let inner = unsafe { &*self.inner };
        inner.tree.borrow().picture_writes()
    }

    pub fn install_io(&mut self, submit: Box<dyn Fn(IoJob) + Send>) {
        let inner = unsafe { &*self.inner };
        *inner.io.borrow_mut() = Some(submit);
    }

    pub fn semantics(&mut self) -> Option<uicore::SemanticsUpdate> {
        let inner = unsafe { &*self.inner };
        inner.tree.borrow().semantics()
    }

    pub fn take_script_error(&self) -> Option<String> {
        self.script_error.borrow_mut().take()
    }

    fn note_script_error(&self, message: String) {
        // SAFETY: `inner` is owned by this session. The log hook does not re-enter it.
        let inner = unsafe { &*self.inner };
        (inner.log)(3, &message);
        *self.script_error.borrow_mut() = Some(message);
    }

    pub fn jobs_pending(&self) -> bool {
        // SAFETY: `inner` is owned by this session. The query only reads the runtime flag.
        let inner = unsafe { &*self.inner };
        unsafe { qjs_jobs_pending(inner.host) != 0 }
    }

    pub fn tap_count(&self) -> usize {
        // SAFETY: `inner` is owned by this session. The query only reads the tap table length.
        let inner = unsafe { &*self.inner };
        unsafe { qjs_tap_count(inner.host) as usize }
    }
}

impl Drop for JsSession {
    fn drop(&mut self) {
        // SAFETY: `inner` came from `Box::into_raw` and has not been freed.
        // Closing the host frees every duplicated JS function before the context.
        unsafe {
            let inner = Box::from_raw(self.inner);
            if !inner.host.is_null() {
                qjs_close(inner.host);
            }
        }
    }
}

impl Session for JsSession {
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
        if metrics.device_pixel_ratio.is_finite() && metrics.device_pixel_ratio > 0.0 {
            let _ = tree.set_pixel_scale(metrics.device_pixel_ratio);
        }
    }

    fn pointer(&mut self, event: uicore::PointerEvent) {
        // SAFETY: the tree borrow ends before dispatch. JS handlers may call
        // back into the tree through the `RefCell`.
        let inner = unsafe { &*self.inner };
        let mut gestures = inner.tree.borrow_mut().pointer(event);
        gestures.extend(inner.tree.borrow_mut().take_scroll_notices());
        bump(&inner.marks.pointers);
        if let Err(message) = dispatch_gestures(inner, gestures) {
            self.note_script_error(message);
        }
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
                let hit = unsafe {
                    qjs_try_shortcut(
                        inner.host,
                        c_spec.as_ptr(),
                        error.as_mut_ptr().cast(),
                        error.len(),
                    )
                };
                if hit < 0 {
                    self.note_script_error(c_message(&error));
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
                unsafe { qjs_dispatch_back(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if handled < 0 {
                self.note_script_error(c_message(&error));
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

    fn take_ime(&mut self) -> Option<session::ImeUpdate> {
        let inner = unsafe { &*self.inner };
        inner.ime.borrow_mut().take()
    }

    fn settings(&mut self, settings: &session::PlatformSettings) {
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
        let dark = i32::from(settings.brightness == session::Brightness::Dark);
        let failed = unsafe {
            qjs_apply_settings(
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
            self.note_script_error(c_message(&error));
        }
    }

    fn semantics_action(&mut self, action: uicore::SemanticsAction) {
        let inner = unsafe { &*self.inner };
        let gestures = inner.tree.borrow_mut().semantics_action(action);
        if let Err(message) = dispatch_gestures(inner, gestures) {
            self.note_script_error(message);
        }
    }

    fn take_fonts(&mut self) -> Vec<(String, Vec<u8>)> {
        let inner = unsafe { &*self.inner };
        inner.fonts.borrow_mut().take()
    }

    fn take_reloads(&mut self) -> Vec<String> {
        let inner = unsafe { &*self.inner };
        std::mem::take(&mut *inner.reloads.borrow_mut())
    }

    fn take_exit(&mut self) -> bool {
        self.exit.replace(false)
    }

    fn take_clipboard(&mut self) -> Option<String> {
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

    fn frame(&mut self, time: session::FrameTime) -> session::FrameOutput {
        let script_started = std::time::Instant::now();
        // SAFETY: `inner` is owned by this session. Job drain and `qjs_frame` may
        // re-enter the tree through the `RefCell`; this function does not hold that borrow yet.
        let inner = unsafe { &*self.inner };
        let stamp = std::time::Instant::now();
        {
            let mut tree = inner.tree.borrow_mut();
            inner.ime.borrow_mut().sync_editor(&tree);
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
            if let Err(message) = drain_jobs(inner.host) {
                return self.hold_frame(message);
            }
            let dispatch_ms = elapsed_ms(stamp);
            let paused = self_paused(inner);
            unsafe { qjs_set_clock(inner.host, time.mono_ms as i64, i32::from(paused)) };
            flush_pending(inner);
            let mut error = [0u8; 512];
            let timer_started = std::time::Instant::now();
            let timer_failed =
                unsafe { qjs_fire_timers(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if timer_failed != 0 {
                self.note_script_error(c_message(&error));
            }
            let timer_ms = elapsed_ms(timer_started);
            if let Err(message) = drain_jobs(inner.host) {
                return self.hold_frame(message);
            }
            error = [0u8; 512];
            let raf_started = std::time::Instant::now();
            let raf_failed =
                unsafe { qjs_fire_raf(inner.host, error.as_mut_ptr().cast(), error.len()) };
            if raf_failed != 0 {
                self.note_script_error(c_message(&error));
            }
            let raf_ms = elapsed_ms(raf_started);
            if let Err(message) = drain_jobs(inner.host) {
                return self.hold_frame(message);
            }
            let ends = inner.tree.borrow_mut().take_motion_ends();
            for (id, prop) in ends {
                let name = cstring(prop.script_name());
                error = [0u8; 512];
                let failed = unsafe {
                    qjs_motion_end(
                        inner.host,
                        id.to_raw(),
                        name.as_ptr(),
                        error.as_mut_ptr().cast(),
                        error.len(),
                    )
                };
                if failed != 0 {
                    self.note_script_error(c_message(&error));
                }
            }
            error = [0u8; 512];
            let callback_started = std::time::Instant::now();
            let failed = unsafe {
                qjs_frame(
                    inner.host,
                    time.target_ms,
                    time.interval_ms,
                    error.as_mut_ptr().cast(),
                    error.len(),
                )
            };
            if failed != 0 {
                return self.hold_frame(c_message(&error));
            }
            let callback_ms = elapsed_ms(callback_started);
            if let Err(message) = drain_jobs(inner.host) {
                return self.hold_frame(message);
            }
            let flush_started = std::time::Instant::now();
            flush_mutations(inner);
            let flush_ms = elapsed_ms(flush_started);
            sweep_detached(inner);
            let script_ms = elapsed_ms(script_started);
            let mut tree = inner.tree.borrow_mut();
            tree.prepare_sync_stats();
            let mut ran_layout = tree.needs_layout();
            if ran_layout {
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_frame(message);
                }
            }
            drop(tree);
            for event in driver::collect_driver_tick(inner) {
                if let Err(message) = driver::emit_driver(inner, &event) {
                    self.note_script_error(message);
                }
            }
            flush_mutations(inner);
            {
                let mut tree = inner.tree.borrow_mut();
                if tree.needs_layout() {
                    if let Err(error) = tree.layout() {
                        let message = error.to_string();
                        drop(tree);
                        return self.hold_frame(message);
                    }
                    ran_layout = true;
                }
            }
            let mut layout_cb_ms = 0.0;
            if ran_layout {
                let callback_started = std::time::Instant::now();
                error = [0u8; 512];
                let failed =
                    unsafe { qjs_layout(inner.host, error.as_mut_ptr().cast(), error.len()) };
                layout_cb_ms = elapsed_ms(callback_started);
                if failed != 0 {
                    return self.hold_frame(c_message(&error));
                }
                flush_mutations(inner);
            }
            let mut tree = inner.tree.borrow_mut();
            if ran_layout && tree.needs_layout() {
                if let Err(error) = tree.layout() {
                    let message = error.to_string();
                    drop(tree);
                    return self.hold_frame(message);
                }
            }
            let record_started = std::time::Instant::now();
            if tree.needs_paint() {
                let mut commands = std::mem::take(&mut self.paint_commands);
                if tree.record_into(&mut commands).is_err() {
                    commands.clear();
                }
                self.paint_commands = commands;
                tree.clear_paint();
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
            let semantics_started = std::time::Instant::now();
            let semantics = tree.semantics_patch();
            let semantics_ms = elapsed_ms(semantics_started);
            let (layout_ms, sync_ms) = tree.layout_timing();
            let layout_trace = tree.take_layout_trace();
            drop(tree);
            let trace = session::FrameTrace {
                laywork_ms: layout_trace.laywork_ms,
                layout_passes: layout_trace.passes,
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
            };
            let again = self.wants_frame();
            return session::FrameOutput {
                layers,
                again,
                ime: self.take_ime(),
                semantics,
                fonts: self.take_fonts(),
                reloads: self.take_reloads(),
                clipboard: self.take_clipboard(),
                exit: self.take_exit(),
                images,
                phases: Some(session::FramePhases {
                    script_ms,
                    layout_ms,
                    layout_cb_ms,
                    sync_ms,
                    record_ms,
                    semantics_ms,
                    script: session::ScriptPhases {
                        motion_ms,
                        dispatch_ms,
                        timer_ms,
                        raf_ms,
                        callback_ms,
                        flush_ms,
                    },
                    trace,
                }),
            };
        }
    }
    fn wants_frame(&mut self) -> bool {
        // SAFETY: `inner` is owned by this session. The call reads and clears one flag.
        let inner = unsafe { &*self.inner };
        if self.suppress_demand.replace(false) {
            inner.settings_frame.set(false);
            let _ = unsafe { qjs_take_wants_frame(inner.host) };
            return false;
        }
        let visual = {
            let tree = inner.tree.borrow();
            tree.needs_layout() || tree.needs_paint() || tree.fling_active()
        };
        let jobs = unsafe { qjs_jobs_pending(inner.host) != 0 };
        let soon = unsafe { qjs_needs_frame(inner.host) != 0 };
        let explicit =
            inner.settings_frame.take() || unsafe { qjs_take_wants_frame(inner.host) != 0 };
        let driver = inner.driver_frame.get();
        explicit || visual || jobs || soon || driver
    }

    fn lifecycle(&mut self, event: session::Lifecycle) {
        let inner = unsafe { &*self.inner };
        let paused = matches!(
            event,
            session::Lifecycle::Hidden | session::Lifecycle::Paused
        );
        inner.paused.set(paused);
        let name = match event {
            session::Lifecycle::Resumed => "resumed",
            session::Lifecycle::Inactive => "inactive",
            session::Lifecycle::Hidden => "hidden",
            session::Lifecycle::Paused => "paused",
            session::Lifecycle::Detached => "detached",
        };
        let state = host::app_state(name);
        session::shared_write(&inner.config.app_state, state.to_owned());
        emit_module(inner, "AppState", "change", 0, state);
    }

    fn window_focus(&mut self, focused: bool) {
        let inner = unsafe { &*self.inner };
        emit_module(
            inner,
            "AppState",
            "focus",
            0,
            if focused { "true" } else { "false" },
        );
    }

    fn io_event(&mut self, event: session::IoEvent) {
        let inner = unsafe { &*self.inner };
        match event {
            session::IoEvent::Http { token, result } => settle_http(inner, token, &result),
            session::IoEvent::HttpHead {
                token,
                status,
                headers,
            } => settle_http_head(inner, token, status, &headers),
            session::IoEvent::HttpChunk {
                token,
                bytes,
                done,
                error,
            } => settle_http_chunk(inner, token, &bytes, done, error.as_ref()),
            session::IoEvent::Plugin { token, result } => settle_plugin(inner, token, &result),
            session::IoEvent::Socket {
                id,
                event,
                text,
                data,
            } => deliver_socket(inner, id, &event, text, &data),
        }
    }

    fn next_wake_ms(&mut self) -> Option<f64> {
        let inner = unsafe { &*self.inner };
        if inner.paused.get() {
            return None;
        }
        let deadline = unsafe { qjs_next_deadline(inner.host) };
        if deadline < 0 {
            None
        } else {
            Some(deadline as f64)
        }
    }

    fn set_launch_url(&mut self, url: &str) {
        let inner = unsafe { &*self.inner };
        session::shared_write(&inner.config.launch_url, Some(url.to_owned()));
        emit_module(inner, "Linking", "url", 1, url);
    }
}

pub(super) fn drain_jobs(host: *mut c_void) -> Result<(), String> {
    let mut error = [0u8; 512];
    // SAFETY: `host` is the interpreter for this session. `error` is writable
    // for the length we pass. Drain runs on the render thread, with the engine.
    let failed = unsafe { qjs_drain_jobs(host, error.as_mut_ptr().cast(), error.len()) };
    if failed == 0 {
        Ok(())
    } else {
        Err(c_message(&error))
    }
}

pub(super) fn c_message(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

pub(super) fn publish(out: *mut u64, id: u64) -> i32 {
    if out.is_null() {
        return -1;
    }
    // SAFETY: glue passes a writable `uint64_t` that lives for this call.
    unsafe { *out = id };
    0
}

pub(super) fn created(result: Result<NodeId, uicore::Error>, out: *mut u64) -> i32 {
    match result {
        Ok(id) => publish(out, id.to_raw()),
        Err(_) => -1,
    }
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
pub(super) unsafe fn inner<'a>(session: *mut c_void) -> &'a Inner {
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
