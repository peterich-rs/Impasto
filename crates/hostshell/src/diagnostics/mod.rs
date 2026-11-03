//! Developer diagnostics: ring buffers plus the devtools host module.

mod hub;
mod inspect;
mod module;
mod recorder;

pub use hub::{DiagnosticsHub, FrameSample, LogSource, NetEntry, NetKind, PhaseSample};
pub use inspect::write_frame;
pub use module::{DevtoolsContext, module};
pub use recorder::wrap_if_devtools;
