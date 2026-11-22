//! Focus events, animations, IME state and font registration for the JS adapter.

use std::collections::HashSet;

use crate::motion::Spring;
use crate::session::{ImeState, utf16_len};
use crate::tree::{FocusDirection, NodeId, Rect};

use super::{Host, Inner, dispatch_gestures};

pub(super) fn focus_step(host: &Host, direction: FocusDirection) -> bool {
    let gestures = host.inner().tree.borrow_mut().focus_step(direction);
    dispatch_gestures(host, gestures).is_ok()
}

pub(super) fn focus_node(host: &Host, node: NodeId) -> bool {
    let Ok(gestures) = host.inner().tree.borrow_mut().focus(node) else {
        return false;
    };
    dispatch_gestures(host, gestures).is_ok()
}

pub(super) fn push_focus_scope(host: &Host, node: NodeId) -> bool {
    let Ok(gestures) = host.inner().tree.borrow_mut().push_focus_scope(node) else {
        return false;
    };
    dispatch_gestures(host, gestures).is_ok()
}

pub(super) fn blur_node(host: &Host) -> bool {
    let gestures = host.inner().tree.borrow_mut().blur();
    dispatch_gestures(host, gestures).is_ok()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript numeric arguments use f64"
)]
pub(super) fn animate(
    host: &Host,
    node: NodeId,
    name: &str,
    values: [f64; 4],
    retarget: bool,
) -> bool {
    let Some(prop) = anim_prop(name) else {
        return false;
    };
    if !values.into_iter().all(f64::is_finite) {
        return false;
    }
    let [from, to, stiffness, damping] = values.map(|value| value as f32);
    if matches!(
        prop,
        crate::motion::AnimProp::Opacity
            | crate::motion::AnimProp::TranslateX
            | crate::motion::AnimProp::TranslateY
            | crate::motion::AnimProp::Rotate
            | crate::motion::AnimProp::Scale
    ) {
        let gestures = {
            let mut tree = host.inner().tree.borrow_mut();
            if tree.flush_styles().is_err() {
                return false;
            }
            tree.take_style_gestures()
        };
        if dispatch_gestures(host, gestures).is_err() {
            return false;
        }
    }
    let mut tree = host.inner().tree.borrow_mut();
    let start = if retarget {
        tree.motion_position(node, prop).unwrap_or(from)
    } else {
        from
    };
    tree.animate(
        node,
        prop,
        Box::new(Spring {
            x0: start,
            v0: 0.0,
            target: to,
            stiffness,
            damping,
        }),
    );
    true
}

pub(super) fn anim_prop(name: &str) -> Option<crate::motion::AnimProp> {
    Some(match name {
        "scrollX" => crate::motion::AnimProp::ScrollX,
        "scrollY" => crate::motion::AnimProp::ScrollY,
        "opacity" => crate::motion::AnimProp::Opacity,
        "translateX" => crate::motion::AnimProp::TranslateX,
        "translateY" => crate::motion::AnimProp::TranslateY,
        "rotate" => crate::motion::AnimProp::Rotate,
        "scale" => crate::motion::AnimProp::Scale,
        "overscrollX" => crate::motion::AnimProp::OverscrollX,
        "overscrollY" => crate::motion::AnimProp::OverscrollY,
        _ => return None,
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
    pub(super) fn enqueue(&mut self, family: String, bytes: Vec<u8>) -> Result<(), ()> {
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

pub(super) fn set_ime_text(inner: &Inner, text: &str) {
    let mut ime = inner.ime.borrow_mut();
    let mut state = ImeState::at_end(text);
    state.client = ime.state.client;
    state.ack = ime.state.ack;
    state.node = ime.state.node;
    state.autofill = ime.state.autofill;
    ime.state = state;
    ime.dirty = true;
}

pub(super) fn hide_ime(inner: &Inner) {
    let mut ime = inner.ime.borrow_mut();
    ime.shown = false;
    ime.dirty = true;
}

pub(super) fn set_ime_state(
    inner: &Inner,
    text: &str,
    (start, end): (u32, u32),
    (comp_start, comp_end): (i32, i32),
    caret: Rect,
) -> bool {
    let mut ime = inner.ime.borrow_mut();
    let text_len = utf16_len(text);
    let selection = start.min(end)..start.max(end);
    if selection.end > text_len {
        return false;
    }
    let composing = if let Ok(start) = u32::try_from(comp_start) {
        let Ok(end) = u32::try_from(comp_end) else {
            return false;
        };
        if start > end || end > text_len {
            return false;
        }
        Some(start..end)
    } else {
        None
    };
    ime.shown = true;
    ime.dirty = true;
    ime.state = ImeState {
        text: text.to_owned(),
        selection,
        composing,
        caret,
        client: ime.state.client,
        ack: ime.state.ack,
        node: ime.state.node,
        autofill: ime.state.autofill,
        action: ime.state.action,
        keyboard: ime.state.keyboard,
    };
    true
}

pub(super) fn tri_state(value: i32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub(super) fn role_of(role: i32) -> Option<crate::semantics::SemanticsRole> {
    Some(match role {
        1 => crate::semantics::SemanticsRole::Button,
        2 => crate::semantics::SemanticsRole::Text,
        3 => crate::semantics::SemanticsRole::Image,
        4 => crate::semantics::SemanticsRole::Scroll,
        5 => crate::semantics::SemanticsRole::Switch,
        6 => crate::semantics::SemanticsRole::TextField,
        7 => crate::semantics::SemanticsRole::SearchField,
        8 => crate::semantics::SemanticsRole::TextArea,
        _ => return None,
    })
}
