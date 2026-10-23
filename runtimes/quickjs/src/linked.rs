//! Live QuickJS session. Tap handlers are JS values owned by the interpreter.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use protocol::{DrawList, TextAlign};
use session::{InputEvent, Key, Metrics, Session};
use ui_core::{Alignment, Justify, NodeId, Shaper, Tree};

use crate::Error;

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
    scroll: unsafe extern "C" fn(*mut c_void, *mut u64) -> i32,
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
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn qjs_frame(host: *mut c_void, time_ms: f64, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_take_wants_frame(host: *mut c_void) -> i32;
    fn qjs_drain_jobs(host: *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn qjs_jobs_pending(host: *mut c_void) -> i32;
    fn qjs_tap_count(host: *mut c_void) -> i32;
    fn qjs_close(host: *mut c_void);
}

struct Inner {
    tree: RefCell<Tree>,
    host: *mut c_void,
    log: Box<dyn Fn(u8, &str)>,
    ime_enabled: Cell<bool>,
    ime_text: RefCell<Option<String>>,
}

pub struct JsSession {
    inner: *mut Inner,
    last_frame: DrawList,
    script_error: RefCell<Option<String>>,
    _thread: PhantomData<Rc<()>>,
}

impl JsSession {
    pub fn open(
        script: &str,
        shaper: impl Shaper + 'static,
        log: Box<dyn Fn(u8, &str)>,
    ) -> Result<Self, Error> {
        let inner = Box::into_raw(Box::new(Inner {
            tree: RefCell::new(Tree::new(shaper)),
            host: ptr::null_mut(),
            log,
            ime_enabled: Cell::new(false),
            ime_text: RefCell::new(None),
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
            measure_text,
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
        let mut error = [0u8; 512];
        // SAFETY: `script` is borrowed for the call. The name is a NUL-terminated
        // literal. `error` is a writable buffer of the length we pass.
        let failed = unsafe {
            qjs_eval(
                host,
                script.as_ptr().cast(),
                script.len(),
                c"counter.js".as_ptr(),
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
        Ok(Self {
            inner,
            last_frame: DrawList {
                commands: Vec::new(),
            },
            script_error: RefCell::new(None),
            _thread: PhantomData,
        })
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
        inner.tree.borrow_mut().set_viewport(
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
        let gestures = inner.tree.borrow_mut().pointer(event);
        if let Err(message) = dispatch_gestures(inner, gestures) {
            self.note_script_error(message);
        }
    }

    fn input(&mut self, event: InputEvent) {
        // SAFETY: `inner` is owned by this session. The tree borrow ends before
        // the callback, which may re-enter the tree.
        let inner = unsafe { &*self.inner };
        let Some(id) = inner.tree.borrow().focused() else {
            return;
        };
        let (kind, text) = match event {
            InputEvent::Key(key) => (8, Some(key_name(key).to_string())),
            InputEvent::Text(text) => (9, Some(text)),
            InputEvent::Composing(text) => (10, text),
        };
        if let Err(message) = dispatch_input(inner, id, kind, text.as_deref()) {
            self.note_script_error(message);
        }
    }

    fn wants_ime(&self) -> bool {
        // SAFETY: `inner` is owned by this session. The flag is a `Cell`.
        let inner = unsafe { &*self.inner };
        inner.ime_enabled.get()
    }

    fn take_ime_text(&mut self) -> Option<String> {
        // SAFETY: `inner` is owned by this session.
        let inner = unsafe { &*self.inner };
        inner.ime_text.borrow_mut().take()
    }

    fn ime_dismissed(&mut self) {
        // SAFETY: `inner` is owned by this session. Blur runs before the callback.
        let inner = unsafe { &*self.inner };
        inner.ime_enabled.set(false);
        let gestures = inner.tree.borrow_mut().blur();
        if let Err(message) = dispatch_gestures(inner, gestures) {
            self.note_script_error(message);
        }
    }

    fn frame(&mut self, time_ms: f64) -> DrawList {
        // SAFETY: `inner` is owned by this session. Job drain and `qjs_frame` may
        // re-enter the tree through the `RefCell`; this function does not hold that borrow yet.
        let inner = unsafe { &*self.inner };
        if let Err(message) = drain_jobs(inner.host) {
            self.note_script_error(message);
            return self.last_frame.clone();
        }
        let mut error = [0u8; 512];
        let failed =
            unsafe { qjs_frame(inner.host, time_ms, error.as_mut_ptr().cast(), error.len()) };
        if failed != 0 {
            self.note_script_error(c_message(&error));
        }
        if let Err(message) = drain_jobs(inner.host) {
            self.note_script_error(message);
            return self.last_frame.clone();
        }
        let mut tree = inner.tree.borrow_mut();
        if tree.is_dirty() {
            if tree.layout().is_err() {
                return DrawList {
                    commands: Vec::new(),
                };
            }
            self.last_frame = tree.record().unwrap_or(DrawList {
                commands: Vec::new(),
            });
        }
        self.last_frame.clone()
    }

    fn wants_frame(&mut self) -> bool {
        // SAFETY: `inner` is owned by this session. The call reads and clears one flag.
        let inner = unsafe { &*self.inner };
        unsafe { qjs_take_wants_frame(inner.host) != 0 }
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

unsafe extern "C" fn insert(session: *mut c_void, parent: u64, child: u64, before: u64) -> i32 {
    let inner = unsafe { inner(session) };
    let before = (before != 0).then(|| NodeId::from_raw(before));
    status(inner.tree.borrow_mut().insert(
        NodeId::from_raw(parent),
        NodeId::from_raw(child),
        before,
    ))
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
    // Blur runs before `js_remove` frees the tap. A handler error is logged
    // and the detach still stands, so the id can be discarded.
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
            .set_align(NodeId::from_raw(node), alignment),
    )
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

unsafe extern "C" fn scroll(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().scroll(), out)
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
    }
}

fn dispatch_gestures(inner: &Inner, gestures: Vec<ui_core::Gesture>) -> Result<(), String> {
    for gesture in gestures {
        let mut error = [0u8; 512];
        let (id, kind, a, b) = gesture_parts(gesture);
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
            return Err(c_message(&error));
        }
    }
    Ok(())
}

fn dispatch_input(inner: &Inner, id: NodeId, kind: i32, text: Option<&str>) -> Result<(), String> {
    let mut error = [0u8; 512];
    let bytes = text.unwrap_or("");
    let ptr = if text.is_some() {
        bytes.as_ptr()
    } else {
        ptr::null()
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

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Backspace => "Backspace",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Left => "Left",
        Key::Right => "Right",
        Key::Up => "Up",
        Key::Down => "Down",
    }
}

unsafe extern "C" fn sized(session: *mut c_void, width: f32, height: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().sized(width, height), out)
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
    inner.ime_enabled.set(enabled != 0);
    0
}

unsafe extern "C" fn set_ime_text(session: *mut c_void, utf8: *const c_char, len: usize) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    *inner.ime_text.borrow_mut() = Some(text.to_owned());
    0
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
