//! Developer diagnostics: ring buffers plus the devtools host module.

mod hub;
mod inspect;
mod module;
mod recorder;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
pub(crate) mod remote;

pub(crate) use hub::CaptureFrame;
pub use hub::{
    DiagnosticsHub, FrameSample, LogEntry, LogSource, NetEntry, NetKind, PREVIEW_CAP, PhaseSample,
};
pub use inspect::write_frame;
pub use module::{DevtoolsContext, module};
pub use recorder::wrap_if_devtools;

mod pftrace;
pub(crate) mod system_trace;
pub mod telemetry;
pub(crate) mod timeline;
mod trace_export;
pub use hub::TelemetryHub;
