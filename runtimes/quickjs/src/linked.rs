//! Live QuickJS session. Tap handlers are JS values owned by the interpreter.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CString, c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use protocol::{Command, TextAlign};
use session::{
    HostPlace, HostWait, HttpError, HttpRequest, HttpResponse, InputEvent, IoJob, Key, Metrics,
    PluginResult, PluginValue, Session, host_method, host_module_linked, host_module_methods,
    module_not_linked,
};
use ui_core::{
    Affinity, Alignment, AxisSize, DragClaim, Edges, FocusDirection, GridLine, Inset, Justify,
    Modifiers, NodeId, Rect, RowCrossAlign, Shaper, TrackSize, Tree,
};

use crate::Error;

mod driver;
mod host;

#[repr(C)]
struct QjsFns {
    session: *mut c_void,
    column: unsafe extern "C" fn(*mut c_void, f32, *mut u64) -> i32,
    row: unsafe extern "C" fn(*mut c_void, f32, *mut u64) -> i32,
    stack: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32,
    padding: unsafe extern "C" fn(*mut c_void, f32, *mut u64) -> i32,
    pointer_region: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32,
    decorated: unsafe extern "C" fn(*mut c_void, *const u8, f32, *mut u64) -> i32,
    text: unsafe extern "C" fn(*mut c_void, *const c_char, usize, f32, *const u8, *mut u64) -> i32,
    append: unsafe extern "C" fn(*mut c_void, u64, u64) -> i32,
    insert: unsafe extern "C" fn(*mut c_void, u64, u64, u64) -> i32,
    remove: unsafe extern "C" fn(*mut c_void, u64, u64) -> i32,
    detach: unsafe extern "C" fn(*mut c_void, u64, u64) -> i32,
    discard: unsafe extern "C" fn(*mut c_void, u64, *mut *mut u64, *mut usize) -> i32,
    free_ids: unsafe extern "C" fn(*mut u64, usize),
    parent: unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> i32,
    first_child: unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> i32,
    next_sibling: unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> i32,
    is_text: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    set_text: unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize) -> i32,
    set_font_size: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_color: unsafe extern "C" fn(*mut c_void, u64, *const u8) -> i32,
    set_gap: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_padding: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    clip: unsafe extern "C" fn(*mut c_void, f32, *mut u64) -> i32,
    opacity: unsafe extern "C" fn(*mut c_void, u8, *mut u64) -> i32,
    set_grow: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    image: unsafe extern "C" fn(*mut c_void, *const c_char, usize, f32, f32, *mut u64) -> i32,
    set_radius: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_alpha: unsafe extern "C" fn(*mut c_void, u64, u8) -> i32,
    set_align: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_size: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    set_image: unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize, f32, f32) -> i32,
    set_max_lines: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_ellipsis: unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize) -> i32,
    set_text_align: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_font_weight: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_text_direction: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    set_line_height: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_family: unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize) -> i32,
    scroll: unsafe extern "C" fn(*mut c_void, i32, *mut u64) -> i32,
    set_scroll_offset: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    scroll_offset: unsafe extern "C" fn(*mut c_void, u64, *mut f32, *mut f32) -> i32,
    rect: unsafe extern "C" fn(*mut c_void, u64, *mut f32, *mut f32, *mut f32, *mut f32) -> i32,
    transform: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32,
    set_transform: unsafe extern "C" fn(*mut c_void, u64, f32, f32, f32, f32) -> i32,
    set_justify: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_wrap: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    set_fill: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    log: unsafe extern "C" fn(*mut c_void, *const c_char, usize, i32),
    sized: unsafe extern "C" fn(*mut c_void, f32, f32, *mut u64) -> i32,
    set_focusable: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    focus: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    blur: unsafe extern "C" fn(*mut c_void) -> i32,
    set_ime_enabled: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    set_ime_text: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    set_ime_state: unsafe extern "C" fn(
        *mut c_void,
        i32,
        *const c_char,
        usize,
        u32,
        u32,
        i32,
        i32,
        f32,
        f32,
        f32,
        f32,
    ) -> i32,
    set_semantics: unsafe extern "C" fn(
        *mut c_void,
        u64,
        i32,
        *const c_char,
        usize,
        *const c_char,
        usize,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        *const c_char,
        usize,
        i32,
        i32,
        i32,
    ) -> i32,
    text_scale: unsafe extern "C" fn(*mut c_void) -> f64,
    reduce_motion: unsafe extern "C" fn(*mut c_void) -> i32,
    measure_text: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        usize,
        f32,
        f32,
        *const u8,
        *mut f32,
        *mut f32,
    ) -> i32,
    grid: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        usize,
        *const c_char,
        usize,
        f32,
        *mut u64,
    ) -> i32,
    set_padding_edges: unsafe extern "C" fn(*mut c_void, u64, f32, f32, f32, f32) -> i32,
    set_margin: unsafe extern "C" fn(*mut c_void, u64, f32, f32, f32, f32) -> i32,
    set_min_size: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    set_max_size: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    set_shrink: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_basis: unsafe extern "C" fn(*mut c_void, u64, f32) -> i32,
    set_align_self: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    set_drag_claim: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    caret_at: unsafe extern "C" fn(
        *mut c_void,
        u64,
        f32,
        f32,
        *mut u32,
        *mut f32,
        *mut f32,
        *mut f32,
        *mut f32,
    ) -> i32,
    word_range: unsafe extern "C" fn(*mut c_void, u64, u32, *mut u32, *mut u32) -> i32,
    focus_step: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    push_focus_scope: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    pop_focus_scope: unsafe extern "C" fn(*mut c_void) -> i32,
    pointer_position: unsafe extern "C" fn(*mut c_void, *mut f32, *mut f32) -> i32,
    text_edit: unsafe extern "C" fn(
        *mut c_void,
        u64,
        i32,
        *const c_char,
        usize,
        *mut u32,
        *mut u32,
    ) -> i32,
    text_edit_bytes: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const u8,
    set_text_scale: unsafe extern "C" fn(*mut c_void, f32) -> i32,
    set_soft_wrap: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    set_overflow: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    caret_at_unit:
        unsafe extern "C" fn(*mut c_void, u64, u32, *mut f32, *mut f32, *mut f32, *mut f32) -> i32,
    selection_rects: unsafe extern "C" fn(*mut c_void, u64, u32, u32) -> i32,
    selection_rect_bytes: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const u8,
    set_ime_config: unsafe extern "C" fn(*mut c_void, i32, i32) -> i32,
    http: unsafe extern "C" fn(
        *mut c_void,
        u64,
        *const c_char,
        usize,
        *const c_char,
        usize,
        *const c_char,
        usize,
        *const u8,
        usize,
    ) -> i32,
    abort_token: unsafe extern "C" fn(*mut c_void, u64),
    module_has: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    module_prepare:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const c_char, usize) -> i32,
    socket_open: unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize),
    socket_send: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize, i32),
    socket_close: unsafe extern "C" fn(*mut c_void, u64),
    focused: unsafe extern "C" fn(*mut c_void) -> u64,
    is_focusable: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    grapheme_index: unsafe extern "C" fn(*const c_char, usize, u32) -> u32,
    grapheme_count: unsafe extern "C" fn(*const c_char, usize) -> u32,
    grapheme_unit: unsafe extern "C" fn(*const c_char, usize, u32) -> u32,
    set_reverse: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    set_axis_size: unsafe extern "C" fn(*mut c_void, u64, i32, f32, i32, f32) -> i32,
    set_aspect_ratio: unsafe extern "C" fn(*mut c_void, u64, i32, f32) -> i32,
    set_position: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    set_inset:
        unsafe extern "C" fn(*mut c_void, u64, i32, f32, i32, f32, i32, f32, i32, f32) -> i32,
    set_grid_placement:
        unsafe extern "C" fn(*mut c_void, u64, i32, i32, i32, i32, i32, i32, i32, i32) -> i32,
    set_scale_xy: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    register_font: unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const u8, usize) -> i32,
    set_image_mode: unsafe extern "C" fn(*mut c_void, u64, i32, i32) -> i32,
    reload: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    push_mutation: unsafe extern "C" fn(
        *mut c_void,
        u32,
        u64,
        u64,
        u64,
        u32,
        f64,
        f64,
        *const u8,
        *const c_char,
        usize,
    ) -> i32,
    layer_stats: unsafe extern "C" fn(*mut c_void, *mut u64, *mut u32, *mut u32) -> i32,
    scroll_into_view: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    animate:
        unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize, f64, f64, f64, f64) -> i32,
    fling_target: unsafe extern "C" fn(*mut c_void, u64, *mut f32, *mut f32) -> i32,
    set_recognizers: unsafe extern "C" fn(*mut c_void, u64, i32, i32, i32, f64) -> i32,
    set_stroke: unsafe extern "C" fn(*mut c_void, u64, f32, *const u8) -> i32,
    module_push_null: unsafe extern "C" fn(*mut c_void),
    module_push_bool: unsafe extern "C" fn(*mut c_void, i32),
    module_push_f64: unsafe extern "C" fn(*mut c_void, f64),
    module_push_str: unsafe extern "C" fn(*mut c_void, *const c_char, usize),
    module_push_bytes: unsafe extern "C" fn(*mut c_void, *const u8, usize),
    module_begin_list: unsafe extern "C" fn(*mut c_void),
    module_end_list: unsafe extern "C" fn(*mut c_void),
    module_begin_map: unsafe extern "C" fn(*mut c_void),
    module_map_key: unsafe extern "C" fn(*mut c_void, *const c_char, usize),
    module_end_map: unsafe extern "C" fn(*mut c_void),
    module_set_token: unsafe extern "C" fn(*mut c_void, u64),
    module_finish: unsafe extern "C" fn(*mut c_void) -> i32,
    module_count: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    module_method_at:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, i32, *mut usize) -> *const c_char,
    align: unsafe extern "C" fn(*mut c_void, u32, *mut u64) -> i32,
    set_cross: unsafe extern "C" fn(*mut c_void, u64, u32) -> i32,
    viewport: unsafe extern "C" fn(*mut c_void, *mut f32, *mut f32) -> i32,
    set_scroll_physics: unsafe extern "C" fn(
        *mut c_void,
        u64,
        f32,
        i32,
        f32,
        f32,
        f32,
        f32,
        i32,
        f32,
        i32,
        i32,
    ) -> i32,
    set_scroll_chain: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
    overscroll: unsafe extern "C" fn(*mut c_void, u64, *mut f32, *mut f32) -> i32,
    arm_fling: unsafe extern "C" fn(*mut c_void, u64, f32, f32) -> i32,
    stop_scroll: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    scroll_to: unsafe extern "C" fn(*mut c_void, u64, f32, f32, f32, f32) -> i32,
    list_create: unsafe extern "C" fn(*mut c_void, i32, f32, *mut u32, *mut u32) -> i32,
    list_release: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    list_bind: unsafe extern "C" fn(*mut c_void, u32, u32, *const u64, i32) -> i32,
    list_configure: unsafe extern "C" fn(*mut c_void, u32, u32, f32, i32, i32) -> i32,
    list_begin: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    list_item: unsafe extern "C" fn(
        *mut c_void,
        u32,
        u32,
        *const c_char,
        usize,
        *const c_char,
        usize,
        i32,
    ) -> i32,
    list_end: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    list_note_row: unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, usize, u64) -> i32,
    scroll_create: unsafe extern "C" fn(*mut c_void, *mut u32, *mut u32) -> i32,
    scroll_release: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    scroll_bind: unsafe extern "C" fn(*mut c_void, u32, u32, u64, u64) -> i32,
    scroll_configure:
        unsafe extern "C" fn(*mut c_void, u32, u32, f32, i32, f32, i32, f32, u32, u32, i32) -> i32,
    scroll_drag_by: unsafe extern "C" fn(
        *mut c_void,
        u32,
        u32,
        f32,
        f32,
        *mut f32,
        *mut f32,
        *mut f32,
        *mut f32,
    ) -> i32,
    scroll_finish: unsafe extern "C" fn(*mut c_void, u32, u32, i32) -> i32,
    scroll_drag_end: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    scroll_wheel: unsafe extern "C" fn(*mut c_void, u32, u32, f32, f32) -> i32,
    scroll_query: unsafe extern "C" fn(*mut c_void, u32, u32, i32, *mut f32, *mut f32) -> i32,
    scroll_consume: unsafe extern "C" fn(*mut c_void, u32, u32, f32, f32) -> i32,
    set_scroll_axis: unsafe extern "C" fn(*mut c_void, u64, i32) -> i32,
}

unsafe extern "C" {
    fn qjs_open(fns: *const QjsFns) -> *mut c_void;
    fn qjs_eval(
        host: *mut c_void,
        src: *const c_char,
        len: usize,
        name: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_set_app_dir(host: *mut c_void, dir: *const c_char);
    fn qjs_dispatch(
        host: *mut c_void,
        id: u64,
        kind: i32,
        a: f64,
        b: f64,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_dispatch_input(
        host: *mut c_void,
        id: u64,
        kind: i32,
        utf8: *const c_char,
        len: usize,
        with_span: i32,
        sel_start: i32,
        sel_end: i32,
        compose_start: i32,
        compose_end: i32,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_dispatch_text(
        host: *mut c_void,
        id: u64,
        utf8: *const c_char,
        len: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_frame(
        host: *mut c_void,
        target_ms: f64,
        interval_ms: f64,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_layout(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_driver(
        host: *mut c_void,
        kind: i32,
        index: u32,
        generation: u32,
        tag: i32,
        i0: i32,
        i1: i32,
        f0: f32,
        f1: f32,
        f2: f32,
        f3: f32,
        f4: f32,
        f5: f32,
        s0: *const c_char,
        n0: usize,
        s1: *const c_char,
        n1: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_scroll_state(
        host: *mut c_void,
        id: u64,
        phase: i32,
        vx: f32,
        vy: f32,
        ox: f32,
        oy: f32,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_forget_ids(host: *mut c_void, ids: *const u64, len: usize);
    fn qjs_take_wants_frame(host: *mut c_void) -> i32;
    fn qjs_drain_jobs(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_jobs_pending(host: *mut c_void) -> i32;
    fn qjs_tap_count(host: *mut c_void) -> i32;
    fn qjs_close(host: *mut c_void);
    fn qjs_dispatch_back(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_try_shortcut(
        host: *mut c_void,
        spec: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_apply_settings(
        host: *mut c_void,
        locale: *const c_char,
        locale_len: usize,
        text_scale: f64,
        dark: i32,
        reduce_motion: i32,
        bold_text: i32,
        high_contrast: i32,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_set_clock(host: *mut c_void, mono_ms: i64, paused: i32);
    fn qjs_fire_timers(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_fire_raf(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_needs_frame(host: *mut c_void) -> i32;
    fn qjs_next_deadline(host: *mut c_void) -> i64;
    fn qjs_settle_http(
        host: *mut c_void,
        token: u64,
        status: i32,
        headers: *const c_char,
        headers_len: usize,
        body: *const u8,
        body_len: usize,
        error: *const c_char,
    ) -> i32;
    fn qjs_value_null(host: *mut c_void);
    fn qjs_value_bool(host: *mut c_void, value: i32);
    fn qjs_value_f64(host: *mut c_void, value: f64);
    fn qjs_value_str(host: *mut c_void, text: *const c_char, len: usize);
    fn qjs_value_bytes(host: *mut c_void, bytes: *const u8, len: usize);
    fn qjs_value_list(host: *mut c_void, count: u32);
    fn qjs_value_map(host: *mut c_void, count: u32);
    fn qjs_settle_top(host: *mut c_void, token: u64, ok: i32) -> i32;
    fn qjs_motion_end(
        host: *mut c_void,
        node: u64,
        prop: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_module_event(
        host: *mut c_void,
        module: *const c_char,
        event: *const c_char,
        shape: i32,
        text: *const c_char,
        len: usize,
    ) -> i32;
    fn qjs_socket_event(
        host: *mut c_void,
        id: u64,
        event: *const c_char,
        is_text: i32,
        data: *const u8,
        len: usize,
    ) -> i32;
}

struct FrameMarks {
    pointers: Cell<u32>,
    keys: Cell<u32>,
    edits: Cell<u32>,
    ime: Cell<u32>,
    scrolls: Cell<u32>,
    focus: Cell<u32>,
    blur: Cell<u32>,
    gestures: Cell<u32>,
}

impl FrameMarks {
    fn reset(&self) {
        self.pointers.set(0);
        self.keys.set(0);
        self.edits.set(0);
        self.ime.set(0);
        self.scrolls.set(0);
        self.focus.set(0);
        self.blur.set(0);
        self.gestures.set(0);
    }
}

impl Default for FrameMarks {
    fn default() -> Self {
        Self {
            pointers: Cell::new(0),
            keys: Cell::new(0),
            edits: Cell::new(0),
            ime: Cell::new(0),
            scrolls: Cell::new(0),
            focus: Cell::new(0),
            blur: Cell::new(0),
            gestures: Cell::new(0),
        }
    }
}

fn bump(cell: &Cell<u32>) {
    cell.set(cell.get().saturating_add(1));
}

fn note_gesture(marks: &FrameMarks, gesture: &ui_core::Gesture) {
    bump(&marks.gestures);
    match gesture {
        ui_core::Gesture::Scroll(_, _, _) | ui_core::Gesture::ScrollPhaseChanged { .. } => {
            bump(&marks.scrolls);
        }
        ui_core::Gesture::Focus(_) => bump(&marks.focus),
        ui_core::Gesture::Blur(_) => bump(&marks.blur),
        _ => {}
    }
}

fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

struct Inner {
    tree: RefCell<Tree>,
    host: *mut c_void,
    log: Box<dyn Fn(u8, &str)>,
    ime: RefCell<ImeSlot>,
    text_scale: Cell<f32>,
    reduce_motion: Cell<bool>,
    platform: RefCell<session::PlatformSettings>,
    settings_frame: Cell<bool>,
    /// Bytes from the latest `text_edit`. Stable until the next edit.
    edit_bytes: RefCell<Vec<u8>>,
    /// Copy or Cut text waiting for the platform clipboard.
    clipboard: RefCell<Option<String>>,
    /// Selection rectangles from the latest query, as packed `x,y,w,h` floats.
    selection_rects: RefCell<Vec<f32>>,
    io: RefCell<Option<Box<dyn Fn(IoJob) + Send>>>,
    http: RefCell<Option<Arc<dyn session::HttpClient>>>,
    launch_url: RefCell<Option<String>>,
    app_state: RefCell<String>,
    paused: Cell<bool>,
    cancels: RefCell<HashMap<u64, Arc<AtomicBool>>>,
    module_clipboard: RefCell<String>,
    pending: RefCell<Pending>,
    fonts: RefCell<FontBook>,
    /// Sources passed to `reload` since the last [`Session::take_reloads`].
    reloads: RefCell<Vec<String>>,
    mutations: RefCell<Vec<PendingMutation>>,
    prepared: RefCell<Option<PreparedCall>>,
    memory_files: RefCell<HashMap<String, Vec<u8>>>,
    memory_store: RefCell<HashMap<String, String>>,
    marks: FrameMarks,
    drivers: RefCell<ui_core::DriverSet>,
    driver_frame: Cell<bool>,
}

struct ArgFrame {
    map: bool,
    items: Vec<PluginValue>,
    keys: Vec<String>,
    pending_key: Option<String>,
}

struct ArgBuild {
    stack: Vec<PluginValue>,
    frames: Vec<ArgFrame>,
}

impl ArgBuild {
    fn new() -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
        }
    }

    fn push(&mut self, value: PluginValue) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.map {
                let key = frame.pending_key.take().unwrap_or_default();
                frame.keys.push(key);
            }
            frame.items.push(value);
            return;
        }
        self.stack.push(value);
    }

    fn begin(&mut self, map: bool) {
        self.frames.push(ArgFrame {
            map,
            items: Vec::new(),
            keys: Vec::new(),
            pending_key: None,
        });
    }

    fn end(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let value = if frame.map {
            PluginValue::Map(frame.keys.into_iter().zip(frame.items).collect())
        } else {
            PluginValue::List(frame.items)
        };
        self.push(value);
    }
}

struct PreparedCall {
    module: String,
    method: String,
    place: HostPlace,
    wait: HostWait,
    token: u64,
    args: ArgBuild,
}

struct Pending {
    http: Vec<(u64, Result<HttpResponse, HttpError>)>,
    plugin: Vec<(u64, PluginResult)>,
    socket: Vec<(u64, String, bool, Vec<u8>)>,
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
        Self::open_with(script, name, shaper, log, None)
    }

    pub fn open_with(
        script: &str,
        name: &str,
        shaper: impl Shaper + 'static,
        log: Box<dyn Fn(u8, &str)>,
        http: Option<Arc<dyn session::HttpClient>>,
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
            launch_url: RefCell::new(None),
            app_state: RefCell::new("unknown".to_owned()),
            paused: Cell::new(false),
            cancels: RefCell::new(HashMap::new()),
            module_clipboard: RefCell::new(String::new()),
            pending: RefCell::new(Pending::new()),
            fonts: RefCell::new(FontBook::new()),
            reloads: RefCell::new(Vec::new()),
            mutations: RefCell::new(Vec::new()),
            prepared: RefCell::new(None),
            memory_files: RefCell::new(HashMap::new()),
            memory_store: RefCell::new(HashMap::new()),
            marks: FrameMarks::default(),
            drivers: RefCell::new(ui_core::DriverSet::new()),
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
            set_aspect_ratio,
            set_position,
            set_inset,
            set_grid_placement,
            set_scale_xy,
            register_font,
            set_image_mode,
            reload,
            push_mutation,
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
            module_count,
            module_method_at,
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

    pub fn semantics(&mut self) -> Option<ui_core::SemanticsUpdate> {
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
        let _ = inner.tree.borrow_mut().set_viewport(
            left,
            top,
            (metrics.width - left - right).max(0.0),
            (metrics.height - top - bottom).max(0.0),
        );
    }

    fn pointer(&mut self, event: ui_core::PointerEvent) {
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
                    ui_core::Edit::DeleteRange { .. } | ui_core::Edit::Backspace => {
                        (8, Some("Backspace".to_string()))
                    }
                    ui_core::Edit::ForwardDelete => (8, Some("Delete".to_string())),
                    ui_core::Edit::Insert(text) => (9, Some(text)),
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

    fn semantics_action(&mut self, action: ui_core::SemanticsAction) {
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
        *inner.app_state.borrow_mut() = state.to_owned();
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
        *inner.launch_url.borrow_mut() = Some(url.to_owned());
        emit_module(inner, "Linking", "url", 1, url);
    }
}

fn drain_jobs(host: *mut c_void) -> Result<(), String> {
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

fn c_message(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn publish(out: *mut u64, id: u64) -> i32 {
    if out.is_null() {
        return -1;
    }
    // SAFETY: glue passes a writable `uint64_t` that lives for this call.
    unsafe { *out = id };
    0
}

fn created(result: Result<NodeId, ui_core::Error>, out: *mut u64) -> i32 {
    match result {
        Ok(id) => publish(out, id.to_raw()),
        Err(_) => -1,
    }
}

fn status(result: Result<(), ui_core::Error>) -> i32 {
    if result.is_ok() { 0 } else { -1 }
}

fn queried(result: Result<Option<NodeId>, ui_core::Error>, out: *mut u64) -> i32 {
    match result {
        Ok(Some(id)) => publish(out, id.to_raw()),
        Ok(None) => publish(out, 0),
        Err(_) => -1,
    }
}

/// # Safety
/// `session` is the `Inner` installed in [`JsSession::open`] and is still alive.
unsafe fn inner<'a>(session: *mut c_void) -> &'a Inner {
    unsafe { &*session.cast::<Inner>() }
}

/// # Safety
/// `ptr` and `len` describe a byte slice that lives for this call.
unsafe fn utf8_of<'a>(ptr: *const c_char, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: the caller guarantees `ptr` points at `len` initialized bytes.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes).ok()
}

/// # Safety
/// `color` points at 4 initialized bytes that live for this call.
unsafe fn rgba(color: *const u8) -> Option<[u8; 4]> {
    if color.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a 4-byte color.
    let bytes = unsafe { std::slice::from_raw_parts(color, 4) };
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

unsafe extern "C" fn column(session: *mut c_void, gap: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().column(gap), out)
}

unsafe extern "C" fn row(session: *mut c_void, gap: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().row(gap), out)
}

unsafe extern "C" fn stack(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().stack(), out)
}

fn alignment_of(align: u32) -> Option<Alignment> {
    match align {
        0 => Some(Alignment::Start),
        1 => Some(Alignment::Center),
        2 => Some(Alignment::End),
        _ => None,
    }
}

unsafe extern "C" fn align(session: *mut c_void, align: u32, out: *mut u64) -> i32 {
    let Some(alignment) = alignment_of(align) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().align(alignment), out)
}

unsafe extern "C" fn padding(session: *mut c_void, edges: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().padding(edges), out)
}

unsafe extern "C" fn pointer_region(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().pointer_region(), out)
}

unsafe extern "C" fn decorated(
    session: *mut c_void,
    color: *const u8,
    radius: f32,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(color) = (unsafe { rgba(color) }) else {
        return -1;
    };
    created(inner.tree.borrow_mut().decorated(color, radius), out)
}

unsafe extern "C" fn text(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    font_size: f32,
    color: *const u8,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let (Some(utf8), Some(color)) = (unsafe { utf8_of(utf8, len) }, unsafe { rgba(color) }) else {
        return -1;
    };
    created(inner.tree.borrow_mut().text(utf8, font_size, color), out)
}

unsafe extern "C" fn append(session: *mut c_void, parent: u64, child: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .add_child(NodeId::from_raw(parent), NodeId::from_raw(child)),
    )
}

fn logged_status(inner: &Inner, result: Result<(), ui_core::Error>, what: &str) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            (inner.log)(3, &format!("{what}: {error}"));
            -1
        }
    }
}

unsafe extern "C" fn insert(session: *mut c_void, parent: u64, child: u64, before: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let before = (before != 0).then(|| NodeId::from_raw(before));
    logged_status(
        inner,
        inner
            .tree
            .borrow_mut()
            .insert(NodeId::from_raw(parent), NodeId::from_raw(child), before),
        "insert failed",
    )
}

unsafe extern "C" fn detach(session: *mut c_void, parent: u64, child: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = match inner
        .tree
        .borrow_mut()
        .detach(NodeId::from_raw(parent), NodeId::from_raw(child))
    {
        Ok(gestures) => gestures,
        Err(_) => return -1,
    };
    if let Err(message) = dispatch_gestures(inner, gestures) {
        (inner.log)(3, &message);
    }
    0
}

unsafe extern "C" fn remove(session: *mut c_void, parent: u64, child: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = match inner
        .tree
        .borrow_mut()
        .remove(NodeId::from_raw(parent), NodeId::from_raw(child))
    {
        Ok(gestures) => gestures,
        Err(_) => return -1,
    };
    // Blur runs before the script continues. The slot stays until
    // `sweep_detached`, so this turn can insert the same id.
    if let Err(message) = dispatch_gestures(inner, gestures) {
        (inner.log)(3, &message);
    }
    0
}

unsafe extern "C" fn discard(
    session: *mut c_void,
    id: u64,
    out_ids: *mut *mut u64,
    out_len: *mut usize,
) -> i32 {
    if out_ids.is_null() || out_len.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let retired = match inner.tree.borrow_mut().discard(NodeId::from_raw(id)) {
        Ok(retired) => retired,
        Err(_) => return -1,
    };
    if let Err(message) = dispatch_gestures(inner, retired.gestures) {
        (inner.log)(3, &message);
    }
    let retired = retired.ids;
    if retired.is_empty() {
        unsafe {
            *out_ids = ptr::null_mut();
            *out_len = 0;
        }
        return 0;
    }
    let raw: Box<[u64]> = retired.into_iter().map(NodeId::to_raw).collect();
    let len = raw.len();
    let ptr = Box::into_raw(raw).cast::<u64>();
    unsafe {
        *out_ids = ptr;
        *out_len = len;
    }
    0
}

unsafe extern "C" fn free_ids(ids: *mut u64, len: usize) {
    if ids.is_null() {
        return;
    }
    // SAFETY: `ids` and `len` came from `discard`, which leaked a `Box<[u64]>`.
    let slice = ptr::slice_from_raw_parts_mut(ids, len);
    drop(unsafe { Box::from_raw(slice) });
}

unsafe extern "C" fn parent(session: *mut c_void, node: u64, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    queried(inner.tree.borrow().parent(NodeId::from_raw(node)), out)
}

unsafe extern "C" fn first_child(session: *mut c_void, node: u64, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    queried(inner.tree.borrow().first_child(NodeId::from_raw(node)), out)
}

unsafe extern "C" fn next_sibling(session: *mut c_void, node: u64, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    queried(
        inner.tree.borrow().next_sibling(NodeId::from_raw(node)),
        out,
    )
}

unsafe extern "C" fn is_text(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().is_text(NodeId::from_raw(node)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn set_text(
    session: *mut c_void,
    node: u64,
    utf8: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(utf8) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_text(NodeId::from_raw(node), utf8),
    )
}

unsafe extern "C" fn set_font_size(session: *mut c_void, node: u64, font_size: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_font_size(NodeId::from_raw(node), font_size),
    )
}

unsafe extern "C" fn set_color(session: *mut c_void, node: u64, color: *const u8) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(color) = (unsafe { rgba(color) }) else {
        return -1;
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_color(NodeId::from_raw(node), color),
    )
}

unsafe extern "C" fn set_gap(session: *mut c_void, node: u64, gap: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_gap(NodeId::from_raw(node), gap))
}

unsafe extern "C" fn set_padding(session: *mut c_void, node: u64, edges: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_padding(NodeId::from_raw(node), edges),
    )
}

unsafe extern "C" fn clip(session: *mut c_void, radius: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().clip(radius), out)
}

unsafe extern "C" fn opacity(session: *mut c_void, alpha: u8, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().opacity(alpha), out)
}

unsafe extern "C" fn set_grow(session: *mut c_void, node: u64, grow: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_grow(NodeId::from_raw(node), grow),
    )
}

unsafe extern "C" fn image(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    width: f32,
    height: f32,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(source) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    created(inner.tree.borrow_mut().image(source, width, height), out)
}

unsafe extern "C" fn set_radius(session: *mut c_void, node: u64, radius: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_radius(NodeId::from_raw(node), radius),
    )
}

unsafe extern "C" fn set_alpha(session: *mut c_void, node: u64, alpha: u8) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_alpha(NodeId::from_raw(node), alpha),
    )
}

unsafe extern "C" fn set_align(session: *mut c_void, node: u64, align: u32) -> i32 {
    let Some(alignment) = alignment_of(align) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_align(NodeId::from_raw(node), alignment),
    )
}

fn cross_of(align: u32) -> Option<RowCrossAlign> {
    match align {
        0 => Some(RowCrossAlign::Start),
        1 => Some(RowCrossAlign::Center),
        2 => Some(RowCrossAlign::End),
        3 => Some(RowCrossAlign::Stretch),
        4 => Some(RowCrossAlign::Baseline),
        _ => None,
    }
}

unsafe extern "C" fn set_cross(session: *mut c_void, node: u64, align: u32) -> i32 {
    let Some(alignment) = cross_of(align) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_row_cross(NodeId::from_raw(node), alignment),
    )
}

unsafe extern "C" fn viewport(session: *mut c_void, width: *mut f32, height: *mut f32) -> i32 {
    if width.is_null() || height.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let (w, h) = inner.tree.borrow().viewport_size();
    unsafe {
        *width = w;
        *height = h;
    }
    0
}

unsafe extern "C" fn set_size(session: *mut c_void, node: u64, width: f32, height: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_size(NodeId::from_raw(node), width, height),
    )
}

unsafe extern "C" fn set_image(
    session: *mut c_void,
    node: u64,
    utf8: *const c_char,
    len: usize,
    width: f32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(source) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_image(NodeId::from_raw(node), source, width, height),
    )
}

unsafe extern "C" fn set_max_lines(session: *mut c_void, node: u64, lines: u32) -> i32 {
    let inner = unsafe { inner(session) };
    let lines = if lines == 0 { None } else { Some(lines) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_max_lines(NodeId::from_raw(node), lines),
    )
}

unsafe extern "C" fn set_ellipsis(
    session: *mut c_void,
    node: u64,
    utf8: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    let text = if text.is_empty() { None } else { Some(text) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_ellipsis(NodeId::from_raw(node), text),
    )
}

unsafe extern "C" fn set_text_align(session: *mut c_void, node: u64, align: u32) -> i32 {
    let align = match align {
        0 => TextAlign::Left,
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => return -1,
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_text_align(NodeId::from_raw(node), align),
    )
}

unsafe extern "C" fn set_font_weight(session: *mut c_void, node: u64, weight: u32) -> i32 {
    let Ok(weight) = u16::try_from(weight) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_font_weight(NodeId::from_raw(node), weight),
    )
}

unsafe extern "C" fn set_text_direction(session: *mut c_void, node: u64, rtl: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_text_direction(NodeId::from_raw(node), rtl != 0),
    )
}

unsafe extern "C" fn set_line_height(session: *mut c_void, node: u64, height: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_line_height(NodeId::from_raw(node), height),
    )
}

unsafe extern "C" fn set_family(
    session: *mut c_void,
    node: u64,
    utf8: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    let name = if name.is_empty() { None } else { Some(name) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_family(NodeId::from_raw(node), name),
    )
}

unsafe extern "C" fn scroll(session: *mut c_void, axis: i32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(
        inner
            .tree
            .borrow_mut()
            .scroll_with(ui_core::ScrollAxis::from_code(axis)),
        out,
    )
}

unsafe extern "C" fn set_scroll_axis(session: *mut c_void, node: u64, axis: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_axis(NodeId::from_raw(node), ui_core::ScrollAxis::from_code(axis)),
    )
}

unsafe extern "C" fn set_scroll_offset(session: *mut c_void, node: u64, dx: f32, dy: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_offset(NodeId::from_raw(node), dx, dy),
    )
}

unsafe extern "C" fn text_edit(
    session: *mut c_void,
    node: u64,
    op: i32,
    utf8: *const c_char,
    len: usize,
    sel_start: *mut u32,
    sel_end: *mut u32,
) -> i32 {
    let inner = unsafe { inner(session) };
    if sel_start.is_null() || sel_end.is_null() {
        return -1;
    }
    let extra = if utf8.is_null() {
        String::new()
    } else {
        let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
            return -1;
        };
        text.to_owned()
    };
    let edit = match op {
        0 => ui_core::Edit::Load(extra),
        1 => ui_core::Edit::Left,
        2 => ui_core::Edit::Right,
        3 => ui_core::Edit::Backspace,
        4 => ui_core::Edit::Insert(extra),
        5 => ui_core::Edit::Composing(if extra.is_empty() { None } else { Some(extra) }),
        6 => ui_core::Edit::Paste(extra),
        7 => ui_core::Edit::Undo,
        8 => ui_core::Edit::Redo,
        9 => ui_core::Edit::SelectAll,
        10 => ui_core::Edit::Copy,
        11 => ui_core::Edit::Cut,
        12 => ui_core::Edit::SetCaret(extra.parse::<u32>().unwrap_or(0)),
        13 => ui_core::Edit::ForwardDelete,
        14 => ui_core::Edit::Newline,
        15 => ui_core::Edit::WordLeft,
        16 => ui_core::Edit::WordRight,
        17 => ui_core::Edit::LineStart,
        18 => ui_core::Edit::LineEnd,
        19 => ui_core::Edit::ExtendLeft,
        20 => ui_core::Edit::ExtendRight,
        21 => ui_core::Edit::ExtendWordLeft,
        22 => ui_core::Edit::ExtendWordRight,
        23 => ui_core::Edit::ExtendLineStart,
        24 => ui_core::Edit::ExtendLineEnd,
        25 => ui_core::Edit::Up,
        26 => ui_core::Edit::Down,
        27 => ui_core::Edit::ExtendUp,
        28 => ui_core::Edit::ExtendDown,
        29 => ui_core::Edit::ExtendCaret(extra.parse::<u32>().unwrap_or(0)),
        30 => ui_core::Edit::DeleteWordBackward,
        31 => ui_core::Edit::DeleteWordForward,
        32 => ui_core::Edit::DeleteToLineStart,
        33 => ui_core::Edit::DeleteToLineEnd,
        34 => ui_core::Edit::DocumentStart,
        35 => ui_core::Edit::DocumentEnd,
        36 => ui_core::Edit::ExtendDocumentStart,
        37 => ui_core::Edit::ExtendDocumentEnd,
        38 => ui_core::Edit::Commit(extra),
        _ => return -1,
    };
    let Ok(value) = inner
        .tree
        .borrow_mut()
        .text_edit(NodeId::from_raw(node), edit)
    else {
        return -1;
    };
    let start = value.selection.base;
    let end = value.selection.extent;
    *inner.edit_bytes.borrow_mut() = value.text.into_bytes();
    if matches!(op, 10 | 11) {
        *inner.clipboard.borrow_mut() = inner
            .tree
            .borrow()
            .text_clipboard(NodeId::from_raw(node))
            .ok()
            .flatten();
    }
    unsafe {
        *sel_start = start;
        *sel_end = end;
    }
    0
}

unsafe extern "C" fn set_soft_wrap(session: *mut c_void, node: u64, wrap: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_soft_wrap(NodeId::from_raw(node), wrap != 0),
    )
}

unsafe extern "C" fn set_overflow(session: *mut c_void, node: u64, overflow: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let overflow = match overflow {
        1 => protocol::TextOverflow::Ellipsis,
        2 => protocol::TextOverflow::Visible,
        _ => protocol::TextOverflow::Clip,
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_overflow(NodeId::from_raw(node), overflow),
    )
}

unsafe extern "C" fn caret_at_unit(
    session: *mut c_void,
    node: u64,
    unit: u32,
    x: *mut f32,
    y: *mut f32,
    w: *mut f32,
    h: *mut f32,
) -> i32 {
    if x.is_null() || y.is_null() || w.is_null() || h.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    match inner
        .tree
        .borrow()
        .caret_at_unit(NodeId::from_raw(node), unit)
    {
        Ok(caret) => {
            unsafe {
                *x = caret.x;
                *y = caret.y;
                *w = caret.w;
                *h = caret.h;
            }
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn selection_rects(session: *mut c_void, node: u64, start: u32, end: u32) -> i32 {
    let inner = unsafe { inner(session) };
    let Ok(rects) = inner
        .tree
        .borrow()
        .selection_rects(NodeId::from_raw(node), start, end)
    else {
        return -1;
    };
    let mut packed = Vec::with_capacity(rects.len() * 4);
    for rect in rects {
        packed.extend([rect.x, rect.y, rect.w, rect.h]);
    }
    let count = i32::try_from(packed.len() / 4).unwrap_or(0);
    *inner.selection_rects.borrow_mut() = packed;
    count
}

unsafe extern "C" fn selection_rect_bytes(session: *mut c_void, out_len: *mut usize) -> *const u8 {
    let inner = unsafe { inner(session) };
    let bytes = inner.selection_rects.borrow();
    if !out_len.is_null() {
        unsafe { *out_len = bytes.len() * 4 };
    }
    if bytes.is_empty() {
        b"\0".as_ptr()
    } else {
        bytes.as_ptr().cast()
    }
}

unsafe extern "C" fn set_ime_config(session: *mut c_void, keyboard: i32, action: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let mut ime = inner.ime.borrow_mut();
    ime.state.keyboard = match keyboard {
        1 => session::KeyboardType::Multiline,
        2 => session::KeyboardType::Number,
        3 => session::KeyboardType::Phone,
        4 => session::KeyboardType::Email,
        5 => session::KeyboardType::Url,
        6 => session::KeyboardType::Name,
        7 => session::KeyboardType::Password,
        _ => session::KeyboardType::Text,
    };
    ime.state.action = match action {
        1 => session::TextInputAction::Done,
        2 => session::TextInputAction::Next,
        3 => session::TextInputAction::Search,
        4 => session::TextInputAction::Go,
        5 => session::TextInputAction::Send,
        6 => session::TextInputAction::Previous,
        7 => session::TextInputAction::Newline,
        _ => session::TextInputAction::None,
    };
    ime.dirty = true;
    0
}

unsafe extern "C" fn set_text_scale(session: *mut c_void, scale: f32) -> i32 {
    let inner = unsafe { inner(session) };
    inner.text_scale.set(scale);
    status(inner.tree.borrow_mut().set_text_scale(scale))
}

unsafe extern "C" fn text_edit_bytes(session: *mut c_void, out_len: *mut usize) -> *const u8 {
    let inner = unsafe { inner(session) };
    let bytes = inner.edit_bytes.borrow();
    if !out_len.is_null() {
        unsafe { *out_len = bytes.len() };
    }
    if bytes.is_empty() {
        b"\0".as_ptr()
    } else {
        bytes.as_ptr()
    }
}

fn snap_grid(interval: f32, align: i32) -> Option<ui_core::SnapGrid> {
    if !(interval.is_finite() && interval > 0.5) {
        return None;
    }
    let align = match align {
        1 => ui_core::SnapAlign::Center,
        2 => ui_core::SnapAlign::End,
        _ => ui_core::SnapAlign::Start,
    };
    Some(ui_core::SnapGrid { interval, align })
}

unsafe extern "C" fn set_scroll_physics(
    session: *mut c_void,
    node: u64,
    tau: f32,
    mode: i32,
    stiffness: f32,
    damping: f32,
    factor: f32,
    snap_x: f32,
    align_x: i32,
    snap_y: f32,
    align_y: i32,
    accept: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let physics = ui_core::ScrollPhysics {
        friction_tau_ms: tau,
        overscroll: if mode == 1 {
            ui_core::OverscrollMode::Clamp
        } else {
            ui_core::OverscrollMode::RubberBand
        },
        overscroll_stiffness: stiffness,
        overscroll_damping: damping,
        overscroll_factor: factor,
        snap_x: snap_grid(snap_x, align_x),
        snap_y: snap_grid(snap_y, align_y),
        accept: ui_core::ScrollAccept::from_code(accept),
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_physics(NodeId::from_raw(node), physics),
    )
}

unsafe extern "C" fn set_scroll_chain(session: *mut c_void, node: u64, chain: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_chain(NodeId::from_raw(node), chain != 0),
    )
}

unsafe extern "C" fn overscroll(session: *mut c_void, node: u64, x: *mut f32, y: *mut f32) -> i32 {
    if x.is_null() || y.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().overscroll(NodeId::from_raw(node)) {
        Ok((ox, oy)) => {
            unsafe {
                *x = ox;
                *y = oy;
            }
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn arm_fling(session: *mut c_void, node: u64, vx: f32, vy: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .arm_fling(NodeId::from_raw(node), vx, vy),
    )
}

unsafe extern "C" fn stop_scroll(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().stop_scroll(NodeId::from_raw(node)))
}

unsafe extern "C" fn scroll_to(
    session: *mut c_void,
    node: u64,
    x: f32,
    y: f32,
    stiffness: f32,
    damping: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let stiffness = (stiffness > 0.0).then_some(stiffness);
    let damping = (damping > 0.0).then_some(damping);
    status(
        inner
            .tree
            .borrow_mut()
            .scroll_to(NodeId::from_raw(node), x, y, stiffness, damping),
    )
}

unsafe extern "C" fn scroll_offset(
    session: *mut c_void,
    node: u64,
    dx: *mut f32,
    dy: *mut f32,
) -> i32 {
    if dx.is_null() || dy.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().scroll_offset(NodeId::from_raw(node)) {
        Ok((x, y)) => {
            // SAFETY: glue passes two writable floats that live for this call.
            unsafe {
                *dx = x;
                *dy = y;
            }
            0
        }
        Err(_) => -1,
    }
}

struct PendingMutation {
    op: u32,
    id: u64,
    parent: u64,
    field: u32,
    number: f64,
    number2: f64,
    text: String,
}

fn sweep_detached(inner: &Inner) {
    let retired = inner.tree.borrow_mut().discard_detached();
    if let Err(message) = dispatch_gestures(inner, retired.gestures) {
        (inner.log)(3, &message);
    }
    if retired.ids.is_empty() {
        return;
    }
    let raw: Vec<u64> = retired.ids.into_iter().map(NodeId::to_raw).collect();
    // SAFETY: `raw` lives until this call returns. `host` is the open QuickJS host.
    unsafe { qjs_forget_ids(inner.host, raw.as_ptr(), raw.len()) };
}

fn flush_mutations(inner: &Inner) -> bool {
    let pending = std::mem::take(&mut *inner.mutations.borrow_mut());
    if pending.is_empty() {
        return false;
    }
    let mut tree = inner.tree.borrow_mut();
    for mutation in pending {
        let _ = apply_mutation(&mut tree, &mutation);
    }
    true
}

fn apply_mutation(tree: &mut Tree, mutation: &PendingMutation) -> Result<(), ui_core::Error> {
    use crate::abi::{MutationOp, StyleField};
    let op = mutation.op;
    let id = NodeId::from_raw(mutation.id);
    if op == MutationOp::SetText as u32 {
        return tree.set_text(id, mutation.text.clone());
    }
    if op == MutationOp::Create as u32 {
        let created = match mutation.field {
            1 => tree.column(mutation.number as f32)?,
            5 => tree.sized(mutation.number as f32, mutation.number2 as f32)?,
            _ => return Err(ui_core::Error::WrongKind),
        };
        if mutation.parent != 0 {
            tree.add_child(NodeId::from_raw(mutation.parent), created)?;
        }
        return Ok(());
    }
    if op != MutationOp::SetStyle as u32 {
        return Ok(());
    }
    match mutation.field {
        value if value == StyleField::Width as u32 => {
            let height = if mutation.number2 > 0.0 {
                mutation.number2 as f32
            } else {
                1.0
            };
            tree.set_size(id, mutation.number as f32, height)
        }
        value if value == StyleField::Height as u32 => {
            let width = if mutation.number2 > 0.0 {
                mutation.number2 as f32
            } else {
                1.0
            };
            tree.set_size(id, width, mutation.number as f32)
        }
        value if value == StyleField::Gap as u32 => tree.set_gap(id, mutation.number as f32),
        value if value == StyleField::Grow as u32 => tree.set_grow(id, mutation.number as f32),
        value if value == StyleField::FontSize as u32 => {
            tree.set_font_size(id, mutation.number as f32)
        }
        value if value == StyleField::Alpha as u32 => {
            tree.set_alpha(id, mutation.number.round().clamp(0.0, 255.0) as u8)
        }
        value if value == StyleField::Radius as u32 => tree.set_radius(id, mutation.number as f32),
        _ => Ok(()),
    }
}

unsafe extern "C" fn push_mutation(
    session: *mut c_void,
    op: u32,
    id: u64,
    parent: u64,
    before: u64,
    field: u32,
    number: f64,
    number2: f64,
    color: *const u8,
    text: *const c_char,
    text_len: usize,
) -> i32 {
    let _ = (before, color);
    let inner = unsafe { inner(session) };
    let bytes = if text.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), text_len) }
    };
    let text = String::from_utf8_lossy(bytes).into_owned();
    inner.mutations.borrow_mut().push(PendingMutation {
        op,
        id,
        parent,
        field,
        number,
        number2,
        text,
    });
    0
}

unsafe extern "C" fn rect(
    session: *mut c_void,
    node: u64,
    x: *mut f32,
    y: *mut f32,
    w: *mut f32,
    h: *mut f32,
) -> i32 {
    if x.is_null() || y.is_null() || w.is_null() || h.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    if flush_mutations(inner) {
        let mut tree = inner.tree.borrow_mut();
        if tree.needs_layout() {
            let _ = tree.layout();
        }
    }
    match inner.tree.borrow().rect(NodeId::from_raw(node)) {
        Ok(rect) => {
            // SAFETY: glue passes four writable floats that live for this call.
            unsafe {
                *x = rect.x;
                *y = rect.y;
                *w = rect.w;
                *h = rect.h;
            }
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn transform(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().transform(), out)
}

unsafe extern "C" fn set_transform(
    session: *mut c_void,
    node: u64,
    tx: f32,
    ty: f32,
    rotate: f32,
    scale: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_transform(NodeId::from_raw(node), tx, ty, rotate, scale),
    )
}

unsafe extern "C" fn set_justify(session: *mut c_void, node: u64, justify: u32) -> i32 {
    let Some(justify) = justify_of(justify) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_justify(NodeId::from_raw(node), justify),
    )
}

unsafe extern "C" fn set_wrap(session: *mut c_void, node: u64, wrap: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_wrap(NodeId::from_raw(node), wrap != 0),
    )
}

unsafe extern "C" fn set_fill(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_fill(NodeId::from_raw(node)))
}

fn parse_tracks(text: &str) -> Result<Vec<TrackSize>, ui_core::Error> {
    if text.is_empty() {
        return Err(ui_core::Error::BadNumber);
    }
    text.split(',').map(parse_track).collect()
}

fn parse_track(token: &str) -> Result<TrackSize, ui_core::Error> {
    let token = token.trim();
    match token {
        "min" => Ok(TrackSize::MinContent),
        "max" => Ok(TrackSize::MaxContent),
        other if let Some(value) = other.strip_suffix("fr") => {
            let value = value
                .parse::<f32>()
                .map_err(|_| ui_core::Error::BadNumber)?;
            if !value.is_finite() || value < 0.0 {
                return Err(ui_core::Error::BadNumber);
            }
            Ok(TrackSize::Fraction(value))
        }
        other => {
            let value = other
                .parse::<f32>()
                .map_err(|_| ui_core::Error::BadNumber)?;
            if !value.is_finite() || value < 0.0 {
                return Err(ui_core::Error::BadNumber);
            }
            Ok(TrackSize::Points(value))
        }
    }
}

unsafe extern "C" fn grid(
    session: *mut c_void,
    columns: *const c_char,
    columns_len: usize,
    rows: *const c_char,
    rows_len: usize,
    gap: f32,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(columns) = (unsafe { utf8_of(columns, columns_len) }) else {
        return -1;
    };
    let Some(rows) = (unsafe { utf8_of(rows, rows_len) }) else {
        return -1;
    };
    let Ok(columns) = parse_tracks(columns) else {
        return -1;
    };
    let Ok(rows) = parse_tracks(rows) else {
        return -1;
    };
    created(
        inner.tree.borrow_mut().grid_tracks(&columns, &rows, gap),
        out,
    )
}

unsafe extern "C" fn set_padding_edges(
    session: *mut c_void,
    node: u64,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_padding_edges(
        NodeId::from_raw(node),
        Edges {
            top,
            right,
            bottom,
            left,
        },
    ))
}

unsafe extern "C" fn set_margin(
    session: *mut c_void,
    node: u64,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_margin(
        NodeId::from_raw(node),
        Edges {
            top,
            right,
            bottom,
            left,
        },
    ))
}

unsafe extern "C" fn set_min_size(session: *mut c_void, node: u64, width: f32, height: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_min_size(NodeId::from_raw(node), width, height),
    )
}

unsafe extern "C" fn set_max_size(session: *mut c_void, node: u64, width: f32, height: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_max_size(NodeId::from_raw(node), width, height),
    )
}

unsafe extern "C" fn set_shrink(session: *mut c_void, node: u64, shrink: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_shrink(NodeId::from_raw(node), shrink),
    )
}

unsafe extern "C" fn set_basis(session: *mut c_void, node: u64, basis: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_basis(NodeId::from_raw(node), basis),
    )
}

unsafe extern "C" fn set_align_self(session: *mut c_void, node: u64, align: u32) -> i32 {
    let alignment = match align {
        0 => Alignment::Start,
        1 => Alignment::Center,
        2 => Alignment::End,
        _ => return -1,
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_align_self(NodeId::from_raw(node), alignment),
    )
}

fn justify_of(value: u32) -> Option<Justify> {
    match value {
        0 => Some(Justify::Start),
        1 => Some(Justify::Center),
        2 => Some(Justify::End),
        3 => Some(Justify::SpaceBetween),
        4 => Some(Justify::SpaceAround),
        5 => Some(Justify::SpaceEvenly),
        _ => None,
    }
}

unsafe extern "C" fn log_message(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    level: i32,
) {
    let inner = unsafe { inner(session) };
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return;
    };
    let level = u8::try_from(level).unwrap_or(0);
    (inner.log)(level, text);
}

fn gesture_parts(gesture: ui_core::Gesture) -> (NodeId, i32, f64, f64) {
    match gesture {
        ui_core::Gesture::Click(id) => (id, 0, 0.0, 0.0),
        ui_core::Gesture::Press(id) => (id, 1, 0.0, 0.0),
        ui_core::Gesture::Release(id) => (id, 2, 0.0, 0.0),
        ui_core::Gesture::DragBegin(id) => (id, 3, 0.0, 0.0),
        ui_core::Gesture::DragBy(id, dx, dy) => (id, 4, f64::from(dx), f64::from(dy)),
        ui_core::Gesture::DragEnd(id, vx, vy) => (id, 5, f64::from(vx), f64::from(vy)),
        ui_core::Gesture::Focus(id) => (id, 6, 0.0, 0.0),
        ui_core::Gesture::Blur(id) => (id, 7, 0.0, 0.0),
        ui_core::Gesture::Hover(id) => (id, 11, 0.0, 0.0),
        ui_core::Gesture::Leave(id) => (id, 12, 0.0, 0.0),
        ui_core::Gesture::Wheel(id, dx, dy) => (id, 13, f64::from(dx), f64::from(dy)),
        ui_core::Gesture::SecondaryClick(id) => (id, 14, 0.0, 0.0),
        ui_core::Gesture::Scroll(id, x, y) => (id, 15, f64::from(x), f64::from(y)),
        ui_core::Gesture::TextReplaced(id) => (id, 16, 0.0, 0.0),
        ui_core::Gesture::ActivateLink { id, link } => (id, 17, f64::from(link), 0.0),
        ui_core::Gesture::LongPress(id) => (id, 18, 0.0, 0.0),
        ui_core::Gesture::DoubleTap(id) => (id, 19, 0.0, 0.0),
        ui_core::Gesture::Zoom { id, scale } => (id, 20, f64::from(scale), 0.0),
        ui_core::Gesture::ScrollPhaseChanged { id, .. } => (id, 21, 0.0, 0.0),
    }
}

fn dispatch_gestures(inner: &Inner, gestures: Vec<ui_core::Gesture>) -> Result<(), String> {
    for gesture in gestures {
        note_gesture(&inner.marks, &gesture);
        match gesture {
            ui_core::Gesture::TextReplaced(id) => {
                let text = inner.tree.borrow().editor_text(id).unwrap_or_default();
                dispatch_text(inner, id, &text)?;
                continue;
            }
            ui_core::Gesture::ActivateLink { id, link } => {
                dispatch_kind(inner, id, 17, f64::from(link), 0.0)?;
                continue;
            }
            ui_core::Gesture::ScrollPhaseChanged {
                id,
                phase,
                vx,
                vy,
                ox,
                oy,
            } => {
                dispatch_scroll_phase(inner, id, phase, vx, vy, ox, oy)?;
                continue;
            }
            other => {
                let (id, kind, a, b) = gesture_parts(other);
                dispatch_kind(inner, id, kind, a, b)?;
            }
        }
    }
    Ok(())
}

fn dispatch_scroll_phase(
    inner: &Inner,
    id: NodeId,
    phase: ui_core::ScrollPhase,
    vx: f32,
    vy: f32,
    ox: f32,
    oy: f32,
) -> Result<(), String> {
    let mut error = [0u8; 512];
    let failed = unsafe {
        qjs_scroll_state(
            inner.host,
            id.to_raw(),
            phase.code(),
            vx,
            vy,
            ox,
            oy,
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if failed == 0 {
        Ok(())
    } else {
        Err(c_message(&error))
    }
}

fn dispatch_kind(inner: &Inner, id: NodeId, kind: i32, a: f64, b: f64) -> Result<(), String> {
    let mut error = [0u8; 512];
    // SAFETY: `error` is writable for the length we pass. `host` is this session's
    // interpreter. `kind` selects one callback. Drag kinds read `a` and `b`.
    let failed = unsafe {
        qjs_dispatch(
            inner.host,
            id.to_raw(),
            kind,
            a,
            b,
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if failed != 0 {
        Err(c_message(&error))
    } else {
        Ok(())
    }
}

fn dispatch_text(inner: &Inner, id: NodeId, text: &str) -> Result<(), String> {
    let mut error = [0u8; 512];
    // SAFETY: `text` lives until this call returns. The interpreter copies it
    // into a JS string before the callback runs.
    let failed = unsafe {
        qjs_dispatch_text(
            inner.host,
            id.to_raw(),
            text.as_ptr().cast(),
            text.len(),
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if failed == 0 {
        Ok(())
    } else {
        Err(c_message(&error))
    }
}

fn dispatch_input(
    inner: &Inner,
    id: NodeId,
    kind: i32,
    text: Option<&str>,
    span: Option<(i32, i32, i32, i32)>,
) -> Result<(), String> {
    let mut error = [0u8; 512];
    let bytes = text.unwrap_or("");
    let ptr = if text.is_some() {
        bytes.as_ptr()
    } else {
        ptr::null()
    };
    let (with_span, sel_start, sel_end, compose_start, compose_end) = match span {
        Some((start, end, compose_start, compose_end)) => {
            (1, start, end, compose_start, compose_end)
        }
        None => (0, -1, -1, -1, -1),
    };
    // SAFETY: `bytes` lives for this call. A null pointer with `text == None` is the
    // composing-ended signal. `error` is writable for the length we pass.
    let failed = unsafe {
        qjs_dispatch_input(
            inner.host,
            id.to_raw(),
            kind,
            ptr.cast(),
            if text.is_some() { bytes.len() } else { 0 },
            with_span,
            sel_start,
            sel_end,
            compose_start,
            compose_end,
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    if failed == 0 {
        Ok(())
    } else {
        Err(c_message(&error))
    }
}

fn shortcut_spec(press: &session::KeyPress) -> String {
    let mut parts = Vec::new();
    if press.modifiers.contains(Modifiers::META) || press.modifiers.contains(Modifiers::CONTROL) {
        parts.push("mod".to_string());
    }
    if press.modifiers.contains(Modifiers::ALT) {
        parts.push("alt".to_string());
    }
    if press.modifiers.contains(Modifiers::SHIFT) || press.shift {
        parts.push("shift".to_string());
    }
    let key = match press.key {
        Key::Char(ch) => ch.to_lowercase().to_string(),
        other => key_name(session::KeyPress {
            key: other,
            shift: false,
            modifiers: Modifiers::empty(),
        })
        .to_lowercase(),
    };
    parts.push(key);
    parts.join("+")
}

fn key_name(press: session::KeyPress) -> String {
    let name = match press.key {
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Left => "Left",
        Key::Right => "Right",
        Key::Up => "Up",
        Key::Down => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::WordLeft => "WordLeft",
        Key::WordRight => "WordRight",
        Key::DocumentStart => "DocumentStart",
        Key::DocumentEnd => "DocumentEnd",
        Key::DeleteWord => "DeleteWord",
        Key::DeleteWordForward => "DeleteWordForward",
        Key::DeleteToLineStart => "DeleteToLineStart",
        Key::DeleteToLineEnd => "DeleteToLineEnd",
        Key::Char(ch) => return ch.to_string(),
    };
    if press.shift {
        format!("Shift+{name}")
    } else {
        name.to_owned()
    }
}

unsafe extern "C" fn sized(session: *mut c_void, width: f32, height: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().sized(width, height), out)
}

unsafe extern "C" fn set_drag_claim(session: *mut c_void, node: u64, claim: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let claim = match claim {
        1 => DragClaim::Horizontal,
        2 => DragClaim::Free,
        _ => DragClaim::None,
    };
    match inner
        .tree
        .borrow_mut()
        .set_drag_claim(NodeId::from_raw(node), claim)
    {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn caret_at(
    session: *mut c_void,
    node: u64,
    x: f32,
    y: f32,
    unit: *mut u32,
    out_x: *mut f32,
    out_y: *mut f32,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().caret_at(NodeId::from_raw(node), x, y) {
        Ok(caret) => {
            // SAFETY: the pointers are local out-params from `js_caret_at`.
            unsafe {
                *unit = caret.unit;
                *out_x = caret.x;
                *out_y = caret.y;
                *out_w = caret.w;
                *out_h = caret.h;
            }
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn word_range(
    session: *mut c_void,
    node: u64,
    unit: u32,
    start: *mut u32,
    end: *mut u32,
) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().word_range(NodeId::from_raw(node), unit) {
        Ok((word_start, word_end)) => {
            unsafe {
                *start = word_start;
                *end = word_end;
            }
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn focus_step(session: *mut c_void, direction: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let direction = match direction {
        0 => FocusDirection::Next,
        1 => FocusDirection::Previous,
        2 => FocusDirection::Left,
        3 => FocusDirection::Right,
        4 => FocusDirection::Up,
        5 => FocusDirection::Down,
        _ => return -1,
    };
    let gestures = inner.tree.borrow_mut().focus_step(direction);
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

unsafe extern "C" fn push_focus_scope(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = match inner
        .tree
        .borrow_mut()
        .push_focus_scope(NodeId::from_raw(node))
    {
        Ok(gestures) => gestures,
        Err(_) => return -1,
    };
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

unsafe extern "C" fn pop_focus_scope(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    inner.tree.borrow_mut().pop_focus_scope();
    0
}

unsafe extern "C" fn pointer_position(session: *mut c_void, x: *mut f32, y: *mut f32) -> i32 {
    let inner = unsafe { inner(session) };
    let (px, py) = inner.tree.borrow().pointer_position();
    unsafe {
        *x = px;
        *y = py;
    }
    0
}

unsafe extern "C" fn set_focusable(session: *mut c_void, node: u64, enabled: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = match inner
        .tree
        .borrow_mut()
        .set_focusable(NodeId::from_raw(node), enabled != 0)
    {
        Ok(gestures) => gestures,
        Err(_) => return -1,
    };
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

unsafe extern "C" fn focus_node(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = match inner.tree.borrow_mut().focus(NodeId::from_raw(node)) {
        Ok(gestures) => gestures,
        Err(_) => return -1,
    };
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

unsafe extern "C" fn focused_id(session: *mut c_void) -> u64 {
    let inner = unsafe { inner(session) };
    inner
        .tree
        .borrow()
        .focused()
        .map(NodeId::to_raw)
        .unwrap_or(0)
}

unsafe extern "C" fn is_focusable(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().is_focusable(NodeId::from_raw(node)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn grapheme_index_of(utf8: *const c_char, len: usize, unit: u32) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_index(text, unit)
}

unsafe extern "C" fn grapheme_count_of(utf8: *const c_char, len: usize) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_count(text)
}

unsafe extern "C" fn grapheme_unit_of(utf8: *const c_char, len: usize, index: u32) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_unit(text, index)
}

unsafe extern "C" fn set_reverse(session: *mut c_void, node: u64, reverse: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_reverse(NodeId::from_raw(node), reverse != 0),
    )
}

unsafe extern "C" fn set_scale_xy(
    session: *mut c_void,
    node: u64,
    scale_x: f32,
    scale_y: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scale_xy(NodeId::from_raw(node), scale_x, scale_y),
    )
}

unsafe extern "C" fn register_font(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    bytes: *const u8,
    byte_len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(family) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    if byte_len > 0 && bytes.is_null() {
        return -1;
    }
    let data = if byte_len == 0 {
        Vec::new()
    } else {
        // SAFETY: `bytes` points at `byte_len` initialized octets for this call.
        unsafe { std::slice::from_raw_parts(bytes, byte_len) }.to_vec()
    };
    if inner
        .fonts
        .borrow_mut()
        .enqueue(family.to_owned(), data)
        .is_err()
    {
        return -1;
    }
    0
}

unsafe extern "C" fn reload(session: *mut c_void, utf8: *const c_char, len: usize) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(source) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    if source.is_empty() {
        return -1;
    }
    inner.reloads.borrow_mut().push(source.to_owned());
    0
}

unsafe extern "C" fn layer_stats(
    session: *mut c_void,
    epoch: *mut u64,
    layers: *mut u32,
    pictures: *mut u32,
) -> i32 {
    if epoch.is_null() || layers.is_null() || pictures.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let (epoch_value, layer_count, picture_count) = inner.tree.borrow().layer_tree_stats();
    unsafe {
        *epoch = epoch_value;
        *layers = u32::try_from(layer_count).unwrap_or(u32::MAX);
        *pictures = u32::try_from(picture_count).unwrap_or(u32::MAX);
    }
    0
}

unsafe extern "C" fn scroll_into_view(session: *mut c_void, node: u64, edge: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let edge = match edge {
        1 => ui_core::ScrollEdge::End,
        2 => ui_core::ScrollEdge::Nearest,
        _ => ui_core::ScrollEdge::Start,
    };
    match inner
        .tree
        .borrow_mut()
        .scroll_into_view(NodeId::from_raw(node), edge)
    {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn animate(
    session: *mut c_void,
    node: u64,
    prop: *const c_char,
    prop_len: usize,
    from: f64,
    to: f64,
    stiffness: f64,
    damping: f64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(prop, prop_len) }) else {
        return -1;
    };
    let Some(prop) = anim_prop(name) else {
        return -1;
    };
    if ![from, to, stiffness, damping]
        .into_iter()
        .all(f64::is_finite)
    {
        return -1;
    }
    inner.tree.borrow_mut().animate(
        NodeId::from_raw(node),
        prop,
        Box::new(ui_core::Spring {
            x0: from as f32,
            v0: 0.0,
            target: to as f32,
            stiffness: stiffness as f32,
            damping: damping as f32,
        }),
    );
    0
}

fn anim_prop(name: &str) -> Option<ui_core::AnimProp> {
    Some(match name {
        "scrollX" => ui_core::AnimProp::ScrollX,
        "scrollY" => ui_core::AnimProp::ScrollY,
        "opacity" => ui_core::AnimProp::Opacity,
        "translateX" => ui_core::AnimProp::TranslateX,
        "translateY" => ui_core::AnimProp::TranslateY,
        "rotate" => ui_core::AnimProp::Rotate,
        "scale" => ui_core::AnimProp::Scale,
        "overscrollX" => ui_core::AnimProp::OverscrollX,
        "overscrollY" => ui_core::AnimProp::OverscrollY,
        _ => return None,
    })
}

unsafe extern "C" fn fling_target(
    session: *mut c_void,
    node: u64,
    out_x: *mut f32,
    out_y: *mut f32,
) -> i32 {
    if out_x.is_null() || out_y.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().fling_target(NodeId::from_raw(node)) {
        Some((x, y)) => {
            unsafe {
                *out_x = x;
                *out_y = y;
            }
            0
        }
        None => 1,
    }
}

unsafe extern "C" fn set_recognizers(
    session: *mut c_void,
    node: u64,
    tap: i32,
    long_press: i32,
    drag: i32,
    double_tap_ms: f64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let drag = match drag {
        0 => Some(ui_core::DragAxis::Free),
        1 => Some(ui_core::DragAxis::Horizontal),
        2 => Some(ui_core::DragAxis::Vertical),
        _ => None,
    };
    let double_tap = (double_tap_ms.is_finite() && double_tap_ms > 0.0).then(|| {
        let mut config = ui_core::DoubleTapConfig::default();
        config.window_ms = double_tap_ms;
        config
    });
    status(inner.tree.borrow_mut().set_recognizers(
        NodeId::from_raw(node),
        ui_core::RecognizerSet {
            tap: tap != 0,
            long_press: long_press != 0,
            drag,
            double_tap,
        },
    ))
}

unsafe extern "C" fn set_stroke(
    session: *mut c_void,
    node: u64,
    width: f32,
    color: *const u8,
) -> i32 {
    let Some(color) = (unsafe { rgba(color) }) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_stroke(NodeId::from_raw(node), width, color),
    )
}

unsafe extern "C" fn set_image_mode(
    session: *mut c_void,
    node: u64,
    mode: i32,
    required: i32,
) -> i32 {
    let mode = match mode {
        0 => protocol::ContentMode::Fill,
        1 => protocol::ContentMode::Fit,
        2 => protocol::ContentMode::Crop,
        _ => return -1,
    };
    if required != 0 && required != 1 {
        return -1;
    }
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_image_mode(NodeId::from_raw(node), mode, required != 0),
    )
}

const FONT_LIMIT: usize = 8;
const FONT_BYTE_LIMIT: usize = 4 * 1024 * 1024;

struct FontBook {
    queued: Vec<(String, Vec<u8>)>,
    handed: HashSet<String>,
}

impl FontBook {
    fn new() -> Self {
        Self {
            queued: Vec::new(),
            handed: HashSet::new(),
        }
    }

    /// Queues one face. The same family replaces bytes and does not take another slot.
    /// `UIFWSans` and `UIFWSansCJK` do not count toward the eight-name cap.
    fn enqueue(&mut self, family: String, bytes: Vec<u8>) -> Result<(), ()> {
        if family.is_empty() || bytes.len() > FONT_BYTE_LIMIT {
            return Err(());
        }
        if let Some(slot) = self.queued.iter_mut().find(|(name, _)| name == &family) {
            slot.1 = bytes;
            return Ok(());
        }
        let seen = self.handed.contains(&family);
        if !seen && !startup_font(&family) && self.occupied() >= FONT_LIMIT {
            return Err(());
        }
        self.queued.push((family, bytes));
        Ok(())
    }

    fn take(&mut self) -> Vec<(String, Vec<u8>)> {
        let taken = std::mem::take(&mut self.queued);
        for (name, _) in &taken {
            self.handed.insert(name.clone());
        }
        taken
    }

    fn occupied(&self) -> usize {
        self.handed
            .iter()
            .filter(|name| !startup_font(name))
            .count()
            + self
                .queued
                .iter()
                .filter(|(name, _)| !startup_font(name))
                .count()
    }
}

fn startup_font(name: &str) -> bool {
    name == "UIFWSans" || name == "UIFWSansCJK"
}

fn axis_from_tag(tag: i32, value: f32) -> Result<AxisSize, ui_core::Error> {
    match tag {
        0 => Ok(AxisSize::Auto),
        1 => Ok(AxisSize::Points(value)),
        2 => Ok(AxisSize::Fraction(value)),
        _ => Err(ui_core::Error::BadNumber),
    }
}

fn inset_from_tag(is_auto: i32, value: f32) -> Option<f32> {
    if is_auto == 0 { Some(value) } else { None }
}

fn grid_from_tag(tag: i32, value: i32) -> Result<GridLine, ui_core::Error> {
    match tag {
        0 => Ok(GridLine::Auto),
        1 => {
            let index = i16::try_from(value).map_err(|_| ui_core::Error::BadNumber)?;
            if index == 0 {
                return Err(ui_core::Error::BadNumber);
            }
            Ok(GridLine::Line(index))
        }
        2 => {
            let count = u16::try_from(value).map_err(|_| ui_core::Error::BadNumber)?;
            if count == 0 {
                return Err(ui_core::Error::BadNumber);
            }
            Ok(GridLine::Span(count))
        }
        _ => Err(ui_core::Error::BadNumber),
    }
}

unsafe extern "C" fn set_axis_size(
    session: *mut c_void,
    node: u64,
    width_tag: i32,
    width: f32,
    height_tag: i32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let width = match axis_from_tag(width_tag, width) {
        Ok(size) => size,
        Err(_) => return -1,
    };
    let height = match axis_from_tag(height_tag, height) {
        Ok(size) => size,
        Err(_) => return -1,
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_axis_size(NodeId::from_raw(node), width, height),
    )
}

unsafe extern "C" fn set_aspect_ratio(
    session: *mut c_void,
    node: u64,
    present: i32,
    ratio: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let ratio = if present == 0 { None } else { Some(ratio) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_aspect_ratio(NodeId::from_raw(node), ratio),
    )
}

unsafe extern "C" fn set_position(session: *mut c_void, node: u64, absolute: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_position(NodeId::from_raw(node), absolute != 0),
    )
}

unsafe extern "C" fn set_inset(
    session: *mut c_void,
    node: u64,
    top_auto: i32,
    top: f32,
    right_auto: i32,
    right: f32,
    bottom_auto: i32,
    bottom: f32,
    left_auto: i32,
    left: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_inset(
        NodeId::from_raw(node),
        Inset {
            top: inset_from_tag(top_auto, top),
            right: inset_from_tag(right_auto, right),
            bottom: inset_from_tag(bottom_auto, bottom),
            left: inset_from_tag(left_auto, left),
        },
    ))
}

unsafe extern "C" fn set_grid_placement(
    session: *mut c_void,
    node: u64,
    column_start_tag: i32,
    column_start: i32,
    column_end_tag: i32,
    column_end: i32,
    row_start_tag: i32,
    row_start: i32,
    row_end_tag: i32,
    row_end: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let column_start = match grid_from_tag(column_start_tag, column_start) {
        Ok(line) => line,
        Err(_) => return -1,
    };
    let column_end = match grid_from_tag(column_end_tag, column_end) {
        Ok(line) => line,
        Err(_) => return -1,
    };
    let row_start = match grid_from_tag(row_start_tag, row_start) {
        Ok(line) => line,
        Err(_) => return -1,
    };
    let row_end = match grid_from_tag(row_end_tag, row_end) {
        Ok(line) => line,
        Err(_) => return -1,
    };
    status(inner.tree.borrow_mut().set_grid_placement(
        NodeId::from_raw(node),
        column_start,
        column_end,
        row_start,
        row_end,
    ))
}

unsafe extern "C" fn blur_node(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = inner.tree.borrow_mut().blur();
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

unsafe extern "C" fn set_ime_enabled(session: *mut c_void, enabled: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let mut ime = inner.ime.borrow_mut();
    ime.shown = enabled != 0;
    ime.dirty = true;
    0
}

unsafe extern "C" fn set_ime_text(session: *mut c_void, utf8: *const c_char, len: usize) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    let mut ime = inner.ime.borrow_mut();
    ime.state = session::ImeState::at_end(text);
    ime.dirty = true;
    0
}

unsafe extern "C" fn set_ime_state(
    session: *mut c_void,
    shown: i32,
    utf8: *const c_char,
    len: usize,
    sel_start: u32,
    sel_end: u32,
    comp_start: i32,
    comp_end: i32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let mut ime = inner.ime.borrow_mut();
    if shown == 0 {
        ime.shown = false;
        ime.dirty = true;
        return 0;
    }
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    let text_len = session::utf16_len(text);
    let (sel_start, sel_end) = if sel_start <= sel_end {
        (sel_start, sel_end)
    } else {
        (sel_end, sel_start)
    };
    if sel_end > text_len {
        return -1;
    }
    let action = ime.state.action;
    let keyboard = ime.state.keyboard;
    let composing = if comp_start < 0 {
        None
    } else {
        let start = comp_start as u32;
        let end = comp_end as u32;
        if start > end || end > text_len {
            return -1;
        }
        Some(start..end)
    };
    ime.shown = true;
    ime.dirty = true;
    ime.state = session::ImeState {
        text: text.to_owned(),
        selection: sel_start..sel_end,
        composing,
        caret: Rect { x, y, w, h },
        action,
        keyboard,
    };
    0
}

fn tri_state(value: i32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

unsafe extern "C" fn set_semantics(
    session: *mut c_void,
    node: u64,
    role: i32,
    label: *const c_char,
    label_len: usize,
    value: *const c_char,
    value_len: usize,
    checked: i32,
    sel_start: i32,
    sel_end: i32,
    disabled: i32,
    editable: i32,
    read_only: i32,
    description: *const c_char,
    description_len: usize,
    order_set: i32,
    order: i32,
    live: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let label = unsafe { utf8_of(label, label_len) }.map(str::to_owned);
    let value = unsafe { utf8_of(value, value_len) }.map(str::to_owned);
    let description = unsafe { utf8_of(description, description_len) }.map(str::to_owned);
    let hint = ui_core::SemanticsHint {
        role: role_of(role),
        label: label.filter(|text| !text.is_empty()),
        value: value.filter(|text| !text.is_empty()),
        checked: tri_state(checked),
        selection: (sel_start >= 0 && sel_end >= sel_start)
            .then_some((sel_start as u32, sel_end as u32)),
        disabled: tri_state(disabled),
        editable: tri_state(editable),
        read_only: tri_state(read_only),
        description: description.filter(|text| !text.is_empty()),
        order: (order_set != 0).then_some(order),
        live: match live {
            0 => Some(ui_core::LiveRegion::Off),
            1 => Some(ui_core::LiveRegion::Polite),
            2 => Some(ui_core::LiveRegion::Assertive),
            _ => None,
        },
    };
    let result = inner
        .tree
        .borrow_mut()
        .set_semantics(NodeId::from_raw(node), hint);
    if result.is_err() { -1 } else { 0 }
}

fn role_of(role: i32) -> Option<ui_core::SemanticsRole> {
    Some(match role {
        1 => ui_core::SemanticsRole::Button,
        2 => ui_core::SemanticsRole::Text,
        3 => ui_core::SemanticsRole::Image,
        4 => ui_core::SemanticsRole::Scroll,
        5 => ui_core::SemanticsRole::Switch,
        6 => ui_core::SemanticsRole::TextField,
        7 => ui_core::SemanticsRole::SearchField,
        8 => ui_core::SemanticsRole::TextArea,
        _ => return None,
    })
}

unsafe extern "C" fn text_scale(session: *mut c_void) -> f64 {
    let inner = unsafe { inner(session) };
    f64::from(inner.text_scale.get())
}

unsafe extern "C" fn reduce_motion(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(inner.reduce_motion.get())
}

unsafe extern "C" fn measure_text(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    font_size: f32,
    max_width: f32,
    color: *const u8,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    if out_w.is_null() || out_h.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let (Some(utf8), Some(color)) = (unsafe { utf8_of(utf8, len) }, unsafe { rgba(color) }) else {
        return -1;
    };
    let size = inner.tree.borrow().measure_text(
        utf8,
        font_size,
        max_width,
        color,
        &protocol::TextStyle::default(),
    );
    // SAFETY: the caller passes writable floats that live for this call.
    unsafe {
        *out_w = size.width;
        *out_h = size.height;
    }
    0
}

fn self_paused(inner: &Inner) -> bool {
    inner.paused.get()
}

fn cstring(text: &str) -> CString {
    let mut bytes = text.replace('\0', "").into_bytes();
    bytes.push(0);
    // SAFETY: `replace` removed every interior NUL, and the last byte is the terminator.
    unsafe { CString::from_vec_with_nul_unchecked(bytes) }
}

fn emit_module(inner: &Inner, module: &str, event: &str, shape: i32, text: &str) {
    let module = cstring(module);
    let event = cstring(event);
    let body = cstring(text);
    let _ = unsafe {
        qjs_module_event(
            inner.host,
            module.as_ptr(),
            event.as_ptr(),
            shape,
            body.as_ptr(),
            text.replace('\0', "").len(),
        )
    };
}

fn emit_value(host: *mut c_void, value: &PluginValue) {
    match value {
        PluginValue::Null => unsafe { qjs_value_null(host) },
        PluginValue::Bool(flag) => unsafe { qjs_value_bool(host, i32::from(*flag)) },
        PluginValue::F64(number) => unsafe { qjs_value_f64(host, *number) },
        PluginValue::Str(text) => {
            let bytes = cstring(text);
            unsafe { qjs_value_str(host, bytes.as_ptr(), text.replace('\0', "").len()) };
        }
        PluginValue::Bytes(bytes) => unsafe {
            let ptr = if bytes.is_empty() {
                b"".as_ptr()
            } else {
                bytes.as_ptr()
            };
            qjs_value_bytes(host, ptr, bytes.len());
        },
        PluginValue::List(items) => {
            for item in items {
                emit_value(host, item);
            }
            unsafe { qjs_value_list(host, items.len() as u32) };
        }
        PluginValue::Map(entries) => {
            for (key, item) in entries {
                let bytes = cstring(key);
                unsafe { qjs_value_str(host, bytes.as_ptr(), key.replace('\0', "").len()) };
                emit_value(host, item);
            }
            unsafe { qjs_value_map(host, entries.len() as u32) };
        }
    }
}

fn settle_plugin(inner: &Inner, token: u64, result: &PluginResult) {
    match result {
        PluginResult::Done(value) => {
            emit_value(inner.host, value);
            let _ = unsafe { qjs_settle_top(inner.host, token, 1) };
        }
        PluginResult::Failed { message } => {
            emit_value(inner.host, &PluginValue::Str(message.clone()));
            let _ = unsafe { qjs_settle_top(inner.host, token, 0) };
        }
    }
}

fn settle_http(inner: &Inner, token: u64, result: &Result<HttpResponse, HttpError>) {
    match result {
        Ok(response) => {
            if response.body.len() > session::HTTP_BODY_LIMIT {
                let message = cstring("TypeError");
                let _ = unsafe {
                    qjs_settle_http(
                        inner.host,
                        token,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        0,
                        message.as_ptr(),
                    )
                };
                return;
            }
            let headers = host::header_block(&response.headers);
            let header_bytes = cstring(&headers);
            let _ = unsafe {
                qjs_settle_http(
                    inner.host,
                    token,
                    i32::from(response.status),
                    header_bytes.as_ptr(),
                    headers.len(),
                    response.body.as_ptr(),
                    response.body.len(),
                    ptr::null(),
                )
            };
        }
        Err(error) => {
            let message = cstring(&error.message);
            let _ = unsafe {
                qjs_settle_http(
                    inner.host,
                    token,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    0,
                    message.as_ptr(),
                )
            };
        }
    }
}

fn flush_pending(inner: &Inner) {
    let pending = std::mem::replace(&mut *inner.pending.borrow_mut(), Pending::new());
    for (token, result) in pending.http {
        if cancelled(inner, token) {
            continue;
        }
        settle_http(inner, token, &result);
    }
    for (token, result) in pending.plugin {
        settle_plugin(inner, token, &result);
    }
    for (id, event, text, data) in pending.socket {
        deliver_socket(inner, id, &event, text, &data);
    }
}

fn cancelled(inner: &Inner, token: u64) -> bool {
    inner
        .cancels
        .borrow()
        .get(&token)
        .is_some_and(|flag| flag.load(Ordering::Acquire))
}

fn deliver_socket(inner: &Inner, id: u64, event: &str, text: bool, data: &[u8]) {
    let event = cstring(event);
    let _ = unsafe {
        qjs_socket_event(
            inner.host,
            id,
            event.as_ptr(),
            i32::from(text),
            data.as_ptr(),
            data.len(),
        )
    };
}

fn submit_or_queue(inner: &Inner, job: IoJob, inline: impl FnOnce()) {
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(job);
    } else {
        inline();
    }
}

fn first_string(args: &[PluginValue]) -> String {
    match args.first() {
        Some(PluginValue::Str(text)) => text.clone(),
        _ => String::new(),
    }
}

unsafe extern "C" fn http_call(
    session: *mut c_void,
    token: u64,
    method: *const c_char,
    method_len: usize,
    url: *const c_char,
    url_len: usize,
    headers: *const c_char,
    headers_len: usize,
    body: *const u8,
    body_len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(method) = (unsafe { utf8_of(method, method_len) }) else {
        return -1;
    };
    let Some(url) = (unsafe { utf8_of(url, url_len) }) else {
        return -1;
    };
    let headers = unsafe { utf8_of(headers, headers_len) }.unwrap_or("");
    if body_len > session::HTTP_BODY_LIMIT {
        inner
            .pending
            .borrow_mut()
            .http
            .push((token, Err(HttpError::new("TypeError"))));
        return 0;
    }
    let bytes = if body.is_null() || body_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(body, body_len).to_vec() }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    inner
        .cancels
        .borrow_mut()
        .insert(token, Arc::clone(&cancel));
    let request = HttpRequest {
        token,
        method: method.to_owned(),
        url: url.to_owned(),
        headers: host::header_pairs(headers),
        body: bytes,
        cancel,
    };
    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        inner
            .pending
            .borrow_mut()
            .http
            .push((token, Err(HttpError::new("unsupported URL"))));
        return 0;
    }
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::Http(request));
        return 0;
    }
    if let Some(client) = inner.http.borrow().as_ref().cloned() {
        let result = client.perform(&request);
        inner.pending.borrow_mut().http.push((token, result));
        return 0;
    }
    inner
        .pending
        .borrow_mut()
        .http
        .push((token, Err(HttpError::new("network is not linked"))));
    0
}

unsafe extern "C" fn abort_token(session: *mut c_void, token: u64) {
    let inner = unsafe { inner(session) };
    if let Some(flag) = inner.cancels.borrow().get(&token) {
        flag.store(true, Ordering::Release);
    }
    inner
        .pending
        .borrow_mut()
        .http
        .retain(|(pending, _)| *pending != token);
    let aborted = Err(HttpError::new("AbortError"));
    settle_http(inner, token, &aborted);
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::Abort { token });
    }
}

fn prepared_mut(inner: &Inner) -> std::cell::RefMut<'_, Option<PreparedCall>> {
    inner.prepared.borrow_mut()
}

fn push_prepared(inner: &Inner, value: PluginValue) {
    if let Some(call) = prepared_mut(inner).as_mut() {
        call.args.push(value);
    }
}

unsafe extern "C" fn module_has(session: *mut c_void, name: *const c_char, len: usize) -> i32 {
    let Some(name) = (unsafe { utf8_of(name, len) }) else {
        return 0;
    };
    let _ = session;
    i32::from(host_module_linked(name))
}

unsafe extern "C" fn module_count(session: *mut c_void, name: *const c_char, len: usize) -> i32 {
    let _ = session;
    let Some(name) = (unsafe { utf8_of(name, len) }) else {
        return 0;
    };
    host_module_methods(name).count() as i32
}

unsafe extern "C" fn module_method_at(
    session: *mut c_void,
    name: *const c_char,
    name_len: usize,
    index: i32,
    out_len: *mut usize,
) -> *const c_char {
    let _ = session;
    let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
        return ptr::null();
    };
    let Some(method) = host_module_methods(name).nth(index as usize) else {
        return ptr::null();
    };
    if !out_len.is_null() {
        unsafe { *out_len = method.name.len() };
    }
    method.name.as_ptr().cast()
}

unsafe extern "C" fn module_prepare(
    session: *mut c_void,
    name: *const c_char,
    name_len: usize,
    method: *const c_char,
    method_len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
        return -1;
    };
    let Some(method) = (unsafe { utf8_of(method, method_len) }) else {
        return -1;
    };
    let Some(spec) = host_method(name, method) else {
        return -1;
    };
    *inner.prepared.borrow_mut() = Some(PreparedCall {
        module: name.to_owned(),
        method: method.to_owned(),
        place: spec.place,
        wait: spec.wait,
        token: 0,
        args: ArgBuild::new(),
    });
    i32::from(spec.wait == HostWait::Promise)
}

unsafe extern "C" fn module_push_null(session: *mut c_void) {
    push_prepared(unsafe { inner(session) }, PluginValue::Null);
}

unsafe extern "C" fn module_push_bool(session: *mut c_void, value: i32) {
    push_prepared(unsafe { inner(session) }, PluginValue::Bool(value != 0));
}

unsafe extern "C" fn module_push_f64(session: *mut c_void, value: f64) {
    push_prepared(unsafe { inner(session) }, PluginValue::F64(value));
}

unsafe extern "C" fn module_push_str(session: *mut c_void, text: *const c_char, len: usize) {
    let text = unsafe { utf8_of(text, len) }.unwrap_or("").to_owned();
    push_prepared(unsafe { inner(session) }, PluginValue::Str(text));
}

unsafe extern "C" fn module_push_bytes(session: *mut c_void, bytes: *const u8, len: usize) {
    let bytes = if bytes.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes, len).to_vec() }
    };
    push_prepared(unsafe { inner(session) }, PluginValue::Bytes(bytes));
}

unsafe extern "C" fn module_begin_list(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.begin(false);
    }
}

unsafe extern "C" fn module_end_list(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.end();
    }
}

unsafe extern "C" fn module_begin_map(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.begin(true);
    }
}

unsafe extern "C" fn module_map_key(session: *mut c_void, text: *const c_char, len: usize) {
    let text = unsafe { utf8_of(text, len) }.unwrap_or("").to_owned();
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut()
        && let Some(frame) = call.args.frames.last_mut()
    {
        frame.pending_key = Some(text);
    }
}

unsafe extern "C" fn module_end_map(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.end();
    }
}

unsafe extern "C" fn module_set_token(session: *mut c_void, token: u64) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.token = token;
    }
}

unsafe extern "C" fn module_finish(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(call) = inner.prepared.borrow_mut().take() else {
        return -1;
    };
    let args = call.args.stack;
    if call.wait == HostWait::Sync {
        let result = session_call(inner, &call.module, &call.method);
        if let PluginResult::Done(value) = result {
            emit_value(inner.host, &value);
            return 0;
        }
        return -1;
    }
    let token = call.token;
    let module = call.module.clone();
    let method = call.method.clone();
    if call.place == HostPlace::Session {
        let result = session_call(inner, &module, &method);
        inner.pending.borrow_mut().plugin.push((token, result));
        return 1;
    }
    submit_or_queue(
        inner,
        IoJob::Plugin {
            token,
            name: module.clone(),
            method: method.clone(),
            args: args.clone(),
        },
        || {
            let result = inline_platform(inner, &module, &method, &args);
            inner.pending.borrow_mut().plugin.push((token, result));
        },
    );
    1
}

fn session_call(inner: &Inner, module: &str, method: &str) -> PluginResult {
    match (module, method) {
        ("AppState", "getCurrentState") => {
            PluginResult::Done(PluginValue::Str(inner.app_state.borrow().clone()))
        }
        ("Linking", "getInitialURL") => {
            let value = match inner.launch_url.borrow().clone() {
                Some(url) => PluginValue::Str(url),
                None => PluginValue::Null,
            };
            PluginResult::Done(value)
        }
        _ => PluginResult::Failed {
            message: module_not_linked(module, Some(method)),
        },
    }
}

fn inline_platform(
    inner: &Inner,
    module: &str,
    method: &str,
    args: &[PluginValue],
) -> PluginResult {
    match (module, method) {
        ("Clipboard", "getString") => {
            PluginResult::Done(PluginValue::Str(inner.module_clipboard.borrow().clone()))
        }
        ("Clipboard", "setString") => {
            *inner.module_clipboard.borrow_mut() = first_string(args);
            PluginResult::Done(PluginValue::Null)
        }
        ("Linking", "openURL") => PluginResult::Done(PluginValue::Null),
        ("FileSystem", "documentDirectory") => {
            PluginResult::Done(PluginValue::Str("memory://documents".to_owned()))
        }
        ("FileSystem", "read") => match session::document_relative(&first_string(args)) {
            Ok(path) => match inner.memory_files.borrow().get(&path) {
                Some(bytes) => PluginResult::Done(PluginValue::Bytes(bytes.clone())),
                None => PluginResult::Failed {
                    message: "file is missing".to_owned(),
                },
            },
            Err(message) => PluginResult::Failed { message },
        },
        ("FileSystem", "write") => match session::document_relative(&first_string(args)) {
            Ok(path) => {
                let bytes = match args.get(1) {
                    Some(PluginValue::Bytes(bytes)) => bytes.clone(),
                    Some(PluginValue::Str(text)) => text.as_bytes().to_vec(),
                    _ => Vec::new(),
                };
                inner.memory_files.borrow_mut().insert(path, bytes);
                PluginResult::Done(PluginValue::Null)
            }
            Err(message) => PluginResult::Failed { message },
        },
        ("FileSystem", "delete") => match session::document_relative(&first_string(args)) {
            Ok(path) => {
                inner.memory_files.borrow_mut().remove(&path);
                PluginResult::Done(PluginValue::Null)
            }
            Err(message) => PluginResult::Failed { message },
        },
        ("FileSystem", "pick") => PluginResult::Done(PluginValue::Null),
        ("AsyncStorage", "multiGet") => {
            let keys = string_list(args.first());
            let store = inner.memory_store.borrow();
            let rows = keys
                .into_iter()
                .map(|key| {
                    let value = store
                        .get(&key)
                        .cloned()
                        .map(PluginValue::Str)
                        .unwrap_or(PluginValue::Null);
                    PluginValue::List(vec![PluginValue::Str(key), value])
                })
                .collect();
            PluginResult::Done(PluginValue::List(rows))
        }
        ("AsyncStorage", "multiSet") => {
            let mut store = inner.memory_store.borrow_mut();
            if let Some(PluginValue::List(rows)) = args.first() {
                for row in rows {
                    if let PluginValue::List(pair) = row
                        && let (Some(PluginValue::Str(key)), Some(PluginValue::Str(value))) =
                            (pair.first(), pair.get(1))
                    {
                        store.insert(key.clone(), value.clone());
                    }
                }
            }
            PluginResult::Done(PluginValue::Null)
        }
        ("AsyncStorage", "multiRemove") => {
            let mut store = inner.memory_store.borrow_mut();
            for key in string_list(args.first()) {
                store.remove(&key);
            }
            PluginResult::Done(PluginValue::Null)
        }
        ("AsyncStorage", "clear") => {
            inner.memory_store.borrow_mut().clear();
            PluginResult::Done(PluginValue::Null)
        }
        ("Permissions", "checkPermissionStatus") => {
            PluginResult::Done(PluginValue::Str("denied".to_owned()))
        }
        ("Permissions", "requestPermissions") => {
            let rows = string_list(args.first())
                .into_iter()
                .map(|name| (name, PluginValue::Str("denied".to_owned())))
                .collect();
            PluginResult::Done(PluginValue::Map(rows))
        }
        ("Permissions", "shouldShowRequestPermissionRationale") => {
            PluginResult::Done(PluginValue::Bool(false))
        }
        ("Permissions", "checkServiceStatus") => {
            PluginResult::Done(PluginValue::Str("notApplicable".to_owned()))
        }
        ("Permissions", "openAppSettings") => PluginResult::Done(PluginValue::Bool(false)),
        _ => PluginResult::Failed {
            message: module_not_linked(module, Some(method)),
        },
    }
}

fn string_list(value: Option<&PluginValue>) -> Vec<String> {
    let Some(PluginValue::List(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            PluginValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

unsafe extern "C" fn socket_open(session: *mut c_void, id: u64, url: *const c_char, len: usize) {
    let inner = unsafe { inner(session) };
    let Some(url) = (unsafe { utf8_of(url, len) }) else {
        return;
    };
    let url = url.to_owned();
    submit_or_queue(
        inner,
        IoJob::SocketOpen {
            id,
            url: url.clone(),
        },
        || {
            inner.pending.borrow_mut().socket.push((
                id,
                "error".to_owned(),
                true,
                b"socket is not linked".to_vec(),
            ));
        },
    );
}

unsafe extern "C" fn socket_send(
    session: *mut c_void,
    id: u64,
    data: *const u8,
    len: usize,
    is_text: i32,
) {
    let inner = unsafe { inner(session) };
    let bytes = if data.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data, len).to_vec() }
    };
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::SocketSend {
            id,
            text: is_text != 0,
            data: bytes,
        });
    }
}

unsafe extern "C" fn socket_close(session: *mut c_void, id: u64) {
    let inner = unsafe { inner(session) };
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::SocketClose { id });
    }
}
