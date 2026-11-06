//! Interpreter function table and C entry points.

use std::ffi::{c_char, c_void};

#[repr(C)]
#[allow(
    dead_code,
    reason = "the interpreter fills every field; Rust does not read each one"
)]
pub(super) struct QjsFns {
    pub(super) session: *mut c_void,
    pub(super) log: unsafe extern "C" fn(*mut c_void, *const c_char, usize, i32),
    pub(super) http: unsafe extern "C" fn(
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
    pub(super) http_stream: unsafe extern "C" fn(
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
    pub(super) abort_token: unsafe extern "C" fn(*mut c_void, u64),
    pub(super) module_has: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    pub(super) module_prepare:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const c_char, usize) -> i32,
    pub(super) socket_open:
        unsafe extern "C" fn(*mut c_void, u64, *const c_char, usize, *const c_char, usize),
    pub(super) socket_send: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize, i32),
    pub(super) socket_close: unsafe extern "C" fn(*mut c_void, u64),
    pub(super) push_mutation: unsafe extern "C" fn(
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
    /// Frees one zero-copy payload the session kept alive for QuickJS.
    pub(super) release_bytes: unsafe extern "C" fn(*mut c_void, u64),
    pub(super) module_push_null: unsafe extern "C" fn(*mut c_void),
    pub(super) module_push_bool: unsafe extern "C" fn(*mut c_void, i32),
    pub(super) module_push_f64: unsafe extern "C" fn(*mut c_void, f64),
    pub(super) module_push_str: unsafe extern "C" fn(*mut c_void, *const c_char, usize),
    pub(super) module_push_bytes: unsafe extern "C" fn(*mut c_void, *const u8, usize),
    pub(super) module_begin_list: unsafe extern "C" fn(*mut c_void),
    pub(super) module_end_list: unsafe extern "C" fn(*mut c_void),
    pub(super) module_begin_map: unsafe extern "C" fn(*mut c_void),
    pub(super) module_map_key: unsafe extern "C" fn(*mut c_void, *const c_char, usize),
    pub(super) module_end_map: unsafe extern "C" fn(*mut c_void),
    pub(super) module_set_token: unsafe extern "C" fn(*mut c_void, u64),
    pub(super) module_finish: unsafe extern "C" fn(*mut c_void) -> i32,
    pub(super) module_error: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const c_char,
    pub(super) module_count: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> i32,
    pub(super) module_method_at:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, i32, *mut usize) -> *const c_char,
    pub(super) module_name_of:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, *mut usize) -> *const c_char,
    pub(super) list_create: unsafe extern "C" fn(*mut c_void, i32, f32, *mut u32, *mut u32) -> i32,
    pub(super) list_release: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    pub(super) list_bind: unsafe extern "C" fn(*mut c_void, u32, u32, *const u64, i32) -> i32,
    pub(super) list_configure: unsafe extern "C" fn(*mut c_void, u32, u32, f32, i32, i32) -> i32,
    pub(super) list_begin: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    pub(super) list_item: unsafe extern "C" fn(
        *mut c_void,
        u32,
        u32,
        *const c_char,
        usize,
        *const c_char,
        usize,
        i32,
    ) -> i32,
    pub(super) list_end: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    pub(super) list_note_row:
        unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, usize, u64) -> i32,
    pub(super) scroll_create: unsafe extern "C" fn(*mut c_void, *mut u32, *mut u32) -> i32,
    pub(super) scroll_release: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    pub(super) scroll_bind: unsafe extern "C" fn(*mut c_void, u32, u32, u64, u64) -> i32,
    pub(super) scroll_configure:
        unsafe extern "C" fn(*mut c_void, u32, u32, f32, i32, f32, i32, f32, u32, u32, i32) -> i32,
    pub(super) scroll_drag_by: unsafe extern "C" fn(
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
    pub(super) scroll_finish: unsafe extern "C" fn(*mut c_void, u32, u32, i32) -> i32,
    pub(super) scroll_drag_end: unsafe extern "C" fn(*mut c_void, u32, u32) -> i32,
    pub(super) scroll_wheel: unsafe extern "C" fn(*mut c_void, u32, u32, f32, f32) -> i32,
    pub(super) scroll_query:
        unsafe extern "C" fn(*mut c_void, u32, u32, i32, *mut f32, *mut f32) -> i32,
    pub(super) scroll_consume: unsafe extern "C" fn(*mut c_void, u32, u32, f32, f32) -> i32,
    pub(super) debug_json: unsafe extern "C" fn(*mut c_void, i32, u64, f32, f32) -> i32,
    pub(super) debug_json_bytes: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    pub(super) debug_set_scan: unsafe extern "C" fn(*mut c_void, i32),
    pub(super) debug_set_pointer_trace: unsafe extern "C" fn(*mut c_void, i32, i32),
    pub(super) debug_set_owned: unsafe extern "C" fn(*mut c_void, u64, i32),
    pub(super) mutate:
        unsafe extern "C" fn(*mut c_void, *const super::mutation::MutRec, *mut u64) -> i32,
    pub(super) query: unsafe extern "C" fn(
        *mut c_void,
        i32,
        u64,
        f64,
        f64,
        *const c_char,
        usize,
        *const u8,
        *mut super::call::CallOut,
    ) -> i32,
    pub(super) effect: unsafe extern "C" fn(
        *mut c_void,
        i32,
        u64,
        *const f64,
        i32,
        *const u8,
        *const c_char,
        usize,
        *const u8,
        usize,
        *mut super::call::CallOut,
    ) -> i32,
    pub(super) fault: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        usize,
        *const c_char,
        usize,
    ),
}

unsafe extern "C" {
    pub(super) fn qjs_open(fns: *const QjsFns) -> *mut c_void;
    pub(super) fn qjs_eval(
        host: *mut c_void,
        src: *const c_char,
        len: usize,
        name: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_set_app_dir(host: *mut c_void, dir: *const c_char);
    pub(super) fn qjs_dispatch(
        host: *mut c_void,
        id: u64,
        kind: i32,
        a: f64,
        b: f64,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_dispatch_input(
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
    pub(super) fn qjs_dispatch_text(
        host: *mut c_void,
        id: u64,
        utf8: *const c_char,
        len: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_frame(
        host: *mut c_void,
        target_ms: f64,
        interval_ms: f64,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_layout(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    pub(super) fn qjs_driver(
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
    pub(super) fn qjs_scroll_state(
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
    pub(super) fn qjs_forget_ids(host: *mut c_void, ids: *const u64, len: usize);
    pub(super) fn qjs_take_wants_frame(host: *mut c_void) -> i32;
    pub(super) fn qjs_drain_jobs(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    pub(super) fn qjs_jobs_pending(host: *mut c_void) -> i32;
    pub(super) fn qjs_tap_count(host: *mut c_void) -> i32;
    pub(super) fn qjs_close(host: *mut c_void);
    pub(super) fn qjs_take_error(
        host: *mut c_void,
        message: *mut *mut c_char,
        stack: *mut *mut c_char,
    );
    pub(super) fn qjs_free(ptr: *mut c_void);
    pub(super) fn qjs_dispatch_back(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    pub(super) fn qjs_try_shortcut(
        host: *mut c_void,
        spec: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_apply_settings(
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
    pub(super) fn qjs_set_clock(host: *mut c_void, mono_ms: i64, paused: i32);
    pub(super) fn qjs_fire_timers(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    pub(super) fn qjs_fire_raf(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    pub(super) fn qjs_needs_frame(host: *mut c_void) -> i32;
    pub(super) fn qjs_next_deadline(host: *mut c_void) -> i64;
    pub(super) fn qjs_settle_http(
        host: *mut c_void,
        token: u64,
        status: i32,
        headers: *const c_char,
        headers_len: usize,
        body: *const u8,
        body_len: usize,
        error: *const c_char,
    ) -> i32;
    pub(super) fn qjs_http_head(
        host: *mut c_void,
        token: u64,
        status: i32,
        headers: *const c_char,
        headers_len: usize,
    ) -> i32;
    pub(super) fn qjs_http_chunk(
        host: *mut c_void,
        token: u64,
        body: *const u8,
        body_len: usize,
        done: i32,
        error: *const c_char,
    ) -> i32;
    pub(super) fn qjs_value_null(host: *mut c_void);
    pub(super) fn qjs_value_bool(host: *mut c_void, value: i32);
    pub(super) fn qjs_value_f64(host: *mut c_void, value: f64);
    pub(super) fn qjs_value_str(host: *mut c_void, text: *const c_char, len: usize);
    pub(super) fn qjs_value_bytes(host: *mut c_void, bytes: *const u8, len: usize);
    pub(super) fn qjs_value_bytes_external(
        host: *mut c_void,
        bytes: *const u8,
        len: usize,
        handle: u64,
    );
    pub(super) fn qjs_value_list(host: *mut c_void, count: u32);
    pub(super) fn qjs_value_map(host: *mut c_void, count: u32);
    pub(super) fn qjs_settle_top(host: *mut c_void, token: u64, ok: i32) -> i32;
    pub(super) fn qjs_motion_end(
        host: *mut c_void,
        node: u64,
        prop: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    pub(super) fn qjs_module_event(
        host: *mut c_void,
        module: *const c_char,
        event: *const c_char,
        shape: i32,
        text: *const c_char,
        len: usize,
    ) -> i32;
    pub(super) fn qjs_socket_event(
        host: *mut c_void,
        id: u64,
        event: *const c_char,
        is_text: i32,
        data: *const u8,
        len: usize,
    ) -> i32;
}
