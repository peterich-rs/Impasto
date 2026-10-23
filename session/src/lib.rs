//! One frame of a language runtime. The host calls this and never sees application state.

use protocol::DrawList;

/// Window metrics in scene points. Insets are the safe area inside the content view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    pub device_pixel_ratio: f32,
    pub inset_left: f32,
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
}

/// App visibility. The host delivers these on the render thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Pause,
    Resume,
}

/// Keys the text field handles itself. Characters and IME commits arrive as
/// [`InputEvent::Text`], not as keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Enter,
    Tab,
    Left,
    Right,
    Up,
    Down,
}

/// One edit delivered to the focused control.
///
/// `Text` is committed characters. `Composing(None)` ends an IME composition.
/// The session routes the event; it does not edit a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key(Key),
    Text(String),
    Composing(Option<String>),
}

/// A language runtime bound to one [`core::Tree`].
///
/// `frame` runs in this order: input has already been delivered through
/// [`Session::pointer`] and [`Session::input`], animation tick (empty), flush
/// handlers that ran during those calls, layout, record, semantics walk (empty).
pub trait Session {
    fn metrics(&mut self, metrics: &Metrics);
    fn pointer(&mut self, event: core::PointerEvent);
    fn frame(&mut self, time_ms: f64) -> DrawList;
    /// True after the session asked for another frame, until the host takes it.
    ///
    /// Animation asks again inside each `frame`. One request schedules one frame.
    fn wants_frame(&mut self) -> bool {
        false
    }
    fn lifecycle(&mut self, _event: Lifecycle) {}
    /// Keyboard or IME input for the focused node. No focus means the session
    /// drops the event.
    fn input(&mut self, _event: InputEvent) {}
    /// Whether the platform keyboard should be up. Polled after the frame.
    /// Unchanged values are not delivered again.
    fn wants_ime(&self) -> bool {
        false
    }
    /// Committed text to copy into the platform IME buffer. `Some` is taken
    /// once. Composition updates do not enqueue a copy; that would clear the
    /// IME's marked range.
    fn take_ime_text(&mut self) -> Option<String> {
        None
    }
    /// The user hid the keyboard. The session blurs and clears [`Session::wants_ime`].
    fn ime_dismissed(&mut self) {}
}
