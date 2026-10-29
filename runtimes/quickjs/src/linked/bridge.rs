//! Sized, animation, font, and host-module callbacks.

use std::collections::HashSet;
use std::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;

use session::{HttpError, HttpFailure, HttpRequest, Invoke, IoJob, PluginResult, PluginValue};
use ui_core::{AxisSize, DragClaim, FocusDirection, GridLine, Inset, NodeId};

use super::ffi::*;
use super::*;

pub(super) unsafe extern "C" fn sized(
    session: *mut c_void,
    width: f32,
    height: f32,
    out: *mut u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    created(inner.tree.borrow_mut().sized(width, height), out)
}

pub(super) unsafe extern "C" fn set_drag_claim(session: *mut c_void, node: u64, claim: i32) -> i32 {
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

pub(super) unsafe extern "C" fn caret_at(
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

pub(super) unsafe extern "C" fn word_range(
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

pub(super) unsafe extern "C" fn focus_step(session: *mut c_void, direction: i32) -> i32 {
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

pub(super) unsafe extern "C" fn push_focus_scope(session: *mut c_void, node: u64) -> i32 {
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

pub(super) unsafe extern "C" fn pop_focus_scope(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    inner.tree.borrow_mut().pop_focus_scope();
    0
}

pub(super) unsafe extern "C" fn pointer_position(
    session: *mut c_void,
    x: *mut f32,
    y: *mut f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let (px, py) = inner.tree.borrow().pointer_position();
    unsafe {
        *x = px;
        *y = py;
    }
    0
}

pub(super) unsafe extern "C" fn set_focusable(
    session: *mut c_void,
    node: u64,
    enabled: i32,
) -> i32 {
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

pub(super) unsafe extern "C" fn focus_node(session: *mut c_void, node: u64) -> i32 {
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

pub(super) unsafe extern "C" fn focused_id(session: *mut c_void) -> u64 {
    let inner = unsafe { inner(session) };
    inner
        .tree
        .borrow()
        .focused()
        .map(NodeId::to_raw)
        .unwrap_or(0)
}

pub(super) unsafe extern "C" fn is_focusable(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    match inner.tree.borrow().is_focusable(NodeId::from_raw(node)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

pub(super) unsafe extern "C" fn grapheme_index_of(
    utf8: *const c_char,
    len: usize,
    unit: u32,
) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_index(text, unit)
}

pub(super) unsafe extern "C" fn grapheme_count_of(utf8: *const c_char, len: usize) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_count(text)
}

pub(super) unsafe extern "C" fn grapheme_unit_of(
    utf8: *const c_char,
    len: usize,
    index: u32,
) -> u32 {
    let text = unsafe { utf8_of(utf8, len) }.unwrap_or("");
    ui_core::grapheme_unit(text, index)
}

pub(super) unsafe extern "C" fn set_reverse(session: *mut c_void, node: u64, reverse: i32) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_reverse(NodeId::from_raw(node), reverse != 0),
    )
}

pub(super) unsafe extern "C" fn set_scale_xy(
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

pub(super) unsafe extern "C" fn register_font(
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

pub(super) unsafe extern "C" fn reload(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
) -> i32 {
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

pub(super) unsafe extern "C" fn layer_stats(
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

pub(super) unsafe extern "C" fn scroll_into_view(
    session: *mut c_void,
    node: u64,
    edge: i32,
) -> i32 {
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

pub(super) unsafe extern "C" fn animate(
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

pub(super) fn anim_prop(name: &str) -> Option<ui_core::AnimProp> {
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

pub(super) unsafe extern "C" fn fling_target(
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

pub(super) unsafe extern "C" fn set_recognizers(
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

pub(super) unsafe extern "C" fn set_stroke(
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

pub(super) unsafe extern "C" fn set_image_mode(
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

pub(super) struct FontBook {
    queued: Vec<(String, Vec<u8>)>,
    handed: HashSet<String>,
}

impl FontBook {
    pub(super) fn new() -> Self {
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

    pub(super) fn take(&mut self) -> Vec<(String, Vec<u8>)> {
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

pub(super) fn startup_font(name: &str) -> bool {
    name == "UIFWSans" || name == "UIFWSansCJK"
}

pub(super) fn axis_from_tag(tag: i32, value: f32) -> Result<AxisSize, ui_core::Error> {
    match tag {
        0 => Ok(AxisSize::Auto),
        1 => Ok(AxisSize::Points(value)),
        2 => Ok(AxisSize::Fraction(value)),
        _ => Err(ui_core::Error::BadNumber),
    }
}

pub(super) fn inset_from_tag(is_auto: i32, value: f32) -> Option<f32> {
    if is_auto == 0 { Some(value) } else { None }
}

pub(super) fn grid_from_tag(tag: i32, value: i32) -> Result<GridLine, ui_core::Error> {
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

pub(super) unsafe extern "C" fn set_panel(session: *mut c_void, node: u64) -> i32 {
    let inner = unsafe { inner(session) };
    status(inner.tree.borrow_mut().set_panel(NodeId::from_raw(node)))
}

pub(super) unsafe extern "C" fn set_axis_size(
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

pub(super) unsafe extern "C" fn set_min_axis(
    session: *mut c_void,
    node: u64,
    width_tag: i32,
    width: f32,
    height_tag: i32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Ok(width) = axis_from_tag(width_tag, width) else {
        return -1;
    };
    let Ok(height) = axis_from_tag(height_tag, height) else {
        return -1;
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_min_axis(NodeId::from_raw(node), width, height),
    )
}

pub(super) unsafe extern "C" fn set_max_axis(
    session: *mut c_void,
    node: u64,
    width_tag: i32,
    width: f32,
    height_tag: i32,
    height: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Ok(width) = axis_from_tag(width_tag, width) else {
        return -1;
    };
    let Ok(height) = axis_from_tag(height_tag, height) else {
        return -1;
    };
    status(
        inner
            .tree
            .borrow_mut()
            .set_max_axis(NodeId::from_raw(node), width, height),
    )
}

/// `mode` 0 max-content, 1 capped-max, 2 definite width. `height` of `f32::NAN`
/// under mode 2 leaves the height natural. Out parameters carry `(w, h)`.
pub(super) unsafe extern "C" fn measure_panel(
    session: *mut c_void,
    node: u64,
    mode: i32,
    width: f32,
    height: f32,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    if out_w.is_null() || out_h.is_null() {
        return -1;
    }
    let space = match mode {
        0 => ui_core::SubtreeSpace::MaxContent,
        1 => ui_core::SubtreeSpace::CappedMax(width),
        2 => ui_core::SubtreeSpace::Definite {
            width,
            height: (height.is_finite()).then_some(height),
        },
        _ => return -1,
    };
    let inner = unsafe { inner(session) };
    // `set_panel` is a separate call so a caller can reuse a declared panel.
    match inner
        .tree
        .borrow_mut()
        .measure_panel(NodeId::from_raw(node), space)
    {
        Ok((w, h)) => {
            unsafe {
                *out_w = w;
                *out_h = h;
            }
            0
        }
        Err(_) => -1,
    }
}

pub(super) unsafe extern "C" fn set_aspect_ratio(
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

pub(super) unsafe extern "C" fn set_position(
    session: *mut c_void,
    node: u64,
    absolute: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    status(
        inner
            .tree
            .borrow_mut()
            .set_position(NodeId::from_raw(node), absolute != 0),
    )
}

pub(super) unsafe extern "C" fn set_inset(
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

pub(super) unsafe extern "C" fn set_grid_placement(
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

pub(super) unsafe extern "C" fn blur_node(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    let gestures = inner.tree.borrow_mut().blur();
    if dispatch_gestures(inner, gestures).is_err() {
        return -1;
    }
    0
}

pub(super) unsafe extern "C" fn set_ime_enabled(session: *mut c_void, enabled: i32) -> i32 {
    let inner = unsafe { inner(session) };
    let mut ime = inner.ime.borrow_mut();
    ime.shown = enabled != 0;
    ime.dirty = true;
    0
}

pub(super) unsafe extern "C" fn set_ime_text(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
        return -1;
    };
    let mut ime = inner.ime.borrow_mut();
    ime.state = session::ImeState::at_end(text);
    ime.dirty = true;
    0
}

pub(super) unsafe extern "C" fn set_ime_state(
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

pub(super) fn tri_state(value: i32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub(super) unsafe extern "C" fn set_semantics(
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

pub(super) fn role_of(role: i32) -> Option<ui_core::SemanticsRole> {
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

pub(super) unsafe extern "C" fn text_scale(session: *mut c_void) -> f64 {
    let inner = unsafe { inner(session) };
    f64::from(inner.text_scale.get())
}

pub(super) unsafe extern "C" fn reduce_motion(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(inner.reduce_motion.get())
}

pub(super) unsafe extern "C" fn measure_text(
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

pub(super) fn self_paused(inner: &Inner) -> bool {
    inner.paused.get()
}

pub(super) fn cstring(text: &str) -> CString {
    let mut bytes = text.replace('\0', "").into_bytes();
    bytes.push(0);
    // SAFETY: `replace` removed every interior NUL, and the last byte is the terminator.
    unsafe { CString::from_vec_with_nul_unchecked(bytes) }
}

pub(super) fn emit_module(inner: &Inner, module: &str, event: &str, shape: i32, text: &str) {
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

/// Payloads at or above this size travel zero-copy; smaller ones are cheaper
/// to duplicate than to track.
const EXTERNAL_BYTES_THRESHOLD: usize = 4096;

/// Park `bytes` in the session slab and hand back `(handle, copy)` whose
/// pointer stays valid until [`release_bytes_cb`] runs. `None` when the host
/// pointer is not this session's.
fn take_external_handle(inner: &Inner, bytes: &[u8]) -> Option<(u64, Vec<u8>)> {
    let mut slab = inner.out_bytes.borrow_mut();
    slab.0 = slab.0.wrapping_add(1).max(1);
    let handle = slab.0;
    let entry = slab.1.entry(handle).or_insert_with(|| bytes.to_vec());
    // The Vec's heap buffer does not move while it sits in the map, so this
    // pointer stays valid until release_bytes_cb removes the entry.
    let stable = unsafe { std::slice::from_raw_parts(entry.as_ptr(), entry.len()).to_vec() };
    Some((handle, stable))
}

/// `fns.release_bytes`: QuickJS freed the view; drop the slab entry, which is
/// the last owner of that heap buffer.
pub(super) unsafe extern "C" fn release_bytes_cb(session: *mut c_void, handle: u64) {
    let inner = unsafe { inner(session) };
    let mut slab = inner.out_bytes.borrow_mut();
    drop(slab.1.remove(&handle));
}

pub(super) fn emit_value(inner: &Inner, value: &PluginValue) {
    let host = inner.host;
    match value {
        PluginValue::Null => unsafe { qjs_value_null(host) },
        PluginValue::Bool(flag) => unsafe { qjs_value_bool(host, i32::from(*flag)) },
        PluginValue::F64(number) => unsafe { qjs_value_f64(host, *number) },
        PluginValue::Str(text) => {
            let bytes = cstring(text);
            unsafe { qjs_value_str(host, bytes.as_ptr(), text.replace('\0', "").len()) };
        }
        PluginValue::Bytes(bytes) => unsafe {
            if bytes.len() >= EXTERNAL_BYTES_THRESHOLD
                && let Some((handle, stable)) = take_external_handle(inner, bytes)
            {
                qjs_value_bytes_external(host, stable.as_ptr(), stable.len(), handle);
                return;
            }
            let ptr = if bytes.is_empty() {
                b"".as_ptr()
            } else {
                bytes.as_ptr()
            };
            qjs_value_bytes(host, ptr, bytes.len());
        },
        PluginValue::List(items) => {
            for item in items {
                emit_value(inner, item);
            }
            unsafe { qjs_value_list(host, items.len() as u32) };
        }
        PluginValue::Map(entries) => {
            for (key, item) in entries {
                let bytes = cstring(key);
                unsafe { qjs_value_str(host, bytes.as_ptr(), key.replace('\0', "").len()) };
                emit_value(inner, item);
            }
            unsafe { qjs_value_map(host, entries.len() as u32) };
        }
    }
}

/// Streamed head: status and headers before any body bytes.
pub(super) fn settle_http_head(
    inner: &Inner,
    token: u64,
    status: u16,
    headers: &[(String, String)],
) {
    let block = host::header_block(headers);
    let header_bytes = cstring(&block);
    let _ = unsafe {
        qjs_http_head(
            inner.host,
            token,
            i32::from(status),
            header_bytes.as_ptr(),
            block.len(),
        )
    };
}

/// One streamed body piece. `done` closes the stream; `error` fails it.
pub(super) fn settle_http_chunk(
    inner: &Inner,
    token: u64,
    bytes: &[u8],
    done: bool,
    error: Option<&HttpError>,
) {
    let message = error.map(|error| cstring(error.dom_name()));
    let _ = unsafe {
        qjs_http_chunk(
            inner.host,
            token,
            bytes.as_ptr(),
            bytes.len(),
            i32::from(done),
            message.as_ref().map_or(ptr::null(), |text| text.as_ptr()),
        )
    };
}

/// Fail a stream that never produced a head.
pub(super) fn deliver_stream_error(inner: &Inner, token: u64, error: &HttpError) {
    settle_http_chunk(inner, token, &[], true, Some(error));
}

pub(super) fn settle_plugin(inner: &Inner, token: u64, result: &PluginResult) {
    match result {
        PluginResult::Done(value) => {
            emit_value(inner, value);
            let _ = unsafe { qjs_settle_top(inner.host, token, 1) };
        }
        PluginResult::Failed { message } => {
            emit_value(inner, &PluginValue::Str(message.clone()));
            let _ = unsafe { qjs_settle_top(inner.host, token, 0) };
        }
    }
}

pub(super) fn settle_http(inner: &Inner, token: u64, result: &Result<HttpResponse, HttpError>) {
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
            // The prelude rejects with the DOMException name so callers can
            // match on it; the detail stays in the session log.
            let message = cstring(error.dom_name());
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

pub(super) fn flush_pending(inner: &Inner) {
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

pub(super) fn cancelled(inner: &Inner, token: u64) -> bool {
    inner
        .cancels
        .borrow()
        .get(&token)
        .is_some_and(|flag| flag.load(Ordering::Acquire))
}

pub(super) fn deliver_socket(inner: &Inner, id: u64, event: &str, text: bool, data: &[u8]) {
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

pub(super) fn submit_or_queue(inner: &Inner, job: IoJob, inline: impl FnOnce()) {
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(job);
    } else {
        inline();
    }
}

pub(super) unsafe extern "C" fn http_call(
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
    unsafe {
        http_call_impl(
            session,
            token,
            method,
            method_len,
            url,
            url_len,
            headers,
            headers_len,
            body,
            body_len,
            false,
        )
    }
}

pub(super) unsafe extern "C" fn http_stream_call(
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
    unsafe {
        http_call_impl(
            session,
            token,
            method,
            method_len,
            url,
            url_len,
            headers,
            headers_len,
            body,
            body_len,
            true,
        )
    }
}

/// Shared body of `http` and `httpStream`. `streaming` picks the job the host
/// network thread runs; a session-local client always downloads whole bodies.
unsafe fn http_call_impl(
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
    streaming: bool,
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
        let error = HttpError::new(HttpFailure::TooLarge, "TypeError");
        if streaming {
            deliver_stream_error(inner, token, &error);
        } else {
            inner.pending.borrow_mut().http.push((token, Err(error)));
        }
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
        timeout_ms: None,
    };
    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        let error = HttpError::new(HttpFailure::BadUrl, "unsupported URL");
        if streaming {
            deliver_stream_error(inner, token, &error);
        } else {
            inner.pending.borrow_mut().http.push((token, Err(error)));
        }
        return 0;
    }
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(if streaming {
            IoJob::HttpStream(request)
        } else {
            IoJob::Http(request)
        });
        return 0;
    }
    if streaming {
        // No host network thread: pull the whole body through the local
        // client, then deliver it as one head plus one chunk.
        if let Some(client) = inner.http.borrow().as_ref().cloned() {
            match client.perform(&request) {
                Ok(response) => {
                    settle_http_head(inner, token, response.status, &response.headers);
                    settle_http_chunk(inner, token, &response.body, true, None);
                }
                Err(error) => deliver_stream_error(inner, token, &error),
            }
            return 0;
        }
        deliver_stream_error(
            inner,
            token,
            &HttpError::new(HttpFailure::Network, "network is not linked"),
        );
        return 0;
    }
    if let Some(client) = inner.http.borrow().as_ref().cloned() {
        let result = client.perform(&request);
        inner.pending.borrow_mut().http.push((token, result));
        return 0;
    }
    let error = HttpError::new(HttpFailure::Network, "network is not linked");
    inner.pending.borrow_mut().http.push((token, Err(error)));
    0
}

pub(super) unsafe extern "C" fn abort_token(session: *mut c_void, token: u64) {
    let inner = unsafe { inner(session) };
    if let Some(flag) = inner.cancels.borrow().get(&token) {
        flag.store(true, Ordering::Release);
    }
    inner
        .pending
        .borrow_mut()
        .http
        .retain(|(pending, _)| *pending != token);
    let aborted = Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
    settle_http(inner, token, &aborted);
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::Abort { token });
    }
}

pub(super) fn prepared_mut(inner: &Inner) -> std::cell::RefMut<'_, Option<PreparedCall>> {
    inner.prepared.borrow_mut()
}

pub(super) fn push_prepared(inner: &Inner, value: PluginValue) {
    if let Some(call) = prepared_mut(inner).as_mut() {
        call.args.push(value);
    }
}

fn set_last_error(inner: &Inner, message: String) {
    *inner.last_error.borrow_mut() = Some(cstring(&message));
}

pub(super) unsafe extern "C" fn module_has(
    session: *mut c_void,
    name: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(name, len) }) else {
        return 0;
    };
    i32::from(inner.config.registry.module(name).is_some())
}

pub(super) unsafe extern "C" fn module_count(
    session: *mut c_void,
    name: *const c_char,
    len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(name, len) }) else {
        return -1;
    };
    inner
        .config
        .registry
        .by_specifier(name)
        .map(|module| module.methods.len() as i32)
        .unwrap_or(-1)
}

pub(super) unsafe extern "C" fn module_method_at(
    session: *mut c_void,
    name: *const c_char,
    name_len: usize,
    index: i32,
    out_len: *mut usize,
) -> *const c_char {
    let inner = unsafe { inner(session) };
    let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
        return ptr::null();
    };
    let Ok(index) = usize::try_from(index) else {
        return ptr::null();
    };
    let Some(method) = inner
        .config
        .registry
        .by_specifier(name)
        .and_then(|module| module.methods.get(index))
    else {
        return ptr::null();
    };
    if !out_len.is_null() {
        unsafe { *out_len = method.name.len() };
    }
    method.name.as_ptr().cast()
}

pub(super) unsafe extern "C" fn module_name_of(
    session: *mut c_void,
    specifier: *const c_char,
    len: usize,
    out_len: *mut usize,
) -> *const c_char {
    let inner = unsafe { inner(session) };
    let Some(specifier) = (unsafe { utf8_of(specifier, len) }) else {
        return ptr::null();
    };
    let Some(module) = inner.config.registry.by_specifier(specifier) else {
        return ptr::null();
    };
    if !out_len.is_null() {
        unsafe { *out_len = module.name.len() };
    }
    module.name.as_ptr().cast()
}

pub(super) unsafe extern "C" fn module_prepare(
    session: *mut c_void,
    name: *const c_char,
    name_len: usize,
    method: *const c_char,
    method_len: usize,
) -> i32 {
    let inner = unsafe { inner(session) };
    inner.last_error.borrow_mut().take();
    let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
        return -1;
    };
    let Some(method) = (unsafe { utf8_of(method, method_len) }) else {
        return -1;
    };
    let Some(entry) = inner.config.registry.method(name, method) else {
        set_last_error(inner, session::module_not_linked(name, Some(method)));
        return -1;
    };
    let kind = match &entry.invoke {
        Invoke::Sync(_) => 0,
        Invoke::Promise(_) => 1,
    };
    *inner.prepared.borrow_mut() = Some(PreparedCall {
        invoke: entry.invoke.clone(),
        token: 0,
        args: ArgBuild::new(),
    });
    kind
}

pub(super) unsafe extern "C" fn module_push_null(session: *mut c_void) {
    push_prepared(unsafe { inner(session) }, PluginValue::Null);
}

pub(super) unsafe extern "C" fn module_push_bool(session: *mut c_void, value: i32) {
    push_prepared(unsafe { inner(session) }, PluginValue::Bool(value != 0));
}

pub(super) unsafe extern "C" fn module_push_f64(session: *mut c_void, value: f64) {
    push_prepared(unsafe { inner(session) }, PluginValue::F64(value));
}

pub(super) unsafe extern "C" fn module_push_str(
    session: *mut c_void,
    text: *const c_char,
    len: usize,
) {
    let text = unsafe { utf8_of(text, len) }.unwrap_or("").to_owned();
    push_prepared(unsafe { inner(session) }, PluginValue::Str(text));
}

pub(super) unsafe extern "C" fn module_push_bytes(
    session: *mut c_void,
    bytes: *const u8,
    len: usize,
) {
    let bytes = if bytes.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes, len).to_vec() }
    };
    push_prepared(unsafe { inner(session) }, PluginValue::Bytes(bytes));
}

pub(super) unsafe extern "C" fn module_begin_list(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.begin(false);
    }
}

pub(super) unsafe extern "C" fn module_end_list(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.end();
    }
}

pub(super) unsafe extern "C" fn module_begin_map(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.begin(true);
    }
}

pub(super) unsafe extern "C" fn module_map_key(
    session: *mut c_void,
    text: *const c_char,
    len: usize,
) {
    let text = unsafe { utf8_of(text, len) }.unwrap_or("").to_owned();
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut()
        && let Some(frame) = call.args.frames.last_mut()
    {
        frame.pending_key = Some(text);
    }
}

pub(super) unsafe extern "C" fn module_end_map(session: *mut c_void) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.args.end();
    }
}

pub(super) unsafe extern "C" fn module_set_token(session: *mut c_void, token: u64) {
    if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
        call.token = token;
    }
}

pub(super) unsafe extern "C" fn module_finish(session: *mut c_void) -> i32 {
    let inner = unsafe { inner(session) };
    let Some(call) = inner.prepared.borrow_mut().take() else {
        return -1;
    };
    let args = call.args.stack;
    match call.invoke {
        Invoke::Sync(f) => match f(&args) {
            Ok(value) => {
                emit_value(inner, &value);
                0
            }
            Err(message) => {
                set_last_error(inner, message);
                -1
            }
        },
        Invoke::Promise(f) => {
            let settle = (inner.config.settle)(call.token);
            f(&args, settle, inner.config.exec.as_ref());
            1
        }
    }
}

pub(super) unsafe extern "C" fn module_error(
    session: *mut c_void,
    out_len: *mut usize,
) -> *const c_char {
    let inner = unsafe { inner(session) };
    match inner.last_error.borrow().as_ref() {
        Some(message) => {
            if !out_len.is_null() {
                unsafe { *out_len = message.as_bytes().len() };
            }
            message.as_ptr()
        }
        None => ptr::null(),
    }
}

pub(super) unsafe extern "C" fn socket_open(
    session: *mut c_void,
    id: u64,
    url: *const c_char,
    len: usize,
    protocols: *const c_char,
    protocols_len: usize,
) {
    let inner = unsafe { inner(session) };
    let Some(url) = (unsafe { utf8_of(url, len) }) else {
        return;
    };
    let url = url.to_owned();
    // A comma-separated offer list, the wire form the header carries.
    let protocols = unsafe { utf8_of(protocols, protocols_len) }
        .map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    submit_or_queue(
        inner,
        IoJob::SocketOpen {
            id,
            url: url.clone(),
            protocols,
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

pub(super) unsafe extern "C" fn socket_send(
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

pub(super) unsafe extern "C" fn socket_close(session: *mut c_void, id: u64) {
    let inner = unsafe { inner(session) };
    if let Some(submit) = inner.io.borrow().as_ref() {
        submit(IoJob::SocketClose { id });
    }
}
