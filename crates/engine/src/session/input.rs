use crate::Rect;
use crate::input::Modifiers;
use crate::text::Edit;

/// App visibility. The host delivers these on the render thread.
///
/// `Inactive` does not cancel an open press. `Hidden` and `Paused` do.
/// `Window` focus is a separate event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Detached,
    Resumed,
    Inactive,
    Hidden,
    Paused,
}

/// Light or dark system appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brightness {
    Light,
    Dark,
}

/// System settings that can change without a resize.
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformSettings {
    pub locale: String,
    pub text_scale: f32,
    pub brightness: Brightness,
    pub reduce_motion: bool,
    pub bold_text: bool,
    pub high_contrast: bool,
}

impl Default for PlatformSettings {
    fn default() -> Self {
        Self {
            locale: String::new(),
            text_scale: 1.0,
            brightness: Brightness::Light,
            reduce_motion: false,
            bold_text: false,
            high_contrast: false,
        }
    }
}

/// Keys the text field handles itself. Characters and IME commits arrive as
/// [`InputEvent::Text`], not as keys.
///
/// Word, line, and document motions are already resolved from the platform
/// shortcut. Shift is [`KeyPress::shift`], not a separate key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Backspace,
    /// Forward delete.
    Delete,
    Enter,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordLeft,
    WordRight,
    DocumentStart,
    DocumentEnd,
    DeleteWord,
    DeleteWordForward,
    DeleteToLineStart,
    DeleteToLineEnd,
    /// A character key. Posted when modifiers are held, instead of [`InputEvent::Text`].
    Char(char),
}

/// One key, plus whether the selection should extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub shift: bool,
    pub modifiers: Modifiers,
}

impl KeyPress {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            shift: false,
            modifiers: Modifiers::empty(),
        }
    }

    pub const fn extending(key: Key) -> Self {
        Self {
            key,
            shift: true,
            modifiers: Modifiers::empty(),
        }
    }

    pub const fn with_modifiers(key: Key, modifiers: Modifiers) -> Self {
        Self {
            key,
            shift: modifiers.contains(Modifiers::SHIFT),
            modifiers,
        }
    }
}

/// What the return key does in the focused field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextInputAction {
    #[default]
    None,
    Done,
    Next,
    Search,
    Go,
    Send,
    Previous,
    /// Insert a line break. Multiline fields use this.
    Newline,
}

/// Which software keyboard the platform should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum KeyboardType {
    #[default]
    Text,
    Multiline,
    Number,
    Phone,
    Email,
    Url,
    Name,
    Password,
}

/// Full editing state for the platform keyboard.
///
/// `selection` and `composing` are UTF-16 code units. `caret` is in scene points.
#[derive(Clone, Debug, PartialEq)]
pub struct ImeState {
    /// Identity of one focus connection and the latest accepted platform edit.
    pub client: u64,
    pub ack: u64,
    pub node: Option<u64>,
    pub autofill: Option<&'static str>,
    pub text: String,
    pub selection: std::ops::Range<u32>,
    pub composing: Option<std::ops::Range<u32>>,
    pub caret: Rect,
    pub action: TextInputAction,
    pub keyboard: KeyboardType,
}

impl ImeState {
    pub fn at_end(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = utf16_len(&text);
        Self {
            client: 0,
            ack: 0,
            node: None,
            autofill: None,
            text,
            selection: end..end,
            composing: None,
            caret: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 0.0,
            },
            action: TextInputAction::Done,
            keyboard: KeyboardType::Text,
        }
    }
}

/// UTF-16 length of `text`. A scalar above U+FFFF counts as two units.
pub fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// One change of keyboard visibility or editing state.
#[derive(Clone, Debug, PartialEq)]
pub enum ImeUpdate {
    Show(ImeState),
    Update(ImeState),
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackGesture {
    Started { from_right: bool },
    Progress(f32),
    Cancelled,
    Committed,
}

/// One edit delivered to the focused control.
///
/// `Text` is committed characters. `Composing(None)` ends an IME composition.
/// The session routes the event; it does not edit a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyPress),
    Text(String),
    Composing(Option<String>),
    /// The system back key, after the keyboard has been dismissed.
    Back,
    Copy,
    Cut,
    /// Clipboard payload read on the platform thread. The editor consumes `plain`.
    Paste {
        plain: String,
        html: Option<String>,
    },
    SelectAll,
    /// A committed IME diff. The session applies it on the focused editor.
    Edit(Edit),
    /// The platform input connection's buffer, selection, and composing span.
    ///
    /// Indices are UTF-16. A composing start below zero means there is no span.
    /// The focused field replaces its string with `text` instead of replaying a diff.
    Replace {
        text: String,
        start: u32,
        end: u32,
        compose_start: i32,
        compose_end: i32,
    },
    ReplaceIme {
        client: u64,
        sequence: u64,
        text: String,
        start: u32,
        end: u32,
        compose_start: i32,
        compose_end: i32,
    },
    DismissIme(u64),
}
