//! One frame of a language runtime. The host calls this and never sees application state.

use std::sync::Arc;

mod io;
mod modules;

pub use io::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    IoJob, PluginResult, PluginValue, document_relative, module_not_linked,
};
pub use modules::{
    HostConfig, HostMethod, HostModule, ImmediateExec, Invoke, MethodExec, ModuleRegistry,
    PromiseFn, Settle, Shared, SyncFn, Work, shared, shared_read, shared_write,
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
    /// A character key. Posted when modifiers are held, instead of [`InputEvent::Text`].
    Char(char),
}

/// One key, plus whether the selection should extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub shift: bool,
    pub modifiers: uicore::Modifiers,
}

impl KeyPress {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            shift: false,
            modifiers: uicore::Modifiers::empty(),
        }
    }

    pub const fn extending(key: Key) -> Self {
        Self {
            key,
            shift: true,
            modifiers: uicore::Modifiers::empty(),
        }
    }

    pub const fn with_modifiers(key: Key, modifiers: uicore::Modifiers) -> Self {
        Self {
            key,
            shift: modifiers.contains(uicore::Modifiers::SHIFT),
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
    pub text: String,
    pub selection: std::ops::Range<u32>,
    pub composing: Option<std::ops::Range<u32>>,
    pub caret: uicore::Rect,
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
            caret: uicore::Rect {
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
    Edit(uicore::Edit),
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
}

/// One IO completion delivered on the render thread.
pub enum IoEvent {
    Http {
        token: u64,
        result: Result<HttpResponse, HttpError>,
    },
    /// Streamed head: status and headers before any body bytes.
    HttpHead {
        token: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// One streamed body piece. The final chunk carries `done: true`; a
    /// failed stream ends with a chunk whose `error` is set.
    HttpChunk {
        token: u64,
        bytes: Vec<u8>,
        done: bool,
        error: Option<HttpError>,
    },
    Plugin {
        token: u64,
        result: PluginResult,
    },
    Socket {
        id: u64,
        event: String,
        text: bool,
        data: Vec<u8>,
    },
}

/// Durations inside one [`Session::frame`]. The host adds gesture time itself.
///
/// `script` splits the script interval. `trace` counts layout work and the
/// input events already handled before `frame` returned.
#[derive(Clone, Copy, Debug, Default)]
pub struct FramePhases {
    pub script_ms: f64,
    pub layout_ms: f64,
    /// Script that ran after the first `layout` and before `record`.
    pub layout_cb_ms: f64,
    pub sync_ms: f64,
    pub record_ms: f64,
    pub semantics_ms: f64,
    pub script: ScriptPhases,
    pub trace: FrameTrace,
}

/// Pieces of [`FramePhases::script_ms`], in call order.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptPhases {
    pub motion_ms: f64,
    pub dispatch_ms: f64,
    pub timer_ms: f64,
    pub raf_ms: f64,
    pub callback_ms: f64,
    pub flush_ms: f64,
}

/// Layout and input counts for one frame. Zeros mean that stage did not run.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTrace {
    /// All `layout` calls this frame, including those made from script.
    pub laywork_ms: f64,
    pub layout_passes: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    /// `none` when nothing marked layout.
    pub layout_cause: &'static str,
    pub pointers: u32,
    pub keys: u32,
    pub edits: u32,
    pub ime: u32,
    pub scrolls: u32,
    pub focus: u32,
    pub blur: u32,
    pub gestures: u32,
}

/// What one `frame` produced. `layers == None` means the host may `PresentCached`.
#[derive(Clone, Debug)]
pub struct FrameOutput {
    /// `Some` when the layer epoch changed. The host submits that tree.
    /// `None` means the host may present the installed tree again.
    pub layers: Option<Arc<uicore::LayerTree>>,
    pub again: bool,
    pub ime: Option<ImeUpdate>,
    pub semantics: Option<uicore::SemanticsPatch>,
    pub fonts: Vec<(String, Vec<u8>)>,
    pub reloads: Vec<String>,
    pub clipboard: Option<String>,
    pub exit: bool,
    /// `None` means the image census generation did not change.
    pub images: Option<Arc<uicore::ImageCensus>>,
    /// `None` when the session did not time the frame.
    pub phases: Option<FramePhases>,
}

impl FrameOutput {
    pub fn unchanged() -> Self {
        Self {
            layers: None,
            again: false,
            ime: None,
            semantics: None,
            fonts: Vec::new(),
            reloads: Vec::new(),
            clipboard: None,
            exit: false,
            images: None,
            phases: None,
        }
    }
}

/// A language runtime bound to one [`uicore::Tree`].
///
/// `frame` runs in this order: input has already been delivered through
/// [`Session::pointer`] and [`Session::input`], due timers, animation callbacks,
/// flush handlers, layout, record.
pub trait Session {
    fn metrics(&mut self, metrics: &Metrics);
    fn pointer(&mut self, event: uicore::PointerEvent);
    fn frame(&mut self, time: FrameTime) -> FrameOutput;
    /// True after the session asked for another frame, until the host takes it.
    ///
    /// Animation asks again inside each `frame`. One request schedules one frame.
    fn wants_frame(&mut self) -> bool {
        false
    }
    fn lifecycle(&mut self, _event: Lifecycle) {}
    fn settings(&mut self, _settings: &PlatformSettings) {}
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
    fn semantics_action(&mut self, _action: uicore::SemanticsAction) {}
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
    /// Resolves one host IO completion. Called on the render thread.
    fn io_event(&mut self, _event: IoEvent) {}
    /// Absolute monotonic milliseconds of the next timer, if one is armed.
    fn next_wake_ms(&mut self) -> Option<f64> {
        None
    }
    /// The URL that opened this process, if the host has one.
    fn set_launch_url(&mut self, _url: &str) {}
}
