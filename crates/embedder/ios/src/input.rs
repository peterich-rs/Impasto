use std::ffi::c_char;

use embedder::{Command, ProjectedNode};
use engine::session::{
    BackGesture, ImeUpdate, InputEvent, Key, KeyPress, KeyboardType, TextInputAction,
};
use engine::{
    input::{
        Button, Buttons, Modifiers, PenSample, Phase, PointerEvent, PointerHistory, PointerId,
        PointerKind, PointerSample,
    },
    semantics::{SemanticsAction, SemanticsId},
    tree::Rect,
};
use serde_json::{Value, json};

use super::{ImpastoIOSHost, string, with_host};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImpastoIOSPointerPoint {
    x: f32,
    y: f32,
    seconds: f64,
}

/// # Safety
/// A non-null history must contain `length` initialized points for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_pointer_move(
    host: *mut ImpastoIOSHost,
    surface: u64,
    pointer: u64,
    kind: u32,
    latest: ImpastoIOSPointerPoint,
    buttons: u32,
    modifiers: u32,
    force: f32,
    radius: f32,
    altitude: f32,
    azimuth: f32,
    history: *const ImpastoIOSPointerPoint,
    length: usize,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) {
            return;
        }
        let Some(kind) = [
            PointerKind::Touch,
            PointerKind::Mouse,
            PointerKind::Pen,
            PointerKind::Trackpad,
        ]
        .get(kind as usize) else {
            return;
        };
        let Some(latest) = pointer_point(latest) else {
            return;
        };
        let pen = (*kind == PointerKind::Pen
            && [force, radius, altitude, azimuth]
                .into_iter()
                .all(f32::is_finite))
        .then_some(PenSample {
            force: force.clamp(0.0, 1.0),
            radius: radius.max(0.0),
            altitude,
            azimuth,
        });
        let event = PointerEvent {
            pointer: PointerId(pointer),
            kind: *kind,
            button: Button::None,
            buttons: Buttons::from_bits(u8::try_from(buttons & 7).unwrap_or_default()),
            modifiers: Modifiers::from_bits(u8::try_from(modifiers & 15).unwrap_or_default()),
            pen,
            ..PointerEvent::new(Phase::Move, latest.x, latest.y, latest.time_ms)
        };
        if history.is_null() || !(1..=PointerHistory::CAPACITY).contains(&length) {
            host.link.post(Command::Pointer(event));
            return;
        }
        // SAFETY: Swift borrows the contiguous point array for the whole call.
        let positions = unsafe { std::slice::from_raw_parts(history, length) };
        let mut history = Box::new(PointerHistory::default());
        for &point in positions {
            if let Some(sample) = pointer_point(point)
                && sample.time_ms < latest.time_ms
            {
                history.push(sample);
            }
        }
        host.link.post(Command::PointerMoves { event, history });
    });
}

fn pointer_point(point: ImpastoIOSPointerPoint) -> Option<PointerSample> {
    let time_ms = point.seconds * 1_000.0;
    (point.x.is_finite() && point.y.is_finite() && time_ms.is_finite()).then_some(PointerSample {
        x: point.x,
        y: point.y,
        time_ms,
    })
}

pub(super) fn ime_message(update: &ImeUpdate) -> String {
    let state = match update {
        ImeUpdate::Show(state) | ImeUpdate::Update(state) => state,
        ImeUpdate::Hide => return "{\"shown\":false}".to_owned(),
    };
    let action = match state.action {
        TextInputAction::None => "none",
        TextInputAction::Done => "done",
        TextInputAction::Next => "next",
        TextInputAction::Search => "search",
        TextInputAction::Go => "go",
        TextInputAction::Send => "send",
        TextInputAction::Previous => "previous",
        TextInputAction::Newline => "newline",
    };
    let keyboard = match state.keyboard {
        KeyboardType::Text => "text",
        KeyboardType::Multiline => "multiline",
        KeyboardType::Number => "number",
        KeyboardType::Phone => "phone",
        KeyboardType::Email => "email",
        KeyboardType::Url => "url",
        KeyboardType::Name => "name",
        KeyboardType::Password => "password",
    };
    let (compose_start, compose_end) = state.composing.as_ref().map_or((-1, -1), |r| {
        (
            i32::try_from(r.start).unwrap_or(-1),
            i32::try_from(r.end).unwrap_or(-1),
        )
    });
    json!({
        "shown": true, "client": state.client, "ack": state.ack, "node": state.node,
        "text": state.text, "start": state.selection.start, "end": state.selection.end,
        "composeStart": compose_start, "composeEnd": compose_end, "action": action,
        "keyboard": keyboard, "autofill": state.autofill,
        "x": state.caret.x, "y": state.caret.y, "w": state.caret.w, "h": state.caret.h,
    })
    .to_string()
}

pub(super) fn semantics_message(nodes: &[ProjectedNode]) -> String {
    Value::Array(nodes.iter().filter(|node| {
        [node.rect.x, node.rect.y, node.rect.w, node.rect.h].iter().all(|v| v.is_finite())
    }).map(|node| {
        json!({
            "id": node.id.0, "parent": node.parent.map(|value| value.0),
            "x": node.rect.x, "y": node.rect.y, "w": node.rect.w, "h": node.rect.h,
            "label": node.label, "value": node.value, "help": node.help,
            "role": format!("{:?}", node.role), "actions": node.actions.0, "flags": node.flags.0,
            "selectionStart": node.selection.map(|selection| selection.0),
            "selectionEnd": node.selection.map(|selection| selection.1),
            "scrollX": node.scroll_offset.0, "scrollY": node.scroll_offset.1,
            "contentWidth": node.content.0, "contentHeight": node.content.1,
            "autofill": node.autofill.as_ref().map(|field| field.hint),
            "live": format!("{:?}", node.live),
            "platformView": node.flags.contains(engine::semantics::SemanticsFlags::PLATFORM_VIEW),
        })
    }).collect()).to_string()
}

pub(super) fn platform_views_value(views: &[engine::tree::PlatformViewPlacement]) -> Value {
    let rect = |rect: Rect| json!({"x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h});
    Value::Array(
        views
            .iter()
            .map(|view| {
                json!({
                    "id": view.node.to_raw(), "kind": view.kind.as_ref(),
                    "rect": rect(view.rect), "clip": rect(view.clip),
                    "geometry": {
                        "width": view.geometry.width, "height": view.geometry.height,
                        "transform": view.geometry.transform, "opacity": view.geometry.opacity,
                        "clips": view.geometry.clips.iter().map(|clip| json!({
                            "width": clip.width, "height": clip.height,
                            "radius": clip.radius, "transform": clip.transform,
                            "path": clip.path.as_ref().map(|path| path.iter().map(path_command).collect::<Vec<_>>()),
                        })).collect::<Vec<_>>()
                    },
                    "occluders": view.occluders.iter().copied().map(rect).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn path_command(verb: &engine::scene::PathVerb) -> Value {
    use engine::scene::PathVerb;
    match *verb {
        PathVerb::Move(x, y) => json!([0, x, y]),
        PathVerb::Line(x, y) => json!([1, x, y]),
        PathVerb::Quad(ax, ay, bx, by) => json!([2, ax, ay, bx, by]),
        PathVerb::Cubic(ax, ay, bx, by, cx, cy) => json!([3, ax, ay, bx, by, cx, cy]),
        PathVerb::Close => json!([4]),
    }
}

pub(super) unsafe fn copy_text(text: *const u8, length: usize) -> Option<String> {
    if length == 0 {
        return Some(String::new());
    }
    if text.is_null() || length > 16 * 1024 * 1024 {
        return None;
    }
    // SAFETY: the foreign input stays readable for the whole C call.
    let bytes = unsafe { std::slice::from_raw_parts(text, length) };
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

fn utf16_boundary(text: &str, offset: u32) -> bool {
    let mut remaining = offset;
    for ch in text.chars() {
        if remaining == 0 {
            return true;
        }
        let width = if ch.len_utf16() == 2 { 2 } else { 1 };
        let Some(next) = remaining.checked_sub(width) else {
            return false;
        };
        remaining = next;
    }
    remaining == 0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_ime_replace(
    host: *mut ImpastoIOSHost,
    surface: u64,
    client: u64,
    sequence: u64,
    text: *const u8,
    length: usize,
    start: u32,
    end: u32,
    compose_start: i32,
    compose_end: i32,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) || client == 0 || sequence == 0 {
            return;
        }
        // SAFETY: the caller supplies the string's byte buffer for this C call.
        let Some(text) = (unsafe { copy_text(text, length) }) else {
            return;
        };
        let compose_valid = match (u32::try_from(compose_start), u32::try_from(compose_end)) {
            (Ok(from), Ok(to)) => {
                from <= to && utf16_boundary(&text, from) && utf16_boundary(&text, to)
            }
            (Err(_), Err(_)) => compose_start == -1 && compose_end == -1,
            _ => false,
        };
        if start <= end
            && utf16_boundary(&text, start)
            && utf16_boundary(&text, end)
            && compose_valid
        {
            host.link.post(Command::Input(InputEvent::ReplaceIme {
                client,
                sequence,
                text,
                start,
                end,
                compose_start,
                compose_end,
            }));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_ime_dismiss(
    host: *mut ImpastoIOSHost,
    surface: u64,
    client: u64,
) {
    with_host(host, |host| {
        if host.current_surface(surface) {
            host.link
                .post(Command::Input(InputEvent::DismissIme(client)));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_keyboard_inset(
    host: *mut ImpastoIOSHost,
    surface: u64,
    bottom: f32,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) || !bottom.is_finite() {
            return;
        }
        let previous = *host.viewport.borrow();
        if let Some(mut metrics) = previous {
            metrics.view_insets.bottom = bottom.clamp(0.0, metrics.height);
            metrics.inset_bottom = metrics.padding.bottom.max(metrics.view_insets.bottom);
            host.update_viewport(surface, metrics);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_keyboard_occlusion(
    host: *mut ImpastoIOSHost,
    surface: u64,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) || [x, y, w, h].iter().any(|v| !v.is_finite()) {
            return;
        }
        let previous = *host.viewport.borrow();
        if let Some(mut metrics) = previous {
            metrics.keyboard_occlusion = (w > 0.0 && h > 0.0).then_some(Rect { x, y, w, h });
            host.update_viewport(surface, metrics);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_key(
    host: *mut ImpastoIOSHost,
    surface: u64,
    key: *const c_char,
    modifiers: u32,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) {
            return;
        }
        // SAFETY: key is borrowed for this call.
        let Some(key) = (unsafe { string(key) }) else {
            return;
        };
        let modifiers = Modifiers::from_bits(u8::try_from(modifiers & 15).unwrap_or_default());
        let key = match key.as_str() {
            "Copy" => InputEvent::Copy,
            "Cut" => InputEvent::Cut,
            "SelectAll" => InputEvent::SelectAll,
            "Undo" => InputEvent::Edit(engine::text::Edit::Undo),
            "Redo" => InputEvent::Edit(engine::text::Edit::Redo),
            _ => {
                let Some(key) = named_key(&key) else { return };
                InputEvent::Key(KeyPress::with_modifiers(key, modifiers))
            }
        };
        host.link.post(Command::Input(key));
    });
}

fn named_key(key: &str) -> Option<Key> {
    Some(match key {
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Escape" => Key::Escape,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "WordLeft" => Key::WordLeft,
        "WordRight" => Key::WordRight,
        "DocumentStart" => Key::DocumentStart,
        "DocumentEnd" => Key::DocumentEnd,
        name => {
            let mut chars = name.chars();
            let ch = chars.next()?;
            if chars.next().is_some() || ch.is_control() {
                return None;
            }
            Key::Char(ch)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_semantics_enabled(
    host: *mut ImpastoIOSHost,
    enabled: bool,
    autofill: bool,
) {
    with_host(host, |host| {
        host.link.post(Command::SemanticsEnabled(enabled));
        host.link.post(Command::AutofillEnabled(autofill));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_semantics_action(
    host: *mut ImpastoIOSHost,
    surface: u64,
    id: u64,
    action: u32,
    dx: f32,
    dy: f32,
    start: u32,
    end: u32,
    text: *const u8,
    length: usize,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) {
            return;
        }
        let id = SemanticsId(id);
        let action = match action {
            0 => SemanticsAction::Activate(id),
            1 if dx.is_finite() && dy.is_finite() => SemanticsAction::Scroll { id, dx, dy },
            2 | 4 => {
                // SAFETY: the caller keeps the UTF-8 buffer alive for this call.
                let Some(text) = (unsafe { copy_text(text, length) }) else {
                    return;
                };
                if action == 2 {
                    SemanticsAction::SetText { id, text }
                } else {
                    SemanticsAction::Autofill { id, text }
                }
            }
            3 => SemanticsAction::SetSelection { id, start, end },
            5 => SemanticsAction::Focus(id),
            _ => return,
        };
        host.link.post(Command::SemanticsAction(action));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_back(
    host: *mut ImpastoIOSHost,
    surface: u64,
    phase: u32,
    progress: f32,
    from_right: bool,
) {
    with_host(host, |host| {
        if !host.current_surface(surface) {
            return;
        }
        let gesture = match phase {
            0 => BackGesture::Started { from_right },
            1 if progress.is_finite() => BackGesture::Progress(progress.clamp(0.0, 1.0)),
            2 => BackGesture::Cancelled,
            3 => BackGesture::Committed,
            _ => return,
        };
        host.link.post(Command::BackGesture(gesture));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_pen(
    host: *mut ImpastoIOSHost,
    surface: u64,
    pointer: u64,
    phase: u32,
    x: f32,
    y: f32,
    seconds: f64,
    force: f32,
    radius: f32,
    altitude: f32,
    azimuth: f32,
    modifiers: u32,
) {
    with_host(host, |host| {
        if !host.current_surface(surface)
            || !seconds.is_finite()
            || [x, y, force, radius, altitude, azimuth]
                .iter()
                .any(|v| !v.is_finite())
        {
            return;
        }
        let Some(phase) = [Phase::Down, Phase::Move, Phase::Up, Phase::Cancel].get(phase as usize)
        else {
            return;
        };
        let held = matches!(phase, Phase::Down | Phase::Move) && force > 0.0;
        let mut event = PointerEvent::new(*phase, x, y, seconds * 1_000.0);
        event.pointer = PointerId(pointer);
        event.kind = PointerKind::Pen;
        event.button = if *phase == Phase::Move {
            Button::None
        } else {
            Button::Primary
        };
        event.buttons = Buttons::from_bits(if held { Buttons::PRIMARY } else { 0 });
        event.modifiers = Modifiers::from_bits(u8::try_from(modifiers & 15).unwrap_or_default());
        event.pen = Some(PenSample {
            force: force.clamp(0.0, 1.0),
            radius: radius.max(0.0),
            altitude,
            azimuth,
        });
        host.link.post(Command::Pointer(event));
    });
}
