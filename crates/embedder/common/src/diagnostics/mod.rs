//! Developer diagnostics and telemetry ring buffers.

mod hub;
mod inspect;
mod plugin_record;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
pub(crate) mod remote;
pub(crate) mod snapshot;

pub(crate) use hub::CaptureFrame;
pub use hub::{DiagnosticsHub, FrameSample, LogEntry, LogSource, NetEntry, NetKind, PhaseSample};
pub use inspect::write_frame;

mod pftrace;
pub(crate) mod system_trace;
pub mod telemetry;
pub(crate) mod timeline;
mod trace_export;
pub use hub::TelemetryHub;
