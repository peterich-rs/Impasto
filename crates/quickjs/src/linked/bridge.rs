//! Sized, animation, font, and host-module callbacks.

use std::collections::HashSet;
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::sync::atomic::Ordering;

use session::{HttpError, HttpFailure, HttpRequest, Invoke, IoJob, PluginResult, PluginValue};
use uicore::{FocusDirection, NodeId};

use super::rt::*;
use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn caret_at(
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
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        match inner
            .tree
            .borrow_mut()
            .caret_at(NodeId::from_raw(node), x, y)
        {
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
    })
}

pub(super) unsafe fn word_range(
    session: *mut c_void,
    node: u64,
    unit: u32,
    start: *mut u32,
    end: *mut u32,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        match inner
            .tree
            .borrow_mut()
            .word_range(NodeId::from_raw(node), unit)
        {
            Ok((word_start, word_end)) => {
                // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                unsafe {
                    *start = word_start;
                    *end = word_end;
                }
                0
            }
            Err(_) => -1,
        }
    })
}

pub(super) unsafe fn focus_step(session: *mut c_void, direction: i32) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
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
    })
}

pub(super) unsafe fn push_focus_scope(session: *mut c_void, node: u64) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
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
    })
}

pub(super) unsafe fn pop_focus_scope(session: *mut c_void) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner.tree.borrow_mut().pop_focus_scope();
        0
    })
}

pub(super) unsafe fn pointer_position(session: *mut c_void, x: *mut f32, y: *mut f32) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let (px, py) = inner.tree.borrow().pointer_position();
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        unsafe {
            *x = px;
            *y = py;
        }
        0
    })
}

pub(super) unsafe fn focus_node(session: *mut c_void, node: u64) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let gestures = match inner.tree.borrow_mut().focus(NodeId::from_raw(node)) {
            Ok(gestures) => gestures,
            Err(_) => return -1,
        };
        if dispatch_gestures(inner, gestures).is_err() {
            return -1;
        }
        0
    })
}

pub(super) unsafe fn focused_id(session: *mut c_void) -> u64 {
    super::vm_entry!(0, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner
            .tree
            .borrow()
            .focused()
            .map(NodeId::to_raw)
            .unwrap_or(0)
    })
}

pub(super) unsafe fn is_focusable(session: *mut c_void, node: u64) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        match inner.tree.borrow().is_focusable(NodeId::from_raw(node)) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(_) => -1,
        }
    })
}

pub(super) unsafe fn register_font(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    bytes: *const u8,
    byte_len: usize,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
    })
}

pub(super) unsafe fn reload(session: *mut c_void, utf8: *const c_char, len: usize) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let Some(source) = (unsafe { utf8_of(utf8, len) }) else {
            return -1;
        };
        if source.is_empty() {
            return -1;
        }
        inner.reloads.borrow_mut().push(source.to_owned());
        0
    })
}

pub(super) unsafe fn layer_stats(
    session: *mut c_void,
    epoch: *mut u64,
    layers: *mut u32,
    pictures: *mut u32,
) -> i32 {
    super::vm_entry!(-1, {
        if epoch.is_null() || layers.is_null() || pictures.is_null() {
            return -1;
        }
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let (epoch_value, layer_count, picture_count) = inner.tree.borrow().layer_tree_stats();
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        unsafe {
            *epoch = epoch_value;
            *layers = u32::try_from(layer_count).unwrap_or(u32::MAX);
            *pictures = u32::try_from(picture_count).unwrap_or(u32::MAX);
        }
        0
    })
}

pub(super) unsafe fn scroll_into_view(session: *mut c_void, node: u64, edge: i32) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let edge = match edge {
            1 => uicore::ScrollEdge::End,
            2 => uicore::ScrollEdge::Nearest,
            _ => uicore::ScrollEdge::Start,
        };
        match inner
            .tree
            .borrow_mut()
            .scroll_into_view(NodeId::from_raw(node), edge)
        {
            Ok(_) => 0,
            Err(_) => -1,
        }
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::too_many_arguments,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(super) unsafe fn animate(
    session: *mut c_void,
    node: u64,
    prop: *const c_char,
    prop_len: usize,
    from: f64,
    to: f64,
    stiffness: f64,
    damping: f64,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
            Box::new(uicore::Spring {
                x0: from as f32,
                v0: 0.0,
                target: to as f32,
                stiffness: stiffness as f32,
                damping: damping as f32,
            }),
        );
        0
    })
}

pub(super) fn anim_prop(name: &str) -> Option<uicore::AnimProp> {
    Some(match name {
        "scrollX" => uicore::AnimProp::ScrollX,
        "scrollY" => uicore::AnimProp::ScrollY,
        "opacity" => uicore::AnimProp::Opacity,
        "translateX" => uicore::AnimProp::TranslateX,
        "translateY" => uicore::AnimProp::TranslateY,
        "rotate" => uicore::AnimProp::Rotate,
        "scale" => uicore::AnimProp::Scale,
        "overscrollX" => uicore::AnimProp::OverscrollX,
        "overscrollY" => uicore::AnimProp::OverscrollY,
        _ => return None,
    })
}

pub(super) unsafe fn fling_target(
    session: *mut c_void,
    node: u64,
    out_x: *mut f32,
    out_y: *mut f32,
) -> i32 {
    super::vm_entry!(-1, {
        if out_x.is_null() || out_y.is_null() {
            return -1;
        }
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        match inner.tree.borrow().fling_target(NodeId::from_raw(node)) {
            Some((x, y)) => {
                // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                unsafe {
                    *out_x = x;
                    *out_y = y;
                }
                0
            }
            None => 1,
        }
    })
}

pub(super) unsafe fn set_recognizers(
    session: *mut c_void,
    node: u64,
    tap: i32,
    long_press: i32,
    drag: i32,
    double_tap_ms: f64,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let drag = match drag {
            0 => Some(uicore::DragAxis::Free),
            1 => Some(uicore::DragAxis::Horizontal),
            2 => Some(uicore::DragAxis::Vertical),
            _ => None,
        };
        let double_tap =
            (double_tap_ms.is_finite() && double_tap_ms > 0.0).then(|| uicore::DoubleTapConfig {
                window_ms: double_tap_ms,
                ..uicore::DoubleTapConfig::default()
            });
        status(inner.tree.borrow_mut().set_recognizers(
            NodeId::from_raw(node),
            uicore::RecognizerSet {
                tap: tap != 0,
                long_press: long_press != 0,
                drag,
                double_tap,
            },
        ))
    })
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
    /// `Sans` and `SansCJK` do not count toward the eight-name cap.
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

    pub(super) fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        self.queued.clone()
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
    name == "Sans" || name == "SansCJK"
}

/// `mode` 0 max-content, 1 capped-max, 2 definite width. `height` of `f32::NAN`
/// under mode 2 leaves the height natural. Out parameters carry `(w, h)`.
pub(super) unsafe fn measure_panel(
    session: *mut c_void,
    node: u64,
    mode: i32,
    width: f32,
    height: f32,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    super::vm_entry!(-1, {
        if out_w.is_null() || out_h.is_null() {
            return -1;
        }
        let space = match mode {
            0 => uicore::SubtreeSpace::MaxContent,
            1 => uicore::SubtreeSpace::CappedMax(width),
            2 => uicore::SubtreeSpace::Definite {
                width,
                height: (height.is_finite()).then_some(height),
            },
            _ => return -1,
        };
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // `set_panel` is a separate call so a caller can reuse a declared panel.
        match inner
            .tree
            .borrow_mut()
            .measure_panel(NodeId::from_raw(node), space)
        {
            Ok((w, h)) => {
                // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                unsafe {
                    *out_w = w;
                    *out_h = h;
                }
                0
            }
            Err(_) => -1,
        }
    })
}

pub(super) unsafe fn blur_node(session: *mut c_void) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let gestures = inner.tree.borrow_mut().blur();
        if dispatch_gestures(inner, gestures).is_err() {
            return -1;
        }
        0
    })
}

pub(super) unsafe fn set_ime_enabled(session: *mut c_void, enabled: i32) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let mut ime = inner.ime.borrow_mut();
        ime.shown = enabled != 0;
        ime.dirty = true;
        0
    })
}

pub(super) unsafe fn set_ime_text(session: *mut c_void, utf8: *const c_char, len: usize) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let Some(text) = (unsafe { utf8_of(utf8, len) }) else {
            return -1;
        };
        let mut ime = inner.ime.borrow_mut();
        ime.state = session::ImeState::at_end(text);
        ime.dirty = true;
        0
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn set_ime_state(
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
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let mut ime = inner.ime.borrow_mut();
        if shown == 0 {
            ime.shown = false;
            ime.dirty = true;
            return 0;
        }
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
    })
}

pub(super) fn tri_state(value: i32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn set_semantics(
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
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let label = unsafe { utf8_of(label, label_len) }.map(str::to_owned);
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let value = unsafe { utf8_of(value, value_len) }.map(str::to_owned);
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let description = unsafe { utf8_of(description, description_len) }.map(str::to_owned);
        let hint = uicore::SemanticsHint {
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
                0 => Some(uicore::LiveRegion::Off),
                1 => Some(uicore::LiveRegion::Polite),
                2 => Some(uicore::LiveRegion::Assertive),
                _ => None,
            },
        };
        let result = inner
            .tree
            .borrow_mut()
            .set_semantics(NodeId::from_raw(node), hint);
        if result.is_err() { -1 } else { 0 }
    })
}

pub(super) fn role_of(role: i32) -> Option<uicore::SemanticsRole> {
    Some(match role {
        1 => uicore::SemanticsRole::Button,
        2 => uicore::SemanticsRole::Text,
        3 => uicore::SemanticsRole::Image,
        4 => uicore::SemanticsRole::Scroll,
        5 => uicore::SemanticsRole::Switch,
        6 => uicore::SemanticsRole::TextField,
        7 => uicore::SemanticsRole::SearchField,
        8 => uicore::SemanticsRole::TextArea,
        _ => return None,
    })
}

pub(super) unsafe fn text_scale(session: *mut c_void) -> f64 {
    super::vm_entry!(0.0, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        f64::from(inner.text_scale.get())
    })
}

pub(super) unsafe fn reduce_motion(session: *mut c_void) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        i32::from(inner.reduce_motion.get())
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn measure_text(
    session: *mut c_void,
    utf8: *const c_char,
    len: usize,
    font_size: f32,
    max_width: f32,
    color: *const u8,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    super::vm_entry!(-1, {
        if out_w.is_null() || out_h.is_null() {
            return -1;
        }
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let (Some(utf8), Some(color)) = (unsafe { utf8_of(utf8, len) }, unsafe { rgba(color) })
        else {
            return -1;
        };
        let size = inner.tree.borrow_mut().measure_text(
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
    })
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
    // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
    let _ = unsafe {
        vm_module_event(
            inner.host,
            module.as_ptr(),
            event.as_ptr(),
            shape,
            body.as_ptr(),
            text.replace('\0', "").len(),
        )
    };
}

pub(super) fn take_sync_result(inner: &Inner) -> Option<PluginValue> {
    inner.sync_result.borrow_mut().take()
}

/// Streamed head: status and headers before any body bytes.
pub(super) fn settle_http_head(
    inner: &Inner,
    token: u64,
    status: u16,
    headers: &[(String, String)],
) {
    let block = bootstrap::header_block(headers);
    let header_bytes = cstring(&block);
    // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
    let _ = unsafe {
        vm_http_head(
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
    // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
    let _ = unsafe {
        vm_http_chunk(
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
    // SAFETY: the session owns Host for this call. The result is owned data.
    let _ = vm_settle_plugin(inner.host, token, result);
}

pub(super) fn settle_http(inner: &Inner, token: u64, result: &Result<HttpResponse, HttpError>) {
    match result {
        Ok(response) => {
            if response.body.len() > session::HTTP_BODY_LIMIT {
                let message = cstring("TypeError");
                // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
                let _ = unsafe {
                    vm_settle_http(
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
            let headers = bootstrap::header_block(&response.headers);
            let header_bytes = cstring(&headers);
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            let _ = unsafe {
                vm_settle_http(
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
            // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
            let _ = unsafe {
                vm_settle_http(
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
    // SAFETY: the session owns Host; all borrowed input and output buffers live through this call.
    let _ = unsafe {
        vm_socket_event(
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

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn http_call(
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
    super::vm_entry!(-1, {
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
pub(super) unsafe fn http_stream_call(
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
    super::vm_entry!(-1, {
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
    })
}

/// Shared body of `http` and `httpStream`. `streaming` picks the job the host
/// network thread runs; a session-local client always downloads whole bodies.
#[expect(
    clippy::too_many_arguments,
    reason = "the existing callback record carries separate scalar fields"
)]
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
    // SAFETY: session is captured from the live Host and used only on the render thread.
    let inner = unsafe { inner(session) };
    // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
    let Some(method) = (unsafe { utf8_of(method, method_len) }) else {
        return -1;
    };
    // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
    let Some(url) = (unsafe { utf8_of(url, url_len) }) else {
        return -1;
    };
    // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
        // SAFETY: the Rust caller supplies the stated number of initialized elements for this call.
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
        headers: bootstrap::header_pairs(headers),
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

pub(super) unsafe fn abort_token(session: *mut c_void, token: u64) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
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
    })
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

pub(super) unsafe fn module_prepare(
    session: *mut c_void,
    name: *const c_char,
    name_len: usize,
    method: *const c_char,
    method_len: usize,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner.last_error.borrow_mut().take();
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let Some(name) = (unsafe { utf8_of(name, name_len) }) else {
            return -1;
        };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
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
            args: Vec::new(),
        });
        kind
    })
}

pub(super) unsafe fn module_set_token(session: *mut c_void, token: u64) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        if let Some(call) = prepared_mut(unsafe { inner(session) }).as_mut() {
            call.token = token;
        }
    })
}

pub(super) unsafe fn module_finish(session: *mut c_void) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner.sync_result.borrow_mut().take();
        let Some(call) = inner.prepared.borrow_mut().take() else {
            return -1;
        };
        let args = call.args;
        match call.invoke {
            Invoke::Sync(f) => match f(&args) {
                Ok(value) => {
                    *inner.sync_result.borrow_mut() = Some(value);
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
    })
}

pub(super) unsafe fn socket_open(
    session: *mut c_void,
    id: u64,
    url: *const c_char,
    len: usize,
    protocols: *const c_char,
    protocols_len: usize,
) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let Some(url) = (unsafe { utf8_of(url, len) }) else {
            return;
        };
        let url = url.to_owned();
        // A comma-separated offer list, the wire form the header carries.
        // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
        let protocols = unsafe { utf8_of(protocols, protocols_len) }
            .map(|text| {
                text.split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        submit_or_queue(inner, IoJob::SocketOpen { id, url, protocols }, || {
            inner.pending.borrow_mut().socket.push((
                id,
                "error".to_owned(),
                true,
                b"socket is not linked".to_vec(),
            ));
        });
    })
}

pub(super) unsafe fn socket_send(
    session: *mut c_void,
    id: u64,
    data: *const u8,
    len: usize,
    is_text: i32,
) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let bytes = if data.is_null() || len == 0 {
            Vec::new()
        } else {
            // SAFETY: the Rust caller supplies the stated number of initialized elements for this call.
            unsafe { std::slice::from_raw_parts(data, len).to_vec() }
        };
        if let Some(submit) = inner.io.borrow().as_ref() {
            submit(IoJob::SocketSend {
                id,
                text: is_text != 0,
                data: bytes,
            });
        }
    })
}

pub(super) unsafe fn socket_close(session: *mut c_void, id: u64) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        if let Some(submit) = inner.io.borrow().as_ref() {
            submit(IoJob::SocketClose { id });
        }
    })
}

// ---- devtools debug surface -------------------------------------------------

/// What `debug_json` should produce. Numbers match the VM bindings.
pub(super) const DEBUG_LAYOUT_ISSUES: i32 = 0;
pub(super) const DEBUG_SNAPSHOT: i32 = 1;
pub(super) const DEBUG_PROBE: i32 = 2;
pub(super) const DEBUG_LAST_POINTER: i32 = 3;

/// Writes one debug JSON document into the session slot. Returns 0 on
/// success; the VM bindings copy it before returning.
pub(super) unsafe fn debug_json(
    session: *mut c_void,
    what: i32,
    _node: u64,
    x: f32,
    y: f32,
) -> i32 {
    super::vm_entry!(-1, {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        let mut tree = inner.tree.borrow_mut();
        let text = match what {
            DEBUG_LAYOUT_ISSUES => tree.inspect_layout_issues(),
            DEBUG_SNAPSHOT => tree.debug_snapshot(),
            DEBUG_PROBE => tree.debug_probe(x, y),
            DEBUG_LAST_POINTER => match tree.debug_last_pointer() {
                Some((px, py, phase)) => {
                    format!("{{\"x\":{px:.2},\"y\":{py:.2},\"phase\":\"{phase}\"}}")
                }
                None => "null".to_owned(),
            },
            _ => return -1,
        };
        match CString::new(text) {
            Ok(c) => {
                *inner.debug_json.borrow_mut() = Some(c);
                0
            }
            Err(_) => -1,
        }
    })
}

pub(super) unsafe fn debug_set_scan(session: *mut c_void, on: i32) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner.tree.borrow_mut().set_debug_scan(on != 0);
    })
}

pub(super) unsafe fn debug_set_pointer_trace(session: *mut c_void, on: i32, suppress: i32) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner
            .tree
            .borrow_mut()
            .set_pointer_trace(on != 0, suppress != 0);
    })
}

pub(super) unsafe fn debug_set_owned(session: *mut c_void, node: u64, on: i32) {
    super::vm_entry!((), {
        // SAFETY: session is captured from the live Host and used only on the render thread.
        let inner = unsafe { inner(session) };
        inner
            .tree
            .borrow_mut()
            .set_debug_owned(NodeId::from_raw(node), on != 0);
    })
}
