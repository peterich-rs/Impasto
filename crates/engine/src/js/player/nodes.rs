//! Tree-building callbacks used by the Rust VM bindings.

use crate::tree::NodeId;

use super::*;

pub(super) fn text_edit(
    inner: &Inner,
    node: NodeId,
    op: i32,
    extra: String,
) -> Option<crate::text::TextEditingValue> {
    let edit = match op {
        0 => crate::text::Edit::Load(extra),
        1 => crate::text::Edit::Left,
        2 => crate::text::Edit::Right,
        3 => crate::text::Edit::Backspace,
        4 => crate::text::Edit::Insert(extra),
        5 => crate::text::Edit::Composing(if extra.is_empty() { None } else { Some(extra) }),
        6 => crate::text::Edit::Paste(extra),
        7 => crate::text::Edit::Undo,
        8 => crate::text::Edit::Redo,
        9 => crate::text::Edit::SelectAll,
        10 => crate::text::Edit::Copy,
        11 => crate::text::Edit::Cut,
        12 => crate::text::Edit::SetCaret(extra.parse::<u32>().unwrap_or(0)),
        13 => crate::text::Edit::ForwardDelete,
        14 => crate::text::Edit::Newline,
        15 => crate::text::Edit::WordLeft,
        16 => crate::text::Edit::WordRight,
        17 => crate::text::Edit::LineStart,
        18 => crate::text::Edit::LineEnd,
        19 => crate::text::Edit::ExtendLeft,
        20 => crate::text::Edit::ExtendRight,
        21 => crate::text::Edit::ExtendWordLeft,
        22 => crate::text::Edit::ExtendWordRight,
        23 => crate::text::Edit::ExtendLineStart,
        24 => crate::text::Edit::ExtendLineEnd,
        25 => crate::text::Edit::Up,
        26 => crate::text::Edit::Down,
        27 => crate::text::Edit::ExtendUp,
        28 => crate::text::Edit::ExtendDown,
        29 => crate::text::Edit::ExtendCaret(extra.parse::<u32>().unwrap_or(0)),
        30 => crate::text::Edit::DeleteWordBackward,
        31 => crate::text::Edit::DeleteWordForward,
        32 => crate::text::Edit::DeleteToLineStart,
        33 => crate::text::Edit::DeleteToLineEnd,
        34 => crate::text::Edit::DocumentStart,
        35 => crate::text::Edit::DocumentEnd,
        36 => crate::text::Edit::ExtendDocumentStart,
        37 => crate::text::Edit::ExtendDocumentEnd,
        38 => crate::text::Edit::Commit(extra),
        _ => return None,
    };
    let value = inner.tree.borrow_mut().text_edit(node, edit).ok()?;
    if matches!(op, 10 | 11) {
        *inner.clipboard.borrow_mut() = inner.tree.borrow().text_clipboard(node).ok().flatten();
    }
    Some(value)
}

pub(super) fn set_ime_config(inner: &Inner, keyboard: i32, action: i32) {
    let mut ime = inner.ime.borrow_mut();
    ime.state.keyboard = match keyboard {
        1 => crate::session::KeyboardType::Multiline,
        2 => crate::session::KeyboardType::Number,
        3 => crate::session::KeyboardType::Phone,
        4 => crate::session::KeyboardType::Email,
        5 => crate::session::KeyboardType::Url,
        6 => crate::session::KeyboardType::Name,
        7 => crate::session::KeyboardType::Password,
        _ => crate::session::KeyboardType::Text,
    };
    ime.state.action = match action {
        1 => crate::session::TextInputAction::Done,
        2 => crate::session::TextInputAction::Next,
        3 => crate::session::TextInputAction::Search,
        4 => crate::session::TextInputAction::Go,
        5 => crate::session::TextInputAction::Send,
        6 => crate::session::TextInputAction::Previous,
        7 => crate::session::TextInputAction::Newline,
        _ => crate::session::TextInputAction::None,
    };
    ime.dirty = true;
}

pub(super) fn snap_grid(interval: f32, align: i32) -> Option<crate::motion::SnapGrid> {
    if !(interval.is_finite() && interval > 0.5) {
        return None;
    }
    let align = match align {
        1 => crate::motion::SnapAlign::Center,
        2 => crate::motion::SnapAlign::End,
        _ => crate::motion::SnapAlign::Start,
    };
    Some(crate::motion::SnapGrid { interval, align })
}

pub(super) fn sweep_detached(host: &Host) {
    let inner = host.inner();
    let retired = inner.tree.borrow_mut().discard_detached();
    if let Err(message) = dispatch_gestures(host, retired.gestures) {
        (inner.log)(3, &message);
    }
    if retired.ids.is_empty() {
        return;
    }
    let raw: Vec<u64> = retired.ids.into_iter().map(NodeId::to_raw).collect();
    host.forget_ids(&raw);
}

pub(super) fn gesture_parts(gesture: crate::tree::Gesture) -> (NodeId, i32, f64, f64) {
    match gesture {
        crate::tree::Gesture::Click(id) => (id, 0, 0.0, 0.0),
        crate::tree::Gesture::Press(id) => (id, 1, 0.0, 0.0),
        crate::tree::Gesture::Release(id) => (id, 2, 0.0, 0.0),
        crate::tree::Gesture::DragBegin(id) => (id, 3, 0.0, 0.0),
        crate::tree::Gesture::DragBy(id, dx, dy) => (id, 4, f64::from(dx), f64::from(dy)),
        crate::tree::Gesture::DragEnd(id, vx, vy) => (id, 5, f64::from(vx), f64::from(vy)),
        crate::tree::Gesture::Focus(id) => (id, 6, 0.0, 0.0),
        crate::tree::Gesture::Blur(id) => (id, 7, 0.0, 0.0),
        crate::tree::Gesture::Hover(id) => (id, 11, 0.0, 0.0),
        crate::tree::Gesture::Leave(id) => (id, 12, 0.0, 0.0),
        crate::tree::Gesture::Wheel(id, dx, dy) => (id, 13, f64::from(dx), f64::from(dy)),
        crate::tree::Gesture::SecondaryClick(id) => (id, 14, 0.0, 0.0),
        crate::tree::Gesture::Scroll(id, x, y) => (id, 15, f64::from(x), f64::from(y)),
        crate::tree::Gesture::TextReplaced(id) => (id, 16, 0.0, 0.0),
        crate::tree::Gesture::ActivateLink { id, link } => (id, 17, f64::from(link), 0.0),
        crate::tree::Gesture::LongPress(id) => (id, 18, 0.0, 0.0),
        crate::tree::Gesture::DoubleTap(id) => (id, 19, 0.0, 0.0),
        crate::tree::Gesture::Zoom { id, scale } => (id, 20, f64::from(scale), 0.0),
        crate::tree::Gesture::ScrollPhaseChanged { id, .. } => (id, 21, 0.0, 0.0),
    }
}

/// `DragEnd` runs before scroll-phase notices so a handler can replace the spring.
///
/// One node keeps only its latest phase from this pointer.
pub(super) fn dispatch_pointer(
    host: &Host,
    gestures: Vec<crate::tree::Gesture>,
) -> Result<(), String> {
    let inner = host.inner();
    let notices = inner.tree.borrow_mut().take_scroll_notices();
    let (lead, early) = split_phases(gestures);
    dispatch_gestures(host, lead)?;
    let mut queued = inner.tree.borrow_mut().take_scroll_notices();
    queued.splice(0..0, notices);
    let (more, late) = split_phases(queued);
    dispatch_gestures(host, more)?;
    let mut phases = early;
    phases.extend(late);
    dispatch_gestures(host, latest_phases(phases))
}

fn split_phases(
    gestures: Vec<crate::tree::Gesture>,
) -> (Vec<crate::tree::Gesture>, Vec<crate::tree::Gesture>) {
    let mut lead = Vec::new();
    let mut phases = Vec::new();
    for gesture in gestures {
        if matches!(gesture, crate::tree::Gesture::ScrollPhaseChanged { .. }) {
            phases.push(gesture);
        } else {
            lead.push(gesture);
        }
    }
    (lead, phases)
}

fn latest_phases(phases: Vec<crate::tree::Gesture>) -> Vec<crate::tree::Gesture> {
    let mut kept = Vec::new();
    for phase in phases {
        let crate::tree::Gesture::ScrollPhaseChanged { id, .. } = phase else {
            continue;
        };
        if let Some(slot) = kept.iter_mut().find(|item| {
            matches!(item, crate::tree::Gesture::ScrollPhaseChanged { id: kept_id, .. } if *kept_id == id)
        }) {
            *slot = phase;
        } else {
            kept.push(phase);
        }
    }
    kept
}

pub(in crate::js::player) fn dispatch_gestures(
    host: &Host,
    gestures: Vec<crate::tree::Gesture>,
) -> Result<(), String> {
    match dispatch_gesture_list(host, gestures) {
        Ok(()) => Ok(()),
        Err(message) => {
            tracing::error!(target: "impasto::gesture", "{message}");
            Err(message)
        }
    }
}

fn dispatch_gesture_list(host: &Host, gestures: Vec<crate::tree::Gesture>) -> Result<(), String> {
    let inner = host.inner();
    for gesture in gestures {
        note_gesture(&inner.marks, &gesture);
        match gesture {
            crate::tree::Gesture::TextReplaced(id) => {
                let snapshot = inner.tree.borrow().editor_snapshot(id);
                if let Some((_, value)) = snapshot {
                    let unit = |position| i32::try_from(position).unwrap_or(i32::MAX);
                    let (from, to) = value
                        .composing
                        .map_or((-1, -1), |range| (unit(range.start), unit(range.end)));
                    dispatch_input(
                        host,
                        id,
                        16,
                        Some(&value.text),
                        Some((
                            unit(value.selection.base),
                            unit(value.selection.extent),
                            from,
                            to,
                        )),
                    )?;
                }
                continue;
            }
            crate::tree::Gesture::ActivateLink { id, link } => {
                dispatch_kind(host, id, 17, f64::from(link), 0.0)?;
                continue;
            }
            crate::tree::Gesture::DragBegin(id) => {
                tracing::debug!(target: "impasto::gesture", kind = "drag-begin", "gesture");
                dispatch_kind(host, id, 3, 0.0, 0.0)?;
                continue;
            }
            crate::tree::Gesture::DragEnd(id, vx, vy) => {
                tracing::debug!(target: "impasto::gesture", kind = "drag-end", "gesture");
                dispatch_kind(host, id, 5, f64::from(vx), f64::from(vy))?;
                continue;
            }
            crate::tree::Gesture::ScrollPhaseChanged {
                id,
                phase,
                vx,
                vy,
                ox,
                oy,
            } => {
                tracing::debug!(
                    target: "impasto::gesture",
                    phase = phase.name(),
                    "scroll phase"
                );
                host.scroll_state(id.to_raw(), phase, (vx, vy), (ox, oy))?;
                continue;
            }
            other => {
                let (id, kind, a, b) = gesture_parts(other);
                dispatch_kind(host, id, kind, a, b)?;
            }
        }
    }
    Ok(())
}

fn dispatch_kind(host: &Host, id: NodeId, kind: i32, a: f64, b: f64) -> Result<(), String> {
    host.dispatch(id.to_raw(), kind, a, b)
}

pub(super) fn dispatch_text(host: &Host, id: NodeId, text: &str) -> Result<(), String> {
    host.dispatch_input(id.to_raw(), 16, Some(text), None)
}

pub(super) fn dispatch_input(
    host: &Host,
    id: NodeId,
    kind: i32,
    text: Option<&str>,
    span: Option<(i32, i32, i32, i32)>,
) -> Result<(), String> {
    host.dispatch_input(id.to_raw(), kind, text, span)
}

fn named_edit(name: &str, extra: String, record_undo: bool) -> Option<crate::text::Edit> {
    let unit = |text: &str| text.parse::<u32>().unwrap_or(0);
    Some(match name {
        "insert" => crate::text::Edit::Insert(extra),
        "backspace" => crate::text::Edit::Backspace,
        "forwardDelete" => crate::text::Edit::ForwardDelete,
        "newline" => crate::text::Edit::Newline,
        "undo" => crate::text::Edit::Undo,
        "redo" => crate::text::Edit::Redo,
        "selectAll" => crate::text::Edit::SelectAll,
        "caret" => crate::text::Edit::SetCaret(unit(&extra)),
        "extendCaret" => crate::text::Edit::ExtendCaret(unit(&extra)),
        "composing" => {
            crate::text::Edit::Composing(if extra.is_empty() { None } else { Some(extra) })
        }
        "commit" => crate::text::Edit::Commit(extra),
        "load" => crate::text::Edit::Load(extra),
        "replace" => crate::text::Edit::Replace {
            text: extra,
            record_undo,
        },
        "left" => crate::text::Edit::Left,
        "right" => crate::text::Edit::Right,
        "extendLeft" => crate::text::Edit::ExtendLeft,
        "extendRight" => crate::text::Edit::ExtendRight,
        "wordLeft" => crate::text::Edit::WordLeft,
        "wordRight" => crate::text::Edit::WordRight,
        "extendWordLeft" => crate::text::Edit::ExtendWordLeft,
        "extendWordRight" => crate::text::Edit::ExtendWordRight,
        "lineStart" => crate::text::Edit::LineStart,
        "lineEnd" => crate::text::Edit::LineEnd,
        "extendLineStart" => crate::text::Edit::ExtendLineStart,
        "extendLineEnd" => crate::text::Edit::ExtendLineEnd,
        "up" => crate::text::Edit::Up,
        "down" => crate::text::Edit::Down,
        "extendUp" => crate::text::Edit::ExtendUp,
        "extendDown" => crate::text::Edit::ExtendDown,
        "documentStart" => crate::text::Edit::DocumentStart,
        "documentEnd" => crate::text::Edit::DocumentEnd,
        "extendDocumentStart" => crate::text::Edit::ExtendDocumentStart,
        "extendDocumentEnd" => crate::text::Edit::ExtendDocumentEnd,
        "deleteWord" => crate::text::Edit::DeleteWordBackward,
        "deleteWordForward" => crate::text::Edit::DeleteWordForward,
        "deleteToLineStart" => crate::text::Edit::DeleteToLineStart,
        "deleteToLineEnd" => crate::text::Edit::DeleteToLineEnd,
        "copy" => crate::text::Edit::Copy,
        "cut" => crate::text::Edit::Cut,
        "paste" => crate::text::Edit::Paste(extra),
        _ => return None,
    })
}

pub(super) fn text_command(
    inner: &Inner,
    node: NodeId,
    name: &str,
    extra: String,
    record_undo: bool,
) -> Option<crate::text::TextEditingValue> {
    let edit = named_edit(name, extra, record_undo)?;
    inner.tree.borrow_mut().text_edit(node, edit).ok()
}
