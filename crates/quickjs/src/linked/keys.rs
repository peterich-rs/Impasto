//! Key names passed to JS shortcut and input callbacks.

use session::Key;
use uicore::Modifiers;

pub(super) fn shortcut_spec(press: &session::KeyPress) -> String {
    let mut parts = Vec::new();
    if press.modifiers.contains(Modifiers::META) || press.modifiers.contains(Modifiers::CONTROL) {
        parts.push("mod".to_string());
    }
    if press.modifiers.contains(Modifiers::ALT) {
        parts.push("alt".to_string());
    }
    if press.modifiers.contains(Modifiers::SHIFT) || press.shift {
        parts.push("shift".to_string());
    }
    let key = match press.key {
        Key::Char(ch) => ch.to_lowercase().to_string(),
        other => key_name(session::KeyPress {
            key: other,
            shift: false,
            modifiers: Modifiers::empty(),
        })
        .to_lowercase(),
    };
    parts.push(key);
    parts.join("+")
}

pub(super) fn key_name(press: session::KeyPress) -> String {
    let name = match press.key {
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Left => "Left",
        Key::Right => "Right",
        Key::Up => "Up",
        Key::Down => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::WordLeft => "WordLeft",
        Key::WordRight => "WordRight",
        Key::DocumentStart => "DocumentStart",
        Key::DocumentEnd => "DocumentEnd",
        Key::DeleteWord => "DeleteWord",
        Key::DeleteWordForward => "DeleteWordForward",
        Key::DeleteToLineStart => "DeleteToLineStart",
        Key::DeleteToLineEnd => "DeleteToLineEnd",
        Key::Char(ch) => return ch.to_string(),
    };
    if press.shift {
        format!("Shift+{name}")
    } else {
        name.to_owned()
    }
}
