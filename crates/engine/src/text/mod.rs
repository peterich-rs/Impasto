//! Text shaping and editing.

mod editor;
pub(crate) mod shaper;

pub use editor::*;
pub use shaper::*;

/// Maximum size of one application-registered font resource.
pub const MAX_APP_FONT_BYTES: usize = 4 * 1024 * 1024;
