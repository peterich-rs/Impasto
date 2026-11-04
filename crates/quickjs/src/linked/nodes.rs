//! Tree-building callbacks the interpreter calls through QjsFns.

use std::ffi::{c_char, c_void};

use uicore::{NodeId, TrackSize};

use super::ffi::*;
use super::*;

pub(super) unsafe extern "C" fn parent(session: *mut c_void, node: u64, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    queried(inner.tree.borrow().parent(NodeId::from_raw(node)), out)
}

pub(super) unsafe extern "C" fn first_child(session: *mut c_void, node: u64, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    queried(inner.tree.borrow().first_child(NodeId::from_raw(node)), out)
}

pub(super) unsafe extern "C" fn next_sibling(
    session: *mut c_void,
    node: u64,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    queried(
        inner.tree.borrow().next_sibling(NodeId::from_raw(node)),
        out,
    )
}

pub(super) unsafe extern "C" fn is_text(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().is_text(NodeId::from_raw(node)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

pub(super) unsafe extern "C" fn viewport(
    session: *mut c_void,
    width: *mut f32,
    height: *mut f32,
) -> i32 {
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

pub(super) unsafe extern "C" fn text_edit(
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
        0 => uicore::Edit::Load(extra),
        1 => uicore::Edit::Left,
        2 => uicore::Edit::Right,
        3 => uicore::Edit::Backspace,
        4 => uicore::Edit::Insert(extra),
        5 => uicore::Edit::Composing(if extra.is_empty() { None } else { Some(extra) }),
        6 => uicore::Edit::Paste(extra),
        7 => uicore::Edit::Undo,
        8 => uicore::Edit::Redo,
        9 => uicore::Edit::SelectAll,
        10 => uicore::Edit::Copy,
        11 => uicore::Edit::Cut,
        12 => uicore::Edit::SetCaret(extra.parse::<u32>().unwrap_or(0)),
        13 => uicore::Edit::ForwardDelete,
        14 => uicore::Edit::Newline,
        15 => uicore::Edit::WordLeft,
        16 => uicore::Edit::WordRight,
        17 => uicore::Edit::LineStart,
        18 => uicore::Edit::LineEnd,
        19 => uicore::Edit::ExtendLeft,
        20 => uicore::Edit::ExtendRight,
        21 => uicore::Edit::ExtendWordLeft,
        22 => uicore::Edit::ExtendWordRight,
        23 => uicore::Edit::ExtendLineStart,
        24 => uicore::Edit::ExtendLineEnd,
        25 => uicore::Edit::Up,
        26 => uicore::Edit::Down,
        27 => uicore::Edit::ExtendUp,
        28 => uicore::Edit::ExtendDown,
        29 => uicore::Edit::ExtendCaret(extra.parse::<u32>().unwrap_or(0)),
        30 => uicore::Edit::DeleteWordBackward,
        31 => uicore::Edit::DeleteWordForward,
        32 => uicore::Edit::DeleteToLineStart,
        33 => uicore::Edit::DeleteToLineEnd,
        34 => uicore::Edit::DocumentStart,
        35 => uicore::Edit::DocumentEnd,
        36 => uicore::Edit::ExtendDocumentStart,
        37 => uicore::Edit::ExtendDocumentEnd,
        38 => uicore::Edit::Commit(extra),
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

pub(super) unsafe extern "C" fn caret_at_unit(
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

pub(super) unsafe extern "C" fn selection_rects(
    session: *mut c_void,
    node: u64,
    start: u32,
    end: u32,
) -> i32 {
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

pub(super) unsafe extern "C" fn selection_rect_bytes(
    session: *mut c_void,
    out_len: *mut usize,
) -> *const u8 {
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

pub(super) unsafe extern "C" fn set_ime_config(
    session: *mut c_void,
    keyboard: i32,
    action: i32,
) -> i32 {
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

pub(super) unsafe extern "C" fn set_text_scale(session: *mut c_void, scale: f32) -> i32 {
    let inner = unsafe { inner(session) };
    inner.text_scale.set(scale);
    status(inner.tree.borrow_mut().set_text_scale(scale))
}

pub(super) unsafe extern "C" fn text_edit_bytes(
    session: *mut c_void,
    out_len: *mut usize,
) -> *const u8 {
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

pub(super) fn snap_grid(interval: f32, align: i32) -> Option<uicore::SnapGrid> {
    if !(interval.is_finite() && interval > 0.5) {
        return None;
    }
    let align = match align {
        1 => uicore::SnapAlign::Center,
        2 => uicore::SnapAlign::End,
        _ => uicore::SnapAlign::Start,
    };
    Some(uicore::SnapGrid { interval, align })
}

pub(super) unsafe extern "C" fn set_scroll_physics(
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
    curve: i32,
    snap_stiffness: f32,
    snap_damping: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let physics = uicore::ScrollPhysics {
        decay: if curve == 2 {
            // The caller spelled the coast as a deceleration rate; `tau`
            // carries the rate and converts on construction.
            uicore::Decay::Rate(f64::from(tau))
        } else {
            uicore::Decay::Tau(f64::from(tau))
        },
        band: if curve == 1 {
            uicore::Band::Curve {
                c: f64::from(factor) as f32,
            }
        } else {
            uicore::Band::Linear { factor }
        },
        overscroll: if mode == 1 {
            uicore::OverscrollMode::Clamp
        } else {
            uicore::OverscrollMode::RubberBand
        },
        overscroll_stiffness: stiffness,
        overscroll_damping: damping,
        overscroll_factor: factor,
        snap_stiffness,
        snap_damping,
        snap_x: snap_grid(snap_x, align_x),
        snap_y: snap_grid(snap_y, align_y),
        accept: uicore::ScrollAccept::from_code(accept),
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_physics(NodeId::from_raw(node), physics),
    )
}

pub(super) unsafe extern "C" fn overscroll(
    session: *mut c_void,
    node: u64,
    x: *mut f32,
    y: *mut f32,
) -> i32 {
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

pub(super) unsafe extern "C" fn arm_fling(
    session: *mut c_void,
    node: u64,
    vx: f32,
    vy: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .arm_fling(NodeId::from_raw(node), vx, vy),
    )
}

pub(super) unsafe extern "C" fn stop_scroll(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().stop_scroll(NodeId::from_raw(node)))
}

pub(super) unsafe extern "C" fn scroll_to(
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

pub(super) unsafe extern "C" fn scroll_offset(
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

pub(super) fn sweep_detached(inner: &Inner) {
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

pub(super) unsafe extern "C" fn rect(
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
    if mutation::flush_for_read(inner) != 0 {
        return -1;
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

pub(in crate::linked) fn parse_tracks(text: &str) -> Result<Vec<TrackSize>, uicore::Error> {
    if text.is_empty() {
        return Err(uicore::Error::BadNumber);
    }
    text.split(',').map(parse_track).collect()
}

pub(super) fn parse_track(token: &str) -> Result<TrackSize, uicore::Error> {
    let token = token.trim();
    match token {
        "min" => Ok(TrackSize::MinContent),
        "max" => Ok(TrackSize::MaxContent),
        other if let Some(value) = other.strip_suffix("fr") => {
            let value = value.parse::<f32>().map_err(|_| uicore::Error::BadNumber)?;
            if !value.is_finite() || value < 0.0 {
                return Err(uicore::Error::BadNumber);
            }
            Ok(TrackSize::Fraction(value))
        }
        other => {
            let value = other.parse::<f32>().map_err(|_| uicore::Error::BadNumber)?;
            if !value.is_finite() || value < 0.0 {
                return Err(uicore::Error::BadNumber);
            }
            Ok(TrackSize::Points(value))
        }
    }
}

pub(super) unsafe extern "C" fn log_message(
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

pub(super) fn gesture_parts(gesture: uicore::Gesture) -> (NodeId, i32, f64, f64) {
    match gesture {
        uicore::Gesture::Click(id) => (id, 0, 0.0, 0.0),
        uicore::Gesture::Press(id) => (id, 1, 0.0, 0.0),
        uicore::Gesture::Release(id) => (id, 2, 0.0, 0.0),
        uicore::Gesture::DragBegin(id) => (id, 3, 0.0, 0.0),
        uicore::Gesture::DragBy(id, dx, dy) => (id, 4, f64::from(dx), f64::from(dy)),
        uicore::Gesture::DragEnd(id, vx, vy) => (id, 5, f64::from(vx), f64::from(vy)),
        uicore::Gesture::Focus(id) => (id, 6, 0.0, 0.0),
        uicore::Gesture::Blur(id) => (id, 7, 0.0, 0.0),
        uicore::Gesture::Hover(id) => (id, 11, 0.0, 0.0),
        uicore::Gesture::Leave(id) => (id, 12, 0.0, 0.0),
        uicore::Gesture::Wheel(id, dx, dy) => (id, 13, f64::from(dx), f64::from(dy)),
        uicore::Gesture::SecondaryClick(id) => (id, 14, 0.0, 0.0),
        uicore::Gesture::Scroll(id, x, y) => (id, 15, f64::from(x), f64::from(y)),
        uicore::Gesture::TextReplaced(id) => (id, 16, 0.0, 0.0),
        uicore::Gesture::ActivateLink { id, link } => (id, 17, f64::from(link), 0.0),
        uicore::Gesture::LongPress(id) => (id, 18, 0.0, 0.0),
        uicore::Gesture::DoubleTap(id) => (id, 19, 0.0, 0.0),
        uicore::Gesture::Zoom { id, scale } => (id, 20, f64::from(scale), 0.0),
        uicore::Gesture::ScrollPhaseChanged { id, .. } => (id, 21, 0.0, 0.0),
    }
}

pub(in crate::linked) fn dispatch_gestures(
    inner: &Inner,
    gestures: Vec<uicore::Gesture>,
) -> Result<(), String> {
    for gesture in gestures {
        note_gesture(&inner.marks, &gesture);
        match gesture {
            uicore::Gesture::TextReplaced(id) => {
                let text = inner.tree.borrow().editor_text(id).unwrap_or_default();
                dispatch_text(inner, id, &text)?;
                continue;
            }
            uicore::Gesture::ActivateLink { id, link } => {
                dispatch_kind(inner, id, 17, f64::from(link), 0.0)?;
                continue;
            }
            uicore::Gesture::ScrollPhaseChanged {
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

pub(super) fn dispatch_scroll_phase(
    inner: &Inner,
    id: NodeId,
    phase: uicore::ScrollPhase,
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

pub(super) fn dispatch_kind(
    inner: &Inner,
    id: NodeId,
    kind: i32,
    a: f64,
    b: f64,
) -> Result<(), String> {
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

pub(super) fn dispatch_text(inner: &Inner, id: NodeId, text: &str) -> Result<(), String> {
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

pub(super) fn dispatch_input(
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

fn named_edit(name: &str, extra: String, record_undo: bool) -> Option<uicore::Edit> {
    let unit = |text: &str| text.parse::<u32>().unwrap_or(0);
    Some(match name {
        "insert" => uicore::Edit::Insert(extra),
        "backspace" => uicore::Edit::Backspace,
        "forwardDelete" => uicore::Edit::ForwardDelete,
        "newline" => uicore::Edit::Newline,
        "undo" => uicore::Edit::Undo,
        "redo" => uicore::Edit::Redo,
        "selectAll" => uicore::Edit::SelectAll,
        "caret" => uicore::Edit::SetCaret(unit(&extra)),
        "extendCaret" => uicore::Edit::ExtendCaret(unit(&extra)),
        "composing" => uicore::Edit::Composing(if extra.is_empty() { None } else { Some(extra) }),
        "commit" => uicore::Edit::Commit(extra),
        "load" => uicore::Edit::Load(extra),
        "replace" => uicore::Edit::Replace {
            text: extra,
            record_undo,
        },
        "left" => uicore::Edit::Left,
        "right" => uicore::Edit::Right,
        "extendLeft" => uicore::Edit::ExtendLeft,
        "extendRight" => uicore::Edit::ExtendRight,
        "wordLeft" => uicore::Edit::WordLeft,
        "wordRight" => uicore::Edit::WordRight,
        "extendWordLeft" => uicore::Edit::ExtendWordLeft,
        "extendWordRight" => uicore::Edit::ExtendWordRight,
        "lineStart" => uicore::Edit::LineStart,
        "lineEnd" => uicore::Edit::LineEnd,
        "extendLineStart" => uicore::Edit::ExtendLineStart,
        "extendLineEnd" => uicore::Edit::ExtendLineEnd,
        "up" => uicore::Edit::Up,
        "down" => uicore::Edit::Down,
        "extendUp" => uicore::Edit::ExtendUp,
        "extendDown" => uicore::Edit::ExtendDown,
        "documentStart" => uicore::Edit::DocumentStart,
        "documentEnd" => uicore::Edit::DocumentEnd,
        "extendDocumentStart" => uicore::Edit::ExtendDocumentStart,
        "extendDocumentEnd" => uicore::Edit::ExtendDocumentEnd,
        "deleteWord" => uicore::Edit::DeleteWordBackward,
        "deleteWordForward" => uicore::Edit::DeleteWordForward,
        "deleteToLineStart" => uicore::Edit::DeleteToLineStart,
        "deleteToLineEnd" => uicore::Edit::DeleteToLineEnd,
        "copy" => uicore::Edit::Copy,
        "cut" => uicore::Edit::Cut,
        "paste" => uicore::Edit::Paste(extra),
        _ => return None,
    })
}

pub(super) unsafe extern "C" fn text_command(
    session: *mut c_void,
    node: u64,
    name: *const c_char,
    name_len: usize,
    extra_ptr: *const c_char,
    extra_len: usize,
    record_undo: i32,
    sel_start: *mut u32,
    sel_end: *mut u32,
) -> i32 {
    if sel_start.is_null() || sel_end.is_null() {
        return -1;
    }
    let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
        return -1;
    };
    let extra = if extra_ptr.is_null() {
        String::new()
    } else {
        let Some(text) = (unsafe { utf8_of(extra_ptr, extra_len) }) else {
            return -1;
        };
        text.to_owned()
    };
    let Some(edit) = named_edit(name, extra, record_undo != 0) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    let Ok(value) = inner
        .tree
        .borrow_mut()
        .text_edit(NodeId::from_raw(node), edit)
    else {
        return -1;
    };
    unsafe {
        *sel_start = value.selection.base;
        *sel_end = value.selection.extent;
    }
    *inner.edit_bytes.borrow_mut() = value.text.into_bytes();
    0
}

pub(super) unsafe extern "C" fn place_panel(
    session: *mut c_void,
    panel: u64,
    anchor: u64,
    align: i32,
) -> i32 {
    let align = match align {
        1 => uicore::PanelAlign::BelowEnd,
        2 => uicore::PanelAlign::BelowCenter,
        3 => uicore::PanelAlign::AboveStart,
        _ => uicore::PanelAlign::BelowStart,
    };
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().place_panel(
        NodeId::from_raw(panel),
        NodeId::from_raw(anchor),
        align,
    ))
}
