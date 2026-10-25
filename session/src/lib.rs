//! One frame of a language runtime. The host calls this and never sees application state.

use protocol::DrawList;

mod io;

pub use io::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpRequest, HttpResponse, IoJob, PluginResult,
    PluginValue,
};

/// One edge of a window, in scene points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Window metrics in scene points.
///
/// `padding` is the always-on safe area. `view_insets` is the keyboard and other
/// transient overlap. `inset_*` is the per-edge maximum of those two, which is
/// what [`crate`] callers already subtract from the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    pub device_pixel_ratio: f32,
    pub inset_left: f32,
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub padding: Insets,
    pub view_insets: Insets,
}

impl Metrics {
    pub fn new(width: f32, height: f32, device_pixel_ratio: f32) -> Self {
        Self::from_insets(
            width,
            height,
            device_pixel_ratio,
            Insets::default(),
            Insets::default(),
        )
    }

    /// `inset_*` becomes the per-edge max so the viewport stays the same size
    /// as the single inset rect hosts used to send.
    pub fn from_insets(
        width: f32,
        height: f32,
        device_pixel_ratio: f32,
        padding: Insets,
        view_insets: Insets,
    ) -> Self {
        Self {
            width,
            height,
            device_pixel_ratio,
            inset_left: padding.left.max(view_insets.left),
            inset_top: padding.top.max(view_insets.top),
            inset_right: padding.right.max(view_insets.right),
            inset_bottom: padding.bottom.max(view_insets.bottom),
            padding,
            view_insets,
        }
    }
}

/// One vsync. Fields are monotonic milliseconds on that platform's clock.
/// Animation samples [`FrameTime::target_ms`].
///
/// [`FrameTime::mono_ms`] is a separate monotonic clock owned by the shell.
/// Timers and `performance.now` sample it. It is not the vsync origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    pub vsync_ms: f64,
    pub target_ms: f64,
    pub interval_ms: f64,
    pub mono_ms: f64,
}

impl FrameTime {
    pub fn at(target_ms: f64) -> Self {
        Self {
            vsync_ms: target_ms,
            target_ms,
            interval_ms: 0.0,
            mono_ms: target_ms,
        }
    }
}

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
}

/// One key, plus whether the selection should extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub shift: bool,
}

impl KeyPress {
    pub const fn new(key: Key) -> Self {
        Self { key, shift: false }
    }

    pub const fn extending(key: Key) -> Self {
        Self { key, shift: true }
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
    pub text: String,
    pub selection: std::ops::Range<u32>,
    pub composing: Option<std::ops::Range<u32>>,
    pub caret: core::Rect,
    pub action: TextInputAction,
    pub keyboard: KeyboardType,
}

impl ImeState {
    pub fn at_end(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = utf16_len(&text);
        Self {
            text,
            selection: end..end,
            composing: None,
            caret: core::Rect {
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
    text.encode_utf16().count() as u32
}

/// One change of keyboard visibility or editing state.
#[derive(Clone, Debug, PartialEq)]
pub enum ImeUpdate {
    Show(ImeState),
    Update(ImeState),
    Hide,
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
    /// Clipboard text read on the platform thread.
    Paste(String),
    SelectAll,
}

/// A language runtime bound to one [`core::Tree`].
///
/// `frame` runs in this order: input has already been delivered through
/// [`Session::pointer`] and [`Session::input`], due timers, animation callbacks,
/// flush handlers, layout, record.
pub trait Session {
    fn metrics(&mut self, metrics: &Metrics);
    fn pointer(&mut self, event: core::PointerEvent);
    fn frame(&mut self, time: FrameTime) -> DrawList;
    /// Semantics produced with this frame. `None` means the tree did not change.
    fn semantics(&mut self) -> Option<core::SemanticsUpdate> {
        None
    }
    /// True after the session asked for another frame, until the host takes it.
    ///
    /// Animation asks again inside each `frame`. One request schedules one frame.
    fn wants_frame(&mut self) -> bool {
        false
    }
    fn lifecycle(&mut self, _event: Lifecycle) {}
    fn settings(&mut self, _settings: &PlatformSettings) {}
    /// Live image nodes, including ones a clip kept out of the draw list.
    ///
    /// `None` means this session does not track images, so the host must not
    /// cancel a decode because a count dropped to zero.
    fn image_source_counts(
        &mut self,
    ) -> Option<std::collections::HashMap<std::sync::Arc<str>, u32>> {
        None
    }
    /// The window became or resigned key. This is not text focus.
    fn window_focus(&mut self, _focused: bool) {}
    /// Keyboard or IME input for the focused node. No focus means the session
    /// drops the event.
    fn input(&mut self, _event: InputEvent) {}
    /// Keyboard visibility or a new editing state. `None` means nothing changed.
    fn take_ime(&mut self) -> Option<ImeUpdate> {
        None
    }
    /// The user hid the keyboard. The session blurs.
    fn ime_dismissed(&mut self) {}
    /// The session asked the process to leave. A back key with no handler sets this.
    fn take_exit(&mut self) -> bool {
        false
    }
    /// Text a Copy or Cut put aside for the platform clipboard. `None` means nothing new.
    fn take_clipboard(&mut self) -> Option<String> {
        None
    }
    /// An accessibility action. The default ignores it.
    fn semantics_action(&mut self, _action: core::SemanticsAction) {}
    /// Font faces queued by the session since the last take.
    ///
    /// The host registers them on the render thread before `submit`. The default
    /// is empty, so a session that does not load faces stays out of the cap.
    fn take_fonts(&mut self) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
    /// Image sources the session asked to fetch again. The host drops those cache
    /// keys on the render thread before it reads the cache. The default is empty.
    fn take_reloads(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Installs the host sink for [`IoJob`]. The default drops every job.
    fn bind_io(&mut self, _submit: Box<dyn Fn(IoJob) + Send>) {}
    /// Resolves one promise allocated for [`IoJob`]. Called on the render thread.
    fn plugin_result(&mut self, _token: u64, _result: PluginResult) {}
    /// Resolves one `fetch` promise. Called on the render thread.
    fn http_result(&mut self, _token: u64, _result: Result<HttpResponse, HttpError>) {}
    /// Delivers one socket event. Called on the render thread.
    fn socket_event(&mut self, _id: u64, _event: &str, _text: bool, _data: &[u8]) {}
    /// Absolute monotonic milliseconds of the next timer, if one is armed.
    fn next_wake_ms(&mut self) -> Option<f64> {
        None
    }
    /// The URL that opened this process, if the host has one.
    fn set_launch_url(&mut self, _url: &str) {}
}
