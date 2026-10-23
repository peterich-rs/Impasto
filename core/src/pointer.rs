/// A pointer sample in scene points, origin at the top left.
///
/// `time_ms` is the host event time in milliseconds. Drag velocity is the
/// difference of two samples on this clock. The epoch does not matter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub phase: Phase,
    pub x: f32,
    pub y: f32,
    pub time_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}
