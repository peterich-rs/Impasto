//! Tree-building callbacks the interpreter calls through QjsFns.

use std::ffi::{c_char, c_void};

use protocol::TextAlign;
use uicore::{Alignment, Edges, Justify, NodeId, RowCrossAlign, TrackSize};

use super::ffi::*;
use super::*;

pub(super) unsafe extern "C" fn column(session: *mut c_void, gap: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().column(gap), out)
}

pub(super) unsafe extern "C" fn row(session: *mut c_void, gap: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().row(gap), out)
}

pub(super) unsafe extern "C" fn stack(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().stack(), out)
}

pub(super) fn alignment_of(align: u32) -> Option<Alignment> {
    match align {
        0 => Some(Alignment::Start),
        1 => Some(Alignment::Center),
        2 => Some(Alignment::End),
        _ => None,
    }
}

pub(super) unsafe extern "C" fn align(session: *mut c_void, align: u32, out: *mut u64) -> i32 {
    let Some(alignment) = alignment_of(align) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().align(alignment), out)
}

pub(super) unsafe extern "C" fn padding(session: *mut c_void, edges: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().padding(edges), out)
}

pub(super) unsafe extern "C" fn pointer_region(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().pointer_region(), out)
}

pub(super) unsafe extern "C" fn decorated(
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

pub(super) unsafe extern "C" fn text(
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

pub(super) unsafe extern "C" fn append(session: *mut c_void, parent: u64, child: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .add_child(NodeId::from_raw(parent), NodeId::from_raw(child)),
    )
}

pub(super) fn logged_status(inner: &Inner, result: Result<(), uicore::Error>, what: &str) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            (inner.log)(3, &format!("{what}: {error}"));
            -1
        }
    }
}

pub(super) unsafe extern "C" fn insert(
    session: *mut c_void,
    parent: u64,
    child: u64,
    before: u64,
) -> i32 {
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

pub(super) unsafe extern "C" fn detach(session: *mut c_void, parent: u64, child: u64) -> i32 {
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

pub(super) unsafe extern "C" fn remove(session: *mut c_void, parent: u64, child: u64) -> i32 {
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

pub(super) unsafe extern "C" fn discard(
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

pub(super) unsafe extern "C" fn free_ids(ids: *mut u64, len: usize) {
    if ids.is_null() {
        return;
    }
    // SAFETY: `ids` and `len` came from `discard`, which leaked a `Box<[u64]>`.
    let slice = ptr::slice_from_raw_parts_mut(ids, len);
    drop(unsafe { Box::from_raw(slice) });
}

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

pub(super) unsafe extern "C" fn set_text(
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

pub(super) unsafe extern "C" fn set_font_size(
    session: *mut c_void,
    node: u64,
    font_size: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_font_size(NodeId::from_raw(node), font_size),
    )
}

pub(super) unsafe extern "C" fn set_color(
    session: *mut c_void,
    node: u64,
    color: *const u8,
) -> i32 {
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

pub(super) unsafe extern "C" fn set_gap(session: *mut c_void, node: u64, gap: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_gap(NodeId::from_raw(node), gap))
}

pub(super) unsafe extern "C" fn set_padding(session: *mut c_void, node: u64, edges: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_padding(NodeId::from_raw(node), edges),
    )
}

pub(super) unsafe extern "C" fn clip(session: *mut c_void, radius: f32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().clip(radius), out)
}

pub(super) unsafe extern "C" fn opacity(session: *mut c_void, alpha: u8, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().opacity(alpha), out)
}

pub(super) unsafe extern "C" fn set_grow(session: *mut c_void, node: u64, grow: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_grow(NodeId::from_raw(node), grow),
    )
}

pub(super) unsafe extern "C" fn image(
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

pub(super) unsafe extern "C" fn set_radius(session: *mut c_void, node: u64, radius: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_radius(NodeId::from_raw(node), radius),
    )
}

pub(super) unsafe extern "C" fn set_alpha(session: *mut c_void, node: u64, alpha: u8) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_alpha(NodeId::from_raw(node), alpha),
    )
}

pub(super) unsafe extern "C" fn set_align(session: *mut c_void, node: u64, align: u32) -> i32 {
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

pub(super) fn cross_of(align: u32) -> Option<RowCrossAlign> {
    match align {
        0 => Some(RowCrossAlign::Start),
        1 => Some(RowCrossAlign::Center),
        2 => Some(RowCrossAlign::End),
        3 => Some(RowCrossAlign::Stretch),
        4 => Some(RowCrossAlign::Baseline),
        _ => None,
    }
}

pub(super) unsafe extern "C" fn set_cross(session: *mut c_void, node: u64, align: u32) -> i32 {
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

pub(super) unsafe extern "C" fn set_size(
    session: *mut c_void,
    node: u64,
    width: f32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_size(NodeId::from_raw(node), width, height),
    )
}

pub(super) unsafe extern "C" fn set_image(
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

pub(super) unsafe extern "C" fn set_max_lines(session: *mut c_void, node: u64, lines: u32) -> i32 {
    let inner = unsafe { inner(session) };
    let lines = if lines == 0 { None } else { Some(lines) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_max_lines(NodeId::from_raw(node), lines),
    )
}

pub(super) unsafe extern "C" fn set_ellipsis(
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

pub(super) unsafe extern "C" fn set_text_align(session: *mut c_void, node: u64, align: u32) -> i32 {
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

pub(super) unsafe extern "C" fn set_font_weight(
    session: *mut c_void,
    node: u64,
    weight: u32,
) -> i32 {
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

pub(super) unsafe extern "C" fn set_text_direction(
    session: *mut c_void,
    node: u64,
    rtl: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_text_direction(NodeId::from_raw(node), rtl != 0),
    )
}

pub(super) unsafe extern "C" fn set_line_height(
    session: *mut c_void,
    node: u64,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_line_height(NodeId::from_raw(node), height),
    )
}

pub(super) unsafe extern "C" fn set_family(
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

pub(super) unsafe extern "C" fn scroll(session: *mut c_void, axis: i32, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(
        inner
            .tree
            .borrow_mut()
            .scroll_with(uicore::ScrollAxis::from_code(axis)),
        out,
    )
}

pub(super) unsafe extern "C" fn set_scroll_axis(session: *mut c_void, node: u64, axis: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_axis(NodeId::from_raw(node), uicore::ScrollAxis::from_code(axis)),
    )
}

pub(super) unsafe extern "C" fn set_scroll_offset(
    session: *mut c_void,
    node: u64,
    dx: f32,
    dy: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_offset(NodeId::from_raw(node), dx, dy),
    )
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

pub(super) unsafe extern "C" fn set_soft_wrap(session: *mut c_void, node: u64, wrap: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_soft_wrap(NodeId::from_raw(node), wrap != 0),
    )
}

pub(super) unsafe extern "C" fn set_overflow(
    session: *mut c_void,
    node: u64,
    overflow: i32,
) -> i32 {
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

pub(super) unsafe extern "C" fn set_scroll_chain(
    session: *mut c_void,
    node: u64,
    chain: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_scroll_chain(NodeId::from_raw(node), chain != 0),
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

pub(super) struct PendingMutation {
    op: u32,
    id: u64,
    parent: u64,
    before: u64,
    field: u32,
    number: f64,
    number2: f64,
    color: [u8; 4],
    text: String,
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

pub(super) fn flush_mutations(inner: &Inner) -> bool {
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

pub(super) fn apply_mutation(
    tree: &mut Tree,
    mutation: &PendingMutation,
) -> Result<(), uicore::Error> {
    use crate::abi::{MutationOp, StyleField};
    let op = mutation.op;
    let id = NodeId::from_raw(mutation.id);
    if op == MutationOp::SetText as u32 {
        return tree.set_text(id, mutation.text.clone());
    }
    if op == MutationOp::Create as u32 {
        // field picks the primitive; number/number2 carry its sizes.
        let created = match mutation.field {
            1 => tree.column(mutation.number as f32)?,
            2 => tree.row(mutation.number as f32)?,
            3 => tree.stack()?,
            4 => tree.padding(mutation.number as f32)?,
            5 => tree.sized(mutation.number as f32, mutation.number2 as f32)?,
            6 => tree.scroll()?,
            _ => return Err(uicore::Error::WrongKind),
        };
        if mutation.parent != 0 {
            tree.add_child(NodeId::from_raw(mutation.parent), created)?;
        }
        return Ok(());
    }
    if op == MutationOp::Insert as u32 {
        let parent = NodeId::from_raw(mutation.parent);
        let before = (mutation.before != 0).then_some(NodeId::from_raw(mutation.before));
        return tree.insert(parent, id, before);
    }
    if op == MutationOp::Remove as u32 {
        let gestures = tree.remove(NodeId::from_raw(mutation.parent), id)?;
        let _ = gestures;
        return Ok(());
    }
    if op == MutationOp::Discard as u32 {
        let discarded = tree.discard(id)?;
        let _ = discarded;
        return Ok(());
    }
    if op == MutationOp::EditText as u32 {
        // number picks the edit: 0 commit composition, 1 insert text, 2
        // backspace, 3 forward delete. The buffer path mirrors the JS session.
        let edit = match mutation.number as i32 {
            0 => uicore::Edit::Composing(None),
            1 => uicore::Edit::Insert(mutation.text.clone()),
            2 => uicore::Edit::Backspace,
            3 => uicore::Edit::ForwardDelete,
            _ => return Err(uicore::Error::BadNumber),
        };
        return tree.text_edit(id, edit).map(|_| ());
    }
    if op != MutationOp::SetStyle as u32 {
        return Ok(());
    }
    // Style mutations: field picks the target, number/number2/color/text the
    // value. Edges pack top/right into number/number2; colors take all four
    // channels.
    let value = mutation.number as f32;
    match mutation.field {
        field if field == StyleField::Width as u32 => {
            tree.set_axis_size(id, uicore::AxisSize::Points(value), uicore::AxisSize::Auto)
        }
        field if field == StyleField::Height as u32 => {
            tree.set_axis_size(id, uicore::AxisSize::Auto, uicore::AxisSize::Points(value))
        }
        field if field == StyleField::Padding as u32 => {
            tree.set_padding_edges(id, Edges::all(value))
        }
        field if field == StyleField::Margin as u32 => tree.set_margin(id, Edges::all(value)),
        field if field == StyleField::Gap as u32 => tree.set_gap(id, value),
        field if field == StyleField::Grow as u32 => tree.set_grow(id, value),
        field if field == StyleField::Shrink as u32 => tree.set_shrink(id, value),
        field if field == StyleField::Basis as u32 => tree.set_basis(id, value),
        field if field == StyleField::MinSize as u32 => {
            tree.set_min_size(id, value, mutation.number2 as f32)
        }
        field if field == StyleField::MaxSize as u32 => {
            tree.set_max_size(id, value, mutation.number2 as f32)
        }
        field if field == StyleField::Align as u32 => match alignment_of(mutation.number as u32) {
            Some(alignment) => tree.set_align_self(id, alignment),
            None => Err(uicore::Error::BadNumber),
        },
        field if field == StyleField::Justify as u32 => match justify_of(mutation.number as u32) {
            Some(justify) => tree.set_justify(id, justify),
            None => Err(uicore::Error::BadNumber),
        },
        field if field == StyleField::Wrap as u32 => tree.set_wrap(id, mutation.number > 0.5),
        field if field == StyleField::Inset as u32 => tree.set_inset(
            id,
            uicore::Inset {
                top: (mutation.number > 0.0).then_some(mutation.number as f32),
                right: (mutation.number2 > 0.0).then_some(mutation.number2 as f32),
                bottom: (mutation.color[0] > 0).then_some(f32::from(mutation.color[0])),
                left: (mutation.color[1] > 0).then_some(f32::from(mutation.color[1])),
            },
        ),
        field if field == StyleField::Grid as u32 => {
            // number/number2 are i16 line indices packed as two f32 lanes per
            // axis is not enough; mutations keep Auto until the buffer grows.
            tree.set_grid_placement(
                id,
                uicore::GridLine::Auto,
                uicore::GridLine::Auto,
                uicore::GridLine::Auto,
                uicore::GridLine::Auto,
            )
        }
        field if field == StyleField::FontSize as u32 => tree.set_font_size(id, value),
        field if field == StyleField::TextAlign as u32 => {
            tree.set_text_align(id, align_text(mutation.number as u32))
        }
        field if field == StyleField::Weight as u32 => {
            tree.set_font_weight(id, value.round().clamp(0.0, 1000.0) as u16)
        }
        field if field == StyleField::MaxLines as u32 => tree.set_max_lines(
            id,
            (mutation.number > 0.0).then_some(mutation.number.round() as u32),
        ),
        field if field == StyleField::LineHeight as u32 => tree.set_line_height(id, value),
        field if field == StyleField::Fill as u32 => tree.set_color(id, mutation.color),
        field if field == StyleField::Radius as u32 => tree.set_radius(id, value),
        field if field == StyleField::Stroke as u32 => tree.set_stroke(id, value, mutation.color),
        field if field == StyleField::Alpha as u32 => {
            tree.set_alpha(id, mutation.number.round().clamp(0.0, 255.0) as u8)
        }
        field if field == StyleField::Blur as u32 => tree.set_blur(id, value),
        _ => Ok(()),
    }
}

fn align_text(mode: u32) -> TextAlign {
    match mode {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

pub(super) unsafe extern "C" fn push_mutation(
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
    let inner = unsafe { inner(session) };
    let bytes = if text.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), text_len) }
    };
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut channels = [0u8; 4];
    if !color.is_null() {
        let raw = unsafe { std::slice::from_raw_parts(color, 4) };
        channels.copy_from_slice(raw);
    }
    inner.mutations.borrow_mut().push(PendingMutation {
        op,
        id,
        parent,
        before,
        field,
        number,
        number2,
        color: channels,
        text,
    });
    0
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

pub(super) unsafe extern "C" fn transform(session: *mut c_void, out: *mut u64) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().transform(), out)
}

pub(super) unsafe extern "C" fn set_transform(
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

pub(super) unsafe extern "C" fn set_justify(session: *mut c_void, node: u64, justify: u32) -> i32 {
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

pub(super) unsafe extern "C" fn set_wrap(session: *mut c_void, node: u64, wrap: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_wrap(NodeId::from_raw(node), wrap != 0),
    )
}

pub(super) unsafe extern "C" fn set_fill(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_fill(NodeId::from_raw(node)))
}

pub(super) fn parse_tracks(text: &str) -> Result<Vec<TrackSize>, uicore::Error> {
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

pub(super) unsafe extern "C" fn grid(
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

pub(super) unsafe extern "C" fn set_padding_edges(
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

pub(super) unsafe extern "C" fn set_margin(
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

pub(super) unsafe extern "C" fn set_min_size(
    session: *mut c_void,
    node: u64,
    width: f32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_min_size(NodeId::from_raw(node), width, height),
    )
}

pub(super) unsafe extern "C" fn set_max_size(
    session: *mut c_void,
    node: u64,
    width: f32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_max_size(NodeId::from_raw(node), width, height),
    )
}

pub(super) unsafe extern "C" fn set_shrink(session: *mut c_void, node: u64, shrink: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_shrink(NodeId::from_raw(node), shrink),
    )
}

pub(super) unsafe extern "C" fn set_basis(session: *mut c_void, node: u64, basis: f32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_basis(NodeId::from_raw(node), basis),
    )
}

pub(super) unsafe extern "C" fn set_align_self(session: *mut c_void, node: u64, align: u32) -> i32 {
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

pub(super) fn justify_of(value: u32) -> Option<Justify> {
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

pub(super) fn dispatch_gestures(
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
