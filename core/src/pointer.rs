/// A pointer sample in scene points, origin at the top left.
///
/// `time_ms` is the host event time in milliseconds. Drag velocity is the
/// difference of two samples on this clock. The epoch does not matter.
/// `dx` and `dy` are wheel deltas in scene points. Other phases leave them at zero.
/// Scene Y grows downward, so a wheel that moves content downward is a positive `dy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub phase: Phase,
    pub x: f32,
    pub y: f32,
    pub time_ms: f64,
    pub button: Button,
    pub modifiers: Modifiers,
    pub dx: f32,
    pub dy: f32,
}

impl PointerEvent {
    pub fn new(phase: Phase, x: f32, y: f32, time_ms: f64) -> Self {
        Self {
            phase,
            x,
            y,
            time_ms,
            button: Button::Primary,
            modifiers: Modifiers::empty(),
            dx: 0.0,
            dy: 0.0,
        }
    }

    pub fn wheel(x: f32, y: f32, time_ms: f64, dx: f32, dy: f32) -> Self {
        Self {
            phase: Phase::Wheel,
            x,
            y,
            time_ms,
            button: Button::None,
            modifiers: Modifiers::empty(),
            dx,
            dy,
        }
    }
}

/// Which button produced the sample. [`Button::None`] is a move with no button held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Button {
    #[default]
    None,
    Primary,
    Secondary,
    Auxiliary,
}

/// Modifier keys held with the sample. Bits are independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    bits: u8,
}

impl Modifiers {
    pub const SHIFT: u8 = 1;
    pub const CONTROL: u8 = 2;
    pub const ALT: u8 = 4;
    pub const META: u8 = 8;

    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self { bits }
    }

    pub const fn bits(self) -> u8 {
        self.bits
    }

    pub const fn contains(self, bit: u8) -> bool {
        self.bits & bit == bit
    }

    pub const fn with(self, bit: u8) -> Self {
        Self {
            bits: self.bits | bit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
    Wheel,
}
