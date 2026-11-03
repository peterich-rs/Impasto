//! Reads and immediate commands. One kind each, dispatched to the tree.
#![allow(
    unsafe_op_in_unsafe_fn,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "the wire is f64, the same casts the previous per-setter wrappers used"
)]

use std::ffi::{c_char, c_void};

use super::bridge;
use super::nodes;
use super::utf8_of;

use crate::abi::{EffectKind, QueryKind};

/// Result buffer shared with `glue.c`. `text` points at session storage and is
/// only valid until the next query or effect on this session.
#[repr(C)]
pub(in crate::linked) struct CallOut {
    pub status: i32,
    pub nnums: i32,
    pub handle: u64,
    pub nums: [f64; 128],
    pub text: *const c_char,
    pub text_len: usize,
}

impl CallOut {
    fn clear(&mut self) {
        self.status = 0;
        self.nnums = 0;
        self.handle = 0;
        self.nums = [0.0; 128];
        self.text = std::ptr::null();
        self.text_len = 0;
    }
}

pub(super) unsafe extern "C" fn query(
    session: *mut c_void,
    kind: i32,
    node: u64,
    a: f64,
    b: f64,
    text: *const c_char,
    text_len: usize,
    color: *const u8,
    out: *mut CallOut,
) -> i32 {
    if out.is_null() {
        return -1;
    }
    // SAFETY: glue passes a writable `CallOut` that lives for this call.
    let out = unsafe { &mut *out };
    out.clear();
    let failed = if kind == QueryKind::Parent as i32 {
        nodes::parent(session, node, &mut out.handle)
    } else if kind == QueryKind::FirstChild as i32 {
        nodes::first_child(session, node, &mut out.handle)
    } else if kind == QueryKind::NextSibling as i32 {
        nodes::next_sibling(session, node, &mut out.handle)
    } else if kind == QueryKind::IsText as i32 {
        let flag = nodes::is_text(session, node);
        if flag < 0 {
            -1
        } else {
            out.nums[0] = f64::from(flag);
            out.nnums = 1;
            0
        }
    } else if kind == QueryKind::Rect as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut w = 0.0;
        let mut h = 0.0;
        let status = nodes::rect(session, node, &mut x, &mut y, &mut w, &mut h);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nums[2] = f64::from(w);
            out.nums[3] = f64::from(h);
            out.nnums = 4;
        }
        status
    } else if kind == QueryKind::ScrollOffset as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let status = nodes::scroll_offset(session, node, &mut x, &mut y);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::Overscroll as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let status = nodes::overscroll(session, node, &mut x, &mut y);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::Metrics as i32 {
        let mut w = 0.0;
        let mut h = 0.0;
        let status = nodes::viewport(session, &mut w, &mut h);
        if status == 0 {
            out.nums[0] = f64::from(w);
            out.nums[1] = f64::from(h);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::CaretAt as i32 {
        let mut unit = 0u32;
        let mut x = 0.0;
        let mut y = 0.0;
        let mut w = 0.0;
        let mut h = 0.0;
        let status = bridge::caret_at(
            session, node, a as f32, b as f32, &mut unit, &mut x, &mut y, &mut w, &mut h,
        );
        if status == 0 {
            out.nums[0] = f64::from(unit);
            out.nums[1] = f64::from(x);
            out.nums[2] = f64::from(y);
            out.nums[3] = f64::from(w);
            out.nums[4] = f64::from(h);
            out.nnums = 5;
        }
        status
    } else if kind == QueryKind::CaretAtUnit as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut w = 0.0;
        let mut h = 0.0;
        let status = nodes::caret_at_unit(session, node, a as u32, &mut x, &mut y, &mut w, &mut h);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nums[2] = f64::from(w);
            out.nums[3] = f64::from(h);
            out.nnums = 4;
        }
        status
    } else if kind == QueryKind::WordRange as i32 {
        let mut start = 0u32;
        let mut end = 0u32;
        let status = bridge::word_range(session, node, a as u32, &mut start, &mut end);
        if status == 0 {
            out.nums[0] = f64::from(start);
            out.nums[1] = f64::from(end);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::MeasureText as i32 {
        let mut w = 0.0;
        let mut h = 0.0;
        let status = bridge::measure_text(
            session, text, text_len, a as f32, b as f32, color, &mut w, &mut h,
        );
        if status == 0 {
            out.nums[0] = f64::from(w);
            out.nums[1] = f64::from(h);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::Focused as i32 {
        out.handle = bridge::focused_id(session);
        0
    } else if kind == QueryKind::IsFocusable as i32 {
        let flag = bridge::is_focusable(session, node);
        if flag < 0 {
            -1
        } else {
            out.nums[0] = f64::from(flag);
            out.nnums = 1;
            0
        }
    } else if kind == QueryKind::Pointer as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let status = bridge::pointer_position(session, &mut x, &mut y);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nnums = 2;
        }
        status
    } else if kind == QueryKind::TextScale as i32 {
        out.nums[0] = bridge::text_scale(session);
        out.nnums = 1;
        0
    } else if kind == QueryKind::ReduceMotion as i32 {
        out.nums[0] = f64::from(bridge::reduce_motion(session));
        out.nnums = 1;
        0
    } else if kind == QueryKind::LayerStats as i32 {
        let mut epoch = 0u64;
        let mut layers = 0u32;
        let mut pictures = 0u32;
        let status = bridge::layer_stats(session, &mut epoch, &mut layers, &mut pictures);
        if status == 0 {
            out.nums[0] = epoch as f64;
            out.nums[1] = f64::from(layers);
            out.nums[2] = f64::from(pictures);
            out.nnums = 3;
        }
        status
    } else if kind == QueryKind::FlingTarget as i32 {
        let mut x = 0.0;
        let mut y = 0.0;
        let status = bridge::fling_target(session, node, &mut x, &mut y);
        if status == 0 {
            out.nums[0] = f64::from(x);
            out.nums[1] = f64::from(y);
            out.nums[2] = 0.0;
            out.nnums = 3;
            0
        } else if status > 0 {
            out.nums[2] = 1.0;
            out.nnums = 3;
            0
        } else {
            -1
        }
    } else if kind == QueryKind::GraphemeIndex as i32 {
        let Some(text) = (unsafe { utf8_of(text, text_len) }) else {
            return -1;
        };
        out.nums[0] = f64::from(uicore::grapheme_index(text, a as u32));
        out.nnums = 1;
        0
    } else if kind == QueryKind::GraphemeCount as i32 {
        let Some(text) = (unsafe { utf8_of(text, text_len) }) else {
            return -1;
        };
        out.nums[0] = f64::from(uicore::grapheme_count(text));
        out.nnums = 1;
        0
    } else if kind == QueryKind::GraphemeUnit as i32 {
        let Some(text) = (unsafe { utf8_of(text, text_len) }) else {
            return -1;
        };
        out.nums[0] = f64::from(uicore::grapheme_unit(text, a as u32));
        out.nnums = 1;
        0
    } else if kind == QueryKind::SelectionRects as i32 {
        let count = nodes::selection_rects(session, node, a as u32, b as u32);
        if count < 0 {
            -1
        } else {
            let mut len = 0usize;
            let raw = nodes::selection_rect_bytes(session, &mut len);
            let floats = len / 4;
            let n = floats.min(out.nums.len());
            if !raw.is_null() && n > 0 {
                // SAFETY: `selection_rect_bytes` returns `len` bytes of f32s owned by the session.
                let bytes = unsafe { std::slice::from_raw_parts(raw, n * 4) };
                for (index, chunk) in bytes.chunks_exact(4).enumerate() {
                    let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    out.nums[index] = f64::from(value);
                }
            }
            out.nnums = n as i32;
            0
        }
    } else {
        -1
    };
    out.status = failed;
    failed
}

pub(super) unsafe extern "C" fn effect(
    session: *mut c_void,
    kind: i32,
    node: u64,
    nums: *const f64,
    nnums: i32,
    color: *const u8,
    text: *const c_char,
    text_len: usize,
    bytes: *const u8,
    byte_len: usize,
    out: *mut CallOut,
) -> i32 {
    if out.is_null() {
        return -1;
    }
    let out = unsafe { &mut *out };
    out.clear();
    let num = |index: usize| -> f64 {
        if nums.is_null() || index >= nnums as usize {
            0.0
        } else {
            unsafe { *nums.add(index) }
        }
    };
    let failed = if kind == EffectKind::Focus as i32 {
        bridge::focus_node(session, node)
    } else if kind == EffectKind::Blur as i32 {
        bridge::blur_node(session)
    } else if kind == EffectKind::FocusStep as i32 {
        bridge::focus_step(session, num(0) as i32)
    } else if kind == EffectKind::PushScope as i32 {
        bridge::push_focus_scope(session, node)
    } else if kind == EffectKind::PopScope as i32 {
        bridge::pop_focus_scope(session);
        0
    } else if kind == EffectKind::ScrollTo as i32 {
        nodes::scroll_to(
            session,
            node,
            num(0) as f32,
            num(1) as f32,
            num(2) as f32,
            num(3) as f32,
        )
    } else if kind == EffectKind::Fling as i32 {
        nodes::arm_fling(session, node, num(0) as f32, num(1) as f32)
    } else if kind == EffectKind::StopScroll as i32 {
        nodes::stop_scroll(session, node)
    } else if kind == EffectKind::Animate as i32 {
        bridge::animate(
            session,
            node,
            text,
            text_len,
            num(0),
            num(1),
            num(2),
            num(3),
        )
    } else if kind == EffectKind::ScrollIntoView as i32 {
        let status = bridge::scroll_into_view(session, node, num(0) as i32);
        if status >= 0 {
            out.nums[0] = f64::from(status);
            out.nnums = 1;
            0
        } else {
            status
        }
    } else if kind == EffectKind::PlacePanel as i32 {
        nodes::place_panel(session, node, num(0) as u64, num(1) as i32)
    } else if kind == EffectKind::TextEdit as i32 {
        let mut start = 0u32;
        let mut end = 0u32;
        let status = nodes::text_edit(
            session,
            node,
            num(0) as i32,
            text,
            text_len,
            &mut start,
            &mut end,
        );
        if status == 0 {
            out.nums[0] = f64::from(start);
            out.nums[1] = f64::from(end);
            out.nnums = 2;
            let mut len = 0usize;
            out.text = nodes::text_edit_bytes(session, &mut len).cast();
            out.text_len = len;
        }
        status
    } else if kind == EffectKind::TextCommand as i32 {
        let raw = unsafe { utf8_of(text, text_len) }.unwrap_or("");
        let (name, extra) = raw.split_once('\0').unwrap_or((raw, ""));
        let name_ptr = text;
        let name_len = name.len();
        let extra_ptr = if extra.is_empty() || text.is_null() {
            std::ptr::null()
        } else {
            unsafe { text.add(name_len + 1) }
        };
        let extra_len = extra.len();
        let text = name_ptr;
        let extra = extra_ptr;
        let mut start = 0u32;
        let mut end = 0u32;
        let status = nodes::text_command(
            session,
            node,
            text,
            name_len,
            extra,
            extra_len,
            num(0) as i32,
            &mut start,
            &mut end,
        );
        if status == 0 {
            out.nums[0] = f64::from(start);
            out.nums[1] = f64::from(end);
            out.nnums = 2;
            let mut len = 0usize;
            out.text = nodes::text_edit_bytes(session, &mut len).cast();
            out.text_len = len;
        }
        status
    } else if kind == EffectKind::SetImeEnabled as i32 {
        bridge::set_ime_enabled(session, num(0) as i32)
    } else if kind == EffectKind::SetImeText as i32 {
        bridge::set_ime_text(session, text, text_len)
    } else if kind == EffectKind::SetImeState as i32 {
        bridge::set_ime_state(
            session,
            num(0) as i32,
            text,
            text_len,
            num(1) as u32,
            num(2) as u32,
            num(3) as i32,
            num(4) as i32,
            num(5) as f32,
            num(6) as f32,
            num(7) as f32,
            num(8) as f32,
        )
    } else if kind == EffectKind::SetImeConfig as i32 {
        nodes::set_ime_config(session, num(0) as i32, num(1) as i32)
    } else if kind == EffectKind::SetSemantics as i32 {
        let raw = unsafe { utf8_of(text, text_len) }.unwrap_or("");
        let mut parts = raw.split('\0');
        let label = parts.next().unwrap_or("");
        let value_text = parts.next().unwrap_or("");
        let description_text = parts.next().unwrap_or("");
        let label_len = label.len();
        let value_len = value_text.len();
        let description_len = description_text.len();
        let value = if text.is_null() {
            std::ptr::null()
        } else {
            unsafe { text.add(label_len + 1) }
        };
        let description = if text.is_null() {
            std::ptr::null()
        } else {
            unsafe { text.add(label_len + 1 + value_len + 1) }
        };
        bridge::set_semantics(
            session,
            node,
            num(0) as i32,
            text,
            label_len,
            value,
            value_len,
            num(1) as i32,
            num(2) as i32,
            num(3) as i32,
            num(4) as i32,
            num(5) as i32,
            num(6) as i32,
            description,
            description_len,
            num(7) as i32,
            num(11) as i32,
            num(12) as i32,
        )
    } else if kind == EffectKind::SetRecognizers as i32 {
        bridge::set_recognizers(
            session,
            node,
            num(0) as i32,
            num(1) as i32,
            num(2) as i32,
            num(3),
        )
    } else if kind == EffectKind::SetScrollPhysics as i32 {
        nodes::set_scroll_physics(
            session,
            node,
            num(0) as f32,
            num(1) as i32,
            num(2) as f32,
            num(3) as f32,
            num(4) as f32,
            num(5) as f32,
            num(6) as i32,
            num(7) as f32,
            num(8) as i32,
            num(9) as i32,
            num(10) as i32,
            num(11) as f32,
            num(12) as f32,
        )
    } else if kind == EffectKind::RegisterFont as i32 {
        bridge::register_font(session, text, text_len, bytes, byte_len)
    } else if kind == EffectKind::Reload as i32 {
        bridge::reload(session, text, text_len)
    } else if kind == EffectKind::MeasurePanel as i32 {
        let mut w = 0.0;
        let mut h = 0.0;
        let status = bridge::measure_panel(
            session,
            node,
            num(0) as i32,
            num(1) as f32,
            num(2) as f32,
            &mut w,
            &mut h,
        );
        if status == 0 {
            out.nums[0] = f64::from(w);
            out.nums[1] = f64::from(h);
            out.nnums = 2;
        }
        status
    } else if kind == EffectKind::SetTextScale as i32 {
        nodes::set_text_scale(session, num(0) as f32)
    } else {
        let _ = color;
        -1
    };
    out.status = failed;
    failed
}
